#![cfg(target_os = "linux")]

mod buffer;
mod texture;
pub use texture::Texture;
pub mod device;
mod vaapi;
mod vector;
pub use buffer::Buffer;
pub use vaapi::{VideoFrame, decoder};

use ash::vk;
use device::Device;
use shrimply_render_core::Nv12LayerParams;
use std::{
    collections::{HashMap, VecDeque},
    ffi::CString,
    rc::Rc,
    sync::Arc,
};

// Vulkan guarantees at least 128 bytes. Larger argument blocks use a buffer.
const PUSH_CONSTANT_BYTES: u32 = 128;

struct Field {
    name: &'static str,
    offset: usize,
    size: usize,
    boolean: bool,
}
struct KernelLayout {
    name: &'static str,
    entry: &'static str,
    buffered: bool,
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
    code: &'static [u8],
    sample_methods: u32,
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
    pool: Rc<Pool>,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    submitted: bool,
    _resources: Vec<Buffer>,
    _video: Option<vaapi::MappedVideo>,
    _prior: Vec<Submission>,
    _images: Vec<skia_safe::Image>,
    _textures: Vec<Texture>,
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
        if self.submitted && self.wait().is_err() {
            std::process::abort();
        }
        unsafe {
            let device = &self.pool.device.raw;
            device.destroy_fence(self.fence, None);
            device.free_command_buffers(self.pool.raw, &[self.command]);
        }
    }
}

pub struct Frame {
    submission: Submission,
    pub output: Buffer,
}
impl Frame {
    pub fn into_parts(self) -> (Buffer, Submission) {
        (self.output, self.submission)
    }
    pub fn completed(&self) -> Result<bool, String> {
        self.submission.completed()
    }
}

