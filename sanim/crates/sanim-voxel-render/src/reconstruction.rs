//! Bounded GPU lighting reconstruction. Albedo and alpha remain unfiltered.
mod math;
use serde::{Deserialize, Serialize};

pub const MAX_SPATIAL_PASSES: u32 = 3;
pub const MAX_HISTORY_FRAMES: u32 = 32;
pub const MAX_ANTIALIAS_SAMPLES: u32 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconstructionMode {
    Off,
    #[default]
    Spatial,
    Temporal,
}
impl ReconstructionMode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Spatial, Self::Temporal];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Spatial => "spatial",
            Self::Temporal => "temporal",
        }
    }
}
impl std::fmt::Display for ReconstructionMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}
impl std::str::FromStr for ReconstructionMode {
    type Err = String;
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.name() == name)
            .ok_or_else(|| {
                format!(
                    "unknown reconstruction '{name}'; choose {}",
                    Self::ALL.map(|mode| mode.name()).join(", ")
                )
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reconstruction {
    pub mode: ReconstructionMode,
    pub spatial_passes: u32,
    pub history_frames: u32,
    pub antialias_samples: u32,
}
impl Default for Reconstruction {
    fn default() -> Self {
        Self {
            mode: ReconstructionMode::default(),
            spatial_passes: 2,
            history_frames: 16,
            antialias_samples: 2,
        }
    }
}
impl Reconstruction {
    pub fn validate(self) -> Result<(), String> {
        if self.spatial_passes > MAX_SPATIAL_PASSES {
            return Err(format!(
                "voxel reconstruction requires 0..={MAX_SPATIAL_PASSES} spatial passes"
            ));
        }
        if !(1..=MAX_HISTORY_FRAMES).contains(&self.history_frames) {
            return Err(format!(
                "voxel reconstruction requires 1..={MAX_HISTORY_FRAMES} history frames"
            ));
        }
        if !(1..=MAX_ANTIALIAS_SAMPLES).contains(&self.antialias_samples) {
            return Err(format!(
                "voxel antialiasing requires 1..={MAX_ANTIALIAS_SAMPLES} primary samples"
            ));
        }
        Ok(())
    }
}

use crate::{RenderSettings, Uniforms};
use fraction::Fraction;
use num_traits::CheckedSub;
use std::borrow::Cow;

pub(crate) const LIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub(crate) const GUIDE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const UNIFORM_STRIDE: usize = 256;
const UNIFORM_SLOTS: usize = MAX_SPATIAL_PASSES as usize + 2;
const UNIFORM_BYTES: u64 = (UNIFORM_STRIDE * UNIFORM_SLOTS) as u64;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniform {
    position: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    previous_position: [f32; 4],
    previous_right: [f32; 4],
    previous_up: [f32; 4],
    previous_forward: [f32; 4],
    size: [u32; 4],
    control: [f32; 4],
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct TargetKey {
    dimensions: [u32; 2],
    passes: u32,
    temporal: bool,
}
impl TargetKey {
    fn new(settings: &RenderSettings) -> Self {
        Self {
            dimensions: [settings.width, settings.height],
            passes: spatial_passes(settings),
            temporal: settings.reconstruction.mode == ReconstructionMode::Temporal,
        }
    }
    fn texture_bytes(self) -> u64 {
        let per_pixel = 32 + self.passes.min(2) * 8 + u32::from(self.temporal) * 32;
        u64::from(self.dimensions[0]) * u64::from(self.dimensions[1]) * u64::from(per_pixel)
    }
    fn texture_count(self) -> usize {
        3 + self.passes.min(2) as usize + usize::from(self.temporal) * 3
    }
}

struct Image {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}
impl Image {
    fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        label: &'static str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view }
    }
}
struct Pipelines {
    layout: wgpu::BindGroupLayout,
    temporal: wgpu::RenderPipeline,
    spatial: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
}
impl Pipelines {
    fn new(device: &wgpu::Device) -> Self {
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: std::num::NonZeroU64::new(size_of::<PostUniform>() as u64),
            },
            count: None,
        }];
        entries.extend((1..=5).map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Voxel reconstruction inputs"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Voxel reconstruction pipeline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Voxel geometry-guided lighting reconstruction"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(concat!(
                include_str!("color.wgsl"),
                "\n",
                include_str!("reconstruction_math.wgsl"),
                "\n",
                include_str!("reconstruction.wgsl")
            ))),
        });
        let pipeline = |entry, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            temporal: pipeline("temporal", LIGHT_FORMAT),
            spatial: pipeline("spatial", LIGHT_FORMAT),
            composite: pipeline("composite", crate::COLOR_FORMAT),
            layout,
        }
    }
}

