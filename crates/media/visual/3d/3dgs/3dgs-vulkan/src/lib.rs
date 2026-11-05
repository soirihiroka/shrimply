use std::{
    mem::{size_of, size_of_val},
    time::Instant,
};

use ash::vk;
use rayon::prelude::*;
use shrimply_asset::AssetSnapshot;

use shrimply_3dgs_core::{RenderParams, RenderSession, shader};

const COLOR_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const ACCUMULATION_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;
const SORT_THREADS: u32 = 256;
const RADIX_BITS: u32 = 4;
const RADIX_PASSES: u32 = u32::BITS / RADIX_BITS;
const RADIX_SIZE: u64 = 1 << RADIX_BITS;
const DRAW_INDIRECT_SIZE: u64 = size_of::<u32>() as u64 * 4;

pub struct RenderContext {
    pub device: ash::Device,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub queue: vk::Queue,
    pub command_pool: vk::CommandPool,
    pub pipeline_cache: vk::PipelineCache,
}

pub struct Renderer {
    context: RenderContext,
    handles: Handles,
    uniform: Buffer,
    uploaded: Option<UploadedCloud>,
    target: Option<Target>,
}

struct UploadedCloud {
    identity: AssetSnapshot,
    gaussians: Buffer,
    higher_order: Buffer,
    sort_keys: Buffer,
    sorted_indices: Buffer,
    scratch_keys: Buffer,
    scratch_indices: Buffer,
    sort_group_offsets: Buffer,
    draw_indirect: Buffer,
    count: u32,
    group_count: u32,
}

impl Renderer {
    pub fn new(context: RenderContext) -> Result<Self, String> {
        let mut handles = Handles::new(context.device.clone());
        handles.accumulation_render_pass = create_accumulation_render_pass(&context.device)?;
        handles.resolve_render_pass = create_resolve_render_pass(&context.device)?;
        handles.descriptor_set_layout = create_descriptor_set_layout(&context.device)?;
        handles.pipeline_layout =
            create_pipeline_layout(&context.device, handles.descriptor_set_layout)?;
        handles.accumulation_pipeline = create_pipeline(
            &context.device,
            context.pipeline_cache,
            handles.accumulation_render_pass,
            handles.pipeline_layout,
            shader::GAUSSIAN_VERTEX_ENTRY_POINT,
            shader::GAUSSIAN_FRAGMENT_ENTRY_POINT,
            true,
        )?;
        handles.resolve_pipeline = create_pipeline(
            &context.device,
            context.pipeline_cache,
            handles.resolve_render_pass,
            handles.pipeline_layout,
            shader::RESOLVE_VERTEX_ENTRY_POINT,
            shader::RESOLVE_FRAGMENT_ENTRY_POINT,
            false,
        )?;
        handles.prepare_sort_pipeline = create_compute_pipeline(
            &context.device,
            context.pipeline_cache,
            handles.pipeline_layout,
            shader::PREPARE_DEPTH_SORT_ENTRY_POINT,
        )?;
        handles.radix_histogram_pipeline = create_compute_pipeline(
            &context.device,
            context.pipeline_cache,
            handles.pipeline_layout,
            shader::RADIX_HISTOGRAM_PASS_ENTRY_POINT,
        )?;
        handles.radix_prefix_pipeline = create_compute_pipeline(
            &context.device,
            context.pipeline_cache,
            handles.pipeline_layout,
            shader::RADIX_PREFIX_PASS_ENTRY_POINT,
        )?;
        handles.radix_scatter_pipeline = create_compute_pipeline(
            &context.device,
            context.pipeline_cache,
            handles.pipeline_layout,
            shader::RADIX_SCATTER_PASS_ENTRY_POINT,
        )?;
        let pool_sizes = reflected_pool_sizes();
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        handles.descriptor_pool =
            unsafe { context.device.create_descriptor_pool(&pool_info, None) }
                .map_err(|error| format!("create 3DGS descriptor pool: {error:?}"))?;
        let layouts = [handles.descriptor_set_layout];
        let allocate = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(handles.descriptor_pool)
            .set_layouts(&layouts);
        handles.descriptor_set = unsafe { context.device.allocate_descriptor_sets(&allocate) }
            .map_err(|error| format!("allocate 3DGS descriptor set: {error:?}"))?[0];
        let uniform = Buffer::new(
            &context,
            size_of::<shader::GaussianUniforms>() as u64,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        Ok(Self {
            context,
            handles,
            uniform,
            uploaded: None,
            target: None,
        })
    }

    pub fn render_to_buffer(
        &mut self,
        session: &RenderSession,
        width: u32,
        height: u32,
        params: &RenderParams,
        output: vk::Buffer,
    ) -> Result<(), String> {
        if !session.identity().is_current() {
            return Err(format!("PLY changed on disk: {}", session.path().display()));
        }
        let width = width.max(1);
        let height = height.max(1);
        self.ensure_uploaded(session)?;
        self.ensure_target(width, height)?;
        let uniforms = params
            .uniforms(session.cloud(), width, height)
            .map_err(|error| error.to_string())?;
        self.uniform.write(std::slice::from_ref(&uniforms))?;
        self.update_descriptors();

        let command = self.begin_commands()?;
        let target = self.target.as_ref().expect("3DGS target was initialized");
        let uploaded = self.uploaded.as_ref().expect("3DGS source was uploaded");
        let instance_copies = if matches!(
            params.camera.projection,
            shrimply_3dgs_core::Projection::Equirectangular
                | shrimply_3dgs_core::Projection::Cylindrical
        ) {
            3
        } else if params.camera.projection == shrimply_3dgs_core::Projection::Perspective
            && params.camera.focus_distance > 0.0
        {
            shrimply_3dgs_core::DEPTH_OF_FIELD_SAMPLES
        } else {
            1
        };
        uploaded
            .count
            .checked_mul(instance_copies)
            .ok_or_else(|| "3DGS instance count exceeds Vulkan draw limits".to_string())?;
        self.sort_gaussians(command, uploaded);
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent: vk::Extent2D { width, height },
        };
        let accumulation_clear = [vk::ClearValue {
            color: vk::ClearColorValue { float32: [0.0; 4] },
        }];
        let accumulation_begin = vk::RenderPassBeginInfo::default()
            .render_pass(self.handles.accumulation_render_pass)
            .framebuffer(target.accumulation_framebuffer)
            .render_area(scissor)
            .clear_values(&accumulation_clear);
        unsafe {
            self.context.device.cmd_begin_render_pass(
                command,
                &accumulation_begin,
                vk::SubpassContents::INLINE,
            );
            self.context.device.cmd_bind_pipeline(
                command,
                vk::PipelineBindPoint::GRAPHICS,
                self.handles.accumulation_pipeline,
            );
            self.bind_graphics(command);
            self.context
                .device
                .cmd_set_viewport(command, 0, &[viewport]);
            self.context.device.cmd_set_scissor(command, 0, &[scissor]);
            self.context.device.cmd_draw_indirect(
                command,
                uploaded.draw_indirect.buffer,
                0,
                1,
                DRAW_INDIRECT_SIZE as u32,
            );
            self.context.device.cmd_end_render_pass(command);

            let shader_barrier = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ);
            self.context.device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[shader_barrier],
                &[],
                &[],
            );

