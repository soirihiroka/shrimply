#![cfg(target_os = "linux")]
mod vulkan;
use hashbrown::HashMap;
use shrimply_cuda::{CudaContext, sys};
use shrimply_visual_frame::VisualFrame;
use std::{
    os::fd::IntoRawFd,
    ptr,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
};
static IMPORTED_VULKAN_FRAMES: AtomicU64 = AtomicU64::new(0);
static IMPORTED_VULKAN_BYTES: AtomicU64 = AtomicU64::new(0);
fn bind_context(context: &CudaContext, operation: &str) -> Result<(), String> {
    context
        .bind_to_thread()
        .map_err(|e| format!("{operation}: {e:?}"))
}
fn cuda_check(result: sys::CUresult, operation: &str) -> Result<(), String> {
    if result == sys::cudaError_enum_CUDA_SUCCESS {
        Ok(())
    } else {
        Err(format!("{operation}: CUDA error {result}"))
    }
}
pub struct Renderer {
    sources: HashMap<usize, CachedManimSource>,
    renderer: vulkan::Renderer,
}

struct CachedManimSource {
    owner: Weak<()>,
    source: ManimCudaSource,
}

struct ManimCudaSource {
    descriptor: shrimply_manim_wgpu::ExternalFrameDescriptor,
    array: sys::CUarray,
    semaphore: usize,
    _import: ImportedManimImage,
}

struct ImportedManimImage {
    context: Arc<CudaContext>,
    mipmapped_array: sys::CUmipmappedArray,
    external_memory: usize,
    external_semaphore: usize,
    allocation_size: u64,
    stream: Arc<shrimply_cuda::CudaStream>,
}

impl Renderer {
    pub fn new(context: &CudaContext) -> Result<Self, String> {
        let started = std::time::Instant::now();
        let renderer = vulkan::Renderer::new(
            context
                .device_uuid()
                .map_err(|error| format!("read CUDA device identity: {error}"))?,
        )?;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            "Manim WGPU renderer initialized",
        );
        Ok(Self {
            sources: HashMap::new(),
            renderer,
        })
    }

    pub fn release_render_surfaces(&mut self) -> bool {
        let released = !self.sources.is_empty();
        self.sources.clear();
        let released = self.renderer.release_render_surfaces() || released;
        released
    }

    pub fn release_gpu_animation_resources(&mut self) -> bool {
        self.renderer.release_gpu_animation_resources()
    }

    pub fn render(
        &mut self,
        context: Arc<CudaContext>,
        stream: Arc<shrimply_cuda::CudaStream>,
        slot: &Arc<()>,
        animation: &shrimply_manim_wgpu::PreparedAnimation,
        frame_index: usize,
        destination: &VisualFrame,
    ) -> Result<(), String> {
        self.remove_expired();
        let slot_id = Arc::as_ptr(slot) as usize;
        if let Some(cached) = self.sources.get(&slot_id) {
            cached
                .source
                ._import
                .stream
                .synchronize()
                .map_err(|e| format!("wait for previous Manim CUDA copy: {e:?}"))?;
        }
        let descriptor = shrimply_manim_wgpu::Renderer::external_frame_descriptor(animation);

        if destination.width() != descriptor.width || destination.height() != descriptor.height {
            return Err("Manim render slot dimensions do not match the animation".to_string());
        }
        if self.renderer.target_descriptor(slot_id) != Some(descriptor) {
            self.sources.remove(&slot_id);
        }
        let _render = shrimply_benchmarking::measure("Manim WGPU draw and export");
        let rendered = self
            .renderer
            .render_external(slot_id, animation, frame_index)?;
        drop(_render);
        if self
            .sources
            .get(&slot_id)
            .map(|cached| cached.source.descriptor)
            != Some(rendered.descriptor)
        {
            self.sources.remove(&slot_id);
            let source = match self.renderer.export_frame(slot_id).and_then(|exported| {
                import_manim_source(
                    context.clone(),
                    stream.clone(),
                    rendered.descriptor,
                    exported,
                )
            }) {
                Ok(source) => source,
                Err(error) => {
                    self.renderer.remove_target(slot_id);
                    return Err(error);
                }
            };
            self.sources.insert(
                slot_id,
                CachedManimSource {
                    owner: Arc::downgrade(slot),
                    source,
                },
            );
            tracing::info!(
                slot = slot_id,
                width = descriptor.width,
                height = descriptor.height,
                samples = descriptor.samples,
                retained_sources = self.sources.len(),
                "imported persistent Manim WGPU image into CUDA",
            );
        }
        let source = self
            .sources
            .get_mut(&slot_id)
            .expect("Manim source was imported");
        let _copy = shrimply_benchmarking::measure("Manim WGPU to CUDA copy");
        copy_manim_source(
            context,
            stream,
            &source.source,
            rendered.semaphore_value,
            destination,
        )
    }

    fn remove_expired(&mut self) -> bool {
        let expired = self
            .sources
            .iter()
            .filter_map(|(&slot, source)| (source.owner.strong_count() == 0).then_some(slot))
            .collect::<Vec<_>>();
        for slot in &expired {
            self.sources.remove(slot);
            self.renderer.remove_target(*slot);
        }
        !expired.is_empty()
    }
}

