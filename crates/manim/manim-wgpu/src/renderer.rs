use std::borrow::Cow;
use std::sync::{Arc, Weak, mpsc};

use hashbrown::HashMap;
use shrimply_manim_ir::{
    CompareFunction, CompiledAnimation, PipelineResource, StencilFaceState, StencilOperation,
    TextureAddress, TextureFilter, TextureResource,
};

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
const PIXEL_BYTES: u32 = 4;

pub struct PreparedAnimation {
    animation: Arc<CompiledAnimation>,
}

impl PreparedAnimation {
    pub fn new(animation: Arc<CompiledAnimation>) -> Result<Self, String> {
        for frame in animation.frames() {
            for draw in &frame.draws {
                let pipeline = animation
                    .pipeline(draw.pipeline)
                    .ok_or_else(|| format!("missing Manim pipeline {}", draw.pipeline))?;
                let geometry = animation
                    .geometry_resource(draw.geometry)
                    .ok_or_else(|| format!("missing Manim geometry {}", draw.geometry))?;
                let uniforms = animation
                    .uniform_block(draw.uniforms)
                    .ok_or_else(|| format!("missing Manim uniforms {}", draw.uniforms))?;
                if pipeline.source.is_empty() {
                    return Err(format!("Manim pipeline {} has no WGSL source", pipeline.id));
                }
                if geometry.bytes.is_empty() || uniforms.bytes.is_empty() {
                    return Err(format!(
                        "Manim draw in frame {} has an empty buffer",
                        frame.index
                    ));
                }
                if draw.vertex_count == 0 {
                    return Err(format!(
                        "Manim draw in frame {} has no vertices",
                        frame.index
                    ));
                }
                for binding in &draw.textures {
                    animation
                        .texture(binding.texture)
                        .ok_or_else(|| format!("missing Manim texture {}", binding.texture))?;
                }
            }
        }
        Ok(Self { animation })
    }

    pub fn scene(&self) -> &shrimply_manim_ir::SceneHeader {
        self.animation.scene()
    }

    pub fn frame_count(&self) -> usize {
        self.animation.frames().len()
    }

    pub fn is_empty(&self) -> bool {
        self.animation.frames().is_empty()
    }
}

pub struct RenderedFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalFrameDescriptor {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
}

impl RenderedFrame {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    animations: Vec<CachedAnimation>,
    targets: HashMap<usize, Target>,
}

struct CachedAnimation {
    animation: Weak<CompiledAnimation>,
    gpu: GpuAnimation,
}

type ResourceGroupKey = (u32, u64, Vec<(u16, u32)>);

struct GpuAnimation {
    frames: Vec<GpuFrame>,
    pipelines: HashMap<u32, GpuPipeline>,
    textures: HashMap<u32, GpuTexture>,
    camera: wgpu::Buffer,
    geometry: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    indices: wgpu::Buffer,
    camera_offsets: Vec<(u32, u64)>,
    geometry_offsets: HashMap<u32, (u32, u64)>,
    uniform_offsets: HashMap<u32, (u32, u64)>,
    index_offsets: HashMap<Vec<u32>, std::ops::Range<u64>>,
    frame_groups: HashMap<(u32, u64), wgpu::BindGroup>,
    mobject_groups: HashMap<(u32, u64), wgpu::BindGroup>,
    resource_groups: HashMap<ResourceGroupKey, wgpu::BindGroup>,
}

struct GpuFrame {
    bundle: wgpu::RenderBundle,
}

struct GpuDraw {
    pipeline: Arc<wgpu::RenderPipeline>,
    frame_bind_group: wgpu::BindGroup,
    mobject_bind_group: wgpu::BindGroup,
    resource_bind_group: wgpu::BindGroup,
    camera_offset: u32,
    mobject_offset: u32,
    resource_offset: u32,
    index_buffer: Option<(wgpu::Buffer, std::ops::Range<u64>)>,
    vertices: u32,
}

