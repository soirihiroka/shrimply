//! Independent GPU voxel renderer. Logical snapshots never own GPU allocations.
mod export;
mod lights;
mod math;
mod reconstruction;
mod resident;

pub use reconstruction::{Reconstruction, ReconstructionMode};
pub use resident::validate_materials;

use fraction::Fraction;
use num_traits::ToPrimitive;
pub use sanim_voxel::{
    AmbientOcclusion, AoMode, Camera, CameraRig, Lighting, Projection, Ray, ShadowMode, Shadows,
    View,
};
use sanim_voxel::{Environment, InstanceScene, Light, Volume};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::VecDeque, sync::Arc};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const UNIFORM_BYTES: u64 = size_of::<Uniforms>() as u64;
const MAX_PATH_SAMPLES: u32 = 256;
const MAX_PATH_BOUNCES: u32 = 8;
/// Per-frame Monte Carlo workload guard. This bounds requested path segments,
/// not GPU execution time: geometry, shadow rays and hardware still affect latency.
pub const MAX_PATH_SEGMENTS: u64 = 512 * 1024 * 1024;
pub const MAX_TRACE_QUERIES: u64 = MAX_PATH_SEGMENTS * 2;
pub const MAX_EXPOSURE_EV: f32 = 16.0;
const MAX_TRAVERSAL_STEPS: u32 = 65_536;
const MAX_IN_FLIGHT: usize = 2;
// DDA boundary alignment relies on a positive power-of-two model brick.
const _: () = assert!(
    sanim_voxel::CHUNK_EDGE.is_power_of_two() && sanim_voxel::CHUNK_EDGE <= i32::MAX as u32
);
/// Bound specular interfaces separately from diffuse bounces. Contiguous liquid
/// cells share one medium and consume only their external entry/exit boundaries.
pub const MAX_DIELECTRIC_INTERFACES: u32 = 16;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_gpu_bytes: u64,
    pub max_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_gpu_bytes: 256 * 1024 * 1024,
            max_pixels: 16 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    #[default]
    Flat,
    PathTraced,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderSettings {
    pub width: u32,
    pub height: u32,
    pub background: [f32; 4],
    pub sky_top: Option<[f32; 4]>,
    pub lighting: Lighting,
    pub lights: Option<Arc<[Light]>>,
    pub environment: Option<Environment>,
    pub ambient: f32,
    pub opacity: f32,
    pub ambient_occlusion: AmbientOcclusion,
    pub exposure_ev: f32,
    pub mode: RenderMode,
    pub samples: u32,
    pub bounces: u32,
    pub seed: u32,
    pub time: Fraction,
    pub reconstruction: Reconstruction,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            background: [0.1333, 0.1333, 0.149, 1.0],
            sky_top: None,
            lighting: Lighting::default(),
            lights: None,
            environment: None,
            ambient: 0.6,
            opacity: 1.0,
            ambient_occlusion: AmbientOcclusion::default(),
            exposure_ev: 0.0,
            mode: RenderMode::Flat,
            samples: 32,
            bounces: 4,
            seed: 0,
            time: Fraction::from(0),
            reconstruction: Reconstruction::default(),
        }
    }
}

impl RenderSettings {
    /// Auto keeps flat previews local and cheap; path tracing uses soft shadows
    /// and its own physical visibility without an extra artistic AO pass.
    pub fn effective_modes(&self) -> (ShadowMode, AoMode) {
        let shadow = match self.lighting.shadows.mode {
            ShadowMode::Auto if self.mode == RenderMode::Flat => ShadowMode::Off,
            ShadowMode::Auto => ShadowMode::Soft,
            mode => mode,
        };
        let shadow = if self.lighting.shadows.strength == 0.0 {
            ShadowMode::Off
        } else if shadow == ShadowMode::Soft && self.lighting.sun_diameter_degrees == 0.0 {
            ShadowMode::Hard
        } else {
            shadow
        };
        let ao = match self.ambient_occlusion.mode {
            AoMode::Auto if self.mode == RenderMode::Flat => AoMode::Local,
            AoMode::Auto => AoMode::Off,
            mode => mode,
        };
        (
            shadow,
            if self.ambient_occlusion.strength == 0.0 {
                AoMode::Off
            } else {
                ao
            },
        )
    }