fn import_manim_source(
    context: Arc<CudaContext>,
    stream: Arc<shrimply_cuda::CudaStream>,
    descriptor: shrimply_manim_wgpu::ExternalFrameDescriptor,
    exported: vulkan::ExportedFrame,
) -> Result<ManimCudaSource, String> {
    bind_context(&context, "bind CUDA context for Manim WGPU import")?;
    let fd = exported.fd.into_raw_fd();
    let mut external_memory = ptr::null_mut();
    let memory_desc = sys::CUDA_EXTERNAL_MEMORY_HANDLE_DESC {
        type_: sys::CUexternalMemoryHandleType_enum_CU_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD,
        handle: sys::CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1 { fd },
        size: exported.allocation_size,
        flags: sys::CUDA_EXTERNAL_MEMORY_DEDICATED,
        reserved: [0; 16],
    };
    if let Err(error) = cuda_check(
        unsafe { sys::cuImportExternalMemory(&mut external_memory, &memory_desc) },
        "cuImportExternalMemory for Manim WGPU",
    ) {
        unsafe { libc::close(fd) };
        return Err(error);
    }
    let mut mipmapped_array = ptr::null_mut();
    let mipmapped_desc = sys::CUDA_EXTERNAL_MEMORY_MIPMAPPED_ARRAY_DESC {
        offset: 0,
        arrayDesc: sys::CUDA_ARRAY3D_DESCRIPTOR {
            Width: exported.width as usize,
            Height: exported.height as usize,
            Depth: 0,
            Format: sys::CUarray_format_enum_CU_AD_FORMAT_UNSIGNED_INT8,
            NumChannels: 4,
            Flags: sys::CUDA_ARRAY3D_COLOR_ATTACHMENT | sys::CUDA_ARRAY3D_SURFACE_LDST,
        },
        numLevels: 1,
        reserved: [0; 16],
    };
    if let Err(error) = cuda_check(
        unsafe {
            sys::cuExternalMemoryGetMappedMipmappedArray(
                &mut mipmapped_array,
                external_memory,
                &mipmapped_desc,
            )
        },
        "cuExternalMemoryGetMappedMipmappedArray for Manim WGPU",
    ) {
        if cuda_check(
            unsafe { sys::cuDestroyExternalMemory(external_memory) },
            "cuDestroyExternalMemory after Manim WGPU mapping failure",
        )
        .is_err()
        {
            std::process::abort();
        }
        return Err(error);
    }
    let mut array = ptr::null_mut();
    if let Err(error) = cuda_check(
        unsafe { sys::cuMipmappedArrayGetLevel(&mut array, mipmapped_array, 0) },
        "cuMipmappedArrayGetLevel for Manim WGPU",
    ) {
        if cuda_check(
            unsafe { sys::cuMipmappedArrayDestroy(mipmapped_array) },
            "cuMipmappedArrayDestroy after Manim WGPU level failure",
        )
        .and_then(|()| {
            cuda_check(
                unsafe { sys::cuDestroyExternalMemory(external_memory) },
                "cuDestroyExternalMemory after Manim WGPU level failure",
            )
        })
        .is_err()
        {
            std::process::abort();
        }
        return Err(error);
    }
    let semaphore_fd = exported.semaphore_fd.into_raw_fd();
    let mut external_semaphore = ptr::null_mut();
    let semaphore_desc = sys::CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC {
        type_: sys::CUexternalSemaphoreHandleType_enum_CU_EXTERNAL_SEMAPHORE_HANDLE_TYPE_TIMELINE_SEMAPHORE_FD,
        handle: sys::CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC_st__bindgen_ty_1 {
            fd: semaphore_fd,
        },
        flags: 0,
        reserved: [0; 16],
    };
    if let Err(error) = cuda_check(
        unsafe { sys::cuImportExternalSemaphore(&mut external_semaphore, &semaphore_desc) },
        "cuImportExternalSemaphore for Manim WGPU",
    ) {
        unsafe { libc::close(semaphore_fd) };
        if cuda_check(
            unsafe { sys::cuMipmappedArrayDestroy(mipmapped_array) },
            "cuMipmappedArrayDestroy after Manim semaphore import failure",
        )
        .and_then(|()| {
            cuda_check(
                unsafe { sys::cuDestroyExternalMemory(external_memory) },
                "cuDestroyExternalMemory after Manim semaphore import failure",
            )
        })
        .is_err()
        {
            std::process::abort();
        }
        return Err(error);
    }
    let allocation_size = exported.allocation_size;
    let imported_frames = IMPORTED_VULKAN_FRAMES.fetch_add(1, Ordering::AcqRel) + 1;
    let imported_bytes =
        IMPORTED_VULKAN_BYTES.fetch_add(allocation_size, Ordering::AcqRel) + allocation_size;
    shrimply_benchmarking::set_counter("Manim Vulkan / CUDA frames retained", imported_frames);
    shrimply_benchmarking::set_counter("Manim Vulkan / CUDA bytes retained", imported_bytes);
    Ok(ManimCudaSource {
        descriptor,
        array,
        semaphore: external_semaphore as usize,
        _import: ImportedManimImage {
            context,
            mipmapped_array,
            external_memory: external_memory as usize,
            external_semaphore: external_semaphore as usize,
            allocation_size,
            stream,
        },
    })
}

