#![recursion_limit = "256"]

#![cfg(any(target_os = "linux", target_os = "android"))]

use shrimply_render_vulkan as gpu;
#[path = "../../preview-render-core/src/gpu_alpha_mask.rs"]
mod alpha_mask;
pub mod analysis;
#[path = "../../preview-render-core/src/gpu_effects.rs"]
mod effects;
#[path = "../../preview-render-core/src/gpu_layered_image.rs"]
mod layered_image;
#[cfg(target_os = "linux")]
mod manim;
pub mod modifier_cache;
mod morph;
mod sam2;
#[path = "../../preview-render-core/src/gpu_sources.rs"]
mod sources;
pub mod worker;
pub use sam2::Sam2ProxyFrameSource;

use gpu::{Buffer, Renderer, Submission};
use shrimply_math_core::Time;
use shrimply_preview_render_core::{FramePlan, Layer, Scene, Source};
use shrimply_project_document::project::Project;
use shrimply_render_core::Nv12LayerParams;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

type ExternalMasks = Vec<(
    shrimply_visual_core::raster_modifiers::ExternalDependency,
    Buffer,
)>;

pub struct Rendered {
    pub source_frame: Option<u64>,
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
    presentation: Rc<Submission>,
}

impl Presented {
    pub fn into_buffers(self) -> (gpu::Texture, Buffer) {
        assert!(
            self.completed().expect("Vulkan GPU submission failed"),
            "Vulkan frame published before GPU completion"
        );
        (self.display, self.rendered.buffer)
    }
    pub fn completed(&self) -> Result<bool, String> {
        self.presentation.completed()
    }
}

pub struct Compositor {
    presentation: Option<(u64, gpu::Texture, Rc<Submission>)>,
    scene: Scene,
    morphs: HashMap<shrimply_preview_render_core::MorphCacheKey, morph::Cached>,
    used_morphs: HashSet<shrimply_preview_render_core::MorphCacheKey>,
    pub renderer: Renderer,
    sources: sources::Sources,
    layered_sources: HashMap<(String, usize), Buffer>,
    used_layered_sources: HashSet<(String, usize)>,
    sam2_proxy: Option<Buffer>,
    sam2_target: Option<shrimply_visual_core::sam2::analysis::AnalysisTarget>,
    #[cfg(target_os = "linux")]
    manim: Option<shrimply_manim_wgpu::vulkan::Renderer>,
    #[cfg(target_os = "linux")]
    manim_slots: HashMap<uuid::Uuid, manim::Slot>,
    #[cfg(target_os = "linux")]
    used_manim: HashSet<uuid::Uuid>,
    #[cfg(target_os = "linux")]
    next_manim_slot: usize,
    manim_updates: Vec<shrimply_manim_state::Update>,
}

