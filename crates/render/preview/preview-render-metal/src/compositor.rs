use shrimply_math_core::Time;
use shrimply_preview_render_core::{FramePlan, Scene, Source};
use shrimply_project_document::project::Project;
use skia_safe::{AlphaType, ColorType, Data, Image, ImageInfo};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(1);

type ExternalMaskBuffers = Vec<(
    shrimply_visual_core::raster_modifiers::ExternalDependency,
    shrimply_render_metal::Buffer,
)>;

/// Renders an accurate frame through the preview compositor and encodes it as PNG.
/// This blocks on media decoding and GPU completion; call it from a worker thread.
/// Caption overlays use preview-pixel sizing and are drawn separately by the host.
pub fn render_png(
    project: &Project,
    time: Time,
    maximum_decoders: usize,
) -> Result<Vec<u8>, String> {
    objc2::rc::autoreleasepool(|_| {
        let mut renderer = Compositor::default();
        renderer.set_decoder_limit(maximum_decoders);
        loop {
            if let Some(image) = renderer.poll_accurate_image(project, time)? {
                return image
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

struct MaterializedMorphEndpoint {
    buffer: shrimply_render_metal::Buffer,
    parameters: shrimply_render_core::Nv12LayerParams,
}

enum RasterMorphState {
    Materializing {
        source: MaterializedMorphEndpoint,
        target: MaterializedMorphEndpoint,
        submissions: Vec<shrimply_render_metal::Submission>,
    },
    Estimating {
        source: MaterializedMorphEndpoint,
        target: MaterializedMorphEndpoint,
        receiver:
            mpsc::Receiver<Result<shrimply_visual_core::raster_morph::OpticalFlowField, String>>,
    },
    Ready {
        source: MaterializedMorphEndpoint,
        target: MaterializedMorphEndpoint,
        flow: shrimply_visual_core::raster_morph::OpticalFlowField,
    },
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
    manim_updates: Vec<shrimply_manim_state::Update>,
    compute: Option<shrimply_render_metal::Renderer>,
    pending: Option<Pending>,
    queued: Option<FramePlan>,
    presented: Option<Presented>,
    revision: u64,
    sources: HashMap<u32, shrimply_render_metal::Buffer>,
    layered_sources: HashMap<(String, usize), shrimply_render_metal::Buffer>,
    used_layered_sources: HashSet<(String, usize)>,
    manim: Option<shrimply_manim_metal::Renderer>,
    gaussian: Option<shrimply_3dgs_metal::Renderer>,
    obj: Option<shrimply_render_3d_metal::Renderer>,
    manim_slots: HashMap<uuid::Uuid, usize>,
    next_manim_slot: usize,
    raster_morphs: HashMap<shrimply_preview_render_core::MorphCacheKey, RasterMorphState>,
    deferred_submissions: Vec<shrimply_render_metal::Submission>,
    sam2_analysis_target: Option<shrimply_visual_core::sam2::analysis::AnalysisTarget>,
    sam2_proxy_buffer: Option<shrimply_render_metal::Buffer>,
    sam2_proxy_ready: Option<Vec<u8>>,
    background_alpha: u8,
}

impl Compositor {
    pub fn set_decoder_limit(&mut self, maximum: usize) {
        self.scene.set_decoder_limit(maximum);
    }

    pub fn warmup(&mut self) -> Result<(), String> {
        let mut compute = shrimply_render_metal::Renderer::new()?;
        compute.warmup()?;
        self.gaussian = Some(shrimply_3dgs_metal::Renderer::new(&compute)?);
        if compute.supports_ray_tracing() {
            self.obj = Some(shrimply_render_3d_metal::Renderer::new(&compute)?);
        }
        self.compute = Some(compute);
        Ok(())
    }

    pub fn set_capture_target(&mut self, target: shrimply_preview_render_core::CaptureTarget) {
        if self.scene.set_capture_target(Some(target)) {
            self.invalidate();
        }
    }

    /// Polls a completed, accurate CPU image without blocking media or GPU work.
    pub fn poll_accurate_image(
        &mut self,
        project: &Project,
        time: Time,
    ) -> Result<Option<Image>, String> {
        self.update(project, time, 0)?;
        for update in self.take_manim_updates() {
            match update {
                shrimply_manim_state::Update::Parameters {
                    render_is_current: false,
                    ..
                } => {
                    return Err("Manim parameters changed while preparing the frame; wait for the preview to update and try again".into());
                }
                shrimply_manim_state::Update::Error {
                    error: Some(error), ..
                } => return Err(error),
                _ => {}
            }
        }
        Ok(self
            .take_presented()
            .filter(|frame| {
                frame.time == time && !frame.loading && frame.accuracy.content_accurate()
            })
            .map(|frame| frame.image))
    }

    pub fn set_background_alpha(&mut self, background_alpha: u8) {
        if self.background_alpha != background_alpha {
            self.background_alpha = background_alpha;
            self.invalidate();
        }
    }

    pub fn set_exclusion(&mut self, excluded_item_id: Option<uuid::Uuid>) {
        self.scene.set_exclusion(excluded_item_id);
    }

    pub fn set_interaction(&mut self, playing: bool, scrubbing: bool) {
        self.scene.set_interaction(playing, scrubbing);
    }

    pub fn set_sam2_analysis_target(
        &mut self,
        target: shrimply_visual_core::sam2::analysis::AnalysisTarget,
    ) {
        if self.sam2_analysis_target.as_ref() == Some(&target) {
            return;
        }
        self.invalidate();
        self.scene
            .set_capture_target(Some(shrimply_preview_render_core::CaptureTarget::Item(
                target.address.clone(),
            )));
        self.sam2_analysis_target = Some(target);
    }

    pub fn take_sam2_proxy(&mut self) -> Option<Vec<u8>> {
        self.sam2_proxy_ready.take()
    }

    pub fn invalidate(&mut self) {
        self.scene.invalidate();
        self.revision += 1;
        self.queued = None;
        self.presented = None;
        self.raster_morphs.clear();
        self.layered_sources.clear();
        self.used_layered_sources.clear();
        self.sam2_proxy_buffer = None;
        self.sam2_proxy_ready = None;
        // In-flight resources remain alive. The old complete frame stays visible
        // until a frame for the new project revision has actually completed.
    }

    pub fn take_presented(&mut self) -> Option<Presented> {
        self.presented.take()
    }

    pub fn take_manim_updates(&mut self) -> Vec<shrimply_manim_state::Update> {
        let mut updates = self.scene.take_manim_updates();
        updates.append(&mut self.manim_updates);
        updates
    }

    pub fn needs_update(&self) -> bool {
        self.pending.is_some() || self.queued.is_some() || self.scene.needs_update()
    }

    pub fn update(&mut self, project: &Project, time: Time, request_id: u64) -> Result<(), String> {
        let mut deferred_submissions = Vec::with_capacity(self.deferred_submissions.len());
        for submission in std::mem::take(&mut self.deferred_submissions) {
            if !submission.completed()? {
                deferred_submissions.push(submission);
            }
        }
        self.deferred_submissions = deferred_submissions;
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
                    if pending.loading {
                        self.sam2_proxy_buffer = None;
                    } else if let Some(proxy) = self.sam2_proxy_buffer.take() {
                        self.sam2_proxy_ready = Some(proxy.copy_bytes());
                    }
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
        self.used_layered_sources.clear();
        let started = std::time::Instant::now();
        let mut external_masks = ExternalMaskBuffers::new();
        for external in &plan.external_layers {
            let Some(layers) = self.render_layers(
                &external.layers,
                (plan.width, plan.height),
                &external_masks,
                &mut effect_submissions,
                &mut used_sources,
                &mut used_manim,
            )?
            else {
                self.deferred_submissions.append(&mut effect_submissions);
                self.queued = Some(plan);
                return Ok(());
            };
            let (buffer, submission) = self
                .compute
                .as_mut()
                .expect("initialized Metal compositor")
                .composite_buffers(&layers, plan.width, plan.height, 0)?
                .into_parts();
            effect_submissions.push(submission);
            external_masks.push((external.dependency.clone(), buffer));
        }
        let rendered = self.render_layers(
            &plan.layers,
            (plan.width, plan.height),
            &external_masks,
            &mut effect_submissions,
            &mut used_sources,
            &mut used_manim,
        )?;
        self.layered_sources
            .retain(|key, _| self.used_layered_sources.contains(key));
        let Some(layers) = rendered else {
            self.deferred_submissions.append(&mut effect_submissions);
            self.queued = Some(plan);
            return Ok(());
        };
        effect_submissions.append(&mut self.deferred_submissions);
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
            .composite_buffers(
                &layers,
                plan.width,
                plan.height,
                u32::from(self.background_alpha) << 24,
            )?;
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
        external_masks: &ExternalMaskBuffers,
        effect_submissions: &mut Vec<shrimply_render_metal::Submission>,
        used_sources: &mut HashSet<u32>,
        used_manim: &mut HashSet<uuid::Uuid>,
    ) -> Result<
        Option<
            Vec<(
                shrimply_render_core::Nv12LayerParams,
                shrimply_render_metal::Buffer,
            )>,
        >,
        String,
    > {
        let mut layers = Vec::with_capacity(source_layers.len());
        for layer in source_layers {
            if let Source::RasterMorph(morph) = &layer.source {
                let Some(mut rendered) = self.render_raster_morph(
                    morph,
                    size,
                    external_masks,
                    used_sources,
                    used_manim,
                    effect_submissions,
                )?
                else {
                    return Ok(None);
                };
                layers.append(&mut rendered);
                continue;
            }
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
                    let Some(children) = self.render_layers(
                        children,
                        size,
                        external_masks,
                        effect_submissions,
                        used_sources,
                        used_manim,
                    )?
                    else {
                        return Ok(None);
                    };
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
                Source::LayeredImage(plan) => {
                    (self.layered_image_buffer(plan, effect_submissions)?, None)
                }
                Source::Gaussian(plan) => {
                    if self.gaussian.is_none() {
                        self.gaussian = Some(shrimply_3dgs_metal::Renderer::new(
                            self.compute.as_ref().expect("initialized Metal compositor"),
                        )?);
                    }
                    let rendered = self
                        .gaussian
                        .as_mut()
                        .expect("initialized Gaussian Metal renderer")
                        .render(
                            self.compute.as_ref().expect("initialized Metal compositor"),
                            &plan.session,
                            plan.width,
                            plan.height,
                            &plan.params,
                        )?;
                    (rendered.buffer, Some(rendered.row_bytes))
                }
                Source::Obj(plan) => {
                    let composite_background = if layers.is_empty() {
                        None
                    } else {
                        let (buffer, submission) = self
                            .compute
                            .as_mut()
                            .expect("initialized Metal compositor")
                            .composite_buffers(&layers, size.0, size.1, 0)?
                            .into_parts();
                        effect_submissions.push(submission);
                        Some(buffer)
                    };
                    if self.obj.is_none() {
                        self.obj = Some(shrimply_render_3d_metal::Renderer::new(
                            self.compute.as_ref().expect("initialized Metal compositor"),
                        )?);
                    }
                    let rendered = self
                        .obj
                        .as_mut()
                        .expect("initialized OBJ Metal renderer")
                        .render(
                            self.compute.as_ref().expect("initialized Metal compositor"),
                            plan,
                            composite_background.as_ref().map(|buffer| {
                                shrimply_render_3d_metal::CompositeBackground {
                                    buffer,
                                    width: size.0,
                                    height: size.1,
                                }
                            }),
                        )?;
                    (rendered.buffer, Some(rendered.row_bytes))
                }
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
                    let rendered = (|| {
                        if self.manim.is_none() {
                            self.manim = Some(shrimply_manim_metal::Renderer::new(
                                self.compute.as_ref().expect("initialized Metal compositor"),
                            )?);
                        }
                        self.manim
                            .as_mut()
                            .expect("initialized Manim Metal renderer")
                            .render(
                                self.compute.as_ref().expect("initialized Metal compositor"),
                                slot,
                                &frame.prepared,
                                frame.frame_index,
                            )
                    })();
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
                Source::RasterMorph(_) => unreachable!("handled before source rendering"),
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
            let mut state = shrimply_render_core::effects::SpatialState {
                parameters,
                transform: layer.transform,
                texture_edges: [0.0; 4],
                modifier_crop: [0.0; 4],
                modifier_crop_pixels: [0.0; 4],
            };
            if let Some(stabilization) = &layer.stabilization {
                (state, buffer) = super::effects::apply_stabilization(
                    self.compute.as_mut().expect("initialized Metal compositor"),
                    buffer,
                    state,
                    stabilization,
                    layer.render_size,
                    effect_submissions,
                )?;
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
                let input_size = (
                    state.parameters.source_width,
                    state.parameters.source_height,
                );
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
                    input_size,
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
                state,
                super::effects::ModifierRequest {
                    operations: &layer.effects,
                    size: layer.render_size,
                    external_masks: super::effects::ExternalMasks {
                        frames: external_masks,
                        size,
                        output_transform: layer.output_transform,
                    },
                    sam2_target: self.sam2_analysis_target.as_ref(),
                    sam2_proxy: &mut self.sam2_proxy_buffer,
                },
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
                if let Some(effect) = stage.effect {
                    (state, buffer) = super::effects::apply_transition(
                        renderer,
                        buffer,
                        state,
                        effect,
                        layer.render_size,
                        effect_submissions,
                    )?;
                }
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
        Ok(Some(layers))
    }

    fn render_raster_morph(
        &mut self,
        morph: &shrimply_preview_render_core::RasterMorph,
        size: (u32, u32),
        external_masks: &ExternalMaskBuffers,
        used_sources: &mut HashSet<u32>,
        used_manim: &mut HashSet<uuid::Uuid>,
        final_submissions: &mut Vec<shrimply_render_metal::Submission>,
    ) -> Result<
        Option<
            Vec<(
                shrimply_render_core::Nv12LayerParams,
                shrimply_render_metal::Buffer,
            )>,
        >,
        String,
    > {
        let state = self.raster_morphs.remove(&morph.key);
        let state = match state {
            None => {
                let mut submissions = Vec::new();
                let source = self.materialize_morph_endpoint(
                    &morph.outgoing,
                    size,
                    external_masks,
                    &mut submissions,
                    used_sources,
                    used_manim,
                )?;
                let target = self.materialize_morph_endpoint(
                    &morph.incoming,
                    size,
                    external_masks,
                    &mut submissions,
                    used_sources,
                    used_manim,
                )?;
                RasterMorphState::Materializing {
                    source,
                    target,
                    submissions,
                }
            }
            Some(state) => state,
        };
        let state = match state {
            RasterMorphState::Materializing {
                source,
                target,
                submissions,
            } => {
                let complete = submissions.iter().try_fold(true, |complete, submission| {
                    submission.completed().map(|ready| complete && ready)
                })?;
                if !complete {
                    self.raster_morphs.insert(
                        morph.key.clone(),
                        RasterMorphState::Materializing {
                            source,
                            target,
                            submissions,
                        },
                    );
                    return Ok(None);
                }
                let source_bytes = source.buffer.copy_bytes();
                let target_bytes = target.buffer.copy_bytes();
                let (sender, receiver) = mpsc::channel();
                let (width, height) = size;
                std::thread::Builder::new()
                    .name("preview-morph-flow".into())
                    .spawn(move || {
                        let result = super::optical_flow::estimate_rgba(
                            &source_bytes,
                            &target_bytes,
                            width,
                            height,
                        );
                        let _ = sender.send(result);
                    })
                    .map_err(|error| {
                        format!("Could not start Morph optical-flow worker: {error}")
                    })?;
                RasterMorphState::Estimating {
                    source,
                    target,
                    receiver,
                }
            }
            state => state,
        };
        let state = match state {
            RasterMorphState::Estimating {
                source,
                target,
                receiver,
            } => match receiver.try_recv() {
                Ok(Ok(flow)) => RasterMorphState::Ready {
                    source,
                    target,
                    flow,
                },
                Ok(Err(error)) => return Err(error),
                Err(TryRecvError::Empty) => {
                    self.raster_morphs.insert(
                        morph.key.clone(),
                        RasterMorphState::Estimating {
                            source,
                            target,
                            receiver,
                        },
                    );
                    return Ok(None);
                }
                Err(TryRecvError::Disconnected) => {
                    return Err("Morph optical-flow worker disconnected".into());
                }
            },
            state => state,
        };
        let RasterMorphState::Ready {
            source,
            target,
            flow,
        } = state
        else {
            unreachable!("pending Morph states returned before rendering")
        };
        let presentation = flow.presentation(morph.progress);
        let source_offsets = presentation
            .source_offsets
            .iter()
            .map(|offset| offset.to_array())
            .collect::<Vec<_>>();
        let target_offsets = presentation
            .target_offsets
            .iter()
            .map(|offset| offset.to_array())
            .collect::<Vec<_>>();
        let renderer = self.compute.as_mut().expect("initialized Metal compositor");
        let (source_output, source_submission) = renderer.mesh_flow(
            source.buffer.clone(),
            size.0,
            size.1,
            presentation.grid_size.x,
            presentation.grid_size.y,
            &source_offsets,
        )?;
        let (target_output, target_submission) = renderer.mesh_flow(
            target.buffer.clone(),
            size.0,
            size.1,
            presentation.grid_size.x,
            presentation.grid_size.y,
            &target_offsets,
        )?;
        final_submissions.push(source_submission);
        final_submissions.push(target_submission);
        let source_parameters = source.parameters;
        let mut target_parameters = target.parameters;
        target_parameters.opacity *= presentation.target_opacity;
        if morph.cacheable {
            self.raster_morphs.insert(
                morph.key.clone(),
                RasterMorphState::Ready {
                    source,
                    target,
                    flow,
                },
            );
        }
        Ok(Some(vec![
            (source_parameters, source_output),
            (target_parameters, target_output),
        ]))
    }

    fn materialize_morph_endpoint(
        &mut self,
        layer: &shrimply_preview_render_core::Layer,
        size: (u32, u32),
        external_masks: &ExternalMaskBuffers,
        submissions: &mut Vec<shrimply_render_metal::Submission>,
        used_sources: &mut HashSet<u32>,
        used_manim: &mut HashSet<uuid::Uuid>,
    ) -> Result<MaterializedMorphEndpoint, String> {
        let Some(mut rendered) = self.render_layers(
            std::slice::from_ref(layer),
            size,
            external_masks,
            submissions,
            used_sources,
            used_manim,
        )?
        else {
            return Err("Nested raster Morph endpoints are unsupported".into());
        };
        if rendered.len() != 1 {
            return Err("Morph endpoint did not produce one composited layer".into());
        }
        let (parameters, buffer) = rendered.pop().expect("one Morph endpoint layer");
        let (source, baked) =
            shrimply_render_core::effects::materialization(parameters, size.0, size.1);
        let (buffer, submission) = self
            .compute
            .as_mut()
            .expect("initialized Metal compositor")
            .composite_buffers(&[(source, buffer)], size.0, size.1, 0)?
            .into_parts();
        submissions.push(submission);
        Ok(MaterializedMorphEndpoint {
            buffer,
            parameters: baked,
        })
    }

    fn layered_image_buffer(
        &mut self,
        plan: &shrimply_visual_core::layered_image::Prepared,
        submissions: &mut Vec<shrimply_render_metal::Submission>,
    ) -> Result<shrimply_render_metal::Buffer, String> {
        let width = plan.document.width.max(1);
        let height = plan.document.height.max(1);
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or("Layered image dimensions overflow Metal address space")?;
        let byte_count = pixel_count
            .checked_mul(size_of::<u32>())
            .ok_or("Layered image byte size overflow")?;
        let pitch = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(size_of::<u32>()))
            .and_then(|pitch| u64::try_from(pitch).ok())
            .ok_or("Layered image row pitch overflow")?;
        let output_count = u64::try_from(pixel_count)
            .map_err(|_| "Layered image pixel count exceeds the shared Slang ABI")?;

        for (index, layer) in plan.document.layers.iter().enumerate() {
            if layer.rgba.len() != byte_count {
                return Err(format!(
                    "Layered image layer {} has {} RGBA bytes, expected {byte_count}",
                    layer.name,
                    layer.rgba.len()
                ));
            }
            let key = (plan.source_key.clone(), index);
            self.used_layered_sources.insert(key.clone());
            if !self.layered_sources.contains_key(&key) {
                let source = self
                    .compute
                    .as_ref()
                    .ok_or("Metal renderer is unavailable for LayeredImage")?
                    .upload(&layer.rgba)?;
                self.layered_sources.insert(key, source);
            }
        }

        // The shared kernel blends into its destination, so the first layer must
        // observe transparent pixels rather than uninitialized Metal storage.
        let output = self
            .compute
            .as_ref()
            .ok_or("Metal renderer is unavailable for LayeredImage")?
            .upload(&vec![0; byte_count])?;
        for layer in &plan.layers {
            let source = self
                .layered_sources
                .get(&(plan.source_key.clone(), layer.source))
                .cloned()
                .ok_or("Shared LayeredImage plan references a missing source layer")?;
            let clipping = layer
                .clipping_base
                .map(|(index, opacity)| {
                    self.layered_sources
                        .get(&(plan.source_key.clone(), index))
                        .cloned()
                        .map(|buffer| (buffer, opacity))
                        .ok_or("Shared LayeredImage plan references a missing clipping base")
                })
                .transpose()?;
            let renderer = self
                .compute
                .as_mut()
                .ok_or("Metal renderer is unavailable for LayeredImage")?;
            let mut arguments = renderer.arguments("composite_layered_image_layer")?;
            arguments
                .set("params.source", &source.address().to_ne_bytes())?
                .set(
                    "params.clipping_base",
                    &clipping
                        .as_ref()
                        .map_or(0, |(buffer, _)| buffer.address())
                        .to_ne_bytes(),
                )?
                .set("params.source_pitch", &pitch.to_ne_bytes())?
                .set(
                    "params.clipping_base_pitch",
                    &clipping.as_ref().map_or(0, |_| pitch).to_ne_bytes(),
                )?
                .set("params.width", &width.to_ne_bytes())?
                .set("params.opacity", &layer.opacity.to_ne_bytes())?
                .set(
                    "params.clipping_base_opacity",
                    &clipping
                        .as_ref()
                        .map_or(1.0, |(_, opacity)| *opacity)
                        .to_ne_bytes(),
                )?
                .set("params.noise_seed", &layer.noise_seed.to_ne_bytes())?
                .set("params.mode", &[layer.mode as u8])?
                .set("output", &output.address().to_ne_bytes())?
                .set("output_count", &output_count.to_ne_bytes())?;
            let mut resources = vec![source, output.clone()];
            resources.extend(clipping.map(|(buffer, _)| buffer));
            submissions
                .push(unsafe { renderer.dispatch(arguments, resources, [pixel_count, 1, 1]) }?);
        }
        Ok(output)
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

pub(super) struct Sam2ProxyFrameSource {
    compositor: Compositor,
    request_id: u64,
    position: Option<Time>,
}

impl Sam2ProxyFrameSource {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            compositor: Compositor::default(),
            request_id: 0,
            position: None,
        })
    }
}

impl shrimply_visual_core::sam2::analysis::ProxyFrameSource for Sam2ProxyFrameSource {
    fn frame(
        &mut self,
        request: shrimply_visual_core::sam2::analysis::ProxyFrameRequest<'_>,
    ) -> Result<shrimply_visual_core::sam2::analysis::ProxyFrameStatus, String> {
        self.compositor
            .set_sam2_analysis_target(request.target.clone());
        if self.position != Some(request.timeline_position) {
            self.position = Some(request.timeline_position);
            self.request_id = self.request_id.wrapping_add(1);
        }
        objc2::rc::autoreleasepool(|_| {
            self.compositor
                .update(request.project, request.timeline_position, self.request_id)?;
            if let Some(rgba) = self.compositor.take_sam2_proxy() {
                let _ = self.compositor.take_presented();
                return Ok(shrimply_visual_core::sam2::analysis::ProxyFrameStatus::Ready(rgba));
            }
            if self.compositor.take_presented().is_some_and(|presented| {
                presented.request_id == self.request_id && !presented.loading
            }) {
                return Err("SAM2 modifier did not capture a Metal proxy frame".to_string());
            }
            Ok(shrimply_visual_core::sam2::analysis::ProxyFrameStatus::Pending)
        })
    }
}
