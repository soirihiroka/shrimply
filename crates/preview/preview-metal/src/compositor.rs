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
            renderer.update(project, time, 0)?;
            for update in renderer.take_manim_updates() {
                match update {
                    shrimply_state::manim_status::Update::Parameters {
                        render_is_current: false,
                        ..
                    } => {
                        return Err(
                            "Manim parameters changed while preparing the frame; wait for the preview to update and try again"
                                .into(),
                        );
                    }
                    shrimply_state::manim_status::Update::Error {
                        error: Some(error), ..
                    } => return Err(error),
                    _ => {}
                }
            }
            if let Some(image) = &renderer.presented
                && !image.loading
                && image.accuracy.content_accurate()
            {
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
    request_id: u64,
    started: std::time::Instant,
    time: Time,
    accuracy: shrimply_preview_render_core::CompositeAccuracy,
    loading: bool,
    audio_analysis: shrimply_preview_render_core::FrameAudioAnalysis,
    frame: shrimply_render_metal::Frame,
    width: u32,
    height: u32,
    revision: u64,
    effect_submissions: Vec<shrimply_render_metal::Submission>,
}

pub(super) struct Presented {
    pub request_id: u64,
    pub image: Image,
    pub time: Time,
    pub render_elapsed: Duration,
    pub accuracy: shrimply_preview_render_core::CompositeAccuracy,
    pub loading: bool,
    pub audio_analysis: shrimply_preview_render_core::FrameAudioAnalysis,
}

#[derive(Default)]
pub(super) struct Compositor {
    scene: Scene,
    manim_updates: Vec<shrimply_state::manim_status::Update>,
    compute: Option<shrimply_render_metal::Renderer>,
    pending: Option<Pending>,
    queued: Option<FramePlan>,
    presented: Option<Presented>,
    revision: u64,
    sources: HashMap<u32, shrimply_render_metal::Buffer>,
    manim: Option<shrimply_manim_metal::Renderer>,
    manim_slots: HashMap<uuid::Uuid, usize>,
    next_manim_slot: usize,
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

    pub fn take_manim_updates(&mut self) -> Vec<shrimply_state::manim_status::Update> {
        let mut updates = self.scene.take_manim_updates();
        updates.append(&mut self.manim_updates);
        updates
    }

    pub fn needs_update(&self) -> bool {
        self.pending.is_some() || self.queued.is_some() || self.scene.needs_update()
    }

