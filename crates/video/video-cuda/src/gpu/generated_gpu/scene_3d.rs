use hashbrown::HashMap;
use shrimply_asset::{Asset, AssetSnapshot};
use std::mem::{align_of, size_of, size_of_val};
use std::ptr;
use std::sync::Arc;

use ash::vk;
use shrimply_cuda::{CudaContext, CudaStream, LaunchConfig, sys};
use shrimply_math_color::Color;
use shrimply_visual_frame::{VisualFormat, VisualFrame, VisualPlane};

use crate::gpu::kernels::PreviewModule;

use super::{
    GeneratedGpuRenderer, ImageCopySource, VulkanCommandBuffer, VulkanDevice, VulkanFence,
    wait_for_vulkan_idle_or_device_lost,
};

const COLOR_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const ENVIRONMENT_FORMAT: vk::Format = vk::Format::R32G32B32A32_SFLOAT;
const OUTLINE_DISTANCE_FORMAT: vk::Format = vk::Format::R32G32_SFLOAT;
const RAY_TRACING_GROUP_COUNT: u32 = 7;
const PATH_TRACING_RECURSION_DEPTH: u32 = 6;

mod images;
mod pipeline;

pub(super) struct Scene3dRenderer {
    vulkan: Arc<VulkanDevice>,
    physical_device: vk::PhysicalDevice,
    queue: vk::Queue,
    command_pool: vk::CommandPool,
    acceleration: ash::khr::acceleration_structure::Device,
    ray_tracing: ash::khr::ray_tracing_pipeline::Device,
    scratch_alignment: u64,
    pipeline: pipeline::ScenePipeline,
    uniform: VulkanBuffer,
    fallback_environment: VulkanTexture,
    environments: HashMap<AssetSnapshot, VulkanTexture>,
    geometries: HashMap<shrimply_render_3d::GeometryIdentity, VulkanGeometry>,
    scenes: HashMap<shrimply_render_3d::SceneIdentity, VulkanScene>,
    transmission_background: Option<TransmissionBackgroundTexture>,
    target: Option<RenderTarget>,
    logged_output: bool,
}

impl GeneratedGpuRenderer {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_scene_3d(
        &mut self,
        context: Arc<CudaContext>,
        stream: &Arc<CudaStream>,
        preview: &PreviewModule,
        session: &mut shrimply_render_3d::ObjRenderSession,
        width: u32,
        height: u32,
        params: &shrimply_render_3d::SceneRenderParams,
        transmission_background: Option<&VisualFrame>,
    ) -> Result<VisualFrame, String> {
        let width = width.max(1);
        let height = height.max(1);
        let mut renderer = match self.scene_3d.take() {
            Some(renderer) => renderer,
            None => Scene3dRenderer::new(self)?,
        };
        let uploaded_background = if let Some(source) = transmission_background {
            let source_plane = source.plane(0).expect("RGBA background has no plane");
            let size = u64::from(source.width())
                .checked_mul(u64::from(source.height()))
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or_else(|| "3D transmission background dimensions overflow".to_string())?;
            let buffer = self.create_exported_buffer(size)?;
            let source_buffer = buffer.buffer;
            let mapping = self.import_buffer_to_cuda(
                context.clone(),
                source.width(),
                source.height(),
                buffer,
            )?;
            let mapping_plane = mapping.plane(0).expect("RGBA Vulkan mapping has no plane");
            super::super::bind_context(
                &context,
                "bind CUDA context for 3D transmission background copy",
            )?;
            let mut copy: sys::CUDA_MEMCPY2D = unsafe { std::mem::zeroed() };
            copy.srcMemoryType = match source.memory_kind(0) {
                Some(shrimply_gpu_memory::MemoryKind::Managed) => {
                    sys::CUmemorytype_enum_CU_MEMORYTYPE_UNIFIED
                }
                _ => sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE,
            };
            copy.srcDevice = source_plane.device_ptr;
            copy.srcPitch = source_plane.pitch_bytes;
            copy.dstMemoryType = sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE;
            copy.dstDevice = mapping_plane.device_ptr;
            copy.dstPitch = mapping_plane.pitch_bytes;
            copy.WidthInBytes = source.width() as usize * 4;
            copy.Height = source.height() as usize;
            super::super::cuda_check(
                unsafe { sys::cuMemcpy2DAsync_v2(&copy, stream.cu_stream()) },
                "copy 3D transmission background to Vulkan memory",
            )?;
            stream
                .synchronize()
                .map_err(|error| format!("synchronize 3D transmission background: {error:?}"))?;
            Some((
                TransmissionBackgroundSource {
                    buffer: source_buffer,
                    width: source.width(),
                    height: source.height(),
                },
                mapping,
            ))
        } else {
            None
        };
        let result = (|| {
            let rendered = renderer.render(
                session,
                width,
                height,
                params,
                uploaded_background.as_ref().map(|(source, _)| source),
            )?;
            if let SceneOutput::Denoise {
                beauty,
                albedo,
                normal,
                background,
                source,
            } = rendered
            {
                let pixel_count = width as usize * height as usize;
                let float_size = u64::try_from(pixel_count)
                    .ok()
                    .and_then(|pixels| pixels.checked_mul(size_of::<Color>() as u64))
                    .ok_or_else(|| "3D HDR output dimensions overflow".to_string())?;
                let buffer_size = float_size
                    .checked_mul(4)
                    .ok_or_else(|| "3D denoiser output dimensions overflow".to_string())?;
                let buffer = self.create_exported_buffer(buffer_size)?;
                self.copy_images_to_buffer(
                    width,
                    height,
                    &[
                        (beauty, 0),
                        (albedo, float_size),
                        (normal, float_size * 2),
                        (background, float_size * 3),
                    ],
                    buffer.buffer,
                    source,
                )?;
                let guides = self.import_buffer_to_cuda(context.clone(), width, height, buffer)?;
                let guide_ptr = guides
                    .plane(0)
                    .expect("RGBA OptiX guide frame has no plane")
                    .device_ptr;
                let mut output = shrimply_gpu_memory::global().allocate_buffer::<u32>(
                    stream,
                    pixel_count,
                    shrimply_gpu_memory::AllocationClass::Transient,
                    "tone-mapped 3D output",
                )?;

                if self
                    .optix_denoiser
                    .as_ref()
                    .is_none_or(|denoiser| !denoiser.matches(width, height))
                {
                    self.optix_denoiser = Some(shrimply_optix_denoiser::OptixDenoiser::new(
                        context.clone(),
                        stream,
                        width,
                        height,
                    )?);
                    tracing::info!(width, height, "Initialized guided OptiX AOV denoiser");
                }
                self.optix_denoiser
                    .as_mut()
                    .expect("OptiX denoiser was initialized")
                    .denoise(
                        stream,
                        shrimply_optix_denoiser::DenoiseInputs {
                            beauty: guide_ptr,
                            refraction: guide_ptr + float_size * 3,
                            albedo: guide_ptr + float_size,
                            normal: guide_ptr + float_size * 2,
                        },
                    )?;
                let launch_count = u32::try_from(pixel_count)
                    .map_err(|_| "3D output is too large for a CUDA launch".to_string())?;
                let toon_color_levels = matches!(
                    params.shading_model,
                    shrimply_render_3d::obj::ShadingModel::Toon
                )
                .then_some(params.toon_color_levels)
                .unwrap_or(0.0);
                unsafe {
                    preview
                        .tone_map_hdr(
                            stream,
                            LaunchConfig::for_num_elems(launch_count),
                            guide_ptr as usize as *const Color,
                            (guide_ptr + float_size * 3) as usize as *const Color,
                            &mut output,
                            toon_color_levels,
                        )
                        .map_err(|error| format!("tone-map OptiX output: {error:?}"))?;
                }
                stream
                    .synchronize()
                    .map_err(|error| format!("synchronize OptiX output: {error:?}"))?;
                let output = output
                    .cast_chunks::<u8>()
                    .map_err(|_| "3D output buffer cannot be viewed as bytes".to_string())?;
                let plane = VisualPlane {
                    device_ptr: output.cu_deviceptr(),
                    pitch_bytes: width as usize * size_of::<u32>(),
                    width_bytes: width as usize * size_of::<u32>(),
                    height: height as usize,
                };
                return unsafe {
                    VisualFrame::from_owned_gpu_buffers(
                        context,
                        VisualFormat::Rgba8,
                        width,
                        height,
                        &[plane],
                        vec![output],
                    )
                };
            }
            let SceneOutput::Rgba { image, source } = rendered else {
                unreachable!();
            };
            let size = u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or_else(|| "3D output dimensions overflow".to_string())?;
            let buffer = self.create_exported_buffer(size)?;
            self.copy_image_to_buffer(width, height, image, buffer.buffer, source)?;
            renderer.log_output_coverage(buffer.buffer, size, width, height)?;
            self.import_buffer_to_cuda(context, width, height, buffer)
        })();
        self.scene_3d = Some(renderer);
        result
    }
}