struct PendingDraw {
    pipeline: u32,
    mobject_offset: u32,
    mobject_size: u64,
    resource_offset: u32,
    resource_size: u64,
    textures: Vec<(u16, u32)>,
    index_range: Option<std::ops::Range<u64>>,
    vertices: u32,
}

struct GpuPipeline {
    pipeline: Arc<wgpu::RenderPipeline>,
    frame_layout: wgpu::BindGroupLayout,
    mobject_layout: wgpu::BindGroupLayout,
    resource_layout: wgpu::BindGroupLayout,
}

struct GpuTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

struct Target {
    width: u32,
    height: u32,
    samples: u32,
    output: wgpu::Texture,
    output_view: wgpu::TextureView,
    _multisample: Option<wgpu::Texture>,
    multisample_view: Option<wgpu::TextureView>,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

impl Renderer {
    pub fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("request Manim WGPU adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Shrimply Manim WGPU device"),
            ..Default::default()
        }))
        .map_err(|e| format!("create Manim WGPU device: {e}"))?;
        Ok(Self::from_device(device, queue))
    }
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            device,
            queue,
            animations: Vec::new(),
            targets: HashMap::new(),
        }
    }

    pub fn render_rgba_for_validation(
        &mut self,
        animation: &PreparedAnimation,
        frame_index: usize,
    ) -> Result<RenderedFrame, String> {
        let descriptor = Self::external_frame_descriptor(animation);
        let output = make_render_texture(
            &self.device,
            "Manim validation output",
            descriptor.width,
            descriptor.height,
            1,
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Manim validation frame"),
            });
        self.encode(0, animation, frame_index, output, &mut encoder)?;
        self.queue.submit([encoder.finish()]);

        let pixels = self
            .targets
            .get(&0)
            .expect("Manim target was initialized")
            .read_pixels(&self.device, &self.queue)?;
        Ok(RenderedFrame {
            width: descriptor.width,
            height: descriptor.height,
            pixels,
        })
    }

    pub fn external_frame_descriptor(animation: &PreparedAnimation) -> ExternalFrameDescriptor {
        let scene = animation.scene();
        ExternalFrameDescriptor {
            width: scene.width.max(1),
            height: scene.height.max(1),
            samples: scene.samples.max(1),
        }
    }

    pub fn target_descriptor(&self, slot: usize) -> Option<ExternalFrameDescriptor> {
        self.targets.get(&slot).map(Target::descriptor)
    }

    pub fn release_render_surfaces(&mut self) -> bool {
        let released = !self.targets.is_empty();
        self.targets.clear();
        released
    }

    pub fn release_gpu_animation_resources(&mut self) -> bool {
        let released = !self.animations.is_empty();
        self.animations.clear();
        released
    }

    pub fn encode(
        &mut self,
        slot: usize,
        animation: &PreparedAnimation,
        frame_index: usize,
        output: wgpu::Texture,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<ExternalFrameDescriptor, String> {
        self.animations
            .retain(|cached| cached.animation.strong_count() != 0);
        let weak = Arc::downgrade(&animation.animation);
        let cache_index = self
            .animations
            .iter()
            .position(|cached| Weak::ptr_eq(&cached.animation, &weak));
        let started = std::time::Instant::now();
        let (cache_index, first_materialization) = match cache_index {
            Some(index) => (index, false),
            None => {
                let gpu = GpuAnimation::new(&self.device, &self.queue, animation)?;
                self.animations.push(CachedAnimation {
                    animation: weak,
                    gpu,
                });
                (self.animations.len() - 1, true)
            }
        };
        let descriptor = Self::external_frame_descriptor(animation);
        if self.target_descriptor(slot) != Some(descriptor)
            || self
                .targets
                .get(&slot)
                .is_some_and(|target| target.output != output)
        {
            self.targets
                .insert(slot, Target::new(&self.device, descriptor, output)?);
        }
        let frame = self.animations[cache_index].gpu.frame(frame_index)?;
        if first_materialization {
            tracing::info!(
                frames = animation.frame_count(),
                frame = frame_index,
                elapsed_ms = started.elapsed().as_millis(),
                "Manim WGPU animation materialized",
            );
        }
        let source = animation
            .animation
            .frames()
            .get(frame_index)
            .expect("validated Manim frame disappeared");
        let target = self
            .targets
            .get(&slot)
            .expect("Manim target was initialized");
        target.render(encoder, frame, source.camera.background_rgba.to_array());
        Ok(descriptor)
    }

    pub fn remove_target(&mut self, slot: usize) -> bool {
        self.targets.remove(&slot).is_some()
    }

    pub fn clear_unused(&mut self) -> bool {
        let before = self.animations.len();
        self.animations
            .retain(|cached| cached.animation.strong_count() != 0);
        before != self.animations.len()
    }
}

