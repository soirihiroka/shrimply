use std::mem::{size_of, size_of_val};

use ash::vk;

use crate::video_shader::mesh_flow as shader;

const THREADS: u32 = 16;

pub(super) struct RenderContext {
    pub device: ash::Device,
    pub memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub queue: vk::Queue,
    pub command_pool: vk::CommandPool,
    pub pipeline_cache: vk::PipelineCache,
}

pub(super) struct Renderer {
    context: RenderContext,
    descriptor_set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    uniforms: Buffer,
    offsets: Option<Buffer>,
}

pub(super) struct RenderRequest<'a> {
    pub input: vk::Buffer,
    pub output: vk::Buffer,
    pub image_size: glam::UVec2,
    pub grid_size: glam::UVec2,
    pub source_offsets: &'a [glam::Vec2],
}

impl Renderer {
    pub(super) fn new(context: RenderContext) -> Result<Self, String> {
        let descriptor_set_layout = create_descriptor_set_layout(&context.device)?;
        let pipeline_layout = create_pipeline_layout(&context.device, descriptor_set_layout)?;
        let pipeline = create_pipeline(&context.device, context.pipeline_cache, pipeline_layout)?;
        let pool_size = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_BUFFER,
                descriptor_count: 3,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER,
                descriptor_count: 1,
            },
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_size);
        let descriptor_pool = unsafe { context.device.create_descriptor_pool(&pool_info, None) }
            .map_err(|error| format!("create MeshFlow descriptor pool: {error:?}"))?;
        let layouts = [descriptor_set_layout];
        let allocate = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&layouts);
        let descriptor_set = unsafe { context.device.allocate_descriptor_sets(&allocate) }
            .map_err(|error| format!("allocate MeshFlow descriptor set: {error:?}"))?[0];
        let uniforms = Buffer::new(
            &context,
            size_of::<shader::MeshFlowUniforms>() as u64,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
        )?;
        Ok(Self {
            context,
            descriptor_set_layout,
            descriptor_pool,
            descriptor_set,
            pipeline_layout,
            pipeline,
            uniforms,
            offsets: None,
        })
    }

    pub(super) fn render(&mut self, request: RenderRequest<'_>) -> Result<(), String> {
        let glam::UVec2 {
            x: width,
            y: height,
        } = request.image_size;
        let glam::UVec2 {
            x: grid_width,
            y: grid_height,
        } = request.grid_size;
        let source_offsets = request.source_offsets;
        if width == 0
            || height == 0
            || grid_width < 2
            || grid_height < 2
            || source_offsets.len() != grid_width as usize * grid_height as usize
        {
            return Err("invalid MeshFlow Vulkan warp dimensions".to_string());
        }
        let offset_bytes = size_of_val(source_offsets) as u64;
        if self
            .offsets
            .as_ref()
            .is_none_or(|buffer| buffer.size != offset_bytes)
        {
            self.offsets = Some(Buffer::new(
                &self.context,
                offset_bytes,
                vk::BufferUsageFlags::STORAGE_BUFFER,
            )?);
        }
        let offsets = self
            .offsets
            .as_ref()
            .expect("MeshFlow offsets were allocated");
        offsets.write(source_offsets)?;
        self.uniforms.write(&[shader::MeshFlowUniforms {
            width,
            height,
            grid_width,
            grid_height,
        }])?;
        let pixel_bytes = u64::from(width) * u64::from(height) * 4;
        let input_info = [vk::DescriptorBufferInfo {
            buffer: request.input,
            offset: 0,
            range: pixel_bytes,
        }];
        let offset_info = [offsets.descriptor()];
        let output_info = [vk::DescriptorBufferInfo {
            buffer: request.output,
            offset: 0,
            range: pixel_bytes,
        }];
        let uniform_info = [self.uniforms.descriptor()];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_set)
                .dst_binding(shader::INPUT_PIXELS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&input_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_set)
                .dst_binding(shader::SOURCE_OFFSETS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&offset_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_set)
                .dst_binding(shader::OUTPUT_PIXELS_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&output_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_set)
                .dst_binding(shader::UNIFORMS_BINDING)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&uniform_info),
        ];
        unsafe { self.context.device.update_descriptor_sets(&writes, &[]) };

        let command = self.begin_commands()?;
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
        }
        self.end_submit_wait(command)
    }

    fn begin_commands(&self) -> Result<vk::CommandBuffer, String> {
        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.context.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command = unsafe { self.context.device.allocate_command_buffers(&allocate) }
            .map_err(|error| format!("allocate MeshFlow command buffer: {error:?}"))?[0];
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe { self.context.device.begin_command_buffer(command, &begin) }
            .map_err(|error| format!("begin MeshFlow command buffer: {error:?}"))?;
        Ok(command)
    }

    fn end_submit_wait(&self, command: vk::CommandBuffer) -> Result<(), String> {
        let result = (|| {
            unsafe { self.context.device.end_command_buffer(command) }
                .map_err(|error| format!("end MeshFlow command buffer: {error:?}"))?;
            let fence = unsafe {
                self.context
                    .device
                    .create_fence(&vk::FenceCreateInfo::default(), None)
            }
            .map_err(|error| format!("create MeshFlow fence: {error:?}"))?;
            let commands = [command];
            let submit = vk::SubmitInfo::default().command_buffers(&commands);
            let submitted = unsafe {
                self.context
                    .device
                    .queue_submit(self.context.queue, &[submit], fence)
            };
            if let Err(error) = submitted {
                unsafe { self.context.device.destroy_fence(fence, None) };
                return Err(format!("submit MeshFlow work: {error:?}"));
            }
            let waited = unsafe {
                self.context
                    .device
                    .wait_for_fences(&[fence], true, u64::MAX)
            };
            unsafe { self.context.device.destroy_fence(fence, None) };
            waited.map_err(|error| format!("wait for MeshFlow work: {error:?}"))
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
    fn new(
        context: &RenderContext,
        size: u64,
        usage: vk::BufferUsageFlags,
    ) -> Result<Self, String> {
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe { context.device.create_buffer(&info, None) }
            .map_err(|error| format!("create MeshFlow buffer: {error:?}"))?;
        let requirements = unsafe { context.device.get_buffer_memory_requirements(buffer) };
        let memory_type = memory_type(
            &context.memory_properties,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type);
        let memory = unsafe { context.device.allocate_memory(&allocation, None) }
            .map_err(|error| format!("allocate MeshFlow buffer: {error:?}"))?;
        if let Err(error) = unsafe { context.device.bind_buffer_memory(buffer, memory, 0) } {
            unsafe {
                context.device.destroy_buffer(buffer, None);
                context.device.free_memory(memory, None);
            }
            return Err(format!("bind MeshFlow buffer memory: {error:?}"));
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
        if bytes > self.size {
            return Err("MeshFlow buffer upload exceeds allocation".to_string());
        }
        let mapped = unsafe {
            self.device
                .map_memory(self.memory, 0, bytes, vk::MemoryMapFlags::empty())
        }
        .map_err(|error| format!("map MeshFlow buffer: {error:?}"))?;
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

fn create_descriptor_set_layout(device: &ash::Device) -> Result<vk::DescriptorSetLayout, String> {
    if shader::DESCRIPTORS
        .iter()
        .any(|descriptor| descriptor.set != 0)
    {
        return Err("MeshFlow Slang module must use descriptor set 0".to_string());
    }
    let bindings = shader::DESCRIPTORS
        .iter()
        .map(|descriptor| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(descriptor.binding)
                .descriptor_type(match descriptor.kind {
                    shader::DescriptorKind::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
                    shader::DescriptorKind::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
                    _ => panic!("unsupported MeshFlow descriptor kind"),
                })
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
        })
        .collect::<Vec<_>>();
    let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    unsafe { device.create_descriptor_set_layout(&info, None) }
        .map_err(|error| format!("create MeshFlow descriptor layout: {error:?}"))
}

fn create_pipeline_layout(
    device: &ash::Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<vk::PipelineLayout, String> {
    let layouts = [descriptor_set_layout];
    let info = vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts);
    unsafe { device.create_pipeline_layout(&info, None) }
        .map_err(|error| format!("create MeshFlow pipeline layout: {error:?}"))
}

fn create_pipeline(
    device: &ash::Device,
    pipeline_cache: vk::PipelineCache,
    layout: vk::PipelineLayout,
) -> Result<vk::Pipeline, String> {
    let spirv = ash::util::read_spv(&mut std::io::Cursor::new(shader::SPIRV_BYTES))
        .map_err(|error| format!("decode MeshFlow SPIR-V: {error}"))?;
    let module_info = vk::ShaderModuleCreateInfo::default().code(&spirv);
    let module = unsafe { device.create_shader_module(&module_info, None) }
        .map_err(|error| format!("create MeshFlow shader module: {error:?}"))?;
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(module)
        .name(shader::MAIN_ENTRY_POINT);
    let info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(layout);
    let result = unsafe { device.create_compute_pipelines(pipeline_cache, &[info], None) }
        .map_err(|(_, error)| format!("create MeshFlow compute pipeline: {error:?}"))
        .map(|pipelines| pipelines[0]);
    unsafe { device.destroy_shader_module(module, None) };
    result
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
        .ok_or_else(|| format!("no Vulkan memory type for MeshFlow {flags:?}"))
}