fn copy_manim_source(
    context: Arc<CudaContext>,
    stream: Arc<shrimply_cuda::CudaStream>,
    source: &ManimCudaSource,
    semaphore_value: u64,
    destination: &VisualFrame,
) -> Result<(), String> {
    let destination_memory = destination.memory_kind(0);
    let destination = destination
        .plane(0)
        .ok_or_else(|| "Manim CUDA output has no RGBA plane".to_string())?;
    bind_context(&context, "bind CUDA context for Manim WGPU copy")?;
    let semaphores = [source.semaphore as sys::CUexternalSemaphore];
    let mut wait: sys::CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS = unsafe { std::mem::zeroed() };
    wait.params.fence.value = semaphore_value;
    cuda_check(
        unsafe {
            sys::cuWaitExternalSemaphoresAsync(semaphores.as_ptr(), &wait, 1, stream.cu_stream())
        },
        "wait for Manim WGPU timeline semaphore",
    )?;
    let copy = sys::CUDA_MEMCPY2D {
        srcXInBytes: 0,
        srcY: 0,
        srcMemoryType: sys::CUmemorytype_enum_CU_MEMORYTYPE_ARRAY,
        srcHost: ptr::null(),
        srcDevice: 0,
        srcArray: source.array,
        srcPitch: 0,
        dstXInBytes: 0,
        dstY: 0,
        dstMemoryType: match destination_memory {
            Some(shrimply_gpu_memory::MemoryKind::Managed) => {
                sys::CUmemorytype_enum_CU_MEMORYTYPE_UNIFIED
            }
            _ => sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE,
        },
        dstHost: ptr::null_mut(),
        dstDevice: destination.device_ptr,
        dstArray: ptr::null_mut(),
        dstPitch: destination.pitch_bytes,
        WidthInBytes: source.descriptor.width as usize * 4,
        Height: source.descriptor.height as usize,
    };
    cuda_check(
        unsafe { sys::cuMemcpy2DAsync_v2(&copy, stream.cu_stream()) },
        "copy Manim WGPU image to CUDA frame",
    )
}

impl Drop for ImportedManimImage {
    fn drop(&mut self) {
        if bind_context(&self.context, "bind CUDA context for Manim WGPU image drop").is_err()
            || self.stream.synchronize().is_err()
            || cuda_check(
                unsafe { sys::cuMipmappedArrayDestroy(self.mipmapped_array) },
                "cuMipmappedArrayDestroy for Manim WGPU image",
            )
            .is_err()
            || cuda_check(
                unsafe {
                    sys::cuDestroyExternalMemory(self.external_memory as sys::CUexternalMemory)
                },
                "cuDestroyExternalMemory for Manim WGPU image",
            )
            .is_err()
            || cuda_check(
                unsafe {
                    sys::cuDestroyExternalSemaphore(
                        self.external_semaphore as sys::CUexternalSemaphore,
                    )
                },
                "cuDestroyExternalSemaphore for Manim WGPU image",
            )
            .is_err()
        {
            std::process::abort();
        }
        let previous_frames = IMPORTED_VULKAN_FRAMES.fetch_sub(1, Ordering::AcqRel);
        let previous_bytes =
            IMPORTED_VULKAN_BYTES.fetch_sub(self.allocation_size, Ordering::AcqRel);
        assert!(
            previous_frames > 0,
            "imported Vulkan frame counter underflowed"
        );
        assert!(
            previous_bytes >= self.allocation_size,
            "imported Vulkan byte counter underflowed"
        );
        shrimply_benchmarking::set_counter(
            "Manim Vulkan / CUDA frames retained",
            previous_frames - 1,
        );
        shrimply_benchmarking::set_counter(
            "Manim Vulkan / CUDA bytes retained",
            previous_bytes - self.allocation_size,
        );
    }
}