impl GpuAnimation {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        prepared: &PreparedAnimation,
    ) -> Result<Self, String> {
        let animation = &prepared.animation;
        let limits = device.limits();
        let uniform_alignment = limits.min_uniform_buffer_offset_alignment as usize;
        let storage_alignment = limits.min_storage_buffer_offset_alignment as usize;
        let mut camera_bytes = Vec::new();
        let mut uniform_bytes = Vec::new();
        let mut geometry_bytes = Vec::new();
        let mut index_bytes = Vec::new();
        let mut camera_offsets = Vec::with_capacity(animation.frames().len());
        let mut uniform_offsets = HashMap::<u32, (u32, u64)>::new();
        let mut geometry_offsets = HashMap::<u32, (u32, u64)>::new();
        let mut index_offsets = HashMap::<Vec<u32>, std::ops::Range<u64>>::new();

        for frame in animation.frames() {
            let offset = append_aligned(
                &mut camera_bytes,
                &frame.camera.uniforms,
                uniform_alignment,
                "Manim camera uniforms",
            )?;
            let size = u64::try_from(frame.camera.uniforms.len())
                .map_err(|_| "Manim camera uniforms are too large".to_string())?;
            camera_offsets.push((offset, size));
            for draw in &frame.draws {
                if !uniform_offsets.contains_key(&draw.uniforms) {
                    let resource = animation
                        .uniform_block(draw.uniforms)
                        .expect("validated Manim uniforms disappeared");
                    let offset = append_aligned(
                        &mut uniform_bytes,
                        &resource.bytes,
                        uniform_alignment,
                        "Manim uniforms",
                    )?;
                    let size = u64::try_from(resource.bytes.len())
                        .map_err(|_| "Manim uniforms are too large".to_string())?;
                    uniform_offsets.insert(draw.uniforms, (offset, size));
                }
                if !geometry_offsets.contains_key(&draw.geometry) {
                    let resource = animation
                        .geometry_resource(draw.geometry)
                        .expect("validated Manim geometry disappeared");
                    let offset = append_aligned(
                        &mut geometry_bytes,
                        &resource.bytes,
                        storage_alignment,
                        "Manim geometry",
                    )?;
                    let size = u64::try_from(resource.bytes.len())
                        .map_err(|_| "Manim geometry is too large".to_string())?;
                    geometry_offsets.insert(draw.geometry, (offset, size));
                }
                if !draw.indices.is_empty() && !index_offsets.contains_key(&draw.indices) {
                    let start = u64::try_from(index_bytes.len())
                        .map_err(|_| "Manim index data is too large".to_string())?;
                    for index in &draw.indices {
                        index_bytes.extend_from_slice(&index.to_ne_bytes());
                    }
                    let end = u64::try_from(index_bytes.len())
                        .map_err(|_| "Manim index data is too large".to_string())?;
                    index_offsets.insert(draw.indices.clone(), start..end);
                }
            }
        }

        pad_copy_bytes(&mut camera_bytes);
        pad_copy_bytes(&mut uniform_bytes);
        pad_copy_bytes(&mut geometry_bytes);
        pad_copy_bytes(&mut index_bytes);
        let camera = upload_immutable_buffer(
            device,
            queue,
            &camera_bytes,
            wgpu::BufferUsages::UNIFORM,
            "Manim camera uniform atlas",
        )?;
        let uniforms = upload_immutable_buffer(
            device,
            queue,
            &uniform_bytes,
            wgpu::BufferUsages::UNIFORM,
            "Manim uniform atlas",
        )?;
        let geometry = upload_immutable_buffer(
            device,
            queue,
            &geometry_bytes,
            wgpu::BufferUsages::STORAGE,
            "Manim geometry atlas",
        )?;
        let indices = upload_immutable_buffer(
            device,
            queue,
            &index_bytes,
            wgpu::BufferUsages::INDEX,
            "Manim index atlas",
        )?;
        let mut gpu = Self {
            frames: Vec::with_capacity(animation.frames().len()),
            pipelines: HashMap::new(),
            textures: HashMap::new(),
            camera,
            geometry,
            uniforms,
            indices,
            camera_offsets,
            geometry_offsets,
            uniform_offsets,
            index_offsets,
            frame_groups: HashMap::new(),
            mobject_groups: HashMap::new(),
            resource_groups: HashMap::new(),
        };
        for frame_index in 0..animation.frames().len() {
            let frame = gpu.make_frame(device, queue, prepared, frame_index)?;
            gpu.frames.push(frame);
        }
        Ok(gpu)
    }

    fn frame(&self, frame_index: usize) -> Result<&GpuFrame, String> {
        self.frames
            .get(frame_index)
            .ok_or_else(|| format!("Manim frame {frame_index} is out of range"))
    }

    fn make_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        prepared: &PreparedAnimation,
        frame_index: usize,
    ) -> Result<GpuFrame, String> {
        let animation = &prepared.animation;
        let frame = &animation.frames()[frame_index];
        let samples = animation.scene().samples.max(1);
        let (camera_offset, camera_size) = self.camera_offsets[frame_index];
        let mut pending = Vec::with_capacity(frame.draws.len());
        for draw in &frame.draws {
            let (resource_offset, resource_size) = self.geometry_offsets[&draw.geometry];
            let (mobject_offset, mobject_size) = self.uniform_offsets[&draw.uniforms];
            if !self.pipelines.contains_key(&draw.pipeline) {
                let source = animation
                    .pipeline(draw.pipeline)
                    .expect("validated Manim pipeline disappeared");
                self.pipelines
                    .insert(draw.pipeline, make_pipeline(device, source, samples)?);
            }
            for binding in &draw.textures {
                if !self.textures.contains_key(&binding.texture) {
                    let resource = animation
                        .texture(binding.texture)
                        .expect("validated Manim texture disappeared");
                    self.textures
                        .insert(binding.texture, make_texture(device, queue, resource)?);
                }
            }
            let index_range = self.index_offsets.get(&draw.indices).cloned();
            pending.push(PendingDraw {
                pipeline: draw.pipeline,
                mobject_offset,
                mobject_size,
                resource_offset,
                resource_size,
                textures: draw
                    .textures
                    .iter()
                    .map(|binding| (binding.binding, binding.texture))
                    .collect(),
                index_range,
                vertices: draw.vertex_count,
            });
        }

        let mut draws = Vec::with_capacity(pending.len());
        for draw in pending {
            let pipeline = &self.pipelines[&draw.pipeline];
            let frame_key = (draw.pipeline, camera_size);
            if !self.frame_groups.contains_key(&frame_key) {
                self.frame_groups.insert(
                    frame_key,
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Manim frame bind group"),
                        layout: &pipeline.frame_layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &self.camera,
                                offset: 0,
                                size: std::num::NonZeroU64::new(camera_size),
                            }),
                        }],
                    }),
                );
            }
            let mobject_key = (draw.pipeline, draw.mobject_size);
            if !self.mobject_groups.contains_key(&mobject_key) {
                self.mobject_groups.insert(
                    mobject_key,
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Manim mobject uniform bind group"),
                        layout: &pipeline.mobject_layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &self.uniforms,
                                offset: 0,
                                size: std::num::NonZeroU64::new(draw.mobject_size),
                            }),
                        }],
                    }),
                );
            }
            let resource_key = (draw.pipeline, draw.resource_size, draw.textures.clone());
            if !self.resource_groups.contains_key(&resource_key) {
                let mut entries = vec![wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.geometry,
                        offset: 0,
                        size: std::num::NonZeroU64::new(draw.resource_size),
                    }),
                }];
                if let Some((_, first_texture)) = draw.textures.first() {
                    entries.push(wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(
                            &self.textures[first_texture].sampler,
                        ),
                    });
                    entries.extend(draw.textures.iter().map(|(binding, texture)| {
                        wgpu::BindGroupEntry {
                            binding: u32::from(*binding),
                            resource: wgpu::BindingResource::TextureView(
                                &self.textures[texture].view,
                            ),
                        }
                    }));
                }
                self.resource_groups.insert(
                    resource_key.clone(),
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Manim resource bind group"),
                        layout: &pipeline.resource_layout,
                        entries: &entries,
                    }),
                );
            }
            draws.push(GpuDraw {
                pipeline: pipeline.pipeline.clone(),
                frame_bind_group: self.frame_groups[&frame_key].clone(),
                mobject_bind_group: self.mobject_groups[&mobject_key].clone(),
                resource_bind_group: self.resource_groups[&resource_key].clone(),
                camera_offset,
                mobject_offset: draw.mobject_offset,
                resource_offset: draw.resource_offset,
                index_buffer: draw.index_range.map(|range| (self.indices.clone(), range)),
                vertices: draw.vertices,
            });
        }
        let color_formats = [Some(COLOR_FORMAT)];
        let mut encoder =
            device.create_render_bundle_encoder(&wgpu::RenderBundleEncoderDescriptor {
                label: Some("Manim frame bundle encoder"),
                color_formats: &color_formats,
                depth_stencil: Some(wgpu::RenderBundleDepthStencil {
                    format: DEPTH_FORMAT,
                    depth_read_only: false,
                    stencil_read_only: false,
                }),
                sample_count: samples,
                multiview: None,
            });
        for draw in &draws {
            encoder.set_pipeline(&draw.pipeline);
            encoder.set_bind_group(0, &draw.frame_bind_group, &[draw.camera_offset]);
            encoder.set_bind_group(1, &draw.mobject_bind_group, &[draw.mobject_offset]);
            encoder.set_bind_group(2, &draw.resource_bind_group, &[draw.resource_offset]);
            if let Some((indices, range)) = &draw.index_buffer {
                encoder.set_index_buffer(indices.slice(range.clone()), wgpu::IndexFormat::Uint32);
                encoder.draw_indexed(0..draw.vertices, 0, 0..1);
            } else {
                encoder.draw(0..draw.vertices, 0..1);
            }
        }
        Ok(GpuFrame {
            bundle: encoder.finish(&wgpu::RenderBundleDescriptor {
                label: Some("Manim frame bundle"),
            }),
        })
    }
}