            let resolve_clear = [vk::ClearValue {
                color: vk::ClearColorValue { float32: [0.0; 4] },
            }];
            let resolve_begin = vk::RenderPassBeginInfo::default()
                .render_pass(self.handles.resolve_render_pass)
                .framebuffer(target.resolve_framebuffer)
                .render_area(scissor)
                .clear_values(&resolve_clear);
            self.context.device.cmd_begin_render_pass(
                command,
                &resolve_begin,
                vk::SubpassContents::INLINE,
            );
            self.context.device.cmd_bind_pipeline(
                command,
                vk::PipelineBindPoint::GRAPHICS,
                self.handles.resolve_pipeline,
            );
            self.bind_graphics(command);
            self.context
                .device
                .cmd_set_viewport(command, 0, &[viewport]);
            self.context.device.cmd_set_scissor(command, 0, &[scissor]);
            self.context.device.cmd_draw(command, 3, 1, 0, 0);
            self.context.device.cmd_end_render_pass(command);

            let transfer_barrier = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
            self.context.device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[transfer_barrier],
                &[],
                &[],
            );
            let copy = vk::BufferImageCopy::default()
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
            self.context.device.cmd_copy_image_to_buffer(
                command,
                target.color.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                output,
                &[copy],
            );
            let output_barrier = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::MEMORY_READ);
            self.context.device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[output_barrier],
                &[],
                &[],
            );
        }
        self.end_submit_wait(command)?;
        tracing::debug!(
            width,
            height,
            gaussians = uploaded.count,
            "Rendered dedicated Slang/Vulkan 3DGS texture"
        );
        Ok(())
    }

    fn sort_gaussians(&self, command: vk::CommandBuffer, uploaded: &UploadedCloud) {
        unsafe {
            let indirect = [4_u32, 0, 0, 0];
            let indirect_bytes =
                std::slice::from_raw_parts(indirect.as_ptr().cast::<u8>(), size_of_val(&indirect));
            self.context.device.cmd_update_buffer(
                command,
                uploaded.draw_indirect.buffer,
                0,
                indirect_bytes,
            );
            let reset_barrier = vk::BufferMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_WRITE)
                .buffer(uploaded.draw_indirect.buffer)
                .offset(0)
                .size(DRAW_INDIRECT_SIZE);
            self.context.device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[reset_barrier],
                &[],
            );
            self.context.device.cmd_bind_descriptor_sets(
                command,
                vk::PipelineBindPoint::COMPUTE,
                self.handles.pipeline_layout,
                0,
                &[self.handles.descriptor_set],
                &[],
            );
            self.context.device.cmd_bind_pipeline(
                command,
                vk::PipelineBindPoint::COMPUTE,
                self.handles.prepare_sort_pipeline,
            );
            self.push_sort_constants(
                command,
                shader::SortConstants {
                    count: uploaded.count,
                    shift: 0,
                    read_scratch: 0,
                    group_count: uploaded.group_count,
                },
            );
            self.context
                .device
                .cmd_dispatch(command, uploaded.group_count, 1, 1);
            self.sort_barrier(
                command,
                vk::PipelineStageFlags::COMPUTE_SHADER | vk::PipelineStageFlags::DRAW_INDIRECT,
            );

            for pass in 0..RADIX_PASSES {
                self.push_sort_constants(
                    command,
                    shader::SortConstants {
                        count: uploaded.count,
                        shift: pass * RADIX_BITS,
                        read_scratch: pass & 1,
                        group_count: uploaded.group_count,
                    },
                );
                self.context.device.cmd_bind_pipeline(
                    command,
                    vk::PipelineBindPoint::COMPUTE,
                    self.handles.radix_histogram_pipeline,
                );
                self.context
                    .device
                    .cmd_dispatch(command, uploaded.group_count, 1, 1);
                self.sort_barrier(command, vk::PipelineStageFlags::COMPUTE_SHADER);
                self.context.device.cmd_bind_pipeline(
                    command,
                    vk::PipelineBindPoint::COMPUTE,
                    self.handles.radix_prefix_pipeline,
                );
                self.context.device.cmd_dispatch(command, 1, 1, 1);
                self.sort_barrier(command, vk::PipelineStageFlags::COMPUTE_SHADER);
                self.context.device.cmd_bind_pipeline(
                    command,
                    vk::PipelineBindPoint::COMPUTE,
                    self.handles.radix_scatter_pipeline,
                );
                self.context
                    .device
                    .cmd_dispatch(command, uploaded.group_count, 1, 1);
                self.sort_barrier(
                    command,
                    if pass + 1 == RADIX_PASSES {
                        vk::PipelineStageFlags::VERTEX_SHADER
                    } else {
                        vk::PipelineStageFlags::COMPUTE_SHADER
                    },
                );
            }
        }
    }

    unsafe fn push_sort_constants(
        &self,
        command: vk::CommandBuffer,
        constants: shader::SortConstants,
    ) {
        let bytes = unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref(&constants).cast::<u8>(),
                size_of::<shader::SortConstants>(),
            )
        };
        unsafe {
            self.context.device.cmd_push_constants(
                command,
                self.handles.pipeline_layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                bytes,
            )
        };
    }

    unsafe fn sort_barrier(&self, command: vk::CommandBuffer, destination: vk::PipelineStageFlags) {
        let mut destination_access = vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE;
        if destination.contains(vk::PipelineStageFlags::DRAW_INDIRECT) {
            destination_access |= vk::AccessFlags::INDIRECT_COMMAND_READ;
        }
        let barrier = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
            .dst_access_mask(destination_access);
        unsafe {
            self.context.device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                destination,
                vk::DependencyFlags::empty(),
                &[barrier],
                &[],
                &[],
            )
        };
    }

    fn bind_graphics(&self, command: vk::CommandBuffer) {
        unsafe {
            self.context.device.cmd_bind_descriptor_sets(
                command,
                vk::PipelineBindPoint::GRAPHICS,
                self.handles.pipeline_layout,
                0,
                &[self.handles.descriptor_set],
                &[],
            )
        }
    }

    fn ensure_uploaded(&mut self, session: &RenderSession) -> Result<(), String> {
        if self
            .uploaded
            .as_ref()
            .is_some_and(|uploaded| &uploaded.identity == session.identity())
        {
            return Ok(());
        }
        let count = u32::try_from(session.cloud().gaussians.len())
            .map_err(|_| "3DGS count exceeds Vulkan draw limits".to_string())?;
        let sources: Vec<_> = session
            .cloud()
            .gaussians
            .par_iter()
            .map(shader::GaussianSource::from_gaussian)
            .collect();
        let gaussians = self.upload_device_buffer(&sources)?;
        let higher_order =
            self.upload_device_buffer(&session.cloud().higher_order_spherical_harmonics)?;
        let group_count = count.div_ceil(SORT_THREADS);
        let sort_size = u64::from(count) * size_of::<u32>() as u64;
        let group_offsets_size = u64::from(group_count) * RADIX_SIZE * size_of::<u32>() as u64;
        let sort_buffer = |size| {
            Buffer::new(
                &self.context,
                size,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )
        };
        self.uploaded = Some(UploadedCloud {
            identity: session.identity().clone(),
            gaussians,
            higher_order,
            sort_keys: sort_buffer(sort_size)?,
            sorted_indices: sort_buffer(sort_size)?,
            scratch_keys: sort_buffer(sort_size)?,
            scratch_indices: sort_buffer(sort_size)?,
            sort_group_offsets: sort_buffer(group_offsets_size)?,
            draw_indirect: Buffer::new(
                &self.context,
                DRAW_INDIRECT_SIZE,
                vk::BufferUsageFlags::STORAGE_BUFFER
                    | vk::BufferUsageFlags::INDIRECT_BUFFER
                    | vk::BufferUsageFlags::TRANSFER_DST,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )?,
            count,
            group_count,
        });
        Ok(())
    }

    fn upload_device_buffer<T: Copy>(&self, values: &[T]) -> Result<Buffer, String> {
        let size = (size_of_val(values) as u64).max(4);
        let staging = Buffer::new(
            &self.context,
            size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        staging.write(values)?;
        let device = Buffer::new(
            &self.context,
            size,
            vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::STORAGE_BUFFER,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let command = self.begin_commands()?;
        let copy = vk::BufferCopy::default().size(size_of_val(values) as u64);
        if !values.is_empty() {
            unsafe {
                self.context
                    .device
                    .cmd_copy_buffer(command, staging.buffer, device.buffer, &[copy])
            };
        }
        let barrier = vk::BufferMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .buffer(device.buffer)
            .offset(0)
            .size(size);
        unsafe {
            self.context.device.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::VERTEX_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[barrier],
                &[],
            )
        };
        self.end_submit_wait(command)?;
        Ok(device)
    }

    fn ensure_target(&mut self, width: u32, height: u32) -> Result<(), String> {
        if self
            .target
            .as_ref()
            .is_some_and(|target| target.width == width && target.height == height)
        {
            return Ok(());
        }
        self.target = Some(Target::new(
            &self.context,
            width,
            height,
            self.handles.accumulation_render_pass,
            self.handles.resolve_render_pass,
        )?);
        Ok(())
    }

    fn update_descriptors(&self) {
        let target = self.target.as_ref().expect("3DGS target was initialized");
        let uploaded = self.uploaded.as_ref().expect("3DGS source was uploaded");
        let gaussian_info = [uploaded.gaussians.descriptor()];
        let higher_order_info = [uploaded.higher_order.descriptor()];
        let accumulation_info = [target.accumulation.descriptor()];
        let uniform_info = [self.uniform.descriptor()];
        let sort_keys_info = [uploaded.sort_keys.descriptor()];
        let sorted_indices_info = [uploaded.sorted_indices.descriptor()];
        let scratch_keys_info = [uploaded.scratch_keys.descriptor()];
        let scratch_indices_info = [uploaded.scratch_indices.descriptor()];
        let sort_group_offsets_info = [uploaded.sort_group_offsets.descriptor()];
        let draw_indirect_info = [uploaded.draw_indirect.descriptor()];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.handles.descriptor_set)
                .dst_binding(shader::GAUSSIANS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&gaussian_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.handles.descriptor_set)
                .dst_binding(shader::HIGHER_ORDER_SH_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&higher_order_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.handles.descriptor_set)
                .dst_binding(shader::ACCUMULATION_TEXTURE_BINDING)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&accumulation_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.handles.descriptor_set)
                .dst_binding(shader::UNIFORMS_BINDING)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&uniform_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.handles.descriptor_set)
                .dst_binding(shader::SORT_KEYS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&sort_keys_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.handles.descriptor_set)
                .dst_binding(shader::SORTED_INDICES_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&sorted_indices_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.handles.descriptor_set)
                .dst_binding(shader::SCRATCH_KEYS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&scratch_keys_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.handles.descriptor_set)
                .dst_binding(shader::SCRATCH_INDICES_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&scratch_indices_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.handles.descriptor_set)
                .dst_binding(shader::SORT_GROUP_OFFSETS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&sort_group_offsets_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.handles.descriptor_set)
                .dst_binding(shader::DRAW_INDIRECT_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&draw_indirect_info),
        ];
        unsafe { self.context.device.update_descriptor_sets(&writes, &[]) };
    }

    fn begin_commands(&self) -> Result<vk::CommandBuffer, String> {
        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.context.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command = unsafe { self.context.device.allocate_command_buffers(&allocate) }
            .map_err(|error| format!("allocate 3DGS command buffer: {error:?}"))?[0];
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        if let Err(error) = unsafe { self.context.device.begin_command_buffer(command, &begin) } {
            unsafe {
                self.context
                    .device
                    .free_command_buffers(self.context.command_pool, &[command])
            };
            return Err(format!("begin 3DGS command buffer: {error:?}"));
        }
        Ok(command)
    }

    fn end_submit_wait(&self, command: vk::CommandBuffer) -> Result<(), String> {
        let result = (|| {
            unsafe { self.context.device.end_command_buffer(command) }
                .map_err(|error| format!("end 3DGS command buffer: {error:?}"))?;
            let fence = unsafe {
                self.context
                    .device
                    .create_fence(&vk::FenceCreateInfo::default(), None)
            }
            .map_err(|error| format!("create 3DGS fence: {error:?}"))?;
            let commands = [command];
            let submit = vk::SubmitInfo::default().command_buffers(&commands);
            let submitted = unsafe {
                self.context
                    .device
                    .queue_submit(self.context.queue, &[submit], fence)
            };
            if let Err(error) = submitted {
                unsafe { self.context.device.destroy_fence(fence, None) };
                return Err(format!("submit 3DGS work: {error:?}"));
            }
            let waited = unsafe {
                self.context
                    .device
                    .wait_for_fences(&[fence], true, u64::MAX)
            };
            unsafe { self.context.device.destroy_fence(fence, None) };
            waited.map_err(|error| format!("wait for 3DGS work: {error:?}"))
        })();
        unsafe {
            self.context
                .device
                .free_command_buffers(self.context.command_pool, &[command])
        };
        result
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        match unsafe { self.context.device.queue_wait_idle(self.context.queue) } {
            Ok(()) | Err(vk::Result::ERROR_DEVICE_LOST) => {}
            Err(error) => {
                tracing::error!(
                    ?error,
                    "Could not make 3DGS Vulkan device idle during cleanup"
                );
                std::process::abort();
            }
        }
        self.target.take();
        self.uploaded.take();
    }
}