    /// Validate portable values without creating or querying a GPU device.
    pub fn validate(&self) -> Result<(), String> {
        self.reconstruction.validate()?;
        if self.time.to_f64().is_none_or(|time| {
            !time.is_finite() || time.abs() > sanim_voxel::OpticalProperties::MAX_WAVE_TIME
        }) {
            return Err(
                "voxel render time is non-finite or exceeds the representable GPU wave-phase range"
                    .into(),
            );
        }
        if self.width == 0 || self.height == 0 {
            return Err("voxel output dimensions must be nonzero".into());
        }
        if self
            .background
            .iter()
            .chain(self.sky_top.as_ref().into_iter().flatten())
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
            || !self.ambient.is_finite()
            || !(0.0..=1.0).contains(&self.ambient)
        {
            return Err("voxel colors, opacity and ambient must be finite within 0..=1".into());
        }
        self.lighting.validate()?;
        if let Some(lights) = &self.lights {
            if lights.len() > sanim_voxel::MAX_LIGHTS {
                return Err(format!(
                    "voxel scene supports at most {} lights",
                    sanim_voxel::MAX_LIGHTS
                ));
            }
            for light in lights.iter() {
                light.validate()?;
            }
        }
        if let Some(environment) = &self.environment {
            environment.parameters()?;
        }
        self.ambient_occlusion.validate()?;
        if !self.exposure_ev.is_finite() || self.exposure_ev.abs() > MAX_EXPOSURE_EV {
            return Err(format!(
                "voxel exposure must be finite and within -{MAX_EXPOSURE_EV}..={MAX_EXPOSURE_EV} EV"
            ));
        }
        if self.samples == 0
            || self.samples > MAX_PATH_SAMPLES
            || self.bounces == 0
            || self.bounces > MAX_PATH_BOUNCES
        {
            return Err(format!(
                "voxel path tracing requires 1..={MAX_PATH_SAMPLES} samples and 1..={MAX_PATH_BOUNCES} bounces"
            ));
        }
        if self.mode == RenderMode::PathTraced {
            let segments = u64::from(self.width)
                .checked_mul(u64::from(self.height))
                .and_then(|pixels| pixels.checked_mul(u64::from(self.samples)))
                .and_then(|samples| samples.checked_mul(u64::from(self.bounces)));
            if segments.is_none_or(|segments| segments > MAX_PATH_SEGMENTS) {
                return Err(format!(
                    "voxel path tracing exceeds the {MAX_PATH_SEGMENTS} path-segment budget per frame (width × height × samples × bounces); reduce output dimensions, samples, or bounces"
                ));
            }
        }
        let (shadow, ao) = self.effective_modes();
        let shadow_queries = match shadow {
            ShadowMode::Off => 0,
            ShadowMode::Hard => 1,
            ShadowMode::Soft => self.lighting.shadows.samples,
            ShadowMode::Auto => unreachable!("automatic shadow mode resolved"),
        };
        let ao_queries = if ao == AoMode::RayTraced {
            self.ambient_occlusion.samples
        } else {
            0
        };
        let (samples, bounces) = if self.mode == RenderMode::PathTraced {
            (self.samples, self.bounces)
        } else {
            (self.reconstruction.antialias_samples, 1)
        };
        let light_connections = if self.mode == RenderMode::Flat {
            self.lights.as_ref().map_or(1, |lights| lights.len() as u64)
        } else {
            1
        };
        let queries = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|pixels| pixels.checked_mul(u64::from(samples)))
            .and_then(|samples| {
                samples.checked_mul(
                    u64::from(bounces) * (1 + u64::from(shadow_queries) * light_connections)
                        + u64::from(ao_queries),
                )
            });
        if queries.is_none_or(|queries| queries > MAX_TRACE_QUERIES) {
            return Err(format!(
                "voxel rendering exceeds the {MAX_TRACE_QUERIES} visibility-ray budget per frame; reduce output size, path samples/bounces, shadow samples, or AO samples"
            ));
        }
        Ok(())
    }
}