fn append_aligned(
    destination: &mut Vec<u8>,
    bytes: &[u8],
    alignment: usize,
    description: &str,
) -> Result<u32, String> {
    let aligned = destination
        .len()
        .checked_add(alignment - 1)
        .map(|length| length / alignment * alignment)
        .ok_or_else(|| format!("{description} offsets overflow"))?;
    destination.resize(aligned, 0);
    let offset = u32::try_from(aligned)
        .map_err(|_| format!("{description} offsets exceed the WGPU dynamic-offset range"))?;
    destination.extend_from_slice(bytes);
    Ok(offset)
}

fn pad_copy_bytes(bytes: &mut Vec<u8>) {
    let alignment = wgpu::COPY_BUFFER_ALIGNMENT as usize;
    bytes.resize(bytes.len().div_ceil(alignment) * alignment, 0);
}

fn upload_immutable_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
    label: &'static str,
) -> Result<wgpu::Buffer, String> {
    let required = u64::try_from(bytes.len().max(wgpu::COPY_BUFFER_ALIGNMENT as usize))
        .map_err(|_| format!("{label} size overflow"))?;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: required,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !bytes.is_empty() {
        queue.write_buffer(&buffer, 0, bytes);
    }
    Ok(buffer)
}

fn make_pipeline(
    device: &wgpu::Device,
    resource: &PipelineResource,
    samples: u32,
) -> Result<GpuPipeline, String> {
    let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Manim frame layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let mobject_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Manim mobject layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let mut resource_layout_entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: true,
            min_binding_size: None,
        },
        count: None,
    }];
    if !resource.texture_names.is_empty() {
        resource_layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        resource_layout_entries.extend(resource.texture_names.iter().enumerate().map(
            |(index, _)| wgpu::BindGroupLayoutEntry {
                binding: u32::try_from(index + 2).expect("Manim texture binding overflow"),
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ));
    }
    let resource_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Manim resource layout"),
        entries: &resource_layout_entries,
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Manim pipeline layout"),
        bind_group_layouts: &[
            Some(&frame_layout),
            Some(&mobject_layout),
            Some(&resource_layout),
        ],
        immediate_size: 0,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Manim WGSL module"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(&resource.source)),
    });
    let color_targets = [Some(wgpu::ColorTargetState {
        format: COLOR_FORMAT,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: if resource.state.color_write {
            wgpu::ColorWrites::ALL
        } else {
            wgpu::ColorWrites::empty()
        },
    })];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Manim render pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(resource.state.depth_write),
            depth_compare: Some(if resource.state.depth_test {
                wgpu::CompareFunction::Less
            } else {
                wgpu::CompareFunction::Always
            }),
            stencil: wgpu::StencilState {
                front: stencil_face(resource.state.stencil_compare, resource.state.stencil_front),
                back: stencil_face(resource.state.stencil_compare, resource.state.stencil_back),
                read_mask: u32::from(u8::MAX),
                write_mask: u32::from(u8::MAX),
            },
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &color_targets,
        }),
        multiview_mask: None,
        cache: None,
    });
    Ok(GpuPipeline {
        pipeline: Arc::new(pipeline),
        frame_layout,
        mobject_layout,
        resource_layout,
    })
}