struct Buffer {
    device: ash::Device,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
}

impl Buffer {
    fn new(
        context: &RenderContext,
        size: u64,
        usage: vk::BufferUsageFlags,
        properties: vk::MemoryPropertyFlags,
    ) -> Result<Self, String> {
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe { context.device.create_buffer(&info, None) }
            .map_err(|error| format!("create 3DGS buffer: {error:?}"))?;
        let requirements = unsafe { context.device.get_buffer_memory_requirements(buffer) };
        let memory_type = match memory_type(
            &context.memory_properties,
            requirements.memory_type_bits,
            properties,
        ) {
            Ok(memory_type) => memory_type,
            Err(error) => {
                unsafe { context.device.destroy_buffer(buffer, None) };
                return Err(error);
            }
        };
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type);
        let memory = match unsafe { context.device.allocate_memory(&allocation, None) } {
            Ok(memory) => memory,
            Err(error) => {
                unsafe { context.device.destroy_buffer(buffer, None) };
                return Err(format!("allocate 3DGS buffer memory: {error:?}"));
            }
        };
        if let Err(error) = unsafe { context.device.bind_buffer_memory(buffer, memory, 0) } {
            unsafe {
                context.device.destroy_buffer(buffer, None);
                context.device.free_memory(memory, None);
            }
            return Err(format!("bind 3DGS buffer memory: {error:?}"));
        }
        Ok(Self {
            device: context.device.clone(),
            buffer,
            memory,
            size,
        })
    }

    fn write<T>(&self, values: &[T]) -> Result<(), String> {
        let bytes = size_of_val(values) as u64;
        if bytes == 0 {
            return Ok(());
        }
        if bytes > self.size {
            return Err("3DGS buffer upload exceeds allocation".to_string());
        }
        let mapped = unsafe {
            self.device
                .map_memory(self.memory, 0, bytes, vk::MemoryMapFlags::empty())
        }
        .map_err(|error| format!("map 3DGS buffer: {error:?}"))?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                values.as_ptr().cast::<u8>(),
                mapped.cast(),
                bytes as usize,
            );
            self.device.unmap_memory(self.memory);
        }
        Ok(())
    }

    fn descriptor(&self) -> vk::DescriptorBufferInfo {
        vk::DescriptorBufferInfo {
            buffer: self.buffer,
            offset: 0,
            range: self.size,
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

struct Texture {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

impl Texture {
    fn new(
        context: &RenderContext,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
    ) -> Result<Self, String> {
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { context.device.create_image(&info, None) }
            .map_err(|error| format!("create 3DGS texture: {error:?}"))?;
        let requirements = unsafe { context.device.get_image_memory_requirements(image) };
        let memory_type = match memory_type(
            &context.memory_properties,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ) {
            Ok(memory_type) => memory_type,
            Err(error) => {
                unsafe { context.device.destroy_image(image, None) };
                return Err(error);
            }
        };
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type);
        let memory = match unsafe { context.device.allocate_memory(&allocation, None) } {
            Ok(memory) => memory,
            Err(error) => {
                unsafe { context.device.destroy_image(image, None) };
                return Err(format!("allocate 3DGS texture memory: {error:?}"));
            }
        };
        if let Err(error) = unsafe { context.device.bind_image_memory(image, memory, 0) } {
            unsafe {
                context.device.destroy_image(image, None);
                context.device.free_memory(memory, None);
            }
            return Err(format!("bind 3DGS texture memory: {error:?}"));
        }
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let view = match unsafe { context.device.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(error) => {
                unsafe {
                    context.device.destroy_image(image, None);
                    context.device.free_memory(memory, None);
                }
                return Err(format!("create 3DGS texture view: {error:?}"));
            }
        };
        Ok(Self {
            device: context.device.clone(),
            image,
            memory,
            view,
        })
    }

    fn descriptor(&self) -> vk::DescriptorImageInfo {
        vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: self.view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        }
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_image_view(self.view, None);
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

struct Target {
    device: ash::Device,
    accumulation: Texture,
    color: Texture,
    accumulation_framebuffer: vk::Framebuffer,
    resolve_framebuffer: vk::Framebuffer,
    width: u32,
    height: u32,
}

impl Target {
    fn new(
        context: &RenderContext,
        width: u32,
        height: u32,
        accumulation_render_pass: vk::RenderPass,
        resolve_render_pass: vk::RenderPass,
    ) -> Result<Self, String> {
        let accumulation = Texture::new(
            context,
            width,
            height,
            ACCUMULATION_FORMAT,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
        )?;
        let color = Texture::new(
            context,
            width,
            height,
            COLOR_FORMAT,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
        )?;
        let accumulation_framebuffer = create_framebuffer(
            &context.device,
            accumulation_render_pass,
            &[accumulation.view],
            width,
            height,
        )?;
        let resolve_framebuffer = match create_framebuffer(
            &context.device,
            resolve_render_pass,
            &[color.view],
            width,
            height,
        ) {
            Ok(framebuffer) => framebuffer,
            Err(error) => {
                unsafe {
                    context
                        .device
                        .destroy_framebuffer(accumulation_framebuffer, None)
                };
                return Err(error);
            }
        };
        Ok(Self {
            device: context.device.clone(),
            accumulation,
            color,
            accumulation_framebuffer,
            resolve_framebuffer,
            width,
            height,
        })
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        unsafe {
            self.device
                .destroy_framebuffer(self.accumulation_framebuffer, None);
            self.device
                .destroy_framebuffer(self.resolve_framebuffer, None);
        }
    }
}

struct Handles {
    device: ash::Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    accumulation_render_pass: vk::RenderPass,
    resolve_render_pass: vk::RenderPass,
    accumulation_pipeline: vk::Pipeline,
    resolve_pipeline: vk::Pipeline,
    prepare_sort_pipeline: vk::Pipeline,
    radix_histogram_pipeline: vk::Pipeline,
    radix_prefix_pipeline: vk::Pipeline,
    radix_scatter_pipeline: vk::Pipeline,
}

impl Handles {
    fn new(device: ash::Device) -> Self {
        Self {
            device,
            descriptor_set_layout: vk::DescriptorSetLayout::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            descriptor_set: vk::DescriptorSet::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            accumulation_render_pass: vk::RenderPass::null(),
            resolve_render_pass: vk::RenderPass::null(),
            accumulation_pipeline: vk::Pipeline::null(),
            resolve_pipeline: vk::Pipeline::null(),
            prepare_sort_pipeline: vk::Pipeline::null(),
            radix_histogram_pipeline: vk::Pipeline::null(),
            radix_prefix_pipeline: vk::Pipeline::null(),
            radix_scatter_pipeline: vk::Pipeline::null(),
        }
    }
}

impl Drop for Handles {
    fn drop(&mut self) {
        unsafe {
            self.device
                .destroy_pipeline(self.accumulation_pipeline, None);
            self.device.destroy_pipeline(self.resolve_pipeline, None);
            self.device
                .destroy_pipeline(self.prepare_sort_pipeline, None);
            self.device
                .destroy_pipeline(self.radix_histogram_pipeline, None);
            self.device
                .destroy_pipeline(self.radix_prefix_pipeline, None);
            self.device
                .destroy_pipeline(self.radix_scatter_pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            self.device
                .destroy_render_pass(self.accumulation_render_pass, None);
            self.device
                .destroy_render_pass(self.resolve_render_pass, None);
        }
    }
}

fn memory_type(
    properties: &vk::PhysicalDeviceMemoryProperties,
    bits: u32,
    flags: vk::MemoryPropertyFlags,
) -> Result<u32, String> {
    (0..properties.memory_type_count)
        .find(|index| {
            bits & (1 << index) != 0
                && properties.memory_types[*index as usize]
                    .property_flags
                    .contains(flags)
        })
        .ok_or_else(|| format!("no Vulkan memory type for 3DGS {flags:?}"))
}

fn create_accumulation_render_pass(device: &ash::Device) -> Result<vk::RenderPass, String> {
    let attachments = [vk::AttachmentDescription::default()
        .format(ACCUMULATION_FORMAT)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
    let references = [vk::AttachmentReference {
        attachment: 0,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    }];
    let subpasses = [vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&references)];
    let info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpasses);
    unsafe { device.create_render_pass(&info, None) }
        .map_err(|error| format!("create 3DGS accumulation pass: {error:?}"))
}

fn create_resolve_render_pass(device: &ash::Device) -> Result<vk::RenderPass, String> {
    let attachments = [vk::AttachmentDescription::default()
        .format(COLOR_FORMAT)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)];
    let references = [vk::AttachmentReference {
        attachment: 0,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    }];
    let subpasses = [vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&references)];
    let info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpasses);
    unsafe { device.create_render_pass(&info, None) }
        .map_err(|error| format!("create 3DGS resolve pass: {error:?}"))
}