impl Scene3dRenderer {
    fn new(owner: &GeneratedGpuRenderer) -> Result<Self, String> {
        let vulkan = owner.vulkan.clone();
        let acceleration =
            ash::khr::acceleration_structure::Device::new(&vulkan.instance, &vulkan.device);
        let ray_tracing =
            ash::khr::ray_tracing_pipeline::Device::new(&vulkan.instance, &vulkan.device);
        let mut acceleration_properties =
            vk::PhysicalDeviceAccelerationStructurePropertiesKHR::default();
        let mut ray_properties = vk::PhysicalDeviceRayTracingPipelinePropertiesKHR::default();
        let mut device_properties = vk::PhysicalDeviceProperties2::default()
            .push_next(&mut acceleration_properties)
            .push_next(&mut ray_properties);
        unsafe {
            vulkan
                .instance
                .get_physical_device_properties2(owner.physical_device, &mut device_properties)
        };
        tracing::info!(
            max_recursion_depth = ray_properties.max_ray_recursion_depth,
            shader_handle_size = ray_properties.shader_group_handle_size,
            shader_base_alignment = ray_properties.shader_group_base_alignment,
            scratch_alignment =
                acceleration_properties.min_acceleration_structure_scratch_offset_alignment,
            "Initializing Vulkan ray-traced 3D renderer"
        );
        if ray_properties.max_ray_recursion_depth < PATH_TRACING_RECURSION_DEPTH {
            return Err(format!(
                "Vulkan device supports ray recursion depth {}, but Shrimply requires {}",
                ray_properties.max_ray_recursion_depth, PATH_TRACING_RECURSION_DEPTH
            ));
        }
        let pipeline =
            pipeline::ScenePipeline::new(vulkan.clone(), owner.physical_device, &ray_tracing)?;
        let uniform = VulkanBuffer::new(
            vulkan.clone(),
            owner.physical_device,
            size_of::<shrimply_render_3d::obj::SceneUniforms>() as u64,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let mut renderer = Self {
            vulkan,
            physical_device: owner.physical_device,
            queue: owner.queue,
            command_pool: owner.command_pool.handle,
            acceleration,
            ray_tracing,
            scratch_alignment: u64::from(
                acceleration_properties.min_acceleration_structure_scratch_offset_alignment,
            ),
            pipeline,
            uniform,
            fallback_environment: VulkanTexture::empty(owner.vulkan.clone()),
            environments: HashMap::new(),
            geometries: HashMap::new(),
            scenes: HashMap::new(),
            transmission_background: None,
            target: None,
            logged_output: false,
        };
        renderer.fallback_environment = renderer.upload_environment(1, 1, &[Color::BLACK])?;
        Ok(renderer)
    }

    fn render(
        &mut self,
        session: &mut shrimply_render_3d::ObjRenderSession,
        width: u32,
        height: u32,
        params: &shrimply_render_3d::SceneRenderParams,
        transmission_background: Option<&TransmissionBackgroundSource>,
    ) -> Result<SceneOutput, String> {
        let scene_identity = session.identity().clone();
        let geometry_identity = session.geometry_identity().clone();
        let geometry_changed = !self.geometries.contains_key(&geometry_identity);
        if geometry_changed {
            if session.positions().is_empty() {
                return Err("OBJ has no vertices to render".to_string());
            }
            let geometry = self.upload_geometry(session)?;
            self.geometries.insert(geometry_identity.clone(), geometry);
            self.geometries
                .retain(|identity, _| *identity == geometry_identity);
            self.scenes.clear();
        }
        if !self.scenes.contains_key(&scene_identity) {
            let geometry = self
                .geometries
                .get(&geometry_identity)
                .expect("uploaded 3D geometry is cached");
            let reusable = self.scenes.drain().next().map(|(_, scene)| scene);
            let scene = if let Some(mut scene) = reusable.filter(|scene| {
                scene.instance_count == session.acceleration_instances().len() as u32
                    && scene.materials.size == size_of_val(session.materials()) as u64
                    && scene.mesh_instances.size == size_of_val(session.mesh_instances()) as u64
            }) {
                self.update_scene(session, geometry, &mut scene)?;
                scene
            } else {
                self.upload_scene(session, geometry)?
            };
            self.scenes.insert(scene_identity.clone(), scene);
        }

        let environment_identity = (params.environment_source
            == shrimply_scene_3d::EnvironmentSource::Image)
            .then_some(params.environment_file.as_ref())
            .flatten()
            .map(Asset::snapshot)
            .transpose()
            .map_err(|error| error.to_string())?;
        if let Some(identity) = environment_identity.as_ref()
            && !self.environments.contains_key(identity)
        {
            let decoded = shrimply_render_3d::load_environment(identity.path())
                .map_err(|error| error.to_string())?;
            identity.verify_current()?;
            let texture =
                self.upload_environment(decoded.width, decoded.height, &decoded.pixels)?;
            self.environments.insert(identity.clone(), texture);
            self.environments
                .retain(|cached, _| cached.asset() != identity.asset() || cached == identity);
        }
        let denoise = params.optix_denoising
            && params.shading_model == shrimply_render_3d::obj::ShadingModel::Pbr;
        self.ensure_target(width, height, denoise)?;
        if let Some(background) = transmission_background {
            self.upload_transmission_background(background)?;
        }
        let environment = environment_identity
            .as_ref()
            .and_then(|identity| self.environments.get(identity))
            .unwrap_or(&self.fallback_environment);
        let mut uniforms = params
            .uniforms(width, height)
            .map_err(|error| error.to_string())?;
        uniforms.transmission_background[1] = u32::from(transmission_background.is_some()) as f32;
        let cache_key = RenderCacheKey {
            mesh: scene_identity.clone(),
            environment: environment_identity.clone(),
            uniforms,
        };
        let target = self.target.as_ref().expect("3D target was initialized");
        if transmission_background.is_none() && target.cache_key.as_ref() == Some(&cache_key) {
            tracing::debug!(width, height, "Reusing cached ray-traced 3D frame");
            return Ok(target.output(ImageCopySource::TransferSource));
        }
        self.uniform.write(std::slice::from_ref(&uniforms))?;
        let geometry = self
            .geometries
            .get(&geometry_identity)
            .expect("uploaded 3D geometry is cached");
        let scene = self
            .scenes
            .get(&scene_identity)
            .expect("uploaded 3D scene is cached");
        let target = self.target.as_ref().expect("3D target was initialized");
        let transmission_background = self
            .transmission_background
            .as_ref()
            .map(|background| &background.texture)
            .unwrap_or(&self.fallback_environment);
        self.update_descriptors(
            environment,
            transmission_background,
            geometry,
            scene,
            target,
        );
        if !target.rendered {
            tracing::info!(
                width,
                height,
                triangles = session.vertex_count() / 3,
                obj = %session.path().display(),
                "Dispatching Vulkan ray-traced 3D scene"
            );
        }

        let command = self.begin_commands("allocate 3D render command buffer")?;
        let pending_tlas = self
            .scenes
            .get(&scene_identity)
            .filter(|scene| scene.tlas_update_pending)
            .map(|scene| {
                (
                    scene.tlas.handle,
                    scene._instances.device_address(),
                    scene.instance_count,
                )
            });
        let _tlas_scratch = if let Some((tlas, instances, instance_count)) = pending_tlas {
            let scratch =
                self.record_tlas_update(command.handle, tlas, instances, instance_count)?;
            self.scenes
                .get_mut(&scene_identity)
                .expect("updated 3D scene is cached")
                .tlas_update_pending = false;
            Some(scratch)
        } else {
            None
        };
        let target = self.target.as_ref().expect("3D target was initialized");
        let transition_output = |image, copied| {
            transition_image(
                &self.vulkan.device,
                command.handle,
                image,
                if target.rendered {
                    if copied {
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL
                    } else {
                        vk::ImageLayout::GENERAL
                    }
                } else {
                    vk::ImageLayout::UNDEFINED
                },
                vk::ImageLayout::GENERAL,
                if target.rendered && copied {
                    vk::PipelineStageFlags::TRANSFER
                } else if target.rendered {
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR
                } else {
                    vk::PipelineStageFlags::TOP_OF_PIPE
                },
                vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                if target.rendered && copied {
                    vk::AccessFlags::TRANSFER_READ
                } else if target.rendered {
                    vk::AccessFlags::SHADER_WRITE
                } else {
                    vk::AccessFlags::empty()
                },
                vk::AccessFlags::SHADER_WRITE,
                0,
                1,
            );
        };
        transition_output(target.color.image, !target.denoise);
        transition_output(target.outline_guide.image, false);
        transition_output(target.outline_distance.image, false);
        if let Some(denoiser) = &target.denoiser {
            transition_output(denoiser.beauty.image, true);
            transition_output(denoiser.albedo.image, true);
            transition_output(denoiser.normal.image, true);
            transition_output(denoiser.background.image, true);
        }
        unsafe {
            self.vulkan.device.cmd_bind_pipeline(
                command.handle,
                vk::PipelineBindPoint::RAY_TRACING_KHR,
                self.pipeline.handle,
            );
            self.vulkan.device.cmd_bind_descriptor_sets(
                command.handle,
                vk::PipelineBindPoint::RAY_TRACING_KHR,
                self.pipeline.layout,
                shrimply_render_3d::obj::SCENE_DESCRIPTOR_SET,
                &[self.pipeline.descriptor_set],
                &[],
            );
            self.ray_tracing.cmd_trace_rays(
                command.handle,
                &self.pipeline.shader_binding_table().raygen,
                &self.pipeline.shader_binding_table().miss,
                &self.pipeline.shader_binding_table().hit,
                &vk::StridedDeviceAddressRegionKHR::default(),
                width,
                height,
                1,
            );
            if params.toon_outline_mode != shrimply_render_3d::obj::OutlineMode::Off
                && params.toon_outline_method
                    == shrimply_render_3d::obj::OutlineMethod::RegionBoundary
            {
                let barriers = [vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)];
                self.vulkan.device.cmd_pipeline_barrier(
                    command.handle,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::DependencyFlags::empty(),
                    &barriers,
                    &[],
                    &[],
                );
                self.ray_tracing.cmd_trace_rays(
                    command.handle,
                    &self.pipeline.shader_binding_table().outline_distance,
                    &self.pipeline.shader_binding_table().miss,
                    &self.pipeline.shader_binding_table().hit,
                    &vk::StridedDeviceAddressRegionKHR::default(),
                    width,
                    height,
                    1,
                );
                self.vulkan.device.cmd_pipeline_barrier(
                    command.handle,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::DependencyFlags::empty(),
                    &barriers,
                    &[],
                    &[],
                );
                self.ray_tracing.cmd_trace_rays(
                    command.handle,
                    &self.pipeline.shader_binding_table().outline,
                    &self.pipeline.shader_binding_table().miss,
                    &self.pipeline.shader_binding_table().hit,
                    &vk::StridedDeviceAddressRegionKHR::default(),
                    width,
                    height,
                    1,
                );
            }
        }
        self.submit_and_wait(command, "submit 3D render")?;
        let target = self.target.as_mut().expect("3D target was initialized");
        target.rendered = true;
        target.cache_key = Some(cache_key);
        Ok(target.output(ImageCopySource::RayTracing))
    }