fn stencil_face(compare: CompareFunction, face: StencilFaceState) -> wgpu::StencilFaceState {
    wgpu::StencilFaceState {
        compare: compare_function(compare),
        fail_op: stencil_operation(face.0),
        depth_fail_op: stencil_operation(face.1),
        pass_op: stencil_operation(face.2),
    }
}

fn compare_function(value: CompareFunction) -> wgpu::CompareFunction {
    match value {
        CompareFunction::Always => wgpu::CompareFunction::Always,
        CompareFunction::Equal => wgpu::CompareFunction::Equal,
        CompareFunction::NotEqual => wgpu::CompareFunction::NotEqual,
    }
}

fn stencil_operation(value: StencilOperation) -> wgpu::StencilOperation {
    match value {
        StencilOperation::Keep => wgpu::StencilOperation::Keep,
        StencilOperation::Zero => wgpu::StencilOperation::Zero,
        StencilOperation::IncrementWrap => wgpu::StencilOperation::IncrementWrap,
        StencilOperation::DecrementWrap => wgpu::StencilOperation::DecrementWrap,
    }
}

fn make_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resource: &TextureResource,
) -> Result<GpuTexture, String> {
    let expected = u64::from(resource.width)
        .checked_mul(u64::from(resource.height))
        .and_then(|pixels| pixels.checked_mul(u64::from(PIXEL_BYTES)))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| format!("Manim texture {} dimensions overflow", resource.id))?;
    if resource.bytes.len() != expected {
        return Err(format!(
            "Manim texture {} byte length is invalid",
            resource.id
        ));
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Manim image"),
        size: wgpu::Extent3d {
            width: resource.width,
            height: resource.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &resource.bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(resource.width * PIXEL_BYTES),
            rows_per_image: Some(resource.height),
        },
        wgpu::Extent3d {
            width: resource.width,
            height: resource.height,
            depth_or_array_layers: 1,
        },
    );
    let address_mode = match resource.address {
        TextureAddress::Clamp => wgpu::AddressMode::ClampToEdge,
        TextureAddress::Repeat => wgpu::AddressMode::Repeat,
        TextureAddress::Mirror => wgpu::AddressMode::MirrorRepeat,
    };
    let filter = match resource.filter {
        TextureFilter::Nearest => wgpu::FilterMode::Nearest,
        TextureFilter::Linear => wgpu::FilterMode::Linear,
    };
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Manim image sampler"),
        address_mode_u: address_mode,
        address_mode_v: address_mode,
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    });
    let view = texture.create_view(&Default::default());
    Ok(GpuTexture {
        _texture: texture,
        view,
        sampler,
    })
}

