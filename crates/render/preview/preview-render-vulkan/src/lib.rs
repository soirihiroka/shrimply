#![cfg(target_os = "linux")]

use shrimply_render_vulkan as gpu;
#[path = "../../preview-render-core/src/gpu_alpha_mask.rs"]
mod alpha_mask;
#[path = "../../preview-render-core/src/gpu_effects.rs"]
mod effects;
#[path = "../../preview-render-core/src/gpu_layered_image.rs"]
mod layered_image;

use gpu::{Buffer, Renderer, Submission, VideoFrame};
use shrimply_math_core::Time;
use shrimply_preview_render_core::{DecodedImage, FramePlan, Layer, Scene, Source};
use shrimply_project_document::project::Project;
use shrimply_render_core::{Nv12LayerParams, effects::SpatialState};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

type ExternalMasks = Vec<(
    shrimply_visual_core::raster_modifiers::ExternalDependency,
    Buffer,
)>;

pub struct Rendered {
    pub buffer: Buffer,
    pub width: u32,
    pub height: u32,
    pub audio_analysis: shrimply_preview_render_core::FrameAudioAnalysis,
    pub loading: bool,
    pub accuracy: shrimply_preview_render_core::CompositeAccuracy,
    _submissions: Vec<Submission>,
}

impl Rendered {
    pub fn wait(&self) -> Result<(), String> {
        if let Some(last) = self._submissions.last() {
            last.wait()?;
        }
        Ok(())
    }
}

pub struct Presented {
    pub display: gpu::Texture,
    pub rendered: Rendered,
    presentation: Submission,
}

impl Presented {
    pub fn into_buffers(self) -> (gpu::Texture, Buffer) {
        assert!(
            self.completed().expect("Intel GPU submission failed"),
            "Intel frame published before GPU completion"
        );
        (self.display, self.rendered.buffer)
    }
    pub fn completed(&self) -> Result<bool, String> {
        self.presentation.completed()
    }
}

pub struct Compositor {
    scene: Scene,
    pub renderer: Renderer,
    images: HashMap<u32, Buffer>,
    layered_sources: HashMap<(String, usize), Buffer>,
    used_layered_sources: HashSet<(String, usize)>,
    videos: HashMap<usize, (Arc<VideoFrame>, Buffer)>,
    used_images: HashSet<u32>,
    used_videos: HashSet<usize>,
    sam2_proxy: Option<Buffer>,
}

impl Compositor {
    pub fn new() -> Result<Self, String> {
        let mut renderer = Renderer::new()?;
        renderer.warmup()?;
        Ok(Self {
            scene: Scene::default(),
            renderer,
            images: HashMap::new(),
            layered_sources: HashMap::new(),
            used_layered_sources: HashSet::new(),
            videos: HashMap::new(),
            used_images: HashSet::new(),
            used_videos: HashSet::new(),
            sam2_proxy: None,
        })
    }
    pub fn set_decoder_limit(&mut self, maximum: usize) {
        self.scene.set_decoder_limit(maximum);
        self.renderer.set_video_context_limit(
            maximum
                .checked_add(1)
                .expect("video context limit overflow"),
        );
    }
    pub fn set_exclusion(&mut self, item: Option<uuid::Uuid>) {
        self.scene.set_exclusion(item);
    }
    pub fn set_capture_target(
        &mut self,
        target: Option<shrimply_preview_render_core::CaptureTarget>,
    ) {
        self.scene.set_capture_target(target);
    }
    pub fn invalidate(&mut self) {
        self.scene.invalidate();
        self.renderer.clear_video_contexts();
        self.images.clear();
        self.layered_sources.clear();
        self.videos.clear();
    }
    pub fn take_manim_updates(&mut self) -> Vec<shrimply_manim_state::Update> {
        self.scene.take_manim_updates()
    }
    pub fn needs_update(&self) -> bool {
        self.scene.needs_update()
    }
    pub fn prepare(
        &mut self,
        project: &Project,
        time: Time,
        accuracy: shrimply_preview_render_core::CompositeAccuracy,
    ) -> Result<Option<FramePlan>, String> {
        self.scene
            .set_interaction(accuracy.continuous_playback(), !accuracy.time_accurate());
        self.scene.prepare(project, time)
    }
    pub fn render(&mut self, plan: FramePlan) -> Result<Presented, String> {
        let rendered = self.render_gpu(plan, 0)?;
        let (display, presentation) = self.renderer.presentation_texture(
            &rendered.buffer,
            rendered.width,
            rendered.height,
        )?;
        Ok(Presented {
            display,
            rendered,
            presentation,
        })
    }