struct History {
    source: u64,
    settings: RenderSettings,
    time: Fraction,
    camera: Uniforms,
    frames: u32,
    sequence: u32,
    limit: u32,
}
struct Target {
    key: TargetKey,
    light: Image,
    guide: Image,
    albedo: Image,
    spatial: Vec<Image>,
    history_light: Vec<Image>,
    history_guide: Option<Image>,
    uniform: wgpu::Buffer,
    previous: Option<History>,
    history_index: usize,
    pending: Option<History>,
    valid_history: bool,
}
impl Target {
    fn new(device: &wgpu::Device, key: TargetKey) -> Self {
        let image = |format, label| Image::new(device, key.dimensions, format, label);
        Self {
            key,
            light: image(LIGHT_FORMAT, "Voxel sampled illumination"),
            guide: image(GUIDE_FORMAT, "Voxel surface depth, normal and material"),
            albedo: image(LIGHT_FORMAT, "Voxel unfiltered albedo and coverage"),
            spatial: (0..key.passes.min(2))
                .map(|_| image(LIGHT_FORMAT, "Voxel spatial illumination"))
                .collect(),
            history_light: (0..if key.temporal { 2 } else { 0 })
                .map(|_| image(LIGHT_FORMAT, "Voxel bounded lighting history"))
                .collect(),
            history_guide: key
                .temporal
                .then(|| image(GUIDE_FORMAT, "Voxel previous surface guide")),
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Voxel reconstruction pass uniforms"),
                size: UNIFORM_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            previous: None,
            pending: None,
            history_index: 0,
            valid_history: false,
        }
    }
}

fn spatial_passes(settings: &RenderSettings) -> u32 {
    let (shadow, ao) = settings.effective_modes();
    if settings.mode == crate::RenderMode::Flat
        && shadow != crate::ShadowMode::Soft
        && ao != crate::AoMode::RayTraced
    {
        0
    } else {
        settings.reconstruction.spatial_passes
    }
}