fn create_framebuffer(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    attachments: &[vk::ImageView],
    width: u32,
    height: u32,
) -> Result<vk::Framebuffer, String> {
    let info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(attachments)
        .width(width)
        .height(height)
        .layers(1);
    unsafe { device.create_framebuffer(&info, None) }
        .map_err(|error| format!("create 3DGS framebuffer: {error:?}"))
}

fn create_descriptor_set_layout(device: &ash::Device) -> Result<vk::DescriptorSetLayout, String> {
    if shader::DESCRIPTORS
        .iter()
        .any(|descriptor| descriptor.set != 0)
    {
        return Err("3DGS Slang module must use descriptor set 0".to_string());
    }
    let bindings: Vec<_> = shader::DESCRIPTORS
        .iter()
        .map(|descriptor| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(descriptor.binding)
                .descriptor_type(descriptor_type(descriptor.kind))
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::ALL_GRAPHICS | vk::ShaderStageFlags::COMPUTE)
        })
        .collect();
    let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    unsafe { device.create_descriptor_set_layout(&info, None) }
        .map_err(|error| format!("create 3DGS descriptor layout: {error:?}"))
}

fn reflected_pool_sizes() -> Vec<vk::DescriptorPoolSize> {
    [
        shader::DescriptorKind::UniformBuffer,
        shader::DescriptorKind::SampledImage,
        shader::DescriptorKind::Sampler,
        shader::DescriptorKind::AccelerationStructure,
        shader::DescriptorKind::StorageImage,
        shader::DescriptorKind::StorageBuffer,
    ]
    .into_iter()
    .filter_map(|kind| {
        let descriptor_count = shader::DESCRIPTORS
            .iter()
            .filter(|descriptor| descriptor.kind == kind)
            .count() as u32;
        (descriptor_count > 0).then_some(vk::DescriptorPoolSize {
            ty: descriptor_type(kind),
            descriptor_count,
        })
    })
    .collect()
}