impl Compositor {
    pub fn new() -> Result<Self, String> {
        let renderer = Renderer::new()?;
        Ok(Self {
            presentation: None,
            scene: Scene::default(),
            morphs: HashMap::new(),
            used_morphs: HashSet::new(),
            renderer,
            sources: sources::Sources::default(),
            layered_sources: HashMap::new(),
            used_layered_sources: HashSet::new(),
            sam2_proxy: None,
            sam2_target: None,
            #[cfg(target_os = "linux")]
            manim: None,
            #[cfg(target_os = "linux")]
            manim_slots: HashMap::new(),
            #[cfg(target_os = "linux")]
            used_manim: HashSet::new(),
            #[cfg(target_os = "linux")]
            next_manim_slot: 0,
            manim_updates: Vec::new(),
        })
    }
    pub fn set_decoder_limit(&mut self, maximum: usize) {
        self.scene.set_decoder_limit(maximum);
        #[cfg(target_os = "linux")]
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
        self.presentation = None;
        self.scene.invalidate();
        self.morphs.clear();
        self.renderer.clear_video_contexts();
        self.sources = sources::Sources::default();
        self.layered_sources.clear();
        #[cfg(target_os = "linux")]
        {
            self.manim_slots.clear();
            if let Some(manim) = &mut self.manim {
                manim.release_render_surfaces();
                manim.release_gpu_animation_resources();
            }
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
        self.scene.set_interaction(accuracy);
        self.scene.prepare(project, time)
    }
    pub fn render(&mut self, plan: FramePlan) -> Result<Presented, String> {
        let continuous = plan.accuracy.continuous_playback();
        let rendered = {
            let _measurement = shrimply_profiling::measure("Vulkan rendering / Compose layers");
            self.render_gpu(plan, 0)?
        };
        let _measurement = shrimply_profiling::measure("Vulkan rendering / Prepare presentation");
        let (display, presentation) = if let Some((source, display, submitted)) = &self.presentation
            && continuous
            && rendered.source_frame == Some(*source)
        {
            shrimply_profiling::increment(
                "Vulkan rendering / Unchanged video presentations reused",
            );
            (display.clone(), submitted.clone())
        } else {
            let (display, submitted) = self.renderer.presentation_texture(
                &rendered.buffer,
                rendered.width,
                rendered.height,
            )?;
            let submitted = Rc::new(submitted);
            self.presentation = rendered
                .source_frame
                .map(|source| (source, display.clone(), submitted.clone()));
            (display, submitted)
        };
        Ok(Presented {
            display,
            rendered,
            presentation,
        })
    }

    pub fn render_rgba(
        &mut self,
        project: &Project,
        position: Time,
        target: Option<shrimply_preview_render_core::CaptureTarget>,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<shrimply_preview_render_core::capture::RgbaFrame, String> {
        self.set_capture_target(target);
        let rendered = self.render_accurate(project, position, 0, cancelled)?;
        rendered.wait()?;
        Ok(shrimply_preview_render_core::capture::RgbaFrame {
            width: i32::try_from(rendered.width).map_err(|_| "Frame width is too large")?,
            height: i32::try_from(rendered.height).map_err(|_| "Frame height is too large")?,
            pixels: self.renderer.readback(&rendered.buffer)?,
        })
    }

    pub fn capture_rgba(
        &mut self,
        project: &Project,
        position: Time,
        target: shrimply_preview_render_core::CaptureTarget,
    ) -> Result<Option<Vec<u8>>, String> {
        self.scene.set_strict_sources(true);
        let result = (|| {
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
        })();
        self.scene.set_strict_sources(false);
        result
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
        let source_frame =
            shrimply_preview_render_core::math::unmodified_video(&plan).map(|video| video.id());
        if self
            .presentation
            .as_ref()
            .is_some_and(|(source, ..)| Some(*source) != source_frame)
        {
            self.presentation = None;
        }
        let direct_video = source_frame.is_some();
        let mut submissions = Vec::new();
        self.sources.begin_frame();
        self.used_morphs.clear();
        self.used_layered_sources.clear();
        #[cfg(target_os = "linux")]
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
            shrimply_profiling::increment("Vulkan rendering / Video composition bypasses");
            layers[0].1.clone()
        } else {
            let _measurement = shrimply_profiling::measure("Vulkan rendering / Final composition");
            let (buffer, composite) = self
                .renderer
                .composite_buffers(&layers, size.0, size.1, background)?
                .into_parts();
            submissions.push(composite);
            buffer
        };
        self.morphs.retain(|key, _| self.used_morphs.contains(key));
        self.sources.retain_used();
        self.layered_sources
            .retain(|key, _| self.used_layered_sources.contains(key));
        #[cfg(target_os = "linux")]
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
            source_frame,
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

    fn layers(
        &mut self,
        layers: &[Layer],
        size: (u32, u32),
        external_masks: &ExternalMasks,
        submissions: &mut Vec<Submission>,
    ) -> Result<Vec<(Nv12LayerParams, Buffer)>, String> {
        let _measurement = shrimply_profiling::measure("Vulkan rendering / Prepare layers");
        let mut result = Vec::with_capacity(layers.len());
        for layer in layers {
            if let Source::RasterMorph(morph) = &layer.source {
                result.extend(self.raster_morph(morph, size, external_masks, submissions)?);
                continue;
            }
            let buffer = match &layer.source {
                Source::Image(image) => self.sources.image(&self.renderer, image)?,
                Source::Video(video) => self.sources.video(&mut self.renderer, video)?,
                Source::Generated(frame) => {
                    let (buffer, submission) =
                        self.renderer.draw_vector(layer.render_size, |canvas| {
                            frame.draw(canvas, &mut self.scene.expressions)
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
                #[cfg(target_os = "linux")]
                Source::Manim(frame) => self.manim_frame(frame)?,
                #[cfg(target_os = "android")]
                Source::Manim(_) => {
                    return Err("Manim rendering is not available on Android".into());
                }
                Source::RasterMorph(_) => unreachable!("handled before source rendering"),
            };
            let (mut parameters, buffer) = effects::apply_layer(
                &mut self.renderer,
                layer,
                (layer.parameters, buffer),
                effects::LayerResources {
                    sources: &mut self.sources,
                    external_masks,
                    canvas_size: size,
                    sam2_target: self.sam2_target.as_ref(),
                    sam2_proxy: &mut self.sam2_proxy,
                },
                submissions,
            )?;
            // Native YCbCr decoder imports have no alpha channel. Effects may
            // introduce transparency, so only untouched sources retain this fact.
            if matches!(layer.source, Source::Video(_)) && !layer.has_effects() {
                parameters.kind = shrimply_render_core::LayerKind::RgbaOpaque;
            }
            result.push((parameters, buffer));
        }
        Ok(result)
    }
}
