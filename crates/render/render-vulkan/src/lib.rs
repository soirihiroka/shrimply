#![cfg(target_os = "linux")]

mod buffer;
pub mod device;
mod vector;
mod video;
pub use buffer::Buffer;
pub use video::{VideoFrame, decoder};

use ash::vk;
use device::Device;
use shrimply_render_core::Nv12LayerParams;
use std::{collections::HashMap, ffi::CString, sync::Arc};

struct Field {
    name: &'static str,
    offset: usize,
    size: usize,
    boolean: bool,
}
struct KernelLayout {
    name: &'static str,
    group: [usize; 3],
    fields: &'static [Field],
}
struct Module {
    code: &'static [u8],
    kernels: &'static [KernelLayout],
}
include!(concat!(env!("OUT_DIR"), "/kernels.rs"));

pub struct Arguments {
    layout: &'static KernelLayout,
    bytes: Vec<u8>,
}
impl Arguments {
    pub fn set(&mut self, name: &str, bytes: &[u8]) -> Result<&mut Self, String> {
        let field = self
            .layout
            .fields
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| format!("Unknown {} argument {name}", self.layout.name))?;
        if field.boolean && field.size == size_of::<u32>() && bytes.len() == 1 {
            self.bytes[field.offset..field.offset + field.size]
                .copy_from_slice(&u32::from(bytes[0]).to_ne_bytes());
            return Ok(self);
        }
        if field.size != bytes.len() {
            return Err(format!(
                "{} argument {name} requires {} bytes, received {}",
                self.layout.name,
                field.size,
                bytes.len()
            ));
        }
        self.bytes[field.offset..field.offset + field.size].copy_from_slice(bytes);
        Ok(self)
    }
    pub fn set_matrix3(
        &mut self,
        name: &str,
        matrix: shrimply_render_core::math::Mat3,
    ) -> Result<&mut Self, String> {
        for (axis, column) in [
            ("x_axis", matrix.x_axis),
            ("y_axis", matrix.y_axis),
            ("z_axis", matrix.z_axis),
        ] {
            let field_name = format!("{name}.{axis}");
            let length = self
                .layout
                .fields
                .iter()
                .find(|f| f.name == field_name)
                .ok_or_else(|| format!("Missing Vulkan matrix argument {field_name}"))?
                .size;
            let bytes = column.extend(0.0).to_array().map(f32::to_ne_bytes);
            self.set(&field_name, &bytes.as_flattened()[..length])?;
        }
        Ok(self)
    }
}

struct Pool {
    device: Arc<Device>,
    raw: vk::CommandPool,
}
impl Drop for Pool {
    fn drop(&mut self) {
        unsafe {
            self.device.raw.destroy_command_pool(self.raw, None);
        }
    }
}

pub struct Submission {
    pool: Arc<Pool>,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    descriptor: Option<vk::DescriptorPool>,
    _resources: Vec<Buffer>,
    _video: Option<ffmpeg_next::frame::Video>,
    _prior: Vec<Submission>,
    _images: Vec<skia_safe::Image>,
}
impl Submission {
    pub fn completed(&self) -> Result<bool, String> {
        unsafe { self.pool.device.raw.get_fence_status(self.fence) }
            .map_err(|e| format!("poll Intel GPU: {e:?}"))
    }
    pub fn wait(&self) -> Result<(), String> {
        unsafe {
            self.pool
                .device
                .raw
                .wait_for_fences(&[self.fence], true, u64::MAX)
        }
        .map_err(|e| format!("wait for Intel GPU: {e:?}"))
    }
}
impl Drop for Submission {
    fn drop(&mut self) {
        if self.wait().is_err() {
            std::process::abort();
        }
        unsafe {
            let device = &self.pool.device.raw;
            device.destroy_fence(self.fence, None);
            if let Some(pool) = self.descriptor {
                device.destroy_descriptor_pool(pool, None);
            }
            device.free_command_buffers(self.pool.raw, &[self.command]);
        }
    }
}