fn descriptor_type(kind: shader::DescriptorKind) -> vk::DescriptorType {
    match kind {
        shader::DescriptorKind::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
        shader::DescriptorKind::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
        shader::DescriptorKind::Sampler => vk::DescriptorType::SAMPLER,
        shader::DescriptorKind::AccelerationStructure => {
            vk::DescriptorType::ACCELERATION_STRUCTURE_KHR
        }
        shader::DescriptorKind::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
        shader::DescriptorKind::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
    }
}

fn create_pipeline_layout(
    device: &ash::Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<vk::PipelineLayout, String> {
    let layouts = [descriptor_set_layout];
    let push_constants = [vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::COMPUTE)
        .offset(0)
        .size(size_of::<shader::SortConstants>() as u32)];
    let info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&layouts)
        .push_constant_ranges(&push_constants);
    unsafe { device.create_pipeline_layout(&info, None) }
        .map_err(|error| format!("create 3DGS pipeline layout: {error:?}"))
}

fn create_pipeline(
    device: &ash::Device,
    pipeline_cache: vk::PipelineCache,
    render_pass: vk::RenderPass,
    layout: vk::PipelineLayout,
    vertex_entry: &std::ffi::CStr,
    fragment_entry: &std::ffi::CStr,
    accumulation: bool,
) -> Result<vk::Pipeline, String> {
    let spirv = ash::util::read_spv(&mut std::io::Cursor::new(shader::SPIRV_BYTES))
        .map_err(|error| format!("decode 3DGS graphics SPIR-V: {error}"))?;
    let module_info = vk::ShaderModuleCreateInfo::default().code(&spirv);
    let module = unsafe { device.create_shader_module(&module_info, None) }
        .map_err(|error| format!("create 3DGS shader module: {error:?}"))?;
    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(module)
            .name(vertex_entry),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(module)
            .name(fragment_entry),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
    let input_assembly =
        vk::PipelineInputAssemblyStateCreateInfo::default().topology(if accumulation {
            vk::PrimitiveTopology::TRIANGLE_STRIP
        } else {
            vk::PrimitiveTopology::TRIANGLE_LIST
        });
    let viewport = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let write_mask = vk::ColorComponentFlags::R
        | vk::ColorComponentFlags::G
        | vk::ColorComponentFlags::B
        | vk::ColorComponentFlags::A;
    let attachments = if accumulation {
        vec![
            vk::PipelineColorBlendAttachmentState::default()
                .blend_enable(true)
                .src_color_blend_factor(vk::BlendFactor::ONE)
                .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .color_blend_op(vk::BlendOp::ADD)
                .src_alpha_blend_factor(vk::BlendFactor::ONE)
                .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .alpha_blend_op(vk::BlendOp::ADD)
                .color_write_mask(write_mask),
        ]
    } else {
        vec![vk::PipelineColorBlendAttachmentState::default().color_write_mask(write_mask)]
    };
    let color_blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
    let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic);
    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport)
        .rasterization_state(&rasterization)
        .multisample_state(&multisample)
        .color_blend_state(&color_blend)
        .dynamic_state(&dynamic_state)
        .layout(layout)
        .render_pass(render_pass)
        .subpass(0);
    tracing::debug!(
        vertex = %vertex_entry.to_string_lossy(),
        fragment = %fragment_entry.to_string_lossy(),
        accumulation,
        "Creating Vulkan 3DGS graphics pipeline from SPIR-V"
    );
    let started = Instant::now();
    let result = unsafe { device.create_graphics_pipelines(pipeline_cache, &[info], None) }
        .map_err(|(_, error)| format!("create 3DGS graphics pipeline: {error:?}"))
        .map(|pipelines| pipelines[0]);
    tracing::debug!(
        vertex = %vertex_entry.to_string_lossy(),
        fragment = %fragment_entry.to_string_lossy(),
        elapsed_us = started.elapsed().as_micros(),
        success = result.is_ok(),
        "Finished Vulkan 3DGS graphics pipeline creation"
    );
    unsafe { device.destroy_shader_module(module, None) };
    result
}

