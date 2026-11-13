#![cfg(target_os = "linux")]

use shrimply_render_vulkan as gpu;
#[path = "../../preview-render-core/src/gpu_alpha_mask.rs"]
mod alpha_mask;
#[path = "../../preview-render-core/src/gpu_effects.rs"]
mod effects;
#[path = "../../preview-render-core/src/gpu_layered_image.rs"]
mod layered_image;
mod manim;
mod morph;
mod sam2;
pub use sam2::Sam2ProxyFrameSource;

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
    pub sanim_sources: Arc<shrimply_visual_core::sanim::inspection::FrameSources>,
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
    morphs: HashMap<shrimply_preview_render_core::MorphCacheKey, morph::Cached>,
    used_morphs: HashSet<shrimply_preview_render_core::MorphCacheKey>,
    pub renderer: Renderer,
    images: HashMap<u32, Buffer>,
    layered_sources: HashMap<(String, usize), Buffer>,
    used_layered_sources: HashSet<(String, usize)>,
    videos: HashMap<usize, (Arc<VideoFrame>, Buffer)>,
    used_images: HashSet<u32>,
    used_videos: HashSet<usize>,
    sam2_proxy: Option<Buffer>,
    sam2_target: Option<shrimply_visual_core::sam2::analysis::AnalysisTarget>,
    manim: Option<shrimply_manim_wgpu::vulkan::Renderer>,
    manim_slots: HashMap<uuid::Uuid, manim::Slot>,
    used_manim: HashSet<uuid::Uuid>,
    next_manim_slot: usize,
    manim_updates: Vec<shrimply_manim_state::Update>,
}