/// Per-render voxel transfer counts exclude the small camera uniform upload.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct RenderStats {
    pub uploaded_chunks: usize,
    pub updated_pages: usize,
    pub upload_bytes: u64,
    pub light_upload_bytes: u64,
    pub buffer_allocations: usize,
    pub texture_allocations: usize,
    pub resident_bytes: u64,
    pub reconstruction_bytes: u64,
    pub reconstruction_passes: u32,
    /// Accepted frame lineage; individual pixels can reject history independently.
    pub history_frames: u32,
    /// Whole-frame rejection, such as source edits, seeks or large light changes.
    pub history_reset: bool,
    /// Small light changes limit history to four frames to reduce temporal trails.
    pub history_reactive: bool,
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub stats: RenderStats,
}

/// All members occupy one WGSL vec4 slot; byte size comes from this ABI structure.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    dimensions: [u32; 4],
    pages: [u32; 4],
    position: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    viewport: [f32; 4],
    background: [f32; 4],
    sky_top: [f32; 4],
    composite: [f32; 4],
    mode: [u32; 4],
    environment_upper: [f32; 4],
    environment_horizon: [f32; 4],
    environment_lower: [f32; 4],
    quality: [u32; 4],
    artistic: [f32; 4],
    clock: [f32; 4],
    reconstruction: [u32; 4],
}

#[derive(Clone, Copy)]
enum Input<'a> {
    Volume(&'a Volume),
    Instances(&'a InstanceScene),
}
impl Input<'_> {
    fn dimensions(self) -> [u32; 3] {
        match self {
            Self::Volume(volume) => volume.dimensions(),
            Self::Instances(scene) => scene.dimensions(),
        }
    }
    fn revision(self) -> u64 {
        match self {
            Self::Volume(volume) => volume.revision(),
            Self::Instances(scene) => scene.revision(),
        }
    }
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    limits: Limits,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    variant_pipelines: [Option<wgpu::RenderPipeline>; 31],
    shader: wgpu::ShaderModule,
    active_pipeline: usize,
    pipeline_layout: wgpu::PipelineLayout,
    uniform: wgpu::Buffer,
    lights: lights::Lights,
    resident: Option<resident::Resident>,
    export: Option<export::Target>,
    reconstruction: reconstruction::Reconstructor,
    in_flight: VecDeque<wgpu::SubmissionIndex>,
}