    pub fn render_accurate(
        &mut self,
        project: &Project,
        position: Time,
        background: u32,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<Rendered, String> {
        let profiling = std::env::var_os("SHRIMPLY_INTEL_PROFILE").is_some();
        let started = std::time::Instant::now();
        self.scene.request_frame();
        loop {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err("Export cancelled".into());
            }
            if let Some(plan) = self.prepare(
                project,
                position,
                shrimply_preview_render_core::CompositeAccuracy::FULLY_ACCURATE,
            )? && !plan.loading
                && plan.accuracy.content_accurate()
            {
                if profiling {
                    eprintln!("Intel scene preparation wall time: {:?}", started.elapsed());
                }
                return self.render_gpu(plan, background);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    pub fn render_gpu(&mut self, plan: FramePlan, background: u32) -> Result<Rendered, String> {
        let profiling = std::env::var_os("SHRIMPLY_INTEL_PROFILE").is_some();
        let started = std::time::Instant::now();
        let mut submissions = Vec::new();
        self.used_images.clear();
        self.used_layered_sources.clear();
        self.used_videos.clear();
        let mut external_masks = Vec::new();
        let size = (plan.width, plan.height);
        for external in &plan.external_layers {
            let layers = self.layers(&external.layers, size, &external_masks, &mut submissions)?;
            let (buffer, submission) = self
                .renderer
                .composite_buffers(&layers, size.0, size.1, 0)?
                .into_parts();
            submissions.push(submission);
            external_masks.push((external.dependency.clone(), buffer));
        }
        let layers = self.layers(&plan.layers, size, &external_masks, &mut submissions)?;
        if profiling {
            for submission in &submissions {
                submission.wait()?;
            }
        }
        let prepared = started.elapsed();
        let (buffer, composite) = self
            .renderer
            .composite_buffers(&layers, size.0, size.1, background)?
            .into_parts();
        if profiling {
            composite.wait()?;
            eprintln!(
                "Intel composition wall times: layers={prepared:?}, composite={:?}",
                started.elapsed() - prepared,
            );
        }
        submissions.push(composite);
        self.images.retain(|key, _| self.used_images.contains(key));
        self.layered_sources
            .retain(|key, _| self.used_layered_sources.contains(key));
        self.videos.retain(|key, _| self.used_videos.contains(key));
        Ok(Rendered {
            buffer,
            width: size.0,
            height: size.1,
            audio_analysis: plan.audio_analysis,
            loading: plan.loading,
            accuracy: plan.accuracy,
            _submissions: submissions,
        })
    }

    fn image(&mut self, image: &skia_safe::Image) -> Result<Buffer, String> {
        let key = image.unique_id();
        self.used_images.insert(key);
        if let Some(buffer) = self.images.get(&key) {
            return Ok(buffer.clone());
        }
        let info = skia_safe::ImageInfo::new(
            image.dimensions(),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let stride = info.min_row_bytes();
        let mut bytes = vec![0; info.compute_byte_size(stride)];
        if !image.read_pixels(
            &info,
            &mut bytes,
            stride,
            (0, 0),
            skia_safe::image::CachingHint::Allow,
        ) {
            return Err("Read static image for Intel compositor".into());
        }
        let buffer = self.renderer.upload(&bytes)?;
        self.images.insert(key, buffer.clone());
        Ok(buffer)
    }
    fn video(
        &mut self,
        video: &Arc<VideoFrame>,
        submissions: &mut Vec<Submission>,
    ) -> Result<Buffer, String> {
        let key = Arc::as_ptr(video) as usize;
        self.used_videos.insert(key);
        if let Some((_, buffer)) = self.videos.get(&key) {
            return Ok(buffer.clone());
        }
        let (buffer, submission) = self.renderer.video(video)?;
        submissions.push(submission);
        self.videos.insert(key, (video.clone(), buffer.clone()));
        Ok(buffer)
    }
    fn decoded(
        &mut self,
        image: &DecodedImage,
        submissions: &mut Vec<Submission>,
    ) -> Result<Buffer, String> {
        match image {
            DecodedImage::Raster(image) => self.image(image),
            DecodedImage::Video(video) => self.video(video, submissions),
        }
    }
    fn layers(
        &mut self,
        layers: &[Layer],
        size: (u32, u32),
        external_masks: &ExternalMasks,
        submissions: &mut Vec<Submission>,
    ) -> Result<Vec<(Nv12LayerParams, Buffer)>, String> {
        let mut result = Vec::with_capacity(layers.len());
        for layer in layers {
            let mut buffer = match &layer.source {
                Source::Image(image) => self.image(image)?,
                Source::Video(video) => self.video(video, submissions)?,
                Source::Generated(frame) => {
                    let (buffer, submission) =
                        self.renderer.draw_vector(layer.render_size, |canvas| {
                            frame.draw(canvas, &mut Default::default())
                        })?;
                    submissions.push(submission);
                    buffer
                }
                Source::Background(uniforms) => {
                    let (buffer, submission) = self.renderer.background(uniforms)?;
                    submissions.push(submission);
                    buffer
                }
                Source::Group(children) => {
                    let children = self.layers(children, size, external_masks, submissions)?;
                    let (buffer, submission) = self
                        .renderer
                        .composite_buffers(&children, size.0, size.1, 0)?
                        .into_parts();
                    submissions.push(submission);
                    buffer
                }
                Source::LayeredImage(plan) => layered_image::composite(
                    &mut self.renderer,
                    plan,
                    &mut self.layered_sources,
                    &mut self.used_layered_sources,
                    submissions,
                )?,
                Source::Gaussian(_) => {
                    return Err("Gaussian rendering is unavailable on this Intel backend".into());
                }
                Source::Obj(_) => {
                    return Err("OBJ rendering requires a ray-tracing-capable backend".into());
                }
                Source::Manim(_) => {
                    return Err("Manim rendering is unavailable on this Intel backend".into());
                }
                Source::RasterMorph(_) => {
                    return Err(
                        "Raster morph estimation is unavailable on this Intel backend".into(),
                    );
                }
            };
            let mut state = SpatialState {
                parameters: layer.parameters,
                transform: layer.transform,
                texture_edges: [0.0; 4],
                modifier_crop: [0.0; 4],
                modifier_crop_pixels: [0.0; 4],
            };
            if let Some(stabilization) = &layer.stabilization {
                (state, buffer) = effects::apply_stabilization(
                    &mut self.renderer,
                    buffer,
                    state,
                    stabilization,
                    layer.render_size,
                    submissions,
                )?;
            }
            if let Some(mask) = &layer.video_mask {
                let mask_buffer = self.decoded(&mask.image, submissions)?;
                let (mut parameters, _) = shrimply_render_core::effects::materialization(
                    layer.parameters,
                    mask.size.0,
                    mask.size.1,
                );
                parameters.source_width = mask.image.width() as u32;
                parameters.source_height = mask.image.height() as u32;
                parameters.rgba_pitch = parameters.source_width as usize * 4;
                parameters.inverse = shrimply_render_core::math::Mat3::IDENTITY;
                parameters.sample_method = mask.sampling;
                let (mask_buffer, submission) = self
                    .renderer
                    .composite_buffers(&[(parameters, mask_buffer)], mask.size.0, mask.size.1, 0)?
                    .into_parts();
                submissions.push(submission);
                buffer = alpha_mask::video(
                    &mut self.renderer,
                    buffer,
                    (
                        state.parameters.source_width,
                        state.parameters.source_height,
                    ),
                    mask_buffer,
                    mask.size,
                    submissions,
                )?;
            }
            (state, buffer) = effects::apply_modifiers(
                &mut self.renderer,
                buffer,
                state,
                effects::ModifierRequest {
                    operations: &layer.effects,
                    size: layer.render_size,
                    external_masks: effects::ExternalMasks {
                        frames: external_masks,
                        size,
                        output_transform: layer.output_transform,
                    },
                    sam2_target: None,
                    sam2_proxy: &mut self.sam2_proxy,
                },
                submissions,
            )?;
            if let Some(samples) = &layer.motion_blur {
                (state, buffer) = effects::apply_motion_blur(
                    &mut self.renderer,
                    buffer,
                    state,
                    samples,
                    layer.render_size,
                    submissions,
                )?;
            }
            for transition in &layer.transitions {
                state.transform = transition.transform * state.transform;
                if let Some(effect) = transition.effect {
                    (state, buffer) = effects::apply_transition(
                        &mut self.renderer,
                        buffer,
                        state,
                        effect,
                        layer.render_size,
                        submissions,
                    )?;
                }
            }
            state.transform = layer.output_transform * state.transform;
            if let Some(mask) = &layer.alpha_mask {
                buffer = alpha_mask::apply(
                    &mut self.renderer,
                    buffer,
                    (
                        state.parameters.source_width,
                        state.parameters.source_height,
                    ),
                    mask,
                    submissions,
                )?;
            }
            result.push((state.sampled(), buffer));
        }
        Ok(result)
    }
}