    fn upload_geometry(
        &self,
        session: &shrimply_render_3d::ObjRenderSession,
    ) -> Result<VulkanGeometry, String> {
        tracing::info!(
            vertices = session.vertex_count(),
            triangles = session.vertex_count() / 3,
            obj = %session.path().display(),
            "Building reusable OBJ bottom-level acceleration structures"
        );
        let vertex_usage = vk::BufferUsageFlags::STORAGE_BUFFER
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
            | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
            | vk::BufferUsageFlags::TRANSFER_DST;
        let storage_usage = vk::BufferUsageFlags::STORAGE_BUFFER
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
            | vk::BufferUsageFlags::TRANSFER_DST;
        let buffer_sizes = [
            size_of_val(session.positions()) as u64,
            size_of_val(session.normals()) as u64,
            size_of_val(session.tangents()) as u64,
            size_of_val(session.tex_coords_0()) as u64,
            size_of_val(session.tex_coords_1()) as u64,
            size_of_val(session.colors()) as u64,
        ];
        let mut buffer_offsets = [0_u64; 6];
        let mut staging_size = 0_u64;
        for (offset, size) in buffer_offsets.iter_mut().zip(buffer_sizes) {
            *offset = staging_size;
            staging_size = staging_size
                .checked_add(size)
                .ok_or_else(|| "3D geometry upload size overflow".to_string())?;
        }
        let positions = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            buffer_sizes[0],
            vertex_usage,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let normals = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            buffer_sizes[1],
            storage_usage,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let tangents = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            buffer_sizes[2],
            storage_usage,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let tex_coords_0 = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            buffer_sizes[3],
            storage_usage,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let tex_coords_1 = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            buffer_sizes[4],
            storage_usage,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let colors = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            buffer_sizes[5],
            storage_usage,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let staging = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            staging_size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        staging.write_at(buffer_offsets[0], session.positions())?;
        staging.write_at(buffer_offsets[1], session.normals())?;
        staging.write_at(buffer_offsets[2], session.tangents())?;
        staging.write_at(buffer_offsets[3], session.tex_coords_0())?;
        staging.write_at(buffer_offsets[4], session.tex_coords_1())?;
        staging.write_at(buffer_offsets[5], session.colors())?;
        let command = self.begin_commands("allocate 3D geometry upload command")?;
        unsafe {
            for ((destination, size), source_offset) in [
                (&positions, buffer_sizes[0]),
                (&normals, buffer_sizes[1]),
                (&tangents, buffer_sizes[2]),
                (&tex_coords_0, buffer_sizes[3]),
                (&tex_coords_1, buffer_sizes[4]),
                (&colors, buffer_sizes[5]),
            ]
            .into_iter()
            .zip(buffer_offsets)
            {
                self.vulkan.device.cmd_copy_buffer(
                    command.handle,
                    staging.buffer,
                    destination.buffer,
                    &[vk::BufferCopy {
                        src_offset: source_offset,
                        dst_offset: 0,
                        size,
                    }],
                );
            }
            let barrier = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(
                    vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR | vk::AccessFlags::SHADER_READ,
                );
            self.vulkan.device.cmd_pipeline_barrier(
                command.handle,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR
                    | vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                vk::DependencyFlags::empty(),
                &[barrier],
                &[],
                &[],
            );
        }
        self.submit_and_wait(command, "upload static 3D geometry")?;
        let texture_atlas = self.upload_material_texture(session.texture_atlas())?;

        let mut geometry_sets = Vec::with_capacity(session.geometries().len());
        let mut primitive_count_sets = Vec::with_capacity(session.geometries().len());
        for geometry in session.geometries() {
            let mut blas_geometry = Vec::with_capacity(geometry.geometry_count as usize);
            let mut primitive_counts = Vec::with_capacity(geometry.geometry_count as usize);
            for slot in 0..geometry.geometry_count as usize {
                let primitive_count = geometry.primitive_counts[slot];
                let vertex_address = positions.device_address()
                    + u64::from(geometry.vertex_offsets[slot]) * size_of::<[f32; 4]>() as u64;
                let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
                    .vertex_format(vk::Format::R32G32B32_SFLOAT)
                    .vertex_data(vk::DeviceOrHostAddressConstKHR {
                        device_address: vertex_address,
                    })
                    .vertex_stride(size_of::<[f32; 4]>() as u64)
                    .max_vertex(primitive_count * 3 - 1)
                    .index_type(vk::IndexType::NONE_KHR);
                blas_geometry.push(
                    vk::AccelerationStructureGeometryKHR::default()
                        .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                        .geometry(vk::AccelerationStructureGeometryDataKHR { triangles })
                        .flags(if geometry.opaque[slot] {
                            vk::GeometryFlagsKHR::OPAQUE
                        } else {
                            vk::GeometryFlagsKHR::NO_DUPLICATE_ANY_HIT_INVOCATION
                        }),
                );
                primitive_counts.push(primitive_count);
            }
            geometry_sets.push(blas_geometry);
            primitive_count_sets.push(primitive_counts);
        }
        let blases = self.build_acceleration_structures(
            vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL,
            &geometry_sets,
            &primitive_count_sets,
            true,
        )?;

        Ok(VulkanGeometry {
            blases,
            positions,
            normals,
            tangents,
            tex_coords_0,
            tex_coords_1,
            colors,
            texture_atlas,
        })
    }

