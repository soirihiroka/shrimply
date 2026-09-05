use shrimply_math_core::Time;
use shrimply_preview_render_core::{FramePlan, Scene, Source};
use shrimply_project::project::Project;
use skia_safe::{AlphaType, ColorType, Data, Image, ImageInfo};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Renders an accurate frame through the preview compositor and encodes it as PNG.
/// This blocks on media decoding and GPU completion; call it from a worker thread.
/// Caption overlays use preview-pixel sizing and are drawn separately by the host.
pub fn render_png(project: &Project, time: Time) -> Result<Vec<u8>, String> {
    objc2::rc::autoreleasepool(|_| {
        let mut renderer = Compositor::default();
        loop {
            renderer.update(project, time)?;
            if let Some(image) = &renderer.presented {
                return image
                    .image
                    .encode(None, skia_safe::EncodedImageFormat::PNG, None)
                    .map(|data| data.as_bytes().to_vec())
                    .ok_or_else(|| "Could not encode the rendered frame as PNG".into());
            }
            std::thread::sleep(FRAME_POLL_INTERVAL);
        }
    })
}

struct Pending {
    started: std::time::Instant,
    time: Time,
    accuracy: shrimply_preview_render_core::CompositeAccuracy,
    audio_analysis: shrimply_preview_render_core::FrameAudioAnalysis,
    frame: shrimply_render_metal::Frame,
    width: u32,
    height: u32,
    revision: u64,
    effect_submissions: Vec<shrimply_render_metal::Submission>,
}

pub(super) struct Presented {
    pub image: Image,
    pub time: Time,
    pub render_elapsed: Duration,
    pub accuracy: shrimply_preview_render_core::CompositeAccuracy,
    pub audio_analysis: shrimply_preview_render_core::FrameAudioAnalysis,
}

#[derive(Default)]
pub(super) struct Compositor {
    scene: Scene,
    compute: Option<shrimply_render_metal::Renderer>,
    pending: Option<Pending>,
    queued: Option<FramePlan>,
    presented: Option<Presented>,
    revision: u64,
    sources: HashMap<u32, shrimply_render_metal::Buffer>,
}

impl Compositor {
    pub fn set_exclusion(&mut self, excluded_item_id: Option<uuid::Uuid>) {
        self.scene.set_exclusion(excluded_item_id);
    }

    pub fn set_interaction(&mut self, playing: bool, scrubbing: bool) {
        self.scene.set_interaction(playing, scrubbing);
    }

    pub fn invalidate(&mut self) {
        self.scene.invalidate();
        self.revision += 1;
        self.queued = None;
        self.presented = None;
        // In-flight resources remain alive. The old complete frame stays visible
        // until a frame for the new project revision has actually completed.
    }

    pub fn take_presented(&mut self) -> Option<Presented> {
        self.presented.take()
    }

    pub fn needs_update(&self) -> bool {
        self.pending.is_some() || self.queued.is_some() || self.scene.needs_update()
    }

