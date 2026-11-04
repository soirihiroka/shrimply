use std::ffi::CStr;
use std::ptr;
use std::sync::Arc;

use libc::size_t;
use shrimply_gpu_cuda::{CudaContext, sys};
use shrimply_gpu_cuda_memory::MemoryKind;

const CU_GRAPHICS_REGISTER_FLAGS_WRITE_DISCARD: u32 = 0x02;

unsafe extern "C" {
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
}

impl CudaTexture {
    pub fn register(
        texture_id: u32,
        target: u32,
        context: Arc<CudaContext>,
    ) -> Result<Self, String> {
        bind_context(&context, "bind CUDA context for GL registration")?;
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

        Ok(Self { resource, context })
    }

    pub fn copy_from_device(
        &self,
        source: sys::CUdeviceptr,
        source_memory: MemoryKind,
        source_pitch_bytes: usize,
        width_bytes: usize,
        height: usize,
    ) -> Result<(), String> {
        if source == 0 || width_bytes == 0 || height == 0 {
            return Ok(());
        }

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
            srcMemoryType: match source_memory {
                MemoryKind::Device => sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE,
                MemoryKind::Managed => sys::CUmemorytype_enum_CU_MEMORYTYPE_UNIFIED,
            },
            srcHost: ptr::null(),
            srcDevice: source,
            srcArray: ptr::null_mut(),
            srcPitch: source_pitch_bytes as size_t,
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
        cuda_check(unsafe { sys::cuMemcpy2D_v2(&copy) }, "cuMemcpy2D")?;
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