impl Target {
    fn new(
        device: &wgpu::Device,
        descriptor: ExternalFrameDescriptor,
        output: wgpu::Texture,
    ) -> Result<Self, String> {
        let ExternalFrameDescriptor {
            width,
            height,
            samples,
        } = descriptor;
        if !matches!(samples, 1 | 2 | 4 | 8 | 16) {
            return Err(format!("unsupported Manim sample count {samples}"));
        }
        let output_view = output.create_view(&Default::default());
        let multisample = (samples > 1).then(|| {
            make_render_texture(
                device,
                "Manim multisample color",
                width,
                height,
                samples,
                COLOR_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let multisample_view = multisample
            .as_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let depth = make_render_texture(
            device,
            "Manim depth and stencil",
            width,
            height,
            samples,
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let depth_view = depth.create_view(&Default::default());
        Ok(Self {
            width,
            height,
            samples,
            output,
            output_view,
            _multisample: multisample,
            multisample_view,
            _depth: depth,
            depth_view,
        })
    }

    fn descriptor(&self) -> ExternalFrameDescriptor {
        ExternalFrameDescriptor {
            width: self.width,
            height: self.height,
            samples: self.samples,
        }
    }

    fn render(&self, encoder: &mut wgpu::CommandEncoder, frame: &GpuFrame, background: [f32; 4]) {
        let color_view = self.multisample_view.as_ref().unwrap_or(&self.output_view);
        {
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: self.multisample_view.as_ref().map(|_| &self.output_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(background[0]),
                        g: f64::from(background[1]),
                        b: f64::from(background[2]),
                        a: f64::from(background[3]),
                    }),
                    store: if self.multisample_view.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Manim frame"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                ..Default::default()
            });
            pass.execute_bundles([&frame.bundle]);
        }
    }

    fn read_pixels(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Vec<u8>, String> {
        let row_bytes = self
            .width
            .checked_mul(PIXEL_BYTES)
            .ok_or_else(|| "Manim readback row size overflow".to_string())?;
        let padded_row_bytes = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback_size = u64::from(padded_row_bytes)
            .checked_mul(u64::from(self.height))
            .ok_or_else(|| "Manim readback size overflow".to_string())?;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Manim test readback"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Manim test readback encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.output,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row_bytes),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        let submission = queue.submit([encoder.finish()]);
        let (send, receive) = mpsc::sync_channel(1);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| format!("wait for Manim test readback: {error}"))?;
        receive
            .recv()
            .map_err(|error| format!("receive Manim test readback: {error}"))?
            .map_err(|error| format!("map Manim test readback: {error}"))?;
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .map_err(|error| format!("read mapped Manim test frame: {error}"))?;
        let row_bytes = usize::try_from(row_bytes)
            .map_err(|_| "Manim row size does not fit memory".to_string())?;
        let padded = usize::try_from(padded_row_bytes)
            .map_err(|_| "Manim padded row size does not fit memory".to_string())?;
        let pixel_bytes = row_bytes
            .checked_mul(self.height as usize)
            .ok_or_else(|| "Manim frame size overflow".to_string())?;
        let mut pixels = Vec::with_capacity(pixel_bytes);
        for row in mapped.chunks_exact(padded).take(self.height as usize) {
            pixels.extend_from_slice(&row[..row_bytes]);
        }
        drop(mapped);
        readback.unmap();
        Ok(pixels)
    }
}

fn make_render_texture(
    device: &wgpu::Device,
    label: &'static str,
    width: u32,
    height: u32,
    samples: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