impl Renderer {
    pub fn headless(limits: Limits) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|error| format!("request voxel GPU adapter: {error}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Sanim voxel device"),
            ..Default::default()
        }))
        .map_err(|error| format!("create voxel GPU device: {error}"))?;
        Self::from_device(device, queue, limits)
    }

    pub fn from_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        limits: Limits,
    ) -> Result<Self, String> {
        if limits.max_gpu_bytes < UNIFORM_BYTES + lights::UNIFORM_BYTES || limits.max_pixels == 0 {
            return Err(
                "voxel GPU byte budget must fit camera uniforms and pixel budget must be positive"
                    .into(),
            );
        }
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut entries = Vec::new();
        for binding in 0..4 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 0 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage { read_only: true }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 5,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 6,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 7,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Voxel layout"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Voxel pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sanim voxel traversal"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!(
                "const CHUNK_EDGE: u32 = {}u;\nconst MATERIAL_WORDS: u32 = {}u;\nconst MAX_INTERFACES: u32 = {}u;\nconst MAX_BUCKET_INSTANCES: u32 = {}u;\nconst INSTANCE_WORDS: u32 = {}u;\nconst MAX_LIGHTS: u32 = {}u;\nconst LIGHT_SEARCH_STEPS: u32 = {}u;\nconst MIN_LIGHT_DISTANCE: f32 = {};\n{}\n{}",
                sanim_voxel::CHUNK_EDGE,
                resident::MATERIAL_WORDS,
                MAX_DIELECTRIC_INTERFACES,
                sanim_voxel::MAX_BUCKET_CANDIDATES,
                resident::INSTANCE_WORDS,
                sanim_voxel::MAX_LIGHTS,
                sanim_voxel::MAX_LIGHTS.ilog2() + 1,
                sanim_voxel::MIN_LIGHT_DISTANCE,
                include_str!("voxel.wgsl"),
                concat!(
                    include_str!("color.wgsl"), "\n", include_str!("reconstruction_math.wgsl"), "\n", include_str!("texture_sampling.wgsl"), "\n",
                    include_str!("math.wgsl"), "\n", include_str!("dielectric.wgsl"),
                    "\n",
                    include_str!("geometry.wgsl"), "\n", include_str!("lights.wgsl"), "\n", include_str!("instances.wgsl")
                ),
            ))),
        });
        let pipeline = create_pipeline(&device, &pipeline_layout, &shader, 0);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Voxel camera"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if let Some(error) = pollster::block_on(errors.pop()) {
            return Err(format!("initialize voxel rendering pipeline: {error}"));
        }
        let lights = lights::Lights::new(&device);
        Ok(Self {
            device,
            queue,
            limits,
            layout,
            pipeline,
            variant_pipelines: std::array::from_fn(|_| None),
            shader,
            active_pipeline: 0,
            pipeline_layout,
            uniform,
            lights,
            resident: None,
            export: None,
            reconstruction: reconstruction::Reconstructor::default(),
            in_flight: VecDeque::new(),
        })
    }

    /// Render to a caller-owned RGBA8 texture view. No CPU readback occurs.
    pub fn render(
        &mut self,
        volume: &Volume,
        camera: &Camera,
        settings: &RenderSettings,
        target: &wgpu::TextureView,
    ) -> Result<RenderStats, String> {
        self.render_input(Input::Volume(volume), camera, settings, target)
    }

    pub fn render_instances(
        &mut self,
        scene: &InstanceScene,
        camera: &Camera,
        settings: &RenderSettings,
        target: &wgpu::TextureView,
    ) -> Result<RenderStats, String> {
        self.render_input(Input::Instances(scene), camera, settings, target)
    }

    fn render_input(
        &mut self,
        input: Input<'_>,
        camera: &Camera,
        settings: &RenderSettings,
        target: &wgpu::TextureView,
    ) -> Result<RenderStats, String> {
        let texture = target.texture();
        if texture.format() != COLOR_FORMAT
            || texture.width() != settings.width
            || texture.height() != settings.height
            || texture.sample_count() != 1
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err("voxel render target must be a single-sampled RGBA8 render attachment matching the output dimensions".into());
        }
        let reserved = self.export.as_ref().map_or(0, export::Target::bytes);
        let stats = self.prepare(input, camera, settings, reserved)?;
        self.draw(target);
        Ok(stats)
    }

    /// Export only: reuse one target and staging buffer, then read tightly packed RGBA.
    pub fn render_rgba(
        &mut self,
        volume: &Volume,
        camera: &Camera,
        settings: &RenderSettings,
    ) -> Result<Frame, String> {
        self.render_rgba_input(Input::Volume(volume), camera, settings)
    }

    pub fn render_instances_rgba(
        &mut self,
        scene: &InstanceScene,
        camera: &Camera,
        settings: &RenderSettings,
    ) -> Result<Frame, String> {
        self.render_rgba_input(Input::Instances(scene), camera, settings)
    }

    fn render_rgba_input(
        &mut self,
        input: Input<'_>,
        camera: &Camera,
        settings: &RenderSettings,
    ) -> Result<Frame, String> {
        self.validate_settings(settings)?;
        let required = export::Target::required_bytes(settings);
        if self
            .export
            .as_ref()
            .is_some_and(|target| target.dimensions != [settings.width, settings.height])
        {
            self.wait_idle()?;
            self.export = None;
        }
        let mut stats = self.prepare(input, camera, settings, required)?;
        if self.export.is_none() {
            stats.buffer_allocations += 1;
            stats.texture_allocations += 1;
        }
        let target = self
            .export
            .get_or_insert_with(|| export::Target::new(&self.device, settings));
        let view = target.texture.create_view(&Default::default());
        self.draw(&view);
        let pixels = self
            .export
            .as_ref()
            .expect("export target initialized")
            .read(&self.device, &self.queue)?;
        Ok(Frame {
            width: settings.width,
            height: settings.height,
            pixels,
            stats,
        })
    }

    /// Explicitly reset progressive lighting when a host changes its evaluation epoch.
    pub fn reset_history(&mut self) {
        self.reconstruction.reset_history();
    }

    pub fn release_volume(&mut self) -> Result<(), String> {
        self.wait_idle()?;
        self.resident = None;
        self.export = None;
        self.lights.reset();
        self.reconstruction.release();
        Ok(())
    }

    /// Drain this renderer's submissions on its rendering thread, not the UI thread.
    pub fn wait_idle(&mut self) -> Result<(), String> {
        if let Some(submission) = self.in_flight.pop_back() {
            self.in_flight.clear();
            self.wait(submission)?;
        }
        Ok(())
    }

    fn wait(&self, submission: wgpu::SubmissionIndex) -> Result<(), String> {
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| format!("wait for bounded voxel submissions: {error}"))?;
        Ok(())
    }

    fn prepare(
        &mut self,
        input: Input<'_>,
        camera: &Camera,
        settings: &RenderSettings,
        reserved: u64,
    ) -> Result<RenderStats, String> {
        let mut uniform = self.uniforms(input.dimensions(), camera, settings)?;
        if !self.reconstruction.matches(settings) {
            self.wait_idle()?;
            self.reconstruction.release();
        }
        let reserved = reserved
            .checked_add(reconstruction::Reconstructor::required_bytes(settings))
            .ok_or("voxel reconstruction targets exceed the GPU byte budget")?;
        if self
            .resident
            .as_ref()
            .is_some_and(|state| state.revision() != input.revision())
        {
            // Retire old resource users before modifying buffers or replacing their storage.
            self.wait_idle()?;
        } else if self.in_flight.len() >= MAX_IN_FLIGHT {
            let submission = self
                .in_flight
                .pop_front()
                .expect("bounded queue has a pending submission");
            self.wait(submission)?;
        }
        let max_gpu_bytes =
            self.limits.max_gpu_bytes.checked_sub(reserved).ok_or(
                "voxel reconstruction/export targets exceed the GPU byte budget; reduce output resolution or disable reconstruction",
            )?;
        let context = resident::Context {
            device: &self.device,
            queue: &self.queue,
            layout: &self.layout,
            uniforms: [&self.uniform, &self.lights.buffer],
            limits: Limits {
                max_gpu_bytes,
                ..self.limits
            },
        };
        let mut stats = match input {
            Input::Volume(volume) => resident::synchronize(context, &mut self.resident, volume)?,
            Input::Instances(scene) => {
                resident::synchronize_instances(context, &mut self.resident, scene)?
            }
        };
        if self
            .resident
            .as_ref()
            .is_some_and(|state| state.has_transmission)
            && settings.lights.is_none()
        {
            let legacy = settings.lighting.parameters()?;
            if legacy
                .key_radiance
                .iter()
                .any(|value| *value > 2.0 * sanim_voxel::MAX_LIGHT_INTENSITY)
            {
                return Err("voxel dielectric rendering requires bounded key-light radiance; reduce sun strength or use SunLight with validated intensity".into());
            }
        }
        let features = self
            .resident
            .as_ref()
            .expect("resident state prepared")
            .shader_features()
            | (usize::from(reconstruction::Reconstructor::enabled(settings)) << 3)
            | (usize::from(settings.mode == RenderMode::PathTraced) << 4);
        if features != 0 && self.variant_pipelines[features - 1].is_none() {
            let errors = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let pipeline =
                create_pipeline(&self.device, &self.pipeline_layout, &self.shader, features);
            if let Some(error) = pollster::block_on(errors.pop()) {
                return Err(format!("initialize voxel material pipeline: {error}"));
            }
            self.variant_pipelines[features - 1] = Some(pipeline);
        }
        self.active_pipeline = features;
        stats.light_upload_bytes =
            self.lights
                .prepare(&self.queue, settings, input.dimensions())?;
        stats.resident_bytes += reserved;
        self.reconstruction.prepare(
            &self.device,
            input.revision(),
            &mut uniform,
            settings,
            &mut stats,
        )?;
        self.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        Ok(stats)
    }

    fn draw(&mut self, target: &wgpu::TextureView) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Voxel frame"),
            });
        {
            let reconstruction = self.reconstruction.attachments();
            let views = reconstruction.map_or_else(|| vec![target], |views| views.into());
            let attachments = views
                .iter()
                .map(|view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                })
                .collect::<Vec<_>>();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Voxel rays"),
                color_attachments: &attachments,
                ..Default::default()
            });
            let features = self.active_pipeline;
            pass.set_pipeline(if features == 0 {
                &self.pipeline
            } else {
                self.variant_pipelines[features - 1]
                    .as_ref()
                    .expect("material pipeline prepared")
            });
            pass.set_bind_group(
                0,
                &self
                    .resident
                    .as_ref()
                    .expect("voxel synchronization creates resident state")
                    .bind_group,
                &[],
            );
            pass.draw(0..3, 0..1);
        }
        self.reconstruction
            .encode(&self.device, &self.queue, &mut encoder, target);
        self.in_flight
            .push_back(self.queue.submit([encoder.finish()]));
    }

    fn validate_settings(&self, settings: &RenderSettings) -> Result<(), String> {
        if settings.width == 0
            || settings.height == 0
            || u64::from(settings.width) * u64::from(settings.height) > self.limits.max_pixels
            || settings.width.max(settings.height) > self.device.limits().max_texture_dimension_2d
        {
            return Err(
                "voxel output dimensions are zero or exceed the configured pixel/device budget"
                    .into(),
            );
        }
        settings.validate()
    }

    fn uniforms(
        &self,
        dimensions: [u32; 3],
        camera: &Camera,
        settings: &RenderSettings,
    ) -> Result<Uniforms, String> {
        self.validate_settings(settings)?;
        let basis = camera.basis([settings.width, settings.height])?;
        let (shadow_mode, ao_mode) = settings.effective_modes();
        let mut lighting_settings = settings.lighting;
        lighting_settings.shadows.mode = shadow_mode;
        let mut lighting = lighting_settings.parameters()?;
        if let Some(environment) = &settings.environment {
            let environment = environment.parameters()?;
            lighting.environment_upper = environment.top;
            lighting.environment_horizon = environment.horizon;
            lighting.environment_lower = environment.ground;
        }
        let traversal_steps = dimensions
            .into_iter()
            .try_fold(16u32, |sum, size| sum.checked_add(size.checked_mul(2)?))
            .filter(|steps| *steps <= MAX_TRAVERSAL_STEPS)
            .ok_or("voxel volume dimensions exceed the bounded GPU traversal budget")?;
        let pages = dimensions.map(|dimension| dimension.div_ceil(sanim_voxel::CHUNK_EDGE));
        Ok(Uniforms {
            dimensions: [
                dimensions[0],
                dimensions[1],
                dimensions[2],
                sanim_voxel::CHUNK_EDGE,
            ],
            reconstruction: [
                settings.reconstruction.mode as u32,
                settings.reconstruction.antialias_samples,
                0,
                0,
            ],
            pages: [pages[0], pages[1], pages[2], traversal_steps],
            position: [
                basis.position[0],
                basis.position[1],
                basis.position[2],
                sanim_voxel::math::SURFACE_OFFSET as f32,
            ],
            right: [
                basis.right[0],
                basis.right[1],
                basis.right[2],
                basis.half_height * basis.aspect,
            ],
            up: [basis.up[0], basis.up[1], basis.up[2], basis.half_height],
            forward: [
                basis.forward[0],
                basis.forward[1],
                basis.forward[2],
                f32::from(basis.perspective),
            ],
            viewport: [
                settings.width as f32,
                settings.height as f32,
                settings.ambient,
                (settings.exposure_ev / 2.2).exp2(),
            ],
            background: settings.background,
            sky_top: settings.sky_top.unwrap_or(settings.background),
            composite: [settings.opacity, 0.0, 0.0, 0.0],
            mode: [
                settings.samples,
                settings.bounces,
                settings.seed,
                u32::from(settings.mode == RenderMode::PathTraced),
            ],
            environment_upper: [
                lighting.environment_upper[0],
                lighting.environment_upper[1],
                lighting.environment_upper[2],
                0.0,
            ],
            environment_horizon: [
                lighting.environment_horizon[0],
                lighting.environment_horizon[1],
                lighting.environment_horizon[2],
                0.0,
            ],
            environment_lower: [
                lighting.environment_lower[0],
                lighting.environment_lower[1],
                lighting.environment_lower[2],
                0.0,
            ],
            quality: [
                match shadow_mode {
                    ShadowMode::Off => 0,
                    ShadowMode::Hard => 1,
                    ShadowMode::Soft => 2,
                    ShadowMode::Auto => unreachable!("automatic shadow mode resolved"),
                },
                if shadow_mode == ShadowMode::Soft {
                    settings.lighting.shadows.samples
                } else {
                    1
                },
                match ao_mode {
                    AoMode::Off => 0,
                    AoMode::Local => 1,
                    AoMode::RayTraced => 2,
                    AoMode::Auto => unreachable!("automatic AO mode resolved"),
                },
                settings.ambient_occlusion.samples,
            ],
            clock: [
                settings.time.to_f32().expect("render time validated"),
                f32::from(
                    settings
                        .environment
                        .as_ref()
                        .is_some_and(|environment| environment.visible),
                ),
                f32::from(settings.environment.is_some()),
                0.0,
            ],
            artistic: [
                settings.lighting.shadows.strength,
                settings.ambient_occlusion.strength,
                settings.ambient_occlusion.distance,
                settings.exposure_ev.exp2(),
            ],
        })
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    pipeline_layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    features: usize,
) -> wgpu::RenderPipeline {
    let reconstruct = features & 8 != 0;
    let formats = if reconstruct {
        vec![
            reconstruction::LIGHT_FORMAT,
            reconstruction::GUIDE_FORMAT,
            reconstruction::LIGHT_FORMAT,
        ]
    } else {
        vec![COLOR_FORMAT]
    };
    let targets = formats
        .into_iter()
        .map(|format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect::<Vec<_>>();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Sanim voxel ray renderer"),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if reconstruct {
                "reconstruct_fragment"
            } else {
                "fragment"
            }),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &[
                    ("TRANSMISSIVE", f64::from(features & 1 != 0)),
                    ("FOLIAGE", f64::from(features & 2 != 0)),
                    ("INSTANCED", f64::from(features & 4 != 0)),
                    ("PATH_TRACED", f64::from(features & 16 != 0)),
                ],
                ..Default::default()
            },
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}
