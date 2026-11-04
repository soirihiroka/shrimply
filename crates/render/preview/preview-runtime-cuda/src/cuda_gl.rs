use std::ffi::CStr;
use std::ptr;
use std::sync::Arc;

use libc::size_t;
use shrimply_gpu_cuda::{CudaContext, CudaEvent, sys};
use shrimply_gpu_cuda_memory::{GpuBuffer, MemoryKind};

const CU_GRAPHICS_REGISTER_FLAGS_WRITE_DISCARD: u32 = 0x02;

unsafe extern "C" {
    fn shrimply_cuda_gl_device(device: *mut sys::CUdevice) -> sys::CUresult;
    fn shrimply_cuda_memcpy_2d_peer(
        descriptor: *const sys::CUDA_MEMCPY2D,
        source_context: sys::CUcontext,
        destination_context: sys::CUcontext,
    ) -> sys::CUresult;
    #[link_name = "shrimply_cuda_graphics_gl_register_image"]
    fn cuGraphicsGLRegisterImage(
        p_cuda_resource: *mut sys::CUgraphicsResource,
        image: u32,
        target: u32,
        flags: u32,
    ) -> sys::CUresult;
}

pub struct CudaTexture {
    resource: sys::CUgraphicsResource,
    context: Arc<CudaContext>,
    copy_complete: CudaEvent,
    source: Option<Arc<GpuBuffer<u32>>>,
}

impl CudaTexture {
    pub fn register(
        texture_id: u32,
        target: u32,
        mut context: Arc<CudaContext>,
    ) -> Result<Self, String> {
        let mut display_device = 0;
        cuda_check(
            unsafe { shrimply_cuda_gl_device(&mut display_device) },
            "GTK preview requires one CUDA-visible NVIDIA OpenGL device",
        )?;
        if display_device != context.cu_device() {
            let ordinal = shrimply_gpu_cuda::devices()
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|device| {
                    let mut candidate = 0;
                    unsafe {
                        sys::cuDeviceGet(&mut candidate, device.ordinal as i32) == sys::CUDA_SUCCESS
                            && candidate == display_device
                    }
                })
                .ok_or("GTK OpenGL device is unavailable to CUDA")?
                .ordinal;
            context = CudaContext::new(ordinal).map_err(|error| error.to_string())?;
        }
        bind_context(&context, "bind CUDA context for GL registration")?;
        let copy_complete = context
            .new_event(Some(sys::CUevent_flags_enum_CU_EVENT_DISABLE_TIMING))
            .map_err(|error| format!("Create CUDA preview copy event: {error}"))?;
        let mut resource = ptr::null_mut();
        cuda_check(
            unsafe {
                cuGraphicsGLRegisterImage(
                    &mut resource,
                    texture_id,
                    target,
                    CU_GRAPHICS_REGISTER_FLAGS_WRITE_DISCARD,
                )
            },
            "cuGraphicsGLRegisterImage",
        )?;