fn create_compute_pipeline(
    device: &ash::Device,
    pipeline_cache: vk::PipelineCache,
    layout: vk::PipelineLayout,
    entry_point: &std::ffi::CStr,
) -> Result<vk::Pipeline, String> {
    let spirv = ash::util::read_spv(&mut std::io::Cursor::new(shader::SPIRV_BYTES))
        .map_err(|error| format!("decode 3DGS sort SPIR-V: {error}"))?;
    let module_info = vk::ShaderModuleCreateInfo::default().code(&spirv);
    let module = unsafe { device.create_shader_module(&module_info, None) }
        .map_err(|error| format!("create 3DGS sort shader module: {error:?}"))?;
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(module)
        .name(entry_point);
    let info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(layout);
    tracing::debug!(
        entry = %entry_point.to_string_lossy(),
        "Creating Vulkan 3DGS compute pipeline from SPIR-V"
    );
    let started = Instant::now();
    let result = unsafe { device.create_compute_pipelines(pipeline_cache, &[info], None) }
        .map_err(|(_, error)| format!("create 3DGS sort pipeline: {error:?}"))
        .map(|pipelines| pipelines[0]);
    tracing::debug!(
        entry = %entry_point.to_string_lossy(),
        elapsed_us = started.elapsed().as_micros(),
        success = result.is_ok(),
        "Finished Vulkan 3DGS compute pipeline creation"
    );
    unsafe { device.destroy_shader_module(module, None) };
    result
}
