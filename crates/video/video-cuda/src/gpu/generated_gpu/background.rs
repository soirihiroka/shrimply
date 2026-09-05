use std::mem::{size_of, size_of_val};

use ash::vk;
use shrimply_background::Background;
use shrimply_project::project::Time;
use shrimply_render_core::background_spirv as shader;
use shrimply_video_core::background::uniforms;

const THREADS: u32 = 16;

pub(super) struct RenderContext {
    pub device: ash::Device,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub queue: vk::Queue,
    pub command_pool: vk::CommandPool,
    pub pipeline_cache: vk::PipelineCache,
}

#[derive(Clone, PartialEq)]
pub(super) struct RenderKey(pub(super) shader::BackgroundUniforms);

impl RenderKey {
    pub(super) fn new(width: u32, height: u32, time: Time, background: &Background) -> Self {
        Self(uniforms(width.max(1), height.max(1), time, background))
    }
}

pub(super) struct Renderer {
    context: RenderContext,
    descriptor_set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    uniforms: Buffer,
    pending: Option<(vk::Fence, vk::CommandBuffer)>,
}

impl Renderer {
    pub(super) fn new(context: RenderContext) -> Result<Self, String> {
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let descriptor_set_layout = unsafe {
            context.device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )
        }
        .map_err(|error| format!("create background descriptor layout: {error:?}"))?;
        let layouts = [descriptor_set_layout];
        let pipeline_layout = unsafe {
            context.device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts),
                None,
            )
        }
        .map_err(|error| format!("create background pipeline layout: {error:?}"))?;
        let spirv = ash::util::read_spv(&mut std::io::Cursor::new(shader::SPIRV_BYTES))
            .map_err(|error| format!("decode background SPIR-V: {error}"))?;
        let module = unsafe {
            context
                .device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&spirv), None)
        }
        .map_err(|error| format!("create background shader module: {error:?}"))?;
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(module)
            .name(shader::MAIN_ENTRY_POINT);
        let pipeline = unsafe {
            context.device.create_compute_pipelines(
                context.pipeline_cache,
                &[vk::ComputePipelineCreateInfo::default()
                    .stage(stage)
                    .layout(pipeline_layout)],
                None,
            )
        }
        .map_err(|(_, error)| format!("create background compute pipeline: {error:?}"))?[0];
        unsafe { context.device.destroy_shader_module(module, None) };

        let pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_BUFFER,
                descriptor_count: 1,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER,
                descriptor_count: 1,
            },
        ];
        let descriptor_pool = unsafe {
            context.device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&pool_sizes),
                None,
            )
        }
        .map_err(|error| format!("create background descriptor pool: {error:?}"))?;
        let descriptor_set = unsafe {
            context.device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(descriptor_pool)
                    .set_layouts(&layouts),
            )
        }
        .map_err(|error| format!("allocate background descriptor set: {error:?}"))?[0];
        let uniforms = Buffer::new(&context, size_of::<shader::BackgroundUniforms>() as u64)?;
        Ok(Self {
            context,
            descriptor_set_layout,
            descriptor_pool,
            descriptor_set,
            pipeline_layout,
            pipeline,
            uniforms,
            pending: None,
        })
    }

    pub(super) fn render(
        &mut self,
        output: vk::Buffer,
        key: &RenderKey,
        signal: vk::Semaphore,
    ) -> Result<(), String> {
        let width = key.0.common.width;
        let height = key.0.common.height;
        self.finish_pending()?;
        self.uniforms.write(std::slice::from_ref(&key.0))?;
        let output_info = [vk::DescriptorBufferInfo {
            buffer: output,
            offset: 0,
            range: u64::from(width) * u64::from(height) * 4,
        }];
        let uniform_info = [self.uniforms.descriptor()];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&output_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&uniform_info),
        ];
        unsafe { self.context.device.update_descriptor_sets(&writes, &[]) };

        let command = unsafe {
            self.context.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.context.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(|error| format!("allocate background command buffer: {error:?}"))?[0];
        unsafe {
            self.context.device.begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
        }
        .map_err(|error| format!("begin background command buffer: {error:?}"))?;
        unsafe {
            self.context.device.cmd_bind_pipeline(
                command,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline,
            );
            self.context.device.cmd_bind_descriptor_sets(
                command,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[self.descriptor_set],
                &[],
            );
            self.context.device.cmd_dispatch(
                command,
                width.div_ceil(THREADS),
                height.div_ceil(THREADS),
                1,
            );
            self.context.device.end_command_buffer(command)
        }
        .map_err(|error| format!("end background command buffer: {error:?}"))?;
        let fence = unsafe {
            self.context
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .map_err(|error| format!("create background fence: {error:?}"))?;
        let submitted = unsafe {
            let commands = [command];
            let signals = [signal];
            self.context.device.queue_submit(
                self.context.queue,
                &[vk::SubmitInfo::default()
                    .command_buffers(&commands)
                    .signal_semaphores(&signals)],
                fence,
            )
        };
        if let Err(error) = submitted {
            unsafe {
                self.context.device.destroy_fence(fence, None);
                self.context
                    .device
                    .free_command_buffers(self.context.command_pool, &[command]);
            }
            return Err(format!("submit background render: {error:?}"));
        }
        self.pending = Some((fence, command));
        Ok(())
    }

    fn finish_pending(&mut self) -> Result<(), String> {
        let Some((fence, command)) = self.pending.take() else {
            return Ok(());
        };
        let result = unsafe {
            self.context
                .device
                .wait_for_fences(&[fence], true, u64::MAX)
        }
        .map_err(|error| format!("wait for background render: {error:?}"));
        unsafe {
            self.context.device.destroy_fence(fence, None);
            self.context
                .device
                .free_command_buffers(self.context.command_pool, &[command]);
        }
        result
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        if let Err(error) = self.finish_pending() {
            tracing::error!(%error, "Could not finish background render during cleanup");
        }
        unsafe {
            self.context.device.destroy_pipeline(self.pipeline, None);
            self.context
                .device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.context
                .device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.context
                .device
                .destroy_descriptor_set_layout(self.descriptor_set_layout, None);
        }
    }
}