        Ok(Self {
            resource,
            context,
            copy_complete,
            source: None,
        })
    }

    pub fn copy_from_device(
        &mut self,
        source: &Arc<GpuBuffer<u32>>,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        let width_bytes = width as usize * size_of::<u32>();
        if source.len() < width as usize * height as usize {
            return Err("CUDA preview frame is smaller than its dimensions".into());
        }

        // A device-to-device copy may outlive cuMemcpy2D. Keep its source leased
        // until this event completes, so the compositor cannot reuse it early.
        self.copy_complete
            .synchronize()
            .map_err(|error| format!("Finish previous CUDA preview copy: {error}"))?;
        self.source = None;

        bind_context(&self.context, "bind CUDA context for GL texture copy")?;
        let mut resource = self.resource;
        cuda_check(
            unsafe { sys::cuGraphicsMapResources(1, &mut resource, ptr::null_mut()) },
            "cuGraphicsMapResources",
        )?;
        let _mapped = MappedResource {
            resource,
            context: self.context.clone(),
        };

        let mut destination = ptr::null_mut();
        cuda_check(
            unsafe { sys::cuGraphicsSubResourceGetMappedArray(&mut destination, resource, 0, 0) },
            "cuGraphicsSubResourceGetMappedArray",
        )?;

        let copy = sys::CUDA_MEMCPY2D {
            srcXInBytes: 0,
            srcY: 0,
            srcMemoryType: match source.memory_kind() {
                MemoryKind::Device => sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE,
                MemoryKind::Managed => sys::CUmemorytype_enum_CU_MEMORYTYPE_UNIFIED,
            },
            srcHost: ptr::null(),
            srcDevice: source.cu_deviceptr(),
            srcArray: ptr::null_mut(),
            srcPitch: width_bytes as size_t,
            dstXInBytes: 0,
            dstY: 0,
            dstMemoryType: sys::CUmemorytype_enum_CU_MEMORYTYPE_ARRAY,
            dstHost: ptr::null_mut(),
            dstDevice: 0,
            dstArray: destination,
            dstPitch: 0,
            WidthInBytes: width_bytes as size_t,
            Height: height as size_t,
        };
        let cross_context = source.context().cu_ctx() != self.context.cu_ctx();
        self.source = Some(source.clone());
        let copied = if cross_context && source.memory_kind() == MemoryKind::Device {
            cuda_check(
                unsafe {
                    shrimply_cuda_memcpy_2d_peer(
                        &copy,
                        source.context().cu_ctx(),
                        self.context.cu_ctx(),
                    )
                },
                "copy preview between NVIDIA devices",
            )
        } else {
            // Managed allocations use their unified address in the destination
            // context; CUDA migrates them as needed, including across GPUs.
            cuda_check(unsafe { sys::cuMemcpy2D_v2(&copy) }, "cuMemcpy2D")
        };
        if let Err(error) = self.copy_complete.record(&self.context.default_stream()) {
            // A failed event must not let a later upload release an in-flight
            // source. Fail clearly if the context cannot establish completion.
            self.context
                .synchronize()
                .expect("Cannot establish completion of CUDA preview copy");
            return Err(format!("Record CUDA preview copy completion: {error}"));
        }
        if cross_context {
            // Keep the source allocation/context alive through completion.
            self.context
                .synchronize()
                .map_err(|error| format!("Finish cross-device preview copy: {error}"))?;
        }
        copied?;
        Ok(())
    }

    fn unregister(&mut self) {
        if self.resource.is_null() {
            return;
        }

        match bind_context(&self.context, "bind CUDA context for GL unregistration") {
            Ok(()) => {
                if let Err(error) = cuda_check(
                    unsafe { sys::cuGraphicsUnregisterResource(self.resource) },
                    "cuGraphicsUnregisterResource",
                ) {
                    tracing::warn!("Could not unregister CUDA GL texture: {error}");
                }
            }
            Err(error) => {
                tracing::warn!("Could not enter CUDA context to unregister GL texture: {error}")
            }
        }
        self.resource = ptr::null_mut();
    }
}

impl Drop for CudaTexture {
    fn drop(&mut self) {
        self.copy_complete
            .synchronize()
            .expect("Cannot release an in-flight CUDA preview copy");
        self.unregister();
    }
}

struct MappedResource {
    resource: sys::CUgraphicsResource,
    context: Arc<CudaContext>,
}

impl Drop for MappedResource {
    fn drop(&mut self) {
        if let Err(error) = bind_context(&self.context, "bind CUDA context for GL unmap") {
            tracing::warn!("Could not enter CUDA context to unmap GL texture: {error}");
            return;
        }

        let mut resource = self.resource;
        if let Err(error) = cuda_check(
            unsafe { sys::cuGraphicsUnmapResources(1, &mut resource, ptr::null_mut()) },
            "cuGraphicsUnmapResources",
        ) {
            tracing::warn!("Could not unmap CUDA GL texture: {error}");
        }
    }
}

fn bind_context(context: &CudaContext, operation: &str) -> Result<(), String> {
    context
        .bind_to_thread()
        .map_err(|error| format!("{operation}: {error:?}"))
}

fn cuda_check(result: sys::CUresult, operation: &str) -> Result<(), String> {
    if result == sys::cudaError_enum_CUDA_SUCCESS {
        return Ok(());
    }

    let mut error_name = ptr::null();
    let mut error_string = ptr::null();
    let name = unsafe {
        (sys::cuGetErrorName(result, &mut error_name) == sys::cudaError_enum_CUDA_SUCCESS
            && !error_name.is_null())
        .then(|| CStr::from_ptr(error_name).to_string_lossy().into_owned())
    };
    let detail = unsafe {
        (sys::cuGetErrorString(result, &mut error_string) == sys::cudaError_enum_CUDA_SUCCESS
            && !error_string.is_null())
        .then(|| CStr::from_ptr(error_string).to_string_lossy().into_owned())
    };

    Err(match (name, detail) {
        (Some(name), Some(detail)) => format!("{operation}: {name}: {detail}"),
        (Some(name), None) => format!("{operation}: {name} ({result})"),
        (None, Some(detail)) => format!("{operation}: {detail} ({result})"),
        (None, None) => format!("{operation}: CUDA error {result}"),
    })
}
