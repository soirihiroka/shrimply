#![cfg(any(target_os = "linux", windows))]
mod vulkan;
use hashbrown::HashMap;
use shrimply_gpu_cuda::{CudaContext, sys};
use shrimply_visual_frame::VisualFrame;
use std::{
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
    device_uuid: [u8; shrimply_gpu_cuda::DEVICE_UUID_BYTES],
}

struct CachedManimSource {
    owner: Weak<()>,
    source: ManimCudaSource,
}

struct ManimCudaSource {
    descriptor: shrimply_manim_wgpu::ExternalFrameDescriptor,
    // Drop the CUDA image before reporting its memory as released.
    image: shrimply_gpu_cuda::external::ImportedImage,
    _accounting: ImportedImageAccounting,
}

struct ImportedImageAccounting {
    allocation_size: u64,
}

impl Renderer {
    pub fn new(context: &CudaContext) -> Result<Self, String> {
        let started = std::time::Instant::now();
        let device_uuid = context
            .device_uuid()
            .map_err(|error| format!("read CUDA device identity: {error}"))?;
        let renderer = vulkan::Renderer::new(device_uuid)?;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            "Manim WGPU renderer initialized",
        );
        Ok(Self {
            sources: HashMap::new(),
            renderer,
            device_uuid,
        })
    }

    pub fn release_render_surfaces(&mut self) -> bool {
        let released = !self.sources.is_empty();
        self.sources.clear();
        self.renderer.release_render_surfaces() || released
    }

    pub fn release_gpu_animation_resources(&mut self) -> bool {
        self.renderer.release_gpu_animation_resources()
    }

    pub fn render(
        &mut self,
        context: Arc<CudaContext>,
        stream: Arc<shrimply_gpu_cuda::CudaStream>,
        slot: &Arc<()>,
        animation: &shrimply_manim_wgpu::PreparedAnimation,
        frame_index: usize,
        destination: &VisualFrame,
    ) -> Result<(), String> {
        if context
            .device_uuid()
            .map_err(|error| format!("read CUDA device identity: {error}"))?
            != self.device_uuid
        {
            return Err("Manim renderer cannot be used with a different CUDA device".into());
        }
        self.remove_expired();
        let slot_id = Arc::as_ptr(slot) as usize;
        if let Some(cached) = self.sources.get(&slot_id) {
            cached
                .source
                .image
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
        let _render = shrimply_profiling::measure("Manim WGPU draw and export");
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
        let _copy = shrimply_profiling::measure("Manim WGPU to CUDA copy");
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
    stream: Arc<shrimply_gpu_cuda::CudaStream>,
    descriptor: shrimply_manim_wgpu::ExternalFrameDescriptor,
    exported: vulkan::ExportedFrame,
) -> Result<ManimCudaSource, String> {
    let allocation_size = exported.allocation_size;
    let image = shrimply_gpu_cuda::external::ImportedImage::new(
        context,
        stream,
        shrimply_gpu_cuda::external::ImageDescriptor {
            #[cfg(target_os = "linux")]
            fd: exported.fd,
            #[cfg(target_os = "linux")]
            semaphore_fd: exported.semaphore_fd,
            #[cfg(windows)]
            handle: exported.handle,
            #[cfg(windows)]
            semaphore_handle: exported.semaphore_handle,
            allocation_size,
            width: exported.width,
            height: exported.height,
        },
    )?;
    let imported_frames = IMPORTED_VULKAN_FRAMES.fetch_add(1, Ordering::AcqRel) + 1;
    let imported_bytes =
        IMPORTED_VULKAN_BYTES.fetch_add(allocation_size, Ordering::AcqRel) + allocation_size;
    shrimply_profiling::set_counter("Manim Vulkan / CUDA frames retained", imported_frames);
    shrimply_profiling::set_counter("Manim Vulkan / CUDA bytes retained", imported_bytes);
    Ok(ManimCudaSource {
        descriptor,
        image,
        _accounting: ImportedImageAccounting { allocation_size },
    })
}

fn copy_manim_source(
    context: Arc<CudaContext>,
    stream: Arc<shrimply_gpu_cuda::CudaStream>,
    source: &ManimCudaSource,
    semaphore_value: u64,
    destination: &VisualFrame,
) -> Result<(), String> {
    let destination_memory = destination.memory_kind(0);
    let destination = destination
        .plane(0)
        .ok_or_else(|| "Manim CUDA output has no RGBA plane".to_string())?;
    bind_context(&context, "bind CUDA context for Manim WGPU copy")?;
    source.image.wait(&stream, semaphore_value)?;
    let copy = sys::CUDA_MEMCPY2D {
        srcXInBytes: 0,
        srcY: 0,
        srcMemoryType: sys::CUmemorytype_enum_CU_MEMORYTYPE_ARRAY,
        srcHost: ptr::null(),
        srcDevice: 0,
        srcArray: source.image.array(),
        srcPitch: 0,
        dstXInBytes: 0,
        dstY: 0,
        dstMemoryType: match destination_memory {
            Some(shrimply_gpu_cuda_memory::MemoryKind::Managed) => {
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

impl Drop for ImportedImageAccounting {
    fn drop(&mut self) {
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
        shrimply_profiling::set_counter("Manim Vulkan / CUDA frames retained", previous_frames - 1);
        shrimply_profiling::set_counter(
            "Manim Vulkan / CUDA bytes retained",
            previous_bytes - self.allocation_size,
        );
    }
}