impl Compositor {
    pub fn new() -> Result<Self, String> {
        let renderer = Renderer::new()?;
        Ok(Self {
            scene: Scene::default(),
            morphs: HashMap::new(),
            used_morphs: HashSet::new(),
            renderer,
            images: HashMap::new(),
            layered_sources: HashMap::new(),
            used_layered_sources: HashSet::new(),
            videos: HashMap::new(),
            used_images: HashSet::new(),
            used_videos: HashSet::new(),
            sam2_proxy: None,
            sam2_target: None,
            manim: None,
            manim_slots: HashMap::new(),
            used_manim: HashSet::new(),
            next_manim_slot: 0,
            manim_updates: Vec::new(),
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
        self.morphs.clear();
        self.renderer.clear_video_contexts();
        self.images.clear();
        self.layered_sources.clear();
        self.videos.clear();
        self.manim_slots.clear();
        if let Some(manim) = &mut self.manim {
            manim.release_render_surfaces();
            manim.release_gpu_animation_resources();
        }
    }
    pub fn take_manim_updates(&mut self) -> Vec<shrimply_manim_state::Update> {
        let mut updates = self.scene.take_manim_updates();
        updates.append(&mut self.manim_updates);
        updates
    }
    pub fn needs_update(&self) -> bool {
        self.scene.needs_update()
    }
    pub fn set_source_waker(&mut self, wake: Option<shrimply_visual_core::generated::SourceWake>) {
        self.scene.set_source_waker(wake);
    }
    pub fn take_sanim_errors(&mut self) -> Vec<String> {
        self.scene.take_sanim_errors()
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
        let rendered = {
            let _measurement = shrimply_profiling::measure("Intel rendering / Compose layers");
            self.render_gpu(plan, 0)?
        };
        let _measurement = shrimply_profiling::measure("Intel rendering / Prepare presentation");
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

    pub fn capture_rgba(
        &mut self,
        project: &Project,
        position: Time,
        target: shrimply_preview_render_core::CaptureTarget,
    ) -> Result<Option<Vec<u8>>, String> {
        self.scene.set_capture_target(Some(target));
        self.scene.request_frame();
        let plan = self.prepare(
            project,
            position,
            shrimply_preview_render_core::CompositeAccuracy::FULLY_ACCURATE,
        )?;
        shrimply_manim_state::validate_render_updates(self.take_manim_updates())?;
        let Some(plan) = plan else {
            return Ok(None);
        };
        if plan.loading || !plan.accuracy.content_accurate() {
            return Ok(None);
        }
        let rendered = self.render_gpu(plan, 0)?;
        rendered.wait()?;
        self.renderer.readback(&rendered.buffer).map(Some)
    }

    pub fn render_accurate(
        &mut self,
        project: &Project,
        position: Time,
        background: u32,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<Rendered, String> {
        let plan = self.prepare_accurate(project, position, cancelled)?;
        self.render_gpu(plan, background)
    }

    pub fn prepare_accurate(
        &mut self,
        project: &Project,
        position: Time,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<FramePlan, String> {
        self.scene.set_strict_sources(true);
        self.scene.request_frame();
        let result = (|| loop {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err("Export cancelled".into());
            }
            let plan = self.prepare(
                project,
                position,
                shrimply_preview_render_core::CompositeAccuracy::FULLY_ACCURATE,
            )?;
            shrimply_manim_state::validate_render_updates(self.take_manim_updates())?;
            if let Some(plan) = plan
                && !plan.loading
                && plan.accuracy.content_accurate()
            {
                return Ok(plan);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        })();
        self.scene.set_strict_sources(false);
        result
    }

    pub fn render_gpu(&mut self, plan: FramePlan, background: u32) -> Result<Rendered, String> {
        let direct_video = shrimply_preview_render_core::math::unmodified_video(&plan).is_some();
        let mut submissions = Vec::new();
        self.used_images.clear();
        self.used_morphs.clear();
        self.used_layered_sources.clear();
        self.used_videos.clear();
        self.used_manim.clear();
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
        let buffer = if direct_video {
            shrimply_profiling::increment("Intel rendering / Video composition bypasses");
            layers[0].1.clone()
        } else {
            let _measurement = shrimply_profiling::measure("Intel rendering / Final composition");
            let (buffer, composite) = self
                .renderer
                .composite_buffers(&layers, size.0, size.1, background)?
                .into_parts();
            submissions.push(composite);
            buffer
        };
        self.morphs.retain(|key, _| self.used_morphs.contains(key));
        self.images.retain(|key, _| self.used_images.contains(key));
        self.layered_sources
            .retain(|key, _| self.used_layered_sources.contains(key));
        self.videos.retain(|key, _| self.used_videos.contains(key));
        self.manim_slots.retain(|key, slot| {
            if self.used_manim.contains(key) {
                return true;
            }
            if let Some(manim) = &mut self.manim {
                manim.remove_target(slot.id);
            }
            false
        });
        Ok(Rendered {
            sanim_sources: plan.sanim_sources,
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
        let _measurement = shrimply_profiling::measure("Intel rendering / Import video");
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
        let _measurement = shrimply_profiling::measure("Intel rendering / Prepare layers");
        let mut result = Vec::with_capacity(layers.len());
        for layer in layers {
            if let Source::RasterMorph(morph) = &layer.source {
                result.extend(self.raster_morph(morph, size, external_masks, submissions)?);
                continue;
            }
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
                Source::Gaussian(plan) => self.renderer.gaussian(
                    &plan.session,
                    (plan.width, plan.height),
                    &plan.params,
                )?,
                Source::Obj(plan) => {
                    let background = if result.is_empty() {
                        None
                    } else {
                        let (buffer, submission) = self
                            .renderer
                            .composite_buffers(&result, size.0, size.1, 0)?
                            .into_parts();
                        submissions.push(submission);
                        Some(buffer)
                    };
                    self.renderer.object(
                        plan,
                        background.as_ref().map(|buffer| (buffer, size)),
                        submissions,
                    )?
                }
                Source::Manim(frame) => self.manim_frame(frame)?,
                Source::RasterMorph(_) => unreachable!("handled before source rendering"),
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
                    sam2_target: self.sam2_target.as_ref(),
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