#[derive(Default)]
pub(crate) struct Reconstructor {
    pipelines: Option<Pipelines>,
    target: Option<Target>,
}
impl Reconstructor {
    pub fn enabled(settings: &RenderSettings) -> bool {
        settings.reconstruction.mode == ReconstructionMode::Temporal
            || (settings.reconstruction.mode == ReconstructionMode::Spatial
                && spatial_passes(settings) != 0)
    }
    pub fn required_bytes(settings: &RenderSettings) -> u64 {
        if !Self::enabled(settings) {
            0
        } else {
            TargetKey::new(settings).texture_bytes() + UNIFORM_BYTES
        }
    }
    pub fn matches(&self, settings: &RenderSettings) -> bool {
        match (&self.target, Self::enabled(settings)) {
            (None, false) => true,
            (Some(target), true) => target.key == TargetKey::new(settings),
            _ => false,
        }
    }
    pub fn release(&mut self) {
        self.target = None;
    }
    pub fn reset_history(&mut self) {
        if let Some(target) = &mut self.target {
            target.previous = None;
        }
    }
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        source: u64,
        camera: &mut Uniforms,
        settings: &RenderSettings,
        stats: &mut crate::RenderStats,
    ) -> Result<(), String> {
        if !Self::enabled(settings) {
            return Ok(());
        }
        if self.pipelines.is_none() {
            self.pipelines = Some(Pipelines::new(device));
        }
        if self.target.is_none() {
            self.target = Some(Target::new(device, TargetKey::new(settings)));
            stats.texture_allocations += TargetKey::new(settings).texture_count();
            stats.buffer_allocations += 1;
        }
        let target = self
            .target
            .as_mut()
            .expect("reconstruction target initialized");
        let mut key = settings.clone();
        key.time = Fraction::from(0);
        let change = if target.key.temporal {
            if let Some(previous) = target.previous.as_ref().filter(|previous| {
                previous.source == source
                    && settings.time >= previous.time
                    && settings
                        .time
                        .checked_sub(&previous.time)
                        .is_some_and(|gap| gap <= Fraction::new(1u8, 4u8))
            }) {
                math::reactivity(previous, &key, camera)?
            } else {
                None
            }
        } else {
            None
        };
        target.valid_history = change.is_some();
        let reactive = change.is_some_and(|change| change > 1e-4);
        let limit = if reactive {
            settings.reconstruction.history_frames.min(4)
        } else {
            settings.reconstruction.history_frames
        };
        let frames = if target.valid_history {
            (target.previous.as_ref().unwrap().frames + 1).min(limit)
        } else {
            1
        };
        let sequence = if target.valid_history {
            target.previous.as_ref().unwrap().sequence.wrapping_add(1)
        } else {
            0
        };
        camera.reconstruction[2] = sequence;
        stats.reconstruction_bytes = Self::required_bytes(settings);
        stats.history_frames = if target.key.temporal { frames } else { 0 };
        stats.history_reset = target.key.temporal && !target.valid_history;
        stats.history_reactive = reactive;
        stats.reconstruction_passes = target.key.passes + u32::from(target.key.temporal) + 1;
        target.pending = Some(History {
            source,
            settings: key,
            time: settings.time,
            camera: *camera,
            frames,
            sequence,
            limit,
        });
        Ok(())
    }
    pub fn attachments(&self) -> Option<[&wgpu::TextureView; 3]> {
        self.target
            .as_ref()
            .map(|target| [&target.light.view, &target.guide.view, &target.albedo.view])
    }
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
    ) {
        let Some(target) = self.target.as_mut() else {
            return;
        };
        let pipelines = self
            .pipelines
            .as_ref()
            .expect("reconstruction pipelines prepared");
        let frame = target
            .pending
            .take()
            .expect("reconstruction frame prepared");
        let previous = target
            .previous
            .as_ref()
            .map_or(&frame.camera, |history| &history.camera);
        let base = PostUniform {
            position: frame.camera.position,
            right: frame.camera.right,
            up: frame.camera.up,
            forward: frame.camera.forward,
            previous_position: previous.position,
            previous_right: previous.right,
            previous_up: previous.up,
            previous_forward: previous.forward,
            size: [
                target.key.dimensions[0],
                target.key.dimensions[1],
                1,
                frame.limit,
            ],
            control: [
                f32::from(frame.settings.mode == crate::RenderMode::PathTraced),
                frame.settings.exposure_ev.exp2(),
                frame.settings.opacity,
                f32::from(target.valid_history),
            ],
        };
        let mut uniforms = [0u8; UNIFORM_STRIDE * UNIFORM_SLOTS];
        let mut slot = 0;
        let mut input = &target.light.view;
        let old_light = target
            .history_light
            .get(target.history_index)
            .map_or(&target.light.view, |image| &image.view);
        let old_guide = target
            .history_guide
            .as_ref()
            .map_or(&target.guide.view, |image| &image.view);
        let mut pass = |pipeline: &wgpu::RenderPipeline,
                        from: &wgpu::TextureView,
                        to: &wgpu::TextureView,
                        step: u32| {
            let mut uniform = base;
            uniform.size[2] = step;
            let start = slot * UNIFORM_STRIDE;
            uniforms[start..start + size_of::<PostUniform>()]
                .copy_from_slice(bytemuck::bytes_of(&uniform));
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &target.uniform,
                    offset: 0,
                    size: std::num::NonZeroU64::new(size_of::<PostUniform>() as u64),
                }),
            }];
            entries.extend(
                [
                    from,
                    &target.guide.view,
                    &target.albedo.view,
                    old_light,
                    old_guide,
                ]
                .into_iter()
                .enumerate()
                .map(|(index, view)| wgpu::BindGroupEntry {
                    binding: index as u32 + 1,
                    resource: wgpu::BindingResource::TextureView(view),
                }),
            );
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Voxel reconstruction frame inputs"),
                layout: &pipelines.layout,
                entries: &entries,
            });
            let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Voxel lighting reconstruction"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: to,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            render.set_pipeline(pipeline);
            render.set_bind_group(0, &group, &[(slot * UNIFORM_STRIDE) as u32]);
            render.draw(0..3, 0..1);
            slot += 1;
        };
        if target.key.temporal {
            let next = 1 - target.history_index;
            pass(
                &pipelines.temporal,
                input,
                &target.history_light[next].view,
                1,
            );
            input = &target.history_light[next].view;
        }
        for index in 0..target.key.passes {
            let next = &target.spatial[index as usize % target.spatial.len()].view;
            pass(&pipelines.spatial, input, next, 1 << index);
            input = next;
        }
        pass(&pipelines.composite, input, output, 1);
        queue.write_buffer(&target.uniform, 0, &uniforms[..slot * UNIFORM_STRIDE]);
        if let Some(previous_guide) = &target.history_guide {
            encoder.copy_texture_to_texture(
                target.guide.texture.as_image_copy(),
                previous_guide.texture.as_image_copy(),
                wgpu::Extent3d {
                    width: target.key.dimensions[0],
                    height: target.key.dimensions[1],
                    depth_or_array_layers: 1,
                },
            );
            target.history_index = 1 - target.history_index;
        }
        target.previous = Some(frame);
    }
}