    fn upload_scene(
        &self,
        session: &shrimply_render_3d::ObjRenderSession,
        geometry: &VulkanGeometry,
    ) -> Result<VulkanScene, String> {
        let vk_instances = self.scene_instances(session, geometry)?;
        let instances = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            size_of_val(vk_instances.as_slice()) as u64,
            vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        instances.write(&vk_instances)?;
        let instance_data = vk::AccelerationStructureGeometryInstancesDataKHR::default()
            .array_of_pointers(false)
            .data(vk::DeviceOrHostAddressConstKHR {
                device_address: instances.device_address(),
            });
        let tlas_geometry = [vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::INSTANCES)
            .geometry(vk::AccelerationStructureGeometryDataKHR {
                instances: instance_data,
            })];
        let mut structures = self.build_acceleration_structures(
            vk::AccelerationStructureTypeKHR::TOP_LEVEL,
            &[tlas_geometry.to_vec()],
            &[vec![vk_instances.len() as u32]],
            false,
        )?;
        let tlas = structures.pop().expect("one TLAS was built");
        let materials = self.upload_storage_buffer(session.materials())?;
        let mesh_instances = self.upload_storage_buffer(session.mesh_instances())?;

        Ok(VulkanScene {
            tlas,
            _instances: instances,
            materials,
            mesh_instances,
            instance_count: vk_instances.len() as u32,
            tlas_update_pending: false,
        })
    }

    fn update_scene(
        &self,
        session: &shrimply_render_3d::ObjRenderSession,
        geometry: &VulkanGeometry,
        scene: &mut VulkanScene,
    ) -> Result<(), String> {
        let vk_instances = self.scene_instances(session, geometry)?;
        scene._instances.write(&vk_instances)?;
        scene.materials.write(session.materials())?;
        scene.mesh_instances.write(session.mesh_instances())?;
        scene.tlas_update_pending = true;
        Ok(())
    }

    fn record_tlas_update(
        &self,
        command: vk::CommandBuffer,
        tlas: vk::AccelerationStructureKHR,
        instances: vk::DeviceAddress,
        instance_count: u32,
    ) -> Result<VulkanBuffer, String> {
        let instance_data = vk::AccelerationStructureGeometryInstancesDataKHR::default()
            .array_of_pointers(false)
            .data(vk::DeviceOrHostAddressConstKHR {
                device_address: instances,
            });
        let geometries = [vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::INSTANCES)
            .geometry(vk::AccelerationStructureGeometryDataKHR {
                instances: instance_data,
            })];
        let primitive_counts = [instance_count];
        let base = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .flags(
                vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE
                    | vk::BuildAccelerationStructureFlagsKHR::ALLOW_UPDATE,
            )
            .mode(vk::BuildAccelerationStructureModeKHR::UPDATE)
            .src_acceleration_structure(tlas)
            .dst_acceleration_structure(tlas)
            .geometries(&geometries);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            self.acceleration.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &base,
                &primitive_counts,
                &mut sizes,
            )
        };
        let scratch_size = sizes
            .update_scratch_size
            .checked_add(self.scratch_alignment - 1)
            .ok_or_else(|| "Vulkan TLAS update scratch size overflow".to_string())?;
        let scratch = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            scratch_size,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let build = vk::AccelerationStructureBuildGeometryInfoKHR {
            scratch_data: vk::DeviceOrHostAddressKHR {
                device_address: scratch
                    .device_address()
                    .next_multiple_of(self.scratch_alignment),
            },
            ..base
        };
        let ranges = [vk::AccelerationStructureBuildRangeInfoKHR {
            primitive_count: primitive_counts[0],
            ..Default::default()
        }];
        unsafe {
            self.acceleration
                .cmd_build_acceleration_structures(command, &[build], &[&ranges]);
            let barrier = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR);
            self.vulkan.device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                vk::DependencyFlags::empty(),
                &[barrier],
                &[],
                &[],
            );
        }
        Ok(scratch)
    }

    fn scene_instances(
        &self,
        session: &shrimply_render_3d::ObjRenderSession,
        geometry: &VulkanGeometry,
    ) -> Result<Vec<vk::AccelerationStructureInstanceKHR>, String> {
        const MAX_INSTANCE_COUNT: usize = 1 << 24;
        if session.acceleration_instances().len() > MAX_INSTANCE_COUNT {
            return Err("3D scene exceeds Vulkan's instance custom-index limit".to_string());
        }
        session
            .acceleration_instances()
            .iter()
            .enumerate()
            .map(|(instance_index, instance)| {
                let blas = geometry
                    .blases
                    .get(instance.geometry_index as usize)
                    .ok_or_else(|| "3D instance references a missing BLAS".to_string())?;
                Ok(vk::AccelerationStructureInstanceKHR {
                    transform: vk::TransformMatrixKHR {
                        matrix: instance.transform,
                    },
                    instance_custom_index_and_mask: vk::Packed24_8::new(
                        instance_index as u32,
                        u8::MAX,
                    ),
                    instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
                        0,
                        vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE
                            .as_raw()
                            .try_into()
                            .expect("Vulkan instance flags fit in eight bits"),
                    ),
                    acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                        device_handle: blas.device_address(),
                    },
                })
            })
            .collect()
    }

    fn upload_storage_buffer<T>(&self, values: &[T]) -> Result<VulkanBuffer, String> {
        let buffer = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            size_of_val(values) as u64,
            vk::BufferUsageFlags::STORAGE_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        buffer.write(values)?;
        Ok(buffer)
    }

    fn log_output_coverage(
        &mut self,
        source: vk::Buffer,
        size: u64,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        if self.logged_output {
            return Ok(());
        }
        let staging = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            size,
            vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let command = self.begin_commands("allocate 3D output diagnostic command buffer")?;
        let barrier = vk::BufferMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .buffer(source)
            .offset(0)
            .size(size);
        unsafe {
            self.vulkan.device.cmd_pipeline_barrier(
                command.handle,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[barrier],
                &[],
            );
            self.vulkan.device.cmd_copy_buffer(
                command.handle,
                source,
                staging.buffer,
                &[vk::BufferCopy {
                    src_offset: 0,
                    dst_offset: 0,
                    size,
                }],
            );
        }
        self.submit_and_wait(command, "read first 3D ray-traced output")?;
        let bytes = staging.read()?;
        let visible = bytes.chunks_exact(4).filter(|pixel| pixel[3] != 0).count();
        self.logged_output = true;
        if visible == 0 {
            tracing::warn!(width, height, "Ray-traced 3D output contains no hit pixels");
        } else {
            tracing::info!(
                width,
                height,
                visible_pixels = visible,
                "Ray-traced 3D output contains visible pixels"
            );
        }
        Ok(())
    }

    fn build_acceleration_structures(
        &self,
        ty: vk::AccelerationStructureTypeKHR,
        geometry_sets: &[Vec<vk::AccelerationStructureGeometryKHR>],
        primitive_count_sets: &[Vec<u32>],
        compact: bool,
    ) -> Result<Vec<VulkanAccelerationStructure>, String> {
        if geometry_sets.is_empty() || geometry_sets.len() != primitive_count_sets.len() {
            return Err("invalid Vulkan acceleration structure build batch".to_string());
        }
        let flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE
            | if compact {
                vk::BuildAccelerationStructureFlagsKHR::ALLOW_COMPACTION
            } else {
                vk::BuildAccelerationStructureFlagsKHR::empty()
            }
            | if ty == vk::AccelerationStructureTypeKHR::TOP_LEVEL {
                vk::BuildAccelerationStructureFlagsKHR::ALLOW_UPDATE
            } else {
                vk::BuildAccelerationStructureFlagsKHR::empty()
            };
        let bases = geometry_sets
            .iter()
            .map(|geometries| {
                vk::AccelerationStructureBuildGeometryInfoKHR::default()
                    .ty(ty)
                    .flags(flags)
                    .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                    .geometries(geometries)
            })
            .collect::<Vec<_>>();
        let sizes = bases
            .iter()
            .zip(primitive_count_sets)
            .map(|(base, primitive_counts)| {
                let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
                unsafe {
                    self.acceleration.get_acceleration_structure_build_sizes(
                        vk::AccelerationStructureBuildTypeKHR::DEVICE,
                        base,
                        primitive_counts,
                        &mut sizes,
                    )
                };
                sizes
            })
            .collect::<Vec<_>>();
        let mut structures = sizes
            .iter()
            .map(|size| self.allocate_acceleration_structure(ty, size.acceleration_structure_size))
            .collect::<Result<Vec<_>, _>>()?;
        let scratch_size = sizes.iter().try_fold(0_u64, |total, size| {
            total.checked_add(
                size.build_scratch_size
                    .next_multiple_of(self.scratch_alignment),
            )
        });
        let scratch_size = scratch_size
            .ok_or_else(|| "Vulkan acceleration structure scratch size overflow".to_string())?;
        let scratch_allocation_size = scratch_size
            .checked_add(self.scratch_alignment - 1)
            .ok_or_else(|| "Vulkan acceleration structure scratch size overflow".to_string())?;
        let scratch = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            scratch_allocation_size,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let scratch_address = scratch
            .device_address()
            .next_multiple_of(self.scratch_alignment);
        let mut scratch_offset = 0_u64;
        let builds = bases
            .iter()
            .zip(&structures)
            .zip(&sizes)
            .map(|((base, structure), size)| {
                let build = vk::AccelerationStructureBuildGeometryInfoKHR {
                    dst_acceleration_structure: structure.handle,
                    scratch_data: vk::DeviceOrHostAddressKHR {
                        device_address: scratch_address + scratch_offset,
                    },
                    ..*base
                };
                scratch_offset += size
                    .build_scratch_size
                    .next_multiple_of(self.scratch_alignment);
                build
            })
            .collect::<Vec<_>>();
        let ranges = primitive_count_sets
            .iter()
            .map(|primitive_counts| {
                primitive_counts
                    .iter()
                    .map(|count| vk::AccelerationStructureBuildRangeInfoKHR {
                        primitive_count: *count,
                        ..Default::default()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let range_slices = ranges.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let query_pool = if compact {
            let info = vk::QueryPoolCreateInfo::default()
                .query_type(vk::QueryType::ACCELERATION_STRUCTURE_COMPACTED_SIZE_KHR)
                .query_count(structures.len() as u32);
            Some(
                unsafe { self.vulkan.device.create_query_pool(&info, None) }
                    .map_err(|error| format!("create BLAS compaction query pool: {error:?}"))?,
            )
        } else {
            None
        };
        let command = match self.begin_commands("allocate acceleration structure build command") {
            Ok(command) => command,
            Err(error) => {
                if let Some(query_pool) = query_pool {
                    unsafe { self.vulkan.device.destroy_query_pool(query_pool, None) };
                }
                return Err(error);
            }
        };
        unsafe {
            if let Some(query_pool) = query_pool {
                self.vulkan.device.cmd_reset_query_pool(
                    command.handle,
                    query_pool,
                    0,
                    structures.len() as u32,
                );
            }
            self.acceleration.cmd_build_acceleration_structures(
                command.handle,
                &builds,
                &range_slices,
            );
            if let Some(query_pool) = query_pool {
                let barrier = vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                    .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR);
                self.vulkan.device.cmd_pipeline_barrier(
                    command.handle,
                    vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                    vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                    vk::DependencyFlags::empty(),
                    &[barrier],
                    &[],
                    &[],
                );
                let handles = structures
                    .iter()
                    .map(|structure| structure.handle)
                    .collect::<Vec<_>>();
                self.acceleration
                    .cmd_write_acceleration_structures_properties(
                        command.handle,
                        &handles,
                        vk::QueryType::ACCELERATION_STRUCTURE_COMPACTED_SIZE_KHR,
                        query_pool,
                        0,
                    );
            }
        }
        let submitted = self.submit_and_wait(command, "build acceleration structures");
        if let Err(error) = submitted {
            if let Some(query_pool) = query_pool {
                unsafe { self.vulkan.device.destroy_query_pool(query_pool, None) };
            }
            return Err(error);
        }
        if let Some(query_pool) = query_pool {
            let mut compact_sizes = vec![0_u64; structures.len()];
            let queried = unsafe {
                self.vulkan.device.get_query_pool_results(
                    query_pool,
                    0,
                    &mut compact_sizes,
                    vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
                )
            };
            unsafe { self.vulkan.device.destroy_query_pool(query_pool, None) };
            queried.map_err(|error| format!("read BLAS compaction sizes: {error:?}"))?;
            let compacted = compact_sizes
                .iter()
                .map(|size| self.allocate_acceleration_structure(ty, *size))
                .collect::<Result<Vec<_>, _>>()?;
            let command = self.begin_commands("allocate BLAS compaction command")?;
            unsafe {
                for (source, destination) in structures.iter().zip(&compacted) {
                    let copy = vk::CopyAccelerationStructureInfoKHR::default()
                        .src(source.handle)
                        .dst(destination.handle)
                        .mode(vk::CopyAccelerationStructureModeKHR::COMPACT);
                    self.acceleration
                        .cmd_copy_acceleration_structure(command.handle, &copy);
                }
            }
            self.submit_and_wait(command, "compact bottom-level acceleration structures")?;
            structures = compacted;
            tracing::debug!(
                count = structures.len(),
                uncompacted_bytes = sizes
                    .iter()
                    .map(|size| size.acceleration_structure_size)
                    .sum::<u64>(),
                compacted_bytes = compact_sizes.iter().sum::<u64>(),
                "Compacted Vulkan bottom-level acceleration structures"
            );
        }
        tracing::debug!(
            ?ty,
            count = structures.len(),
            storage_bytes = sizes
                .iter()
                .map(|size| size.acceleration_structure_size)
                .sum::<u64>(),
            scratch_bytes = scratch_size,
            "Built Vulkan acceleration structures"
        );
        Ok(structures)
    }

    fn allocate_acceleration_structure(
        &self,
        ty: vk::AccelerationStructureTypeKHR,
        size: u64,
    ) -> Result<VulkanAccelerationStructure, String> {
        if size == 0 {
            return Err("Vulkan reported a zero-sized acceleration structure".to_string());
        }
        let storage = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            size,
            vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let create = vk::AccelerationStructureCreateInfoKHR::default()
            .buffer(storage.buffer)
            .size(size)
            .ty(ty);
        let handle = unsafe {
            self.acceleration
                .create_acceleration_structure(&create, None)
        }
        .map_err(|error| format!("create Vulkan acceleration structure: {error:?}"))?;
        Ok(VulkanAccelerationStructure {
            loader: self.acceleration.clone(),
            handle,
            _storage: storage,
        })
    }

    fn update_descriptors(
        &self,
        environment: &VulkanTexture,
        transmission_background: &VulkanTexture,
        geometry: &VulkanGeometry,
        scene: &VulkanScene,
        target: &RenderTarget,
    ) {
        let buffer = [vk::DescriptorBufferInfo {
            buffer: self.uniform.buffer,
            offset: 0,
            range: size_of::<shrimply_render_3d::obj::SceneUniforms>() as u64,
        }];
        let image = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: environment.view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        }];
        let sampler = [vk::DescriptorImageInfo {
            sampler: self.pipeline.environment_sampler,
            image_view: vk::ImageView::null(),
            image_layout: vk::ImageLayout::UNDEFINED,
        }];
        let output = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: target.color.view,
            image_layout: vk::ImageLayout::GENERAL,
        }];
        let denoiser_images = target.denoiser_images();
        let denoiser_beauty = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: denoiser_images[0].view,
            image_layout: vk::ImageLayout::GENERAL,
        }];
        let denoiser_albedo = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: denoiser_images[1].view,
            image_layout: vk::ImageLayout::GENERAL,
        }];
        let denoiser_normal = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: denoiser_images[2].view,
            image_layout: vk::ImageLayout::GENERAL,
        }];
        let denoiser_background = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: denoiser_images[3].view,
            image_layout: vk::ImageLayout::GENERAL,
        }];
        let outline_guide = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: target.outline_guide.view,
            image_layout: vk::ImageLayout::GENERAL,
        }];
        let outline_distance = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: target.outline_distance.view,
            image_layout: vk::ImageLayout::GENERAL,
        }];
        let positions = [vk::DescriptorBufferInfo {
            buffer: geometry.positions.buffer,
            offset: 0,
            range: geometry.positions.size,
        }];
        let normals = [vk::DescriptorBufferInfo {
            buffer: geometry.normals.buffer,
            offset: 0,
            range: geometry.normals.size,
        }];
        let tangents = [vk::DescriptorBufferInfo {
            buffer: geometry.tangents.buffer,
            offset: 0,
            range: geometry.tangents.size,
        }];
        let tex_coords_0 = [vk::DescriptorBufferInfo {
            buffer: geometry.tex_coords_0.buffer,
            offset: 0,
            range: geometry.tex_coords_0.size,
        }];
        let tex_coords_1 = [vk::DescriptorBufferInfo {
            buffer: geometry.tex_coords_1.buffer,
            offset: 0,
            range: geometry.tex_coords_1.size,
        }];
        let colors = [vk::DescriptorBufferInfo {
            buffer: geometry.colors.buffer,
            offset: 0,
            range: geometry.colors.size,
        }];
        let materials = [vk::DescriptorBufferInfo {
            buffer: scene.materials.buffer,
            offset: 0,
            range: scene.materials.size,
        }];
        let mesh_instances = [vk::DescriptorBufferInfo {
            buffer: scene.mesh_instances.buffer,
            offset: 0,
            range: scene.mesh_instances.size,
        }];
        let material_texture = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: geometry.texture_atlas.view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        }];
        let material_sampler = [vk::DescriptorImageInfo {
            sampler: self.pipeline.material_sampler,
            image_view: vk::ImageView::null(),
            image_layout: vk::ImageLayout::UNDEFINED,
        }];
        let transmission_background = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: transmission_background.view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        }];
        let structures = [scene.tlas.handle];
        let mut acceleration = vk::WriteDescriptorSetAccelerationStructureKHR::default()
            .acceleration_structures(&structures);
        let mut acceleration_write = vk::WriteDescriptorSet::default()
            .dst_set(self.pipeline.descriptor_set)
            .dst_binding(shrimply_render_3d::obj::SCENE_ACCELERATION_BINDING)
            .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .push_next(&mut acceleration);
        acceleration_write.descriptor_count = structures.len() as u32;
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::SCENE_BINDING)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buffer),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::ENVIRONMENT_TEXTURE_BINDING)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::ENVIRONMENT_SAMPLER_BINDING)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&sampler),
            acceleration_write,
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::OUTPUT_IMAGE_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&output),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::POSITIONS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&positions),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::NORMALS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&normals),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::DENOISER_BEAUTY_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&denoiser_beauty),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::DENOISER_ALBEDO_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&denoiser_albedo),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::DENOISER_NORMAL_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&denoiser_normal),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::DENOISER_BACKGROUND_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&denoiser_background),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::TANGENTS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&tangents),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::TEX_COORDS_0_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&tex_coords_0),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::TEX_COORDS_1_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&tex_coords_1),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::VERTEX_COLORS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&colors),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::MESH_MATERIALS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&materials),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::MATERIAL_TEXTURE_BINDING)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&material_texture),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::MATERIAL_SAMPLER_BINDING)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&material_sampler),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::TRANSMISSION_BACKGROUND_TEXTURE_BINDING)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&transmission_background),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::OUTLINE_GUIDE_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&outline_guide),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::OUTLINE_DISTANCE_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&outline_distance),
            vk::WriteDescriptorSet::default()
                .dst_set(self.pipeline.descriptor_set)
                .dst_binding(shrimply_render_3d::obj::MESH_INSTANCES_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&mesh_instances),
        ];
        unsafe { self.vulkan.device.update_descriptor_sets(&writes, &[]) };
    }

    fn ensure_target(&mut self, width: u32, height: u32, denoise: bool) -> Result<(), String> {
        if self.target.as_ref().is_some_and(|target| {
            target.width == width && target.height == height && target.denoise == denoise
        }) {
            return Ok(());
        }
        self.target = Some(RenderTarget::new(
            self.vulkan.clone(),
            self.physical_device,
            width,
            height,
            denoise,
        )?);
        Ok(())
    }

    fn upload_transmission_background(
        &mut self,
        source: &TransmissionBackgroundSource,
    ) -> Result<(), String> {
        if source.width == 0 || source.height == 0 {
            return Err("3D transmission background dimensions must be nonzero".to_string());
        }
        let mip_levels = 32 - source.width.max(source.height).leading_zeros();
        let recreate = self
            .transmission_background
            .as_ref()
            .is_none_or(|background| {
                background.width != source.width || background.height != source.height
            });
        if recreate {
            self.transmission_background = Some(TransmissionBackgroundTexture {
                texture: VulkanTexture::new(
                    self.vulkan.clone(),
                    self.physical_device,
                    source.width,
                    source.height,
                    mip_levels,
                    COLOR_FORMAT,
                    vk::ImageUsageFlags::TRANSFER_SRC
                        | vk::ImageUsageFlags::TRANSFER_DST
                        | vk::ImageUsageFlags::SAMPLED,
                    vk::ImageAspectFlags::COLOR,
                )?,
                width: source.width,
                height: source.height,
                initialized: false,
            });
        }
        let background = self
            .transmission_background
            .as_ref()
            .expect("3D transmission background was initialized");
        let command = self.begin_commands("allocate 3D transmission background command buffer")?;
        transition_image(
            &self.vulkan.device,
            command.handle,
            background.texture.image,
            if background.initialized {
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
            } else {
                vk::ImageLayout::UNDEFINED
            },
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            if background.initialized {
                vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR
            } else {
                vk::PipelineStageFlags::TOP_OF_PIPE
            },
            vk::PipelineStageFlags::TRANSFER,
            if background.initialized {
                vk::AccessFlags::SHADER_READ
            } else {
                vk::AccessFlags::empty()
            },
            vk::AccessFlags::TRANSFER_WRITE,
            0,
            mip_levels,
        );
        let copy = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(vk::Extent3D {
                width: source.width,
                height: source.height,
                depth: 1,
            });
        unsafe {
            self.vulkan.device.cmd_copy_buffer_to_image(
                command.handle,
                source.buffer,
                background.texture.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[copy],
            );
        }
        let mut mip_width = source.width as i32;
        let mut mip_height = source.height as i32;
        for mip in 1..mip_levels {
            transition_image(
                &self.vulkan.device,
                command.handle,
                background.texture.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::TRANSFER_READ,
                mip - 1,
                1,
            );
            let next_width = (mip_width / 2).max(1);
            let next_height = (mip_height / 2).max(1);
            let blit = vk::ImageBlit::default()
                .src_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: mip - 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .src_offsets([
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D {
                        x: mip_width,
                        y: mip_height,
                        z: 1,
                    },
                ])
                .dst_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: mip,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .dst_offsets([
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D {
                        x: next_width,
                        y: next_height,
                        z: 1,
                    },
                ]);
            unsafe {
                self.vulkan.device.cmd_blit_image(
                    command.handle,
                    background.texture.image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    background.texture.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[blit],
                    vk::Filter::LINEAR,
                );
            }
            mip_width = next_width;
            mip_height = next_height;
        }
        if mip_levels > 1 {
            transition_image(
                &self.vulkan.device,
                command.handle,
                background.texture.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                vk::AccessFlags::TRANSFER_READ,
                vk::AccessFlags::SHADER_READ,
                0,
                mip_levels - 1,
            );
        }
        transition_image(
            &self.vulkan.device,
            command.handle,
            background.texture.image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            mip_levels - 1,
            1,
        );
        self.submit_and_wait(command, "upload 3D transmission background")?;
        self.transmission_background
            .as_mut()
            .expect("3D transmission background remains initialized")
            .initialized = true;
        Ok(())
    }

    fn begin_commands(&self, operation: &str) -> Result<VulkanCommandBuffer, String> {
        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let handle = unsafe { self.vulkan.device.allocate_command_buffers(&allocate) }
            .map_err(|error| format!("{operation}: {error:?}"))?[0];
        let command = VulkanCommandBuffer {
            vulkan: self.vulkan.clone(),
            pool: self.command_pool,
            handle,
        };
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe { self.vulkan.device.begin_command_buffer(handle, &begin) }
            .map_err(|error| format!("begin Vulkan command buffer: {error:?}"))?;
        Ok(command)
    }

    fn submit_and_wait(&self, command: VulkanCommandBuffer, operation: &str) -> Result<(), String> {
        unsafe { self.vulkan.device.end_command_buffer(command.handle) }
            .map_err(|error| format!("end {operation}: {error:?}"))?;
        let fence_handle = unsafe {
            self.vulkan
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .map_err(|error| format!("create {operation} fence: {error:?}"))?;
        let fence = VulkanFence {
            vulkan: self.vulkan.clone(),
            handle: fence_handle,
        };
        let commands = [command.handle];
        let submit = vk::SubmitInfo::default().command_buffers(&commands);
        unsafe {
            self.vulkan
                .device
                .queue_submit(self.queue, &[submit], fence.handle)
        }
        .map_err(|error| format!("{operation}: {error:?}"))?;
        if let Err(error) = unsafe {
            self.vulkan
                .device
                .wait_for_fences(&[fence.handle], true, u64::MAX)
        } {
            wait_for_vulkan_idle_or_device_lost(&self.vulkan.device);
            return Err(format!("wait for {operation}: {error:?}"));
        }
        Ok(())
    }
}