    pub fn update(&mut self, project: &Project, time: Time, request_id: u64) -> Result<(), String> {
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
                        request_id: pending.request_id,
                        time: pending.time,
                        render_elapsed: pending.started.elapsed(),
                        accuracy: pending.accuracy,
                        loading: pending.loading,
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
        let mut used_manim = HashSet::new();
        let started = std::time::Instant::now();
        let layers = self.render_layers(
            &plan.layers,
            (plan.width, plan.height),
            &mut effect_submissions,
            &mut used_sources,
            &mut used_manim,
        )?;
        self.sources.retain(|id, _| used_sources.contains(id));
        self.manim_slots
            .retain(|item_id, _| used_manim.contains(item_id));
        if let Some(manim) = &mut self.manim {
            let active = self.manim_slots.values().copied().collect::<Vec<_>>();
            manim.retain_slots(&active);
        }
        let frame = self
            .compute
            .as_mut()
            .expect("initialized Metal compositor")
            .composite_buffers(&layers, plan.width, plan.height, 0)?;
        self.pending = Some(Pending {
            request_id,
            started,
            time: plan.time,
            accuracy: plan.accuracy,
            loading: plan.loading,
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
        used_manim: &mut HashSet<uuid::Uuid>,
    ) -> Result<
        Vec<(
            shrimply_render_core::Nv12LayerParams,
            shrimply_render_metal::Buffer,
        )>,
        String,
    > {
        let mut layers = Vec::with_capacity(source_layers.len());
        for layer in source_layers {
            let (mut buffer, rgba_pitch) = match &layer.source {
                Source::Generated(frame) => (
                    self.compute
                        .as_mut()
                        .expect("initialized Metal compositor")
                        .draw_vector(layer.render_size, |canvas| {
                            frame.draw(canvas, &mut Default::default())
                        })?,
                    None,
                ),
                Source::Group(children) => {
                    let children = self.render_layers(
                        children,
                        size,
                        effect_submissions,
                        used_sources,
                        used_manim,
                    )?;
                    let (buffer, submission) = self
                        .compute
                        .as_mut()
                        .expect("initialized Metal compositor")
                        .composite_buffers(&children, size.0, size.1, 0)?
                        .into_parts();
                    effect_submissions.push(submission);
                    (buffer, None)
                }

                Source::Background(uniforms) => {
                    let (buffer, submission) = self
                        .compute
                        .as_mut()
                        .expect("initialized Metal compositor")
                        .background(uniforms)?;
                    effect_submissions.push(submission);
                    (buffer, None)
                }
                Source::Image(image) => (self.image_buffer(image, used_sources)?, None),
                Source::Manim(frame) => {
                    used_manim.insert(frame.item_id);
                    let slot = if let Some(slot) = self.manim_slots.get(&frame.item_id) {
                        *slot
                    } else {
                        let slot = self.next_manim_slot;
                        self.next_manim_slot = self
                            .next_manim_slot
                            .checked_add(1)
                            .expect("Manim render slot overflow");
                        self.manim_slots.insert(frame.item_id, slot);
                        slot
                    };
                    if self.manim.is_none() {
                        self.manim = Some(shrimply_manim_metal::Renderer::new(
                            self.compute.as_ref().expect("initialized Metal compositor"),
                        )?);
                    }
                    let rendered = self
                        .manim
                        .as_mut()
                        .expect("initialized Manim Metal renderer")
                        .render(
                            self.compute.as_ref().expect("initialized Metal compositor"),
                            slot,
                            &frame.prepared,
                            frame.frame_index,
                        );
                    let rendered = match rendered {
                        Ok(rendered) => {
                            self.manim_updates.push(frame.source.error(None));
                            rendered
                        }
                        Err(error) => {
                            self.manim_updates
                                .push(frame.source.error(Some(error.clone())));
                            return Err(error);
                        }
                    };
                    (rendered.buffer, Some(rendered.row_bytes))
                }
            };
            let mut parameters = layer.parameters;
            if let Some(rgba_pitch) = rgba_pitch {
                parameters.rgba_pitch = rgba_pitch;
                let packed_pitch = parameters.source_width as usize * size_of::<u32>();
                if rgba_pitch != packed_pitch {
                    let mut source = parameters;
                    source.inverse = shrimply_render_core::math::Mat3::IDENTITY;
                    source.opacity = 1.0;
                    source.blend_mode = shrimply_render_core::LayerBlendMode::Normal;
                    source.sample_method = shrimply_render_core::VideoSampleMethod::Nearest;
                    let (packed, submission) = self
                        .compute
                        .as_mut()
                        .expect("initialized Metal compositor")
                        .composite_buffers(
                            &[(source, buffer)],
                            parameters.source_width,
                            parameters.source_height,
                            0,
                        )?
                        .into_parts();
                    effect_submissions.push(submission);
                    buffer = packed;
                    parameters.rgba_pitch = packed_pitch;
                }
            }
            let mask_buffer = layer
                .video_mask
                .as_ref()
                .map(|mask| self.image_buffer(&mask.image, used_sources))
                .transpose()?;
            let renderer = self.compute.as_mut().expect("initialized Metal compositor");
            let buffer = if let Some((mask, mask_buffer)) =
                layer.video_mask.as_ref().zip(mask_buffer)
            {
                let (mut parameters, _) = shrimply_render_core::effects::materialization(
                    layer.parameters,
                    mask.size.0,
                    mask.size.1,
                );
                parameters.source_width = mask.image.width() as u32;
                parameters.source_height = mask.image.height() as u32;
                parameters.rgba_pitch = mask.image.width() as usize * size_of::<u32>();
                parameters.inverse = shrimply_render_core::math::Mat3::IDENTITY;
                parameters.sample_method = mask.sampling;
                let (mask_buffer, submission) = renderer
                    .composite_buffers(&[(parameters, mask_buffer)], mask.size.0, mask.size.1, 0)?
                    .into_parts();
                effect_submissions.push(submission);
                super::alpha_mask::video(
                    renderer,
                    buffer,
                    (parameters.source_width, parameters.source_height),
                    mask_buffer,
                    mask.size,
                    effect_submissions,
                )?
            } else {
                buffer
            };
            let (mut state, buffer) = super::effects::apply_modifiers(
                renderer,
                buffer,
                super::effects::State {
                    parameters,
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
            if let Some(mask) = &layer.alpha_mask {
                buffer = super::alpha_mask::apply(
                    renderer,
                    buffer,
                    (
                        state.parameters.source_width,
                        state.parameters.source_height,
                    ),
                    mask,
                    effect_submissions,
                )?;
            }
            layers.push((state.sampled(), buffer));
        }
        Ok(layers)
    }

    fn image_buffer(
        &mut self,
        image: &Image,
        used_sources: &mut HashSet<u32>,
    ) -> Result<shrimply_render_metal::Buffer, String> {
        used_sources.insert(image.unique_id());
        if let Some(buffer) = self.sources.get(&image.unique_id()) {
            return Ok(buffer.clone());
        }
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
            return Err("Could not read source pixels for the Metal compositor".into());
        }
        let buffer = self
            .compute
            .as_ref()
            .expect("initialized Metal compositor")
            .upload(&bytes)?;
        self.sources.insert(image.unique_id(), buffer.clone());
        Ok(buffer)
    }
}