pub struct Frame {
    submission: Submission,
    output: Buffer,
}
impl Frame {
    pub fn into_parts(self) -> (Buffer, Submission) {
        (self.output, self.submission)
    }
    pub fn completed(&self) -> Result<bool, String> {
        self.submission.completed()
    }
}

struct Kernel {
    pipeline: vk::Pipeline,
    layout: &'static KernelLayout,
}
pub struct Renderer {
    skia: Option<skia_safe::gpu::DirectContext>,
    device: Arc<Device>,
    pool: Arc<Pool>,
    descriptor_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    kernels: HashMap<&'static str, Kernel>,
}
impl Renderer {
    pub fn new() -> Result<Self, String> {
        let device = Device::get()?;
        let pool = unsafe {
            device.raw.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(device.family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
        }
        .map_err(|e| format!("create Vulkan command pool: {e:?}"))?;
        let bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE)];
        let descriptor_layout = unsafe {
            device.raw.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )
        }
        .map_err(|e| format!("create Vulkan descriptors: {e:?}"))?;
        let layouts = [descriptor_layout];
        let pipeline_layout = unsafe {
            device.raw.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts),
                None,
            )
        }
        .map_err(|e| format!("create Vulkan pipeline layout: {e:?}"))?;
        Ok(Self {
            skia: None,
            pool: Arc::new(Pool {
                device: device.clone(),
                raw: pool,
            }),
            device,
            descriptor_layout,
            pipeline_layout,
            kernels: HashMap::new(),
        })
    }
    pub fn warmup(&mut self) -> Result<(), String> {
        self.kernel("composite_nv12_layers").map(|_| ())
    }
    fn kernel(&mut self, name: &str) -> Result<&Kernel, String> {
        if !self.kernels.contains_key(name) {
            let (module, layout) = MODULES
                .iter()
                .find_map(|m| m.kernels.iter().find(|k| k.name == name).map(|k| (m, k)))
                .ok_or_else(|| format!("Unknown Vulkan kernel {name}"))?;
            let code = ash::util::read_spv(&mut std::io::Cursor::new(module.code))
                .map_err(|e| e.to_string())?;
            let shader = unsafe {
                self.device
                    .raw
                    .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)
            }
            .map_err(|e| format!("load Slang Vulkan module: {e:?}"))?;
            let entry = CString::new(name).map_err(|e| e.to_string())?;
            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(shader)
                .name(&entry);
            let pipelines = unsafe {
                self.device.raw.create_compute_pipelines(
                    vk::PipelineCache::null(),
                    &[vk::ComputePipelineCreateInfo::default()
                        .stage(stage)
                        .layout(self.pipeline_layout)],
                    None,
                )
            };
            unsafe {
                self.device.raw.destroy_shader_module(shader, None);
            }
            let pipeline = pipelines.map_err(|(created, e)| {
                for pipeline in created {
                    unsafe {
                        self.device.raw.destroy_pipeline(pipeline, None);
                    }
                }
                format!("compile Slang Vulkan kernel {name}: {e:?}")
            })?[0];
            self.kernels
                .insert(layout.name, Kernel { pipeline, layout });
        }
        Ok(self.kernels.get(name).expect("inserted Vulkan kernel"))
    }
    pub fn arguments(&mut self, name: &str) -> Result<Arguments, String> {
        let layout = self.kernel(name)?.layout;
        let size = layout
            .fields
            .iter()
            .map(|f| f.offset + f.size)
            .max()
            .unwrap_or(0)
            .next_multiple_of(16);
        Ok(Arguments {
            layout,
            bytes: vec![0; size],
        })
    }
    pub fn upload(&self, bytes: &[u8]) -> Result<Buffer, String> {
        let buffer = Buffer::new(self.device.clone(), bytes.len(), true, false)?;
        buffer.write(bytes)?;
        Ok(buffer)
    }
    pub fn allocate(&self, length: usize) -> Result<Buffer, String> {
        Buffer::new(self.device.clone(), length, false, false)
    }
    fn command(&self) -> Result<vk::CommandBuffer, String> {
        let command = unsafe {
            self.device.raw.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.pool.raw)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(|e| format!("allocate Vulkan command: {e:?}"))?[0];
        unsafe {
            self.device.raw.begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
        }
        .map_err(|e| format!("begin Vulkan command: {e:?}"))?;
        Ok(command)
    }
    fn submit(
        &self,
        command: vk::CommandBuffer,
        resources: Vec<Buffer>,
        descriptor: Option<vk::DescriptorPool>,
    ) -> Result<Submission, String> {
        unsafe { self.device.raw.end_command_buffer(command) }
            .map_err(|e| format!("finish Vulkan command: {e:?}"))?;
        let fence = unsafe {
            self.device
                .raw
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .map_err(|e| format!("create Vulkan fence: {e:?}"))?;
        let signals = resources
            .iter()
            .filter_map(Buffer::ready)
            .collect::<Vec<_>>();
        self.device.submit(&[command], fence, &signals)?;
        Ok(Submission {
            pool: self.pool.clone(),
            command,
            fence,
            descriptor,
            _resources: resources,
            _video: None,
            _prior: Vec::new(),
            _images: Vec::new(),
        })
    }
    /// # Safety
    /// Every GPU address in arguments must reference a retained resource of sufficient size.
    pub unsafe fn dispatch(
        &mut self,
        arguments: Arguments,
        mut resources: Vec<Buffer>,
        extent: [usize; 3],
    ) -> Result<Submission, String> {
        if extent.contains(&0) {
            return Err("Vulkan dispatch dimensions must be nonzero".into());
        }
        let uniforms = self.upload(&arguments.bytes)?;
        let kernel = self.kernel(arguments.layout.name)?;
        let pipeline = kernel.pipeline;
        let group = kernel.layout.group;
        let sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)];
        let pool = unsafe {
            self.device.raw.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&sizes),
                None,
            )
        }
        .map_err(|e| format!("create dispatch descriptors: {e:?}"))?;
        let layouts = [self.descriptor_layout];
        let set = unsafe {
            self.device.raw.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(pool)
                    .set_layouts(&layouts),
            )
        }
        .map_err(|e| format!("allocate dispatch descriptors: {e:?}"))?[0];
        let info = [vk::DescriptorBufferInfo::default()
            .buffer(uniforms.raw())
            .range(uniforms.len() as u64)];
        unsafe {
            self.device.raw.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(&info)],
                &[],
            );
        }
        let command = self.command()?;
        unsafe {
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::MEMORY_WRITE | vk::AccessFlags::HOST_WRITE)
                    .dst_access_mask(
                        vk::AccessFlags::SHADER_READ
                            | vk::AccessFlags::SHADER_WRITE
                            | vk::AccessFlags::UNIFORM_READ,
                    )],
                &[],
                &[],
            );
            self.device
                .raw
                .cmd_bind_pipeline(command, vk::PipelineBindPoint::COMPUTE, pipeline);
            self.device.raw.cmd_bind_descriptor_sets(
                command,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[set],
                &[],
            );
            self.device.raw.cmd_dispatch(
                command,
                extent[0].div_ceil(group[0]) as u32,
                extent[1].div_ceil(group[1]) as u32,
                extent[2].div_ceil(group[2]) as u32,
            );
        }
        resources.push(uniforms);
        self.submit(command, resources, Some(pool))
    }

    pub fn composite_buffers(
        &mut self,
        layers: &[(Nv12LayerParams, Buffer)],
        width: u32,
        height: u32,
        background: u32,
    ) -> Result<Frame, String> {
        self.composite_buffers_with_transforms(layers, width, height, background, &[])
    }
    pub fn composite_buffers_with_transforms(
        &mut self,
        layers: &[(Nv12LayerParams, Buffer)],
        width: u32,
        height: u32,
        background: u32,
        transforms: &[shrimply_render_core::math::Mat3],
    ) -> Result<Frame, String> {
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or("Canvas size overflow")?;
        u32::try_from(count).map_err(|_| "Canvas exceeds the Slang pixel index range")?;
        let output = self.allocate(count.checked_mul(4).ok_or("Canvas byte size overflow")?)?;
        let mut resources = vec![output.clone()];
        let mut parameters = Vec::with_capacity(layers.len());
        for (params, buffer) in layers {
            let mut params = *params;
            if params.source_width == 0
                || params.source_height == 0
                || params
                    .rgba_pitch
                    .checked_mul(params.source_height as usize)
                    .is_none_or(|length| length > buffer.len())
            {
                return Err("Invalid Vulkan layer dimensions".into());
            }
            params.rgba = buffer.address() as usize as *const u32;
            params.y_plane = std::ptr::null();
            params.uv_plane = std::ptr::null();
            params.canvas_width = width;
            parameters.push(params);
            resources.push(buffer.clone());
        }
        let layers_address = if parameters.is_empty() {
            0
        } else {
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    parameters.as_ptr().cast(),
                    std::mem::size_of_val(parameters.as_slice()),
                )
            };
            let buffer = self.upload(bytes)?;
            let address = buffer.address();
            resources.push(buffer);
            address
        };
        let transforms_address = if transforms.is_empty() {
            0
        } else {
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    transforms.as_ptr().cast(),
                    std::mem::size_of_val(transforms),
                )
            };
            let buffer = self.upload(bytes)?;
            let address = buffer.address();
            resources.push(buffer);
            address
        };
        let mut arguments = self.arguments("composite_nv12_layers")?;
        arguments
            .set("layers", &layers_address.to_ne_bytes())?
            .set("layer_count", &(parameters.len() as u64).to_ne_bytes())?
            .set("transforms", &transforms_address.to_ne_bytes())?
            .set("transform_count", &(transforms.len() as u64).to_ne_bytes())?
            .set("output", &output.address().to_ne_bytes())?
            .set("output_count", &(count as u64).to_ne_bytes())?
            .set("background", &background.to_ne_bytes())?;
        let submission = unsafe { self.dispatch(arguments, resources, [count, 1, 1]) }?;
        Ok(Frame { submission, output })
    }

    pub fn presentation_buffer(&self, frame: &Frame) -> Result<(Buffer, Submission), String> {
        let output = Buffer::new(self.device.clone(), frame.output.len(), false, true)?;
        let command = self.command()?;
        unsafe {
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)],
                &[],
                &[],
            );
            self.device.raw.cmd_copy_buffer(
                command,
                frame.output.raw(),
                output.raw(),
                &[vk::BufferCopy::default().size(output.len() as u64)],
            );
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .src_queue_family_index(self.device.family)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                    .buffer(output.raw())
                    .size(vk::WHOLE_SIZE)],
                &[],
            );
        }
        let submission = self.submit(command, vec![frame.output.clone(), output.clone()], None)?;
        Ok((output, submission))
    }

    pub fn readback(&self, source: &Buffer) -> Result<Vec<u8>, String> {
        let output = Buffer::new(self.device.clone(), source.len(), true, false)?;
        let command = self.command()?;
        unsafe {
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)],
                &[],
                &[],
            );
            self.device.raw.cmd_copy_buffer(
                command,
                source.raw(),
                output.raw(),
                &[vk::BufferCopy::default().size(source.len() as u64)],
            );
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::HOST,
                vk::DependencyFlags::empty(),
                &[vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::HOST_READ)],
                &[],
                &[],
            );
        }
        self.submit(command, vec![source.clone(), output.clone()], None)?
            .wait()?;
        output.copy_bytes()
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        if unsafe { self.device.raw.device_wait_idle() }.is_err() {
            std::process::abort();
        }
        if let Some(skia) = &mut self.skia {
            skia.release_resources_and_abandon();
        }
        unsafe {
            for kernel in self.kernels.values() {
                self.device.raw.destroy_pipeline(kernel.pipeline, None);
            }
            self.device
                .raw
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device
                .raw
                .destroy_descriptor_set_layout(self.descriptor_layout, None);
        }
    }
}
