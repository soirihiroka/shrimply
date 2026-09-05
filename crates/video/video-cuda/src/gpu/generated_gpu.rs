use std::ffi::{CStr, CString};
use std::fmt::Write;
use std::fs;
use std::os::fd::IntoRawFd;
use std::path::{Path, PathBuf};
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use ash::vk::Handle;
use ash::{Entry, Instance, vk};
use hashbrown::HashMap;
use shrimply_cuda::{CudaContext, sys};
use shrimply_evaluation::{TransformEvaluation, TransformExpressionCache};
use shrimply_project::project::{CanvasSize, SkiaDrawingStrategy};
use shrimply_video_core::generated::draw_visual;
use shrimply_visual_frame::{VisualFormat, VisualFrame, VisualPlane};
use skia_safe::ColorType;
use skia_safe::gpu::{self, backend_render_targets, direct_contexts, surfaces, vk as skia_vk};

use super::{bind_context, cuda_check};
use crate::layer::VectorOperation;

mod background;
mod mesh_flow;
mod scene_3d;
mod visual;

pub(crate) use shrimply_video_core::generated::GeneratedVisual;
pub(super) use visual::visual_frame_from_canvas;

const VECTOR_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const VECTOR_SKIA_FORMAT: skia_vk::Format = skia_vk::Format::R8G8B8A8_UNORM;
static IMPORTED_VULKAN_FRAMES: AtomicU64 = AtomicU64::new(0);
static IMPORTED_VULKAN_BYTES: AtomicU64 = AtomicU64::new(0);

pub struct GeneratedGpuRenderer {
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
    queue: vk::Queue,
    external_memory_fd: ash::khr::external_memory_fd::Device,
    external_semaphore_fd: ash::khr::external_semaphore_fd::Device,
    skia: gpu::DirectContext,
    expression_cache: TransformExpressionCache,
    gaussian_3d: Option<shrimply_3dgs::Renderer>,
    gaussian_frames: Arc<Mutex<Vec<ImportedVulkanFrame>>>,
    background_frame: Option<(background::RenderKey, VisualFrame)>,
    background: Option<background::Renderer>,
    manim: Option<shrimply_manim_cuda::Renderer>,
    scene_3d: Option<scene_3d::Scene3dRenderer>,
    mesh_flow: Option<mesh_flow::Renderer>,
    optix_denoiser: Option<shrimply_optix_denoiser::OptixDenoiser>,
    // Drop after every renderer that owns command buffers allocated from it.
    command_pool: VulkanCommandPool,
    // Drop last so Skia and all renderer-owned Vulkan handles are gone before the device.
    vulkan: Arc<VulkanDevice>,
}