    pub fn update(&mut self, project: &Project, time: Time) -> Result<(), String> {
        if let Some(pending) = self.pending.take() {
            let effects_ready = pending
                .effect_submissions
                .iter()
                .try_fold(true, |ready, submission| {
                    submission.completed().map(|complete| ready && complete)
                })?;
            if let Some(pixels) = if effects_ready {
                pending.frame.pixels()?
            } else {
                None
            } {
                if pending.revision == self.revision {
                    let info = ImageInfo::new(
                        (pending.width as i32, pending.height as i32),
                        ColorType::RGBA8888,
                        AlphaType::Unpremul,
                        None,
                    );
                    self.presented = Some(Presented {
                        time: pending.time,
                        render_elapsed: pending.started.elapsed(),
                        accuracy: pending.accuracy,
                        audio_analysis: pending.audio_analysis,
                        image: skia_safe::images::raster_from_data(
                            &info,
                            Data::new_copy(pixels),
                            pending.width as usize * size_of::<u32>(),
                        )
                        .ok_or("Could not present the completed Metal frame")?,
                    });
                }
            } else {
                self.pending = Some(pending);
            }
        }
        if let Some(plan) = self.scene.prepare(project, time)? {
            self.queued = Some(plan);
        }
        if self.pending.is_some() {
            return Ok(());
        }
        let Some(plan) = self.queued.take() else {
            return Ok(());
        };
        if self.compute.is_none() {
            self.compute = Some(shrimply_render_metal::Renderer::new()?);
        }
        let mut effect_submissions = Vec::new();
        let mut used_sources = HashSet::new();
        let started = std::time::Instant::now();
        let layers = self.render_layers(
            &plan.layers,
            (plan.width, plan.height),
            &mut effect_submissions,
            &mut used_sources,
        )?;
        self.sources.retain(|id, _| used_sources.contains(id));
        let frame = self
            .compute
            .as_mut()
            .expect("initialized Metal compositor")
            .composite_buffers(&layers, plan.width, plan.height, 0)?;
        self.pending = Some(Pending {
            started,
            time: plan.time,
            accuracy: plan.accuracy,
            audio_analysis: plan.audio_analysis,
            frame,
            width: plan.width,
            height: plan.height,
            revision: self.revision,
            effect_submissions,
        });
        Ok(())
    }
    fn render_layers(
        &mut self,
        source_layers: &[shrimply_preview_render_core::Layer],
        size: (u32, u32),
        effect_submissions: &mut Vec<shrimply_render_metal::Submission>,
        used_sources: &mut HashSet<u32>,
    ) -> Result<
        Vec<(
            shrimply_render_core::Nv12LayerParams,
            shrimply_render_metal::Buffer,
        )>,
        String,
    > {
        let mut layers = Vec::with_capacity(source_layers.len());
        for layer in source_layers {
            let buffer = match &layer.source {
                Source::Generated(frame) => self
                    .compute
                    .as_mut()
                    .expect("initialized Metal compositor")
                    .draw_vector(layer.render_size, |canvas| {
                        frame.draw(canvas, &mut Default::default())
                    })?,
                Source::Group(children) => {
                    let children =
                        self.render_layers(children, size, effect_submissions, used_sources)?;
                    let (buffer, submission) = self
                        .compute
                        .as_mut()
                        .expect("initialized Metal compositor")
                        .composite_buffers(&children, size.0, size.1, 0)?
                        .into_parts();
                    effect_submissions.push(submission);
                    buffer
                }

                Source::Background(uniforms) => {
                    let (buffer, submission) = self
                        .compute
                        .as_mut()
                        .expect("initialized Metal compositor")
                        .background(uniforms)?;
                    effect_submissions.push(submission);
                    buffer
                }
                Source::Image(image) => {
                    used_sources.insert(image.unique_id());
                    if let Some(buffer) = self.sources.get(&image.unique_id()) {
                        buffer.clone()
                    } else {
                        let info = ImageInfo::new(
                            image.dimensions(),
                            ColorType::RGBA8888,
                            AlphaType::Unpremul,
                            None,
                        );
                        let row_bytes = image.width() as usize * size_of::<u32>();
                        let mut bytes = vec![
                            0;
                            row_bytes
                                .checked_mul(image.height() as usize)
                                .ok_or("Source image size overflow")?
                        ];
                        if !image.read_pixels(
                            &info,
                            &mut bytes,
                            row_bytes,
                            (0, 0),
                            skia_safe::image::CachingHint::Allow,
                        ) {
                            return Err(
                                "Could not read source pixels for the Metal compositor".into()
                            );
                        }
                        let buffer = self
                            .compute
                            .as_ref()
                            .expect("initialized Metal compositor")
                            .upload(&bytes)?;
                        self.sources.insert(image.unique_id(), buffer.clone());
                        buffer
                    }
                }
            };
            let renderer = self.compute.as_mut().expect("initialized Metal compositor");
            let (mut state, buffer) = super::effects::apply_modifiers(
                renderer,
                buffer,
                super::effects::State {
                    parameters: layer.parameters,
                    transform: layer.transform,
                },
                &layer.effects,
                layer.render_size,
                effect_submissions,
            )?;
            let mut buffer = buffer;
            if let Some(samples) = &layer.motion_blur {
                (state, buffer) = super::effects::apply_motion_blur(
                    renderer,
                    buffer,
                    state,
                    samples,
                    layer.render_size,
                    effect_submissions,
                )?;
            }
            // Item transitions precede clip transitions, as in CUDA.
            for stage in &layer.transitions {
                state.transform = stage.transform * state.transform;
                (state, buffer) = super::effects::apply(
                    renderer,
                    buffer,
                    state,
                    stage.effect.as_slice(),
                    layer.render_size,
                    effect_submissions,
                )?;
            }
            state.transform = layer.output_transform * state.transform;
            layers.push((state.sampled(), buffer));
        }
        Ok(layers)
    }
}