impl Drop for Scene3dRenderer {
    fn drop(&mut self) {
        wait_for_vulkan_idle_or_device_lost(&self.vulkan.device);
        self.target.take();
    }
}

include!("scene_3d/resources.rs");

#[allow(clippy::too_many_arguments)]
fn transition_image(
    device: &ash::Device,
    command: vk::CommandBuffer,
    image: vk::Image,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
    src_stage: vk::PipelineStageFlags,
    dst_stage: vk::PipelineStageFlags,
    src_access: vk::AccessFlags,
    dst_access: vk::AccessFlags,
    base_mip: u32,
    mip_count: u32,
) {
    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(old_layout)
        .new_layout(new_layout)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: base_mip,
            level_count: mip_count,
            base_array_layer: 0,
            layer_count: 1,
        });
    unsafe {
        device.cmd_pipeline_barrier(
            command,
            src_stage,
            dst_stage,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
    }
}

fn memory_type(
    vulkan: &VulkanDevice,
    physical_device: vk::PhysicalDevice,
    type_bits: u32,
    properties: vk::MemoryPropertyFlags,
) -> Result<u32, String> {
    let memory = unsafe {
        vulkan
            .instance
            .get_physical_device_memory_properties(physical_device)
    };
    for index in 0..memory.memory_type_count {
        if type_bits & (1 << index) != 0
            && memory.memory_types[index as usize]
                .property_flags
                .contains(properties)
        {
            return Ok(index);
        }
    }
    Err("no compatible Vulkan memory type for 3D rendering".to_string())
}

const _: () = assert!(align_of::<shrimply_render_3d::obj::SceneUniforms>() >= 16);