pub struct Renderer {
    skia: Option<skia_safe::gpu::DirectContext>,
    device: Arc<Device>,
    pool: Rc<Pool>,
    pipeline_layout: vk::PipelineLayout,
    kernels: HashMap<(&'static str, u32), vk::Pipeline>,
    video_contexts: VecDeque<((usize, i32), Rc<vaapi::FramesContext>)>,
    video_context_limit: usize,
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
        let pool = Rc::new(Pool {
            device: device.clone(),
            raw: pool,
        });
        let pipeline_layout = unsafe {
            device.raw.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&[
                    vk::PushConstantRange::default()
                        .stage_flags(vk::ShaderStageFlags::COMPUTE)
                        .size(PUSH_CONSTANT_BYTES),
                ]),
                None,
            )
        }
        .map_err(|e| format!("create Vulkan pipeline layout: {e:?}"))?;
        Ok(Self {
            skia: None,
            pool,
            device,
            pipeline_layout,
            kernels: HashMap::new(),
            video_contexts: VecDeque::new(),
            // A source decoder and an encoder; the compositor applies its decoder limit.
            video_context_limit: 2,
        })
    }
    pub fn set_video_context_limit(&mut self, maximum: usize) {
        assert!(maximum > 0, "video context limit must be positive");
        self.video_context_limit = maximum;
        while self.video_contexts.len() > maximum {
            self.video_contexts.pop_front();
        }
    }
    pub fn clear_video_contexts(&mut self) {
        self.video_contexts.clear();
    }
    pub fn warmup(&mut self) -> Result<(), String> {
        let mut arguments = self.arguments("composite_nv12_layers")?;
        arguments.sample_methods = (1 << shrimply_render_core::VideoSampleMethod::Nearest as u32)
            | (1 << shrimply_render_core::VideoSampleMethod::Bilinear as u32);
        self.kernel(&arguments).map(|_| ())
    }
    fn kernel(&mut self, arguments: &Arguments) -> Result<vk::Pipeline, String> {
        let layout = arguments.layout;
        let key = (layout.name, arguments.sample_methods);
        if !self.kernels.contains_key(&key) {
            let code = ash::util::read_spv(&mut std::io::Cursor::new(arguments.code))
                .map_err(|e| e.to_string())?;
            let shader = unsafe {
                self.device
                    .raw
                    .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)
            }
            .map_err(|e| format!("load Slang Vulkan module: {e:?}"))?;
            let entry = CString::new(layout.entry).map_err(|e| e.to_string())?;
            let entries = [vk::SpecializationMapEntry::default()
                .constant_id(0)
                .size(size_of::<u32>())];
            let data = arguments.sample_methods.to_ne_bytes();
            let specialization = vk::SpecializationInfo::default()
                .map_entries(&entries)
                .data(&data);
            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(shader)
                .name(&entry)
                .specialization_info(&specialization);
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
                format!("compile Slang Vulkan kernel {}: {e:?}", layout.name)
            })?[0];
            self.kernels.insert(key, pipeline);
        }
        Ok(*self.kernels.get(&key).expect("inserted Vulkan kernel"))
    }
    pub fn arguments(&self, name: &str) -> Result<Arguments, String> {
        let (module, layout) = MODULES
            .iter()
            .find_map(|m| m.kernels.iter().find(|k| k.name == name).map(|k| (m, k)))
            .ok_or_else(|| format!("Unknown Vulkan kernel {name}"))?;
        let size = layout
            .fields
            .iter()
            .map(|f| f.offset + f.size)
            .max()
            .unwrap_or(0)
            .next_multiple_of(16);
        Ok(Arguments {
            layout,
            code: module.code,
            sample_methods: u32::MAX,
            bytes: vec![0; size],
        })
    }
    pub fn upload(&self, bytes: &[u8]) -> Result<Buffer, String> {
        let buffer = Buffer::new(self.device.clone(), bytes.len(), true)?;
        buffer.write(bytes)?;
        Ok(buffer)
    }
    pub fn allocate(&self, length: usize) -> Result<Buffer, String> {
        Buffer::new(self.device.clone(), length, false)
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
        .map_err(|e| {
            unsafe {
                self.device
                    .raw
                    .free_command_buffers(self.pool.raw, &[command]);
            }
            format!("begin Vulkan command: {e:?}")
        })?;
        Ok(command)
    }
    fn submit(
        &self,
        command: vk::CommandBuffer,
        resources: Vec<Buffer>,
    ) -> Result<Submission, String> {
        let mut submission = Submission {
            pool: self.pool.clone(),
            command,
            fence: vk::Fence::null(),
            submitted: false,
            _resources: resources,
            _video: None,
            _prior: Vec::new(),
            _images: Vec::new(),
            _textures: Vec::new(),
        };
        unsafe { self.device.raw.end_command_buffer(command) }
            .map_err(|e| format!("finish Vulkan command: {e:?}"))?;
        let fence = unsafe {
            self.device
                .raw
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .map_err(|e| format!("create Vulkan fence: {e:?}"))?;
        submission.fence = fence;
        self.device.submit(&[command], fence, &[])?;
        submission.submitted = true;
        Ok(submission)
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
        let pipeline = self.kernel(&arguments)?;
        let group = arguments.layout.group;
        let mut groups = [0_u32; 3];
        for axis in 0..groups.len() {
            groups[axis] = u32::try_from(extent[axis].div_ceil(group[axis]))
                .map_err(|_| "Vulkan dispatch group count overflow")?;
            if groups[axis] > self.device.limits.max_compute_work_group_count[axis] {
                return Err(format!(
                    "Intel dispatch exceeds the GPU workgroup limit on axis {axis}: {} > {}",
                    groups[axis], self.device.limits.max_compute_work_group_count[axis]
                ));
            }
        }
        let push_data = if arguments.layout.buffered {
            let uniforms = self.upload(&arguments.bytes)?;
            let address = uniforms.address().to_ne_bytes().to_vec();
            resources.push(uniforms);
            address
        } else {
            arguments.bytes
        };
        if push_data.len() > PUSH_CONSTANT_BYTES as usize {
            return Err(format!(
                "{} requires an argument buffer",
                arguments.layout.name
            ));
        }
        let command = self.command()?;
        unsafe {
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
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
            self.device.raw.cmd_push_constants(
                command,
                self.pipeline_layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                &push_data,
            );
            self.device
                .raw
                .cmd_dispatch(command, groups[0], groups[1], groups[2]);
        }
        self.submit(command, resources)
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
        if transforms.iter().any(|transform| !transform.is_finite()) {
            return Err("Vulkan motion transforms must be finite".into());
        }
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
                || params.kind != shrimply_render_core::LayerKind::Rgba
                || params.rgba_pitch < params.source_width as usize * size_of::<u32>()
                || !params.rgba_pitch.is_multiple_of(size_of::<u32>())
                || params.motion_transform_count > params.motion_sample_count
                || params
                    .motion_transform_offset
                    .checked_add(params.motion_transform_count)
                    .is_none_or(|end| end as usize > transforms.len())
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
        arguments.sample_methods = layers.iter().fold(0, |mask, (layer, _)| {
            mask | (1 << layer.sample_method as u32)
        });
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

    pub fn presentation_texture(
        &self,
        source: &Buffer,
        width: u32,
        height: u32,
    ) -> Result<(Texture, Submission), String> {
        if (width as usize)
            .checked_mul(height as usize)
            .and_then(|count| count.checked_mul(size_of::<u32>()))
            != Some(source.len())
        {
            return Err("Presentation dimensions do not match the composited buffer".into());
        }
        let texture = Texture::new(self.device.clone(), width, height)?;
        let command = self.command()?;
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(texture.levels())
            .layer_count(1);
        let begin = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .image(texture.image())
            .subresource_range(range);
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
                &[begin],
            );
            self.device.raw.cmd_copy_buffer_to_image(
                command,
                source.raw(),
                texture.image(),
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[vk::BufferImageCopy::default()
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    })],
            );
            // Build mipmaps once on Vulkan. GL consumers only sample this image,
            // so multiple preview surfaces cannot race while generating mips.
            for level in 1..texture.levels() {
                let previous = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(level - 1)
                    .level_count(1)
                    .layer_count(1);
                self.device.raw.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(texture.image())
                        .subresource_range(previous)],
                );
                let layers = vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .layer_count(1);
                self.device.raw.cmd_blit_image(
                    command,
                    texture.image(),
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    texture.image(),
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[vk::ImageBlit::default()
                        .src_subresource(layers.mip_level(level - 1))
                        .src_offsets([
                            vk::Offset3D::default(),
                            vk::Offset3D {
                                x: (width >> (level - 1)).max(1) as i32,
                                y: (height >> (level - 1)).max(1) as i32,
                                z: 1,
                            },
                        ])
                        .dst_subresource(layers.mip_level(level))
                        .dst_offsets([
                            vk::Offset3D::default(),
                            vk::Offset3D {
                                x: (width >> level).max(1) as i32,
                                y: (height >> level).max(1) as i32,
                                z: 1,
                            },
                        ])],
                    vk::Filter::LINEAR,
                );
            }
            let mut releases = Vec::new();
            for (base, count, layout) in [
                (
                    0,
                    texture.levels() - 1,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                ),
                (
                    texture.levels() - 1,
                    1,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                ),
            ] {
                if count != 0 {
                    releases.push(
                        vk::ImageMemoryBarrier::default()
                            .old_layout(layout)
                            .new_layout(vk::ImageLayout::GENERAL)
                            .src_queue_family_index(self.device.family)
                            .dst_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                            .src_access_mask(
                                vk::AccessFlags::TRANSFER_READ | vk::AccessFlags::TRANSFER_WRITE,
                            )
                            .image(texture.image())
                            .subresource_range(range.base_mip_level(base).level_count(count)),
                    );
                }
            }
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &releases,
            );
        }
        let mut submission = self.submit(command, vec![source.clone()])?;
        submission._textures.push(texture.clone());
        Ok((texture, submission))
    }

    pub fn readback(&self, source: &Buffer) -> Result<Vec<u8>, String> {
        let output = Buffer::new(self.device.clone(), source.len(), true)?;
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
        self.submit(command, vec![source.clone(), output.clone()])?
            .wait()?;
        output.copy_bytes()
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        let _guard = self.device.queue_lock();
        if unsafe { self.device.raw.queue_wait_idle(self.device.queue) }.is_err() {
            std::process::abort();
        }
        if let Some(skia) = &mut self.skia {
            skia.release_resources_and_abandon();
        }
        unsafe {
            for pipeline in self.kernels.values() {
                self.device.raw.destroy_pipeline(*pipeline, None);
            }
            self.device
                .raw
                .destroy_pipeline_layout(self.pipeline_layout, None);
        }
    }
}