impl GeneratedGpuRenderer {
    pub fn new() -> Result<Self, String> {
        let entry = unsafe { Entry::load() }.map_err(|error| format!("load Vulkan: {error}"))?;
        let app_name = CString::new("Shrimply Generated Renderer").unwrap();
        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(0)
            .engine_name(&app_name)
            .engine_version(0)
            .api_version(vk::make_api_version(0, 1, 2, 0));
        let instance_info = vk::InstanceCreateInfo::default().application_info(&app_info);
        let instance = unsafe { entry.create_instance(&instance_info, None) }
            .map_err(|error| format!("create Vulkan instance: {error:?}"))?;
        let (physical_device, queue_family_index) = match pick_physical_device(&instance) {
            Ok(selection) => selection,
            Err(error) => {
                unsafe { instance.destroy_instance(None) };
                return Err(error);
            }
        };

        let priority = [1.0];
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priority)];
        let external_memory_name = CString::new("VK_KHR_external_memory").unwrap();
        let external_semaphore_name = CString::new("VK_KHR_external_semaphore").unwrap();
        let device_extensions = [
            external_memory_name.as_ptr(),
            ash::khr::external_memory_fd::NAME.as_ptr(),
            external_semaphore_name.as_ptr(),
            ash::khr::external_semaphore_fd::NAME.as_ptr(),
            ash::khr::buffer_device_address::NAME.as_ptr(),
            ash::khr::deferred_host_operations::NAME.as_ptr(),
            ash::khr::acceleration_structure::NAME.as_ptr(),
            ash::khr::ray_tracing_pipeline::NAME.as_ptr(),
            ash::khr::spirv_1_4::NAME.as_ptr(),
            ash::khr::shader_float_controls::NAME.as_ptr(),
        ];
        let mut buffer_address =
            vk::PhysicalDeviceBufferDeviceAddressFeatures::default().buffer_device_address(true);
        let mut acceleration = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
            .acceleration_structure(true);
        let mut ray_tracing = vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default()
            .ray_tracing_pipeline(true)
            .ray_traversal_primitive_culling(true);
        let features = vk::PhysicalDeviceFeatures::default().geometry_shader(true);
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_info)
            .enabled_extension_names(&device_extensions)
            .enabled_features(&features)
            .push_next(&mut buffer_address)
            .push_next(&mut acceleration)
            .push_next(&mut ray_tracing);
        let device = match unsafe { instance.create_device(physical_device, &device_info, None) } {
            Ok(device) => device,
            Err(error) => {
                unsafe { instance.destroy_instance(None) };
                return Err(format!(
                    "create Vulkan ray-tracing device (KHR acceleration structure/pipeline required): {error:?}"
                ));
            }
        };
        let pipeline_cache_path = pipeline_cache_path(unsafe {
            instance
                .get_physical_device_properties(physical_device)
                .pipeline_cache_uuid
        });
        let pipeline_cache = match create_pipeline_cache(&device, pipeline_cache_path.as_deref()) {
            Ok(cache) => cache,
            Err(error) => {
                unsafe {
                    device.destroy_device(None);
                    instance.destroy_instance(None);
                }
                return Err(error);
            }
        };
        let vulkan = Arc::new(VulkanDevice {
            _entry: entry,
            instance,
            device,
            pipeline_cache,
            pipeline_cache_path,
        });
        let queue = unsafe { vulkan.device.get_device_queue(queue_family_index, 0) };
        let command_pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let command_pool = unsafe { vulkan.device.create_command_pool(&command_pool_info, None) }
            .map_err(|error| format!("create Vulkan command pool: {error:?}"))?;
        let command_pool = VulkanCommandPool {
            vulkan: vulkan.clone(),
            handle: command_pool,
        };
        let external_memory_fd =
            ash::khr::external_memory_fd::Device::new(&vulkan.instance, &vulkan.device);
        let external_semaphore_fd =
            ash::khr::external_semaphore_fd::Device::new(&vulkan.instance, &vulkan.device);

        let skia = make_skia_context(
            &vulkan._entry,
            &vulkan.instance,
            &vulkan.device,
            physical_device,
            queue,
            queue_family_index,
        )?;

        Ok(Self {
            vulkan,
            physical_device,
            queue_family_index,
            queue,
            command_pool,
            external_memory_fd,
            external_semaphore_fd,
            skia,
            expression_cache: TransformExpressionCache::default(),
            gaussian_3d: None,
            gaussian_frames: Arc::new(Mutex::new(Vec::new())),
            background_frame: None,
            background: None,
            manim: None,
            scene_3d: None,
            mesh_flow: None,
            optix_denoiser: None,
        })
    }

    pub(crate) fn render_gaussian_3d(
        &mut self,
        context: Arc<CudaContext>,
        session: &shrimply_3dgs::RenderSession,
        width: u32,
        height: u32,
        params: &shrimply_3dgs::RenderParams,
    ) -> Result<VisualFrame, String> {
        let width = width.max(1);
        let height = height.max(1);
        let size = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "3DGS output dimensions overflow".to_string())?;
        let imported = {
            let mut frames = self
                .gaussian_frames
                .lock()
                .expect("3DGS interop frame cache lock poisoned");
            frames
                .iter()
                .position(|frame| frame._buffer.buffer_size == size)
                .map(|index| frames.swap_remove(index))
        };
        let imported = match imported {
            Some(imported) => imported,
            None => {
                let buffer = self.create_exported_buffer(size)?;
                self.import_buffer_to_cuda_allocation(context.clone(), buffer, None)?
            }
        };
        let mut renderer = match self.gaussian_3d.take() {
            Some(renderer) => renderer,
            None => shrimply_3dgs::Renderer::new(shrimply_3dgs::RenderContext {
                device: self.vulkan.device.clone(),
                memory_properties: unsafe {
                    self.vulkan
                        .instance
                        .get_physical_device_memory_properties(self.physical_device)
                },
                queue: self.queue,
                command_pool: self.command_pool.handle,
                pipeline_cache: self.vulkan.pipeline_cache,
            })?,
        };
        let rendered =
            renderer.render_to_buffer(session, width, height, params, imported._buffer.buffer);
        self.gaussian_3d = Some(renderer);
        rendered?;
        self.visual_frame_from_imported(width, height, imported, Some(self.gaussian_frames.clone()))
    }

    pub(crate) fn render_background(
        &mut self,
        context: Arc<CudaContext>,
        stream: Arc<shrimply_cuda::CudaStream>,
        width: u32,
        height: u32,
        time: shrimply_project::project::Time,
        background_config: &shrimply_background::Background,
    ) -> Result<VisualFrame, String> {
        let key = background::RenderKey::new(width, height, time, background_config);
        if let Some((cached_key, frame)) = &self.background_frame
            && cached_key == &key
        {
            shrimply_benchmarking::increment("Background raster cache / Hit");
            return Ok(frame.clone());
        }
        shrimply_benchmarking::increment("Background raster cache / Miss");
        self.background_frame = None;
        let width = key.0.common.width;
        let height = key.0.common.height;
        let size = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "background output dimensions overflow".to_string())?;
        let buffer = self.create_exported_buffer(size)?;
        let semaphore = self.create_exported_semaphore()?;
        let output = buffer.buffer;
        let mut renderer = match self.background.take() {
            Some(renderer) => renderer,
            None => background::Renderer::new(background::RenderContext {
                device: self.vulkan.device.clone(),
                memory_properties: unsafe {
                    self.vulkan
                        .instance
                        .get_physical_device_memory_properties(self.physical_device)
                },
                queue: self.queue,
                command_pool: self.command_pool.handle,
                pipeline_cache: self.vulkan.pipeline_cache,
            })?,
        };
        let rendered = renderer.render(output, &key, semaphore.handle);
        self.background = Some(renderer);
        rendered?;
        let frame = self
            .import_submitted_buffer_to_cuda(context, stream, width, height, buffer, semaphore)?;
        self.background_frame = Some((key, frame.clone()));
        Ok(frame)
    }

    pub(crate) fn render_manim(
        &mut self,
        context: Arc<CudaContext>,
        stream: Arc<shrimply_cuda::CudaStream>,
        slot: &Arc<()>,
        animation: &shrimply_manim_wgpu::PreparedAnimation,
        frame_index: usize,
        destination: &VisualFrame,
    ) -> Result<(), String> {
        if self.manim.is_none() {
            self.manim = Some(shrimply_manim_cuda::Renderer::new()?);
        }
        self.manim
            .as_mut()
            .expect("Manim renderer was initialized")
            .render(context, stream, slot, animation, frame_index, destination)
    }

    pub(crate) fn render_mesh_flow(
        &mut self,
        context: Arc<CudaContext>,
        stream: &shrimply_cuda::CudaStream,
        source: &VisualFrame,
        grid_width: u32,
        grid_height: u32,
        source_offsets: &[glam::Vec2],
    ) -> Result<VisualFrame, String> {
        let size = u64::from(source.width())
            .checked_mul(u64::from(source.height()))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "MeshFlow frame dimensions overflow".to_string())?;
        let input = self.create_exported_buffer(size)?;
        let input_buffer = input.buffer;
        let input =
            self.import_buffer_to_cuda(context.clone(), source.width(), source.height(), input)?;
        let input_plane = input.plane(0).expect("RGBA MeshFlow input has no plane");
        let source_plane = source.plane(0).expect("RGBA MeshFlow source has no plane");
        bind_context(&context, "bind CUDA context for MeshFlow input copy")?;
        cuda_check(
            unsafe {
                sys::cuMemcpyDtoDAsync_v2(
                    input_plane.device_ptr,
                    source_plane.device_ptr,
                    size as usize,
                    stream.cu_stream(),
                )
            },
            "copy MeshFlow input to Vulkan memory",
        )?;
        stream
            .synchronize()
            .map_err(|error| format!("synchronize MeshFlow input copy: {error:?}"))?;

        let output = self.create_exported_buffer(size)?;
        let output_buffer = output.buffer;
        let mut renderer = match self.mesh_flow.take() {
            Some(renderer) => renderer,
            None => mesh_flow::Renderer::new(mesh_flow::RenderContext {
                device: self.vulkan.device.clone(),
                memory_properties: unsafe {
                    self.vulkan
                        .instance
                        .get_physical_device_memory_properties(self.physical_device)
                },
                queue: self.queue,
                command_pool: self.command_pool.handle,
                pipeline_cache: self.vulkan.pipeline_cache,
            })?,
        };
        let rendered = renderer.render(mesh_flow::RenderRequest {
            input: input_buffer,
            output: output_buffer,
            image_size: glam::UVec2::new(source.width(), source.height()),
            grid_size: glam::UVec2::new(grid_width, grid_height),
            source_offsets,
        });
        self.mesh_flow = Some(renderer);
        rendered?;
        drop(input);
        self.import_buffer_to_cuda(context, source.width(), source.height(), output)
    }

    pub(crate) fn render_visual(
        &mut self,
        context: Arc<CudaContext>,
        canvas_sizes: (CanvasSize, CanvasSize),
        visual: &dyn GeneratedVisual,
        eval: &TransformEvaluation,
        operations: &[VectorOperation],
        drawing_strategy: SkiaDrawingStrategy,
    ) -> Result<VisualFrame, String> {
        let _measurement = shrimply_benchmarking::measure("Generated visual / Total");
        let render_size = canvas_sizes.0;
        let width = render_size.width.max(1);
        let height = render_size.height.max(1);
        let (image, buffer) = {
            let _measurement =
                shrimply_benchmarking::measure("Generated visual / Allocate targets");
            (
                self.create_render_image(width, height)?,
                self.create_exported_buffer(width as u64 * height as u64 * 4)?,
            )
        };

        {
            let _measurement =
                shrimply_benchmarking::measure("Generated visual / Skia draw and sync");
            let mut surface = self.surface_for_image(width, height, image.image)?;
            draw_visual(
                surface.canvas(),
                canvas_sizes,
                visual,
                operations,
                drawing_strategy,
                eval,
                &mut self.expression_cache,
            );
            self.skia
                .flush_and_submit_surface(&mut surface, skia_safe::gpu::SyncCpu::Yes);
            if self.skia.oomed() {
                return Err("Skia GPU ran out of memory".to_string());
            }
            shrimply_benchmarking::set_counter(
                "Generated Skia GPU cache bytes",
                self.skia.resource_cache_usage().resource_bytes as u64,
            );
        }

        {
            let _measurement =
                shrimply_benchmarking::measure("Generated visual / Vulkan copy and wait");
            self.copy_image_to_buffer(
                width,
                height,
                image.image,
                buffer.buffer,
                ImageCopySource::ColorAttachment,
            )?;
        }
        let _measurement = shrimply_benchmarking::measure("Generated visual / CUDA import");
        self.import_buffer_to_cuda(context, width, height, buffer)
    }

    fn create_render_image(&mut self, width: u32, height: u32) -> Result<VulkanImage, String> {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(VECTOR_FORMAT)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { self.vulkan.device.create_image(&image_info, None) }
            .map_err(|error| format!("create Vulkan generated image: {error:?}"))?;
        let requirements = unsafe { self.vulkan.device.get_image_memory_requirements(image) };
        let memory_type = match self.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ) {
            Ok(memory_type) => memory_type,
            Err(error) => {
                unsafe { self.vulkan.device.destroy_image(image, None) };
                return Err(error);
            }
        };
        let allocate_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type);
        let memory = match unsafe { self.vulkan.device.allocate_memory(&allocate_info, None) } {
            Ok(memory) => memory,
            Err(error) => {
                unsafe { self.vulkan.device.destroy_image(image, None) };
                return Err(format!("allocate Vulkan generated image memory: {error:?}"));
            }
        };
        if let Err(error) = unsafe { self.vulkan.device.bind_image_memory(image, memory, 0) } {
            unsafe {
                self.vulkan.device.destroy_image(image, None);
                self.vulkan.device.free_memory(memory, None);
            }
            return Err(format!("bind Vulkan generated image memory: {error:?}"));
        }
        Ok(VulkanImage {
            vulkan: self.vulkan.clone(),
            image,
            memory,
        })
    }

    fn create_exported_buffer(&mut self, size: u64) -> Result<ExportedBuffer, String> {
        let mut external_info = vk::ExternalMemoryBufferCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(
                vk::BufferUsageFlags::TRANSFER_DST
                    | vk::BufferUsageFlags::TRANSFER_SRC
                    | vk::BufferUsageFlags::STORAGE_BUFFER,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .push_next(&mut external_info);
        let buffer = unsafe { self.vulkan.device.create_buffer(&buffer_info, None) }
            .map_err(|error| format!("create Vulkan generated export buffer: {error:?}"))?;
        let requirements = unsafe { self.vulkan.device.get_buffer_memory_requirements(buffer) };
        let memory_type = match self.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ) {
            Ok(memory_type) => memory_type,
            Err(error) => {
                unsafe { self.vulkan.device.destroy_buffer(buffer, None) };
                return Err(error);
            }
        };
        let mut export_info = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let allocate_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type)
            .push_next(&mut export_info);
        let memory = match unsafe { self.vulkan.device.allocate_memory(&allocate_info, None) } {
            Ok(memory) => memory,
            Err(error) => {
                unsafe { self.vulkan.device.destroy_buffer(buffer, None) };
                return Err(format!(
                    "allocate Vulkan generated export memory: {error:?}"
                ));
            }
        };
        if let Err(error) = unsafe { self.vulkan.device.bind_buffer_memory(buffer, memory, 0) } {
            unsafe {
                self.vulkan.device.destroy_buffer(buffer, None);
                self.vulkan.device.free_memory(memory, None);
            }
            return Err(format!("bind Vulkan generated export memory: {error:?}"));
        }
        Ok(ExportedBuffer {
            vulkan: self.vulkan.clone(),
            buffer,
            memory,
            allocation_size: requirements.size,
            buffer_size: size,
        })
    }

    fn create_exported_semaphore(&self) -> Result<VulkanSemaphore, String> {
        let mut export = vk::ExportSemaphoreCreateInfo::default()
            .handle_types(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
        let handle = unsafe { self.vulkan.device.create_semaphore(&info, None) }
            .map_err(|error| format!("create background export semaphore: {error:?}"))?;
        Ok(VulkanSemaphore {
            vulkan: self.vulkan.clone(),
            handle,
        })
    }

    pub(super) fn release_render_surfaces(&mut self, requested_bytes: u64) -> Result<bool, String> {
        let background_frame = self.background_frame.take().map(|(_, frame)| frame.bytes());
        let mut released = background_frame.is_some();
        let gaussian_frames = self
            .gaussian_frames
            .lock()
            .expect("3DGS interop frame cache lock poisoned")
            .drain(..)
            .count();
        released |= gaussian_frames > 0;
        if let Some(manim) = self.manim.as_mut() {
            let manim_released = manim.release_render_surfaces();
            released |= manim_released;
            if manim_released {
                shrimply_gpu_memory::global().note_manim_render_surface_release();
            }
        }
        let retained = self.skia.resource_cache_usage().resource_bytes;
        shrimply_benchmarking::set_counter("Generated Skia GPU cache bytes", retained as u64);
        if released {
            tracing::warn!(
                requested_bytes,
                background_frame_bytes = background_frame.unwrap_or(0),
                gaussian_frames,
                retained,
                "released cached generated-renderer GPU resources"
            );
        } else {
            tracing::debug!(
                requested_bytes,
                retained,
                "generated renderer had no cached GPU resources to release"
            );
        }
        Ok(released)
    }

    pub(super) fn release_gpu_animation_resources(&mut self) -> bool {
        let released = self
            .manim
            .as_mut()
            .is_some_and(ManimRenderer::release_gpu_animation_resources);
        if released {
            shrimply_gpu_memory::global().note_manim_gpu_animation_release();
        }
        released
    }

    pub(super) fn release_external_gpu_resources(&mut self) -> bool {
        let mut released = self.gaussian_3d.take().is_some();
        released |= self.background.take().is_some();
        released |= self.scene_3d.take().is_some();
        released |= self.mesh_flow.take().is_some();
        released |= self.optix_denoiser.take().is_some();
        let skia_before = self.skia.resource_cache_usage().resource_bytes;
        self.skia.free_gpu_resources();
        let skia_retained = self.skia.resource_cache_usage().resource_bytes;
        released |= skia_retained < skia_before;
        shrimply_benchmarking::set_counter("Generated Skia GPU cache bytes", skia_retained as u64);
        released
    }

    fn surface_for_image(
        &mut self,
        width: u32,
        height: u32,
        image: vk::Image,
    ) -> Result<skia_safe::Surface, String> {
        let alloc = skia_vk::Alloc::default();
        let image_info = unsafe {
            skia_vk::ImageInfo::new(
                image.as_raw() as _,
                alloc,
                skia_vk::ImageTiling::OPTIMAL,
                skia_vk::ImageLayout::UNDEFINED,
                VECTOR_SKIA_FORMAT,
                1,
                self.queue_family_index,
                None,
                None,
                skia_vk::SharingMode::EXCLUSIVE,
            )
        };
        let target = backend_render_targets::make_vk((width as i32, height as i32), &image_info);
        let surface = surfaces::wrap_backend_render_target(
            &mut self.skia,
            &target,
            gpu::SurfaceOrigin::TopLeft,
            ColorType::RGBA8888,
            None,
            None,
        );
        if surface.is_none() && self.skia.oomed() {
            return Err("Skia GPU ran out of memory while creating a surface".to_string());
        }
        surface.ok_or_else(|| "wrap Vulkan generated image in Skia surface".to_string())
    }

    fn copy_image_to_buffer(
        &self,
        width: u32,
        height: u32,
        image: vk::Image,
        buffer: vk::Buffer,
        source: ImageCopySource,
    ) -> Result<(), String> {
        self.copy_images_to_buffer(width, height, &[(image, 0)], buffer, source)
    }

    fn copy_images_to_buffer(
        &self,
        width: u32,
        height: u32,
        images: &[(vk::Image, u64)],
        buffer: vk::Buffer,
        source: ImageCopySource,
    ) -> Result<(), String> {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool.handle)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffer = unsafe { self.vulkan.device.allocate_command_buffers(&allocate_info) }
            .map_err(|error| format!("allocate Vulkan generated copy command buffer: {error:?}"))?
            [0];
        let command_buffer = VulkanCommandBuffer {
            vulkan: self.vulkan.clone(),
            pool: self.command_pool.handle,
            handle: command_buffer,
        };
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            self.vulkan
                .device
                .begin_command_buffer(command_buffer.handle, &begin)
        }
        .map_err(|error| format!("begin Vulkan generated copy command buffer: {error:?}"))?;

        let (old_layout, source_stage, source_access) = match source {
            ImageCopySource::ColorAttachment => (
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
            ImageCopySource::RayTracing => (
                vk::ImageLayout::GENERAL,
                vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                vk::AccessFlags::SHADER_WRITE,
            ),
            ImageCopySource::TransferSource => (
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::PipelineStageFlags::TRANSFER,
                vk::AccessFlags::TRANSFER_READ,
            ),
        };
        let barriers: Vec<_> = images
            .iter()
            .map(|(image, _)| {
                vk::ImageMemoryBarrier::default()
                    .old_layout(old_layout)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_access_mask(source_access)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .image(*image)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
            })
            .collect();
        unsafe {
            self.vulkan.device.cmd_pipeline_barrier(
                command_buffer.handle,
                source_stage,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barriers,
            );
        }
        for (image, offset) in images {
            let copy = vk::BufferImageCopy::default()
                .buffer_offset(*offset)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_extent(vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                });
            unsafe {
                self.vulkan.device.cmd_copy_image_to_buffer(
                    command_buffer.handle,
                    *image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    buffer,
                    &[copy],
                );
            }
        }
        unsafe { self.vulkan.device.end_command_buffer(command_buffer.handle) }
            .map_err(|error| format!("end Vulkan generated copy command buffer: {error:?}"))?;
        let submit =
            vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command_buffer.handle));
        let fence_info = vk::FenceCreateInfo::default();
        let fence = unsafe { self.vulkan.device.create_fence(&fence_info, None) }
            .map_err(|error| format!("create Vulkan generated copy fence: {error:?}"))?;
        let fence = VulkanFence {
            vulkan: self.vulkan.clone(),
            handle: fence,
        };
        unsafe {
            self.vulkan
                .device
                .queue_submit(self.queue, &[submit], fence.handle)
        }
        .map_err(|error| format!("submit Vulkan generated image copy: {error:?}"))?;
        if let Err(error) = unsafe {
            self.vulkan
                .device
                .wait_for_fences(&[fence.handle], true, u64::MAX)
        } {
            wait_for_vulkan_idle_or_device_lost(&self.vulkan.device);
            return Err(format!("wait for Vulkan generated image copy: {error:?}"));
        }
        Ok(())
    }

    fn import_buffer_to_cuda(
        &self,
        context: Arc<CudaContext>,
        width: u32,
        height: u32,
        buffer: ExportedBuffer,
    ) -> Result<VisualFrame, String> {
        self.import_buffer_to_cuda_inner(context, width, height, buffer, None)
    }

    fn import_submitted_buffer_to_cuda(
        &self,
        context: Arc<CudaContext>,
        stream: Arc<shrimply_cuda::CudaStream>,
        width: u32,
        height: u32,
        buffer: ExportedBuffer,
        semaphore: VulkanSemaphore,
    ) -> Result<VisualFrame, String> {
        let fd_info = vk::SemaphoreGetFdInfoKHR::default()
            .semaphore(semaphore.handle)
            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD);
        let fd = match unsafe { self.external_semaphore_fd.get_semaphore_fd(&fd_info) } {
            Ok(fd) => fd,
            Err(error) => {
                wait_for_vulkan_idle_or_device_lost(&self.vulkan.device);
                return Err(format!("export background semaphore fd: {error:?}"));
            }
        };
        if let Err(error) = bind_context(&context, "bind CUDA context for background import") {
            unsafe { libc::close(fd) };
            wait_for_vulkan_idle_or_device_lost(&self.vulkan.device);
            return Err(error);
        }
        let mut external_semaphore = ptr::null_mut();
        let descriptor = sys::CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC {
            type_:
                sys::CUexternalSemaphoreHandleType_enum_CU_EXTERNAL_SEMAPHORE_HANDLE_TYPE_OPAQUE_FD,
            handle: sys::CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC_st__bindgen_ty_1 { fd },
            flags: 0,
            reserved: [0; 16],
        };
        if let Err(error) = cuda_check(
            unsafe { sys::cuImportExternalSemaphore(&mut external_semaphore, &descriptor) },
            "cuImportExternalSemaphore for background",
        ) {
            unsafe { libc::close(fd) };
            wait_for_vulkan_idle_or_device_lost(&self.vulkan.device);
            return Err(error);
        }
        self.import_buffer_to_cuda_inner(
            context,
            width,
            height,
            buffer,
            Some(PendingVulkanWait {
                stream,
                semaphore,
                external_semaphore: external_semaphore as usize,
            }),
        )
    }

    fn import_buffer_to_cuda_inner(
        &self,
        context: Arc<CudaContext>,
        width: u32,
        height: u32,
        buffer: ExportedBuffer,
        wait: Option<PendingVulkanWait>,
    ) -> Result<VisualFrame, String> {
        let imported = self.import_buffer_to_cuda_allocation(context, buffer, wait)?;
        self.visual_frame_from_imported(width, height, imported, None)
    }

    fn import_buffer_to_cuda_allocation(
        &self,
        context: Arc<CudaContext>,
        buffer: ExportedBuffer,
        wait: Option<PendingVulkanWait>,
    ) -> Result<ImportedVulkanFrame, String> {
        let fd_info = vk::MemoryGetFdInfoKHR::default()
            .memory(buffer.memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let fd = match unsafe { self.external_memory_fd.get_memory_fd(&fd_info) } {
            Ok(fd) => fd,
            Err(error) => {
                cleanup_failed_wait(&self.vulkan.device, &context, wait.as_ref());
                return Err(format!("export Vulkan generated memory fd: {error:?}"));
            }
        };
        if let Err(error) = bind_context(&context, "bind CUDA context for Vulkan generated import")
        {
            unsafe { libc::close(fd) };
            cleanup_failed_wait(&self.vulkan.device, &context, wait.as_ref());
            return Err(error);
        }
        let mut external_memory = ptr::null_mut();
        let handle = sys::CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1 { fd };
        let memory_desc = sys::CUDA_EXTERNAL_MEMORY_HANDLE_DESC {
            type_: sys::CUexternalMemoryHandleType_enum_CU_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD,
            handle,
            size: buffer.allocation_size,
            flags: 0,
            reserved: [0; 16],
        };
        if let Err(error) = cuda_check(
            unsafe { sys::cuImportExternalMemory(&mut external_memory, &memory_desc) },
            "cuImportExternalMemory",
        ) {
            unsafe {
                libc::close(fd);
            }
            cleanup_failed_wait(&self.vulkan.device, &context, wait.as_ref());
            return Err(error);
        }
        let mut ptr = 0;
        let buffer_desc = sys::CUDA_EXTERNAL_MEMORY_BUFFER_DESC {
            offset: 0,
            size: buffer.buffer_size,
            flags: 0,
            reserved: [0; 16],
        };
        if let Err(error) = cuda_check(
            unsafe {
                sys::cuExternalMemoryGetMappedBuffer(&mut ptr, external_memory, &buffer_desc)
            },
            "cuExternalMemoryGetMappedBuffer",
        ) {
            cleanup_failed_wait(&self.vulkan.device, &context, wait.as_ref());
            if let Err(cleanup_error) = cuda_check(
                unsafe { sys::cuDestroyExternalMemory(external_memory) },
                "cuDestroyExternalMemory",
            ) {
                tracing::error!(
                    "Could not clean up CUDA external memory after mapping failed: {cleanup_error}"
                );
                std::process::abort();
            }
            return Err(error);
        }
        if let Some(wait) = &wait {
            let semaphores = [wait.external_semaphore as sys::CUexternalSemaphore];
            let params: sys::CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS = unsafe { std::mem::zeroed() };
            if let Err(error) = cuda_check(
                unsafe {
                    sys::cuWaitExternalSemaphoresAsync(
                        semaphores.as_ptr(),
                        &params,
                        1,
                        wait.stream.cu_stream(),
                    )
                },
                "wait for background Vulkan semaphore",
            ) {
                cleanup_failed_wait(&self.vulkan.device, &context, Some(wait));
                if cuda_check(
                    unsafe { sys::cuMemFree_v2(ptr) },
                    "cuMemFree after background wait failure",
                )
                .and_then(|()| {
                    cuda_check(
                        unsafe { sys::cuDestroyExternalMemory(external_memory) },
                        "cuDestroyExternalMemory after background wait failure",
                    )
                })
                .is_err()
                {
                    std::process::abort();
                }
                return Err(error);
            }
        }
        let allocation_size = buffer.allocation_size;
        let imported_frames = IMPORTED_VULKAN_FRAMES.fetch_add(1, Ordering::AcqRel) + 1;
        let imported_bytes =
            IMPORTED_VULKAN_BYTES.fetch_add(allocation_size, Ordering::AcqRel) + allocation_size;
        shrimply_benchmarking::set_counter(
            "Generated Vulkan / CUDA frames retained",
            imported_frames,
        );
        shrimply_benchmarking::set_counter(
            "Generated Vulkan / CUDA bytes retained",
            imported_bytes,
        );
        Ok(ImportedVulkanFrame {
            context,
            ptr,
            external_memory: external_memory as usize,
            allocation_size,
            stream: wait.as_ref().map(|wait| wait.stream.clone()),
            external_semaphore: wait.as_ref().map(|wait| wait.external_semaphore),
            _semaphore: wait.map(|wait| wait.semaphore),
            _buffer: buffer,
        })
    }

    fn visual_frame_from_imported(
        &self,
        width: u32,
        height: u32,
        imported: ImportedVulkanFrame,
        pool: Option<Arc<Mutex<Vec<ImportedVulkanFrame>>>>,
    ) -> Result<VisualFrame, String> {
        let plane = VisualPlane {
            device_ptr: imported.ptr,
            pitch_bytes: width as usize * 4,
            width_bytes: width as usize * 4,
            height: height as usize,
        };
        let context = imported.context.clone();
        let owner: Box<dyn std::any::Any + Send + Sync> = match pool {
            Some(pool) => Box::new(PooledImportedVulkanFrame {
                imported: Some(imported),
                pool,
            }),
            None => Box::new(imported),
        };
        unsafe {
            VisualFrame::from_external_gpu(
                context,
                VisualFormat::Rgba8,
                width,
                height,
                &[plane],
                owner,
            )
        }
    }

    fn memory_type(
        &self,
        type_bits: u32,
        properties: vk::MemoryPropertyFlags,
    ) -> Result<u32, String> {
        let memory = unsafe {
            self.vulkan
                .instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        for index in 0..memory.memory_type_count {
            let supported = type_bits & (1 << index) != 0;
            let has_properties = memory.memory_types[index as usize]
                .property_flags
                .contains(properties);
            if supported && has_properties {
                return Ok(index);
            }
        }
        Err("no compatible Vulkan memory type for generated layer".to_string())
    }
}

pub(super) fn wait_for_vulkan_idle_or_device_lost(device: &ash::Device) {
    match unsafe { device.device_wait_idle() } {
        Ok(()) | Err(vk::Result::ERROR_DEVICE_LOST) => {}
        Err(error) => {
            tracing::error!(?error, "Could not make Vulkan device idle during cleanup");
            std::process::abort();
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum ImageCopySource {
    ColorAttachment,
    RayTracing,
    TransferSource,
}

impl Drop for GeneratedGpuRenderer {
    fn drop(&mut self) {
        self.skia.release_resources_and_abandon();
    }
}

struct VulkanDevice {
    _entry: Entry,
    instance: Instance,
    device: ash::Device,
    pipeline_cache: vk::PipelineCache,
    pipeline_cache_path: Option<PathBuf>,
}

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            if let Some(path) = &self.pipeline_cache_path {
                match self.device.get_pipeline_cache_data(self.pipeline_cache) {
                    Ok(data) => save_pipeline_cache(path, &data),
                    Err(error) => tracing::warn!(
                        ?error,
                        path = %path.display(),
                        "Could not read Vulkan pipeline cache"
                    ),
                }
            }
            self.device
                .destroy_pipeline_cache(self.pipeline_cache, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

fn pipeline_cache_path(uuid: [u8; vk::UUID_SIZE]) -> Option<PathBuf> {
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    let mut name = String::with_capacity(uuid.len() * 2);
    for byte in uuid {
        write!(name, "{byte:02x}").expect("write Vulkan pipeline cache UUID");
    }
    Some(
        root.join("shrimply")
            .join(format!("vulkan-pipelines-{name}.bin")),
    )
}

fn create_pipeline_cache(
    device: &ash::Device,
    path: Option<&Path>,
) -> Result<vk::PipelineCache, String> {
    let initial_data = path.and_then(|path| match fs::read(path) {
        Ok(data) => Some(data),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                "Could not read Vulkan pipeline cache: {error}"
            );
            None
        }
    });
    let info = initial_data
        .as_deref()
        .map_or_else(vk::PipelineCacheCreateInfo::default, |data| {
            vk::PipelineCacheCreateInfo::default().initial_data(data)
        });
    match unsafe { device.create_pipeline_cache(&info, None) } {
        Ok(cache) => {
            if let (Some(path), Some(data)) = (path, initial_data) {
                tracing::debug!(
                    bytes = data.len(),
                    path = %path.display(),
                    "Loaded Vulkan pipeline cache"
                );
            }
            Ok(cache)
        }
        Err(error) if initial_data.is_some() => {
            tracing::warn!(?error, "Ignoring invalid Vulkan pipeline cache");
            unsafe { device.create_pipeline_cache(&vk::PipelineCacheCreateInfo::default(), None) }
                .map_err(|error| format!("create empty Vulkan pipeline cache: {error:?}"))
        }
        Err(error) => Err(format!("create Vulkan pipeline cache: {error:?}")),
    }
}

fn save_pipeline_cache(path: &Path, data: &[u8]) {
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(error) = fs::create_dir_all(parent) {
        tracing::warn!(path = %path.display(), "Could not create Vulkan cache directory: {error}");
        return;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    if let Err(error) = fs::write(&temporary, data).and_then(|()| fs::rename(&temporary, path)) {
        tracing::warn!(path = %path.display(), "Could not save Vulkan pipeline cache: {error}");
        return;
    }
    tracing::debug!(
        bytes = data.len(),
        path = %path.display(),
        "Saved Vulkan pipeline cache"
    );
}

struct VulkanCommandPool {
    vulkan: Arc<VulkanDevice>,
    handle: vk::CommandPool,
}

impl Drop for VulkanCommandPool {
    fn drop(&mut self) {
        unsafe { self.vulkan.device.destroy_command_pool(self.handle, None) };
    }
}

struct VulkanCommandBuffer {
    vulkan: Arc<VulkanDevice>,
    pool: vk::CommandPool,
    handle: vk::CommandBuffer,
}

impl Drop for VulkanCommandBuffer {
    fn drop(&mut self) {
        unsafe {
            self.vulkan
                .device
                .free_command_buffers(self.pool, &[self.handle]);
        }
    }
}

struct VulkanFence {
    vulkan: Arc<VulkanDevice>,
    handle: vk::Fence,
}

struct VulkanSemaphore {
    vulkan: Arc<VulkanDevice>,
    handle: vk::Semaphore,
}

impl Drop for VulkanSemaphore {
    fn drop(&mut self) {
        unsafe { self.vulkan.device.destroy_semaphore(self.handle, None) };
    }
}

impl Drop for VulkanFence {
    fn drop(&mut self) {
        unsafe { self.vulkan.device.destroy_fence(self.handle, None) };
    }
}

struct VulkanImage {
    vulkan: Arc<VulkanDevice>,
    image: vk::Image,
    memory: vk::DeviceMemory,
}

impl Drop for VulkanImage {
    fn drop(&mut self) {
        unsafe {
            self.vulkan.device.destroy_image(self.image, None);
            self.vulkan.device.free_memory(self.memory, None);
        }
    }
}

struct ExportedBuffer {
    vulkan: Arc<VulkanDevice>,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    allocation_size: u64,
    buffer_size: u64,
}

struct PendingVulkanWait {
    stream: Arc<shrimply_cuda::CudaStream>,
    semaphore: VulkanSemaphore,
    external_semaphore: usize,
}

struct ImportedVulkanFrame {
    context: Arc<CudaContext>,
    ptr: sys::CUdeviceptr,
    external_memory: usize,
    allocation_size: u64,
    stream: Option<Arc<shrimply_cuda::CudaStream>>,
    external_semaphore: Option<usize>,
    _semaphore: Option<VulkanSemaphore>,
    _buffer: ExportedBuffer,
}

struct PooledImportedVulkanFrame {
    imported: Option<ImportedVulkanFrame>,
    pool: Arc<Mutex<Vec<ImportedVulkanFrame>>>,
}

impl Drop for PooledImportedVulkanFrame {
    fn drop(&mut self) {
        self.pool
            .lock()
            .expect("3DGS interop frame cache lock poisoned")
            .push(
                self.imported
                    .take()
                    .expect("pooled 3DGS interop frame was not returned"),
            );
    }
}

impl Drop for ImportedVulkanFrame {
    fn drop(&mut self) {
        drop_imported_memory(
            &self.context,
            self.ptr,
            self.external_memory,
            self.allocation_size,
            self.stream.as_deref(),
            self.external_semaphore,
        );
    }
}

fn cleanup_failed_wait(
    device: &ash::Device,
    context: &Arc<CudaContext>,
    wait: Option<&PendingVulkanWait>,
) {
    let Some(wait) = wait else {
        return;
    };
    wait_for_vulkan_idle_or_device_lost(device);
    if bind_context(
        context,
        "bind CUDA context for failed background import cleanup",
    )
    .is_err()
        || cuda_check(
            unsafe {
                sys::cuDestroyExternalSemaphore(wait.external_semaphore as sys::CUexternalSemaphore)
            },
            "cuDestroyExternalSemaphore after background import failure",
        )
        .is_err()
    {
        std::process::abort();
    }
}

fn drop_imported_memory(
    context: &Arc<CudaContext>,
    ptr: sys::CUdeviceptr,
    external_memory: usize,
    allocation_size: u64,
    stream: Option<&shrimply_cuda::CudaStream>,
    external_semaphore: Option<usize>,
) {
    if let Err(error) = bind_context(context, "bind CUDA context for Vulkan visual frame drop") {
        tracing::error!(
            cuda_ptr = ptr,
            external_memory,
            %error,
            "Could not bind CUDA context while dropping imported Vulkan frame",
        );
        std::process::abort();
    }
    if stream.is_some_and(|stream| stream.synchronize().is_err()) {
        std::process::abort();
    }
    if let Err(error) = cuda_check(unsafe { sys::cuMemFree_v2(ptr) }, "cuMemFree") {
        tracing::error!(
            cuda_ptr = ptr,
            external_memory,
            %error,
            "Could not free imported Vulkan frame mapping",
        );
        std::process::abort();
    }
    if let Err(error) = cuda_check(
        unsafe { sys::cuDestroyExternalMemory(external_memory as sys::CUexternalMemory) },
        "cuDestroyExternalMemory",
    ) {
        tracing::error!(
            cuda_ptr = ptr,
            external_memory,
            %error,
            "Could not destroy imported Vulkan external memory",
        );
        std::process::abort();
    }
    if let Some(external_semaphore) = external_semaphore
        && let Err(error) = cuda_check(
            unsafe {
                sys::cuDestroyExternalSemaphore(external_semaphore as sys::CUexternalSemaphore)
            },
            "cuDestroyExternalSemaphore",
        )
    {
        tracing::error!(external_semaphore, %error, "Could not destroy imported Vulkan external semaphore");
        std::process::abort();
    }
    let previous_frames = IMPORTED_VULKAN_FRAMES.fetch_sub(1, Ordering::AcqRel);
    let previous_bytes = IMPORTED_VULKAN_BYTES.fetch_sub(allocation_size, Ordering::AcqRel);
    assert!(
        previous_frames > 0,
        "imported Vulkan frame counter underflowed"
    );
    assert!(
        previous_bytes >= allocation_size,
        "imported Vulkan byte counter underflowed"
    );
    shrimply_benchmarking::set_counter(
        "Generated Vulkan / CUDA frames retained",
        previous_frames - 1,
    );
    shrimply_benchmarking::set_counter(
        "Generated Vulkan / CUDA bytes retained",
        previous_bytes - allocation_size,
    );
}

impl Drop for ExportedBuffer {
    fn drop(&mut self) {
        unsafe {
            self.vulkan.device.destroy_buffer(self.buffer, None);
            self.vulkan.device.free_memory(self.memory, None);
        }
    }
}

fn pick_physical_device(instance: &Instance) -> Result<(vk::PhysicalDevice, u32), String> {
    let devices = unsafe { instance.enumerate_physical_devices() }
        .map_err(|error| format!("enumerate Vulkan physical devices: {error:?}"))?;
    for physical_device in devices {
        let properties = unsafe { instance.get_physical_device_properties(physical_device) };
        if properties.vendor_id != 0x10de {
            continue;
        }
        let queues =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
        for (index, queue) in queues.iter().enumerate() {
            if queue.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                return Ok((physical_device, index as u32));
            }
        }
    }
    Err("no NVIDIA Vulkan graphics device found for CUDA generated rendering".to_string())
}

fn make_skia_context(
    entry: &Entry,
    instance: &Instance,
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    queue: vk::Queue,
    queue_family_index: u32,
) -> Result<gpu::DirectContext, String> {
    let get_proc = |get: skia_vk::GetProcOf| {
        let proc = match get {
            skia_vk::GetProcOf::Instance(instance_handle, name) => {
                let instance = vk::Instance::from_raw(instance_handle as u64);
                unsafe { entry.get_instance_proc_addr(instance, name) }
            }
            skia_vk::GetProcOf::Device(device_handle, name) => {
                let device = vk::Device::from_raw(device_handle as u64);
                unsafe { instance.get_device_proc_addr(device, name) }
            }
        };
        proc.map(|func| func as _).unwrap_or(ptr::null())
    };
    let backend = unsafe {
        skia_vk::BackendContext::new_builder(
            instance.handle().as_raw() as _,
            physical_device.as_raw() as _,
            device.handle().as_raw() as _,
            (queue.as_raw() as _, queue_family_index as usize),
            &get_proc,
            Some(skia_vk::Version::new(1, 1, 0)),
        )
        .with_extensions(
            &[],
            &[
                "VK_KHR_external_memory",
                extension_name(ash::khr::external_memory_fd::NAME),
            ],
        )
        .build()
    };
    direct_contexts::make_vulkan(&backend, None)
        .ok_or_else(|| "create Skia Vulkan direct context".to_string())
}

fn extension_name(name: &'static CStr) -> &'static str {
    name.to_str().unwrap_or_default()
}