struct Buffer {
    device: ash::Device,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
}

impl Buffer {
    fn new(context: &RenderContext, size: u64) -> Result<Self, String> {
        let buffer = unsafe {
            context.device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }
        .map_err(|error| format!("create background uniform buffer: {error:?}"))?;
        let requirements = unsafe { context.device.get_buffer_memory_requirements(buffer) };
        let flags = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        let memory_type = (0..context.memory_properties.memory_type_count)
            .find(|index| {
                requirements.memory_type_bits & (1 << index) != 0
                    && context.memory_properties.memory_types[*index as usize]
                        .property_flags
                        .contains(flags)
            })
            .ok_or_else(|| "no host-visible Vulkan memory for background uniforms".to_string())?;
        let memory = unsafe {
            context.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(memory_type),
                None,
            )
        }
        .map_err(|error| format!("allocate background uniform memory: {error:?}"))?;
        unsafe { context.device.bind_buffer_memory(buffer, memory, 0) }
            .map_err(|error| format!("bind background uniform memory: {error:?}"))?;
        Ok(Self {
            device: context.device.clone(),
            buffer,
            memory,
            size,
        })
    }

    fn write<T>(&self, values: &[T]) -> Result<(), String> {
        let bytes = size_of_val(values) as u64;
        if bytes > self.size {
            return Err("background uniform upload exceeds allocation".to_string());
        }
        let mapped = unsafe {
            self.device
                .map_memory(self.memory, 0, bytes, vk::MemoryMapFlags::empty())
        }
        .map_err(|error| format!("map background uniform memory: {error:?}"))?;
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
