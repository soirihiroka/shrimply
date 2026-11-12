use super::*;
use crate::items::PreparedItem;

struct MorphEndpoint {
    address: shrimply_project_document::project::ItemAddress,
    layer: Layer,
    audio_pending: bool,
}

impl Scene {
    pub(super) fn layers(
        &mut self,
        project: &Project,
        audio: &FrameAudioAnalysis,
        items: Vec<PreparedItem<'_>>,
    ) -> Result<Vec<Layer>, String> {
        let mut layers = Vec::with_capacity(items.len());
        let mut pending_morph = None;
        for prepared in items {
            let time = prepared.time;
            let content_time = prepared.content_time;
            let capture_host = prepared.capture_branch
                == shrimply_visual_core::modifier_input::CaptureBranch::Host;
            let modifier_input = matches!(
                &self.capture_target,
                Some(CaptureTarget::ModifierInput { .. })
            );
            let clip_transition = prepared.clip_transition;
            let paired_morph = prepared.morph_peer.is_some();
            let audio = prepared.audio.as_ref().unwrap_or(audio);
            let address = prepared.address.clone();
            let item = &prepared.item;
            let children = prepared
                .children
                .map(|children| self.layers(project, audio, children))
                .transpose()?;
            if children.as_ref().is_some_and(Vec::is_empty) {
                continue;
            }

            let item = item.as_ref();
            let motion_blur_item = prepared.motion_blur_source.as_deref().unwrap_or(item);
            shrimply_project_document::project::validate_visual_transitions(item)?;
            let evaluation = VisualEvaluation::for_item_with_audio(project, item, time, audio);
            let transform = shrimply_project_evaluation::resolve_item_transform_with_audio(
                project,
                item,
                time,
                audio,
                &mut self.expressions,
            );
            let motion_blur_transform = if prepared.motion_blur_source.is_some() {
                shrimply_project_evaluation::resolve_item_transform_with_audio(
                    project,
                    motion_blur_item,
                    time,
                    audio,
                    &mut self.expressions,
                )
            } else {
                transform
            };
            let generated_transition =
                shrimply_visual_core::generated::transition(item, time, false);
            let mut motion_blur = shrimply_visual_core::motion_blur::sample_transforms(
                shrimply_visual_core::motion_blur::Request {
                    project,
                    item: motion_blur_item,
                    position: time,
                    current: motion_blur_transform.composed(),
                    content_accurate: self.requested_accuracy.content_accurate(),
                },
                &mut self.expressions,
                |position| {
                    let audio = self
                        .audio_sampler
                        .sample(project, position, self.audio_revision);
                    self.sampled_audio.push(audio.clone());
                    audio
                },
            )
            .and_then(|samples| {
                shrimply_math_geometry::relative_motion_transforms(
                    motion_blur_transform.composed(),
                    samples,
                )
            });
            let vector_source = matches!(
                item.content,
                VideoItemContent::Shape(_)
                    | VideoItemContent::Text(_)
                    | VideoItemContent::Paint(_)
                    | VideoItemContent::Svg
            ) || shrimply_visual_core::vectorize::Plan::modifier_for_item(item)
                .is_some();
            let mut vector = vector_source
                .then(|| {
                    self.vector(generated::VectorRequest {
                        project,
                        address: &address,
                        item,
                        position: content_time,
                        scope_positions: &prepared.scope_positions,
                        require_complete_assets: modifier_input,
                        evaluation: evaluation.clone(),
                        native: project.canvas_size,
                        transform: transform.composed(),
                        transition: generated_transition,
                        svg: match self.media.frame(&prepared.address, media::Plane::Content) {
                            Some(media::Frame::Svg(svg)) => Some(svg.clone()),
                            _ => None,
                        },
                    })
                })
                .transpose()?;
            let render_canvas = vector
                .as_ref()
                .map_or(project.canvas_size, |vector| vector.frame.render_size);
            let sample_method = vector.as_ref().map_or_else(
                || item.sample_method.value_at(evaluation.local_time()),
                |vector| vector.sample_method,
            );
            let sample_method = shrimply_visual_core::generated::sampling(
                sample_method,
                self.requested_accuracy.content_accurate(),
            );
            // The existing background renderer generates a canvas-sized texture
            // and bakes the source transform before item effects and transitions.
            let mut raster_transform = if capture_host
                || vector.is_some()
                || matches!(
                    item.content,
                    VideoItemContent::Background(_)
                        | VideoItemContent::Sanim(_)
                        | VideoItemContent::Gaussian(_)
                        | VideoItemContent::Obj(_)
                ) {
                shrimply_render_core::math::Mat3::IDENTITY
            } else {
                transform.matrix()
            };
            let mut opacity = resolve_scalar(
                &item.compositing.opacity,
                &evaluation,
                &mut self.expressions,
            )
            .clamp(0.0, 1.0);
            let mut transitions = Vec::new();
            if let Some(vector) = vector.as_mut().filter(|vector| vector.is_vector)
                && let Some(samples) = motion_blur.take()
            {
                vector.frame.operations.push(
                    shrimply_visual_core::generated::VectorOperation::MotionBlur(samples.into()),
                );
            }
            if let Some((_, transition, visible, _)) =
                shrimply_visual_core::transition::active_visual_transition(item, time)
            {
                use shrimply_project_document::project::VisualTransitionKind;
                let supported = matches!(
                    transition.kind,
                    VisualTransitionKind::Fade
                        | VisualTransitionKind::Slide
                        | VisualTransitionKind::SlideFade
                        | VisualTransitionKind::Zoom
                        | VisualTransitionKind::Spin
                        | VisualTransitionKind::Wipe
                        | VisualTransitionKind::Iris
                        | VisualTransitionKind::ClockWipe
                        | VisualTransitionKind::Dissolve
                        | VisualTransitionKind::TriangularFold
                        | VisualTransitionKind::StreakWipe
                        | VisualTransitionKind::Blur
                        | VisualTransitionKind::Pixelate
                        | VisualTransitionKind::Origami
                ) || vector.is_some() && generated_transition.is_some();
                if !supported {
                    return Err(format!(
                        "Clip {} has a transition effect not yet connected to Metal",
                        item.id
                    ));
                }
                let spatial = shrimply_visual_core::transition::spatial(
                    transition,
                    visible,
                    transform.position,
                );
                let effect = shrimply_visual_core::transition::raster_plan(
                    transition,
                    visible,
                    transform.position,
                );
                opacity *= spatial.opacity;
                let stage_transform =
                    if let Some(vector) = vector.as_mut().filter(|vector| vector.is_vector) {
                        vector.frame.operations.push(
                            shrimply_visual_core::generated::VectorOperation::Transform(
                                shrimply_math_geometry::ComposedTransform2D {
                                    matrix: spatial.transform,
                                },
                            ),
                        );
                        shrimply_render_core::math::Mat3::IDENTITY
                    } else {
                        spatial.transform
                    };
                transitions.push(TransitionStage {
                    transform: stage_transform,
                    effect,
                });
            }
            if let Some(transition) = clip_transition {
                use shrimply_visual_core::clip_transition::{self, ClipTransitionRole};
                if transition.definition.kind
                    != shrimply_project_document::project::VisualClipTransitionKind::Morph
                {
                    let spatial = clip_transition::spatial(transition, render_canvas);
                    opacity *= spatial.opacity;
                    let stage_transform =
                        if let Some(vector) = vector.as_mut().filter(|vector| vector.is_vector) {
                            vector.frame.operations.push(
                                shrimply_visual_core::generated::VectorOperation::Transform(
                                    shrimply_math_geometry::ComposedTransform2D {
                                        matrix: spatial.transform,
                                    },
                                ),
                            );
                            shrimply_render_core::math::Mat3::IDENTITY
                        } else {
                            spatial.transform
                        };
                    let effect = (transition.role == ClipTransitionRole::Incoming)
                        .then(|| {
                            shrimply_visual_core::transition::clip_mask(
                                &transition.definition,
                                transition.progress,
                            )
                        })
                        .flatten()
                        .map(shrimply_visual_core::transition::RasterTransition::Pixel);
                    transitions.push(TransitionStage {
                        transform: stage_transform,
                        effect,
                    });
                }
            }
            let morph_scene = vector
                .as_ref()
                .and_then(|vector| vector.morph_scene(project.canvas_size));
            let vector_effects = vector
                .as_mut()
                .map(|vector| std::mem::take(&mut vector.effects));
            let mut decoded_presentation_time = None;
            let mut source_scale = glam::Vec2::ONE;
            let (source, source_width, source_height) = match &item.content {
                VideoItemContent::Shape(_)
                | VideoItemContent::Text(_)
                | VideoItemContent::Paint(_)
                | VideoItemContent::Svg => (
                    Source::Generated(Box::new(vector.expect("prepared generated source").frame)),
                    render_canvas.width,
                    render_canvas.height,
                ),
                VideoItemContent::FoldedSequence(_) => (
                    Source::Group(children.expect("folded sequence children prepared")),
                    project.canvas_size.width,
                    project.canvas_size.height,
                ),

                VideoItemContent::Background(background) => {
                    let background = shrimply_visual_core::background::resolve(
                        background,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let local_time =
                        shrimply_project_document::project::generated_item_time(item, content_time)
                            .expect("active generated source time");
                    let uniforms = shrimply_visual_core::background::uniforms(
                        project.canvas_size.width,
                        project.canvas_size.height,
                        local_time,
                        &background,
                    );
                    (
                        Source::Background(Box::new(uniforms)),
                        project.canvas_size.width,
                        project.canvas_size.height,
                    )
                }
                VideoItemContent::Sanim(_) => {
                    self.sanim_active.insert(item.id);
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        self.sanim.entry(item.id)
                    {
                        let mut source = shrimply_visual_core::sanim::Source::with_viewport(
                            item,
                            shrimply_visual_core::sanim::Viewport {
                                width: project.canvas_size.width,
                                height: project.canvas_size.height,
                                rate: project.fps,
                                ..Default::default()
                            },
                        )?;
                        source.set_waker(self.source_waker.clone());
                        entry.insert(source);
                    }
                    let source = self
                        .sanim
                        .get_mut(&item.id)
                        .expect("initialized Sanim source");
                    let status = source.poll(
                        item,
                        time,
                        project.canvas_size,
                        project.fps,
                        self.strict_sources,
                    )?;
                    if let Some(error) = source.take_error() {
                        self.sanim_errors.push(error);
                    }
                    let frame = match status {
                        shrimply_visual_core::sanim::Status::Empty => continue,
                        shrimply_visual_core::sanim::Status::Loading(frame) => {
                            self.sanim_loading = true;
                            frame
                        }
                        shrimply_visual_core::sanim::Status::Failed(frame)
                        | shrimply_visual_core::sanim::Status::Ready(frame) => frame,
                    };
                    let width = frame.width;
                    let height = frame.height;
                    let operations = if capture_host {
                        Vec::new()
                    } else {
                        vec![shrimply_visual_core::generated::VectorOperation::Transform(
                            transform.composed(),
                        )]
                    };
                    let generated = shrimply_visual_core::generated::GeneratedFrame {
                        visual: Box::new(shrimply_visual_core::sanim::RecordedVisual(frame)),
                        evaluation: evaluation.clone(),
                        operations,
                        render_size: render_canvas,
                        canvas_size: project.canvas_size,
                        drawing_strategy: item.skia_drawing_strategy,
                    };
                    (Source::Generated(Box::new(generated)), width, height)
                }

                VideoItemContent::Manim(_) => {
                    let fps = shrimply_manim_wgpu::effective_fps(item, project.fps);
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        self.manim.entry(item.id)
                    {
                        entry.insert(shrimply_manim_wgpu::Source::new(
                            item,
                            project.canvas_size,
                            fps,
                        )?);
                    }
                    let source = self
                        .manim
                        .get_mut(&item.id)
                        .expect("Manim source was initialized");
                    let outcome = source.poll(item, project.canvas_size, fps, content_time);
                    let identity = source.identity();
                    self.manim_updates.extend(source.take_updates());
                    match outcome {
                        Err(error) => {
                            self.manim_updates.push(identity.error(Some(error.clone())));
                            return Err(error);
                        }
                        Ok(Ok(frame)) => {
                            let source_width = frame.prepared.scene().width.max(1);
                            let source_height = frame.prepared.scene().height.max(1);
                            (
                                Source::Manim(ManimFrame {
                                    item_id: item.id,
                                    source: identity,
                                    prepared: frame.prepared,
                                    frame_index: frame.frame_index,
                                }),
                                source_width,
                                source_height,
                            )
                        }
                        Ok(Err(shrimply_manim_wgpu::SourceStatus::Loading {
                            progress,
                            changed,
                        })) => {
                            self.manim_loading = true;
                            if changed && progress.is_none() {
                                self.manim_updates.push(identity.error(None));
                            }
                            if !changed {
                                self.manim_pending = true;
                                return Ok(Vec::new());
                            }
                            let pixels = shrimply_manim_wgpu::loading_pixels(
                                project.canvas_size.width,
                                project.canvas_size.height,
                                progress,
                            )?;
                            let info = skia_safe::ImageInfo::new(
                                (
                                    project.canvas_size.width as i32,
                                    project.canvas_size.height as i32,
                                ),
                                skia_safe::ColorType::RGBA8888,
                                skia_safe::AlphaType::Unpremul,
                                None,
                            );
                            let image = skia_safe::images::raster_from_data(
                                &info,
                                skia_safe::Data::new_copy(&pixels),
                                project.canvas_size.width as usize * size_of::<u32>(),
                            )
                            .ok_or("Could not create the Manim loading frame")?;
                            (
                                Source::Image(image),
                                project.canvas_size.width,
                                project.canvas_size.height,
                            )
                        }
                        Ok(Err(shrimply_manim_wgpu::SourceStatus::NeedsParameters)) => {
                            self.manim_loading = true;
                            self.manim_pending = true;
                            return Ok(Vec::new());
                        }
                    }
                }
                VideoItemContent::Blender(_) => {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        self.blender.entry(item.id)
                    {
                        entry.insert(shrimply_visual_core::blender::Source::new(
                            item,
                            project.canvas_size,
                        )?);
                    }
                    let status = self
                        .blender
                        .get_mut(&item.id)
                        .expect("Blender source was initialized")
                        .poll(
                            item,
                            project.canvas_size,
                            content_time,
                            self.requested_accuracy.content_accurate(),
                        )?;
                    let image = match status {
                        shrimply_visual_core::blender::Status::Empty => continue,
                        shrimply_visual_core::blender::Status::Loading => {
                            self.blender_loading = true;
                            if self
                                .blender_loading_image
                                .as_ref()
                                .is_none_or(|(size, _)| *size != project.canvas_size)
                            {
                                let pixels = shrimply_loading_screen_skia::render(
                                    project.canvas_size.width,
                                    project.canvas_size.height,
                                    shrimply_i18n::text("Starting Blender…").as_ref(),
                                    shrimply_math_color::Color::new(104, 51, 12, 255),
                                    shrimply_math_color::Color::new(255, 174, 85, 255),
                                )?;
                                let info = skia_safe::ImageInfo::new(
                                    (
                                        project.canvas_size.width as i32,
                                        project.canvas_size.height as i32,
                                    ),
                                    skia_safe::ColorType::RGBA8888,
                                    skia_safe::AlphaType::Unpremul,
                                    None,
                                );
                                let image = skia_safe::images::raster_from_data(
                                    &info,
                                    skia_safe::Data::new_copy(&pixels),
                                    project.canvas_size.width as usize * size_of::<u32>(),
                                )
                                .ok_or("Could not create Blender loading frame")?;
                                self.blender_loading_image = Some((project.canvas_size, image));
                            }
                            self.blender_loading_image
                                .as_ref()
                                .expect("Blender loading frame was prepared")
                                .1
                                .clone()
                        }
                        shrimply_visual_core::blender::Status::Ready(frame) => {
                            source_scale = frame.display_scale;
                            let replace = self
                                .blender_images
                                .get(&item.id)
                                .is_none_or(|(cached, _)| !std::sync::Arc::ptr_eq(cached, &frame));
                            if replace {
                                let info = skia_safe::ImageInfo::new(
                                    (frame.width as i32, frame.height as i32),
                                    skia_safe::ColorType::RGBA8888,
                                    skia_safe::AlphaType::Unpremul,
                                    None,
                                );
                                let image = skia_safe::images::raster_from_data(
                                    &info,
                                    skia_safe::Data::new_copy(&frame.pixels),
                                    frame.width as usize * size_of::<u32>(),
                                )
                                .ok_or("Could not create Blender preview frame")?;
                                self.blender_images.insert(item.id, (frame.clone(), image));
                            }
                            self.blender_images
                                .get(&item.id)
                                .expect("Blender preview frame was prepared")
                                .1
                                .clone()
                        }
                    };
                    let width = image.width() as u32;
                    let height = image.height() as u32;
                    (Source::Image(image), width, height)
                }
                VideoItemContent::LayeredImage(layered) => {
                    let Some(media::Frame::LayeredImage(frame)) =
                        self.media.frame(&prepared.address, media::Plane::Content)
                    else {
                        return Err("Layered image document was not prepared".into());
                    };
                    let width = frame.document.width.max(1);
                    let height = frame.document.height.max(1);
                    let plan = shrimply_visual_core::layered_image::prepare(
                        frame.document.clone(),
                        frame.source_key.clone(),
                        layered,
                        &evaluation,
                        &mut self.expressions,
                    );
                    (Source::LayeredImage(Box::new(plan)), width, height)
                }
                VideoItemContent::Gaussian(_) => {
                    if self
                        .gaussians
                        .get(&item.id)
                        .is_none_or(|source| !source.matches(item))
                    {
                        self.gaussians
                            .insert(item.id, shrimply_visual_core::gaussian::Source::new(item)?);
                    }
                    let plan = self
                        .gaussians
                        .get(&item.id)
                        .expect("Gaussian source was initialized")
                        .prepare(
                            shrimply_visual_core::gaussian::Request {
                                project,
                                item,
                                position: content_time,
                                audio,
                                canvas: render_canvas,
                                sequence_path: prepared.address.sequence_path(),
                                track_id: prepared.address.track_id(),
                            },
                            &mut self.expressions,
                        )?;
                    let width = plan.width;
                    let height = plan.height;
                    (Source::Gaussian(Box::new(plan)), width, height)
                }
                VideoItemContent::Obj(_) => {
                    let plan = self.objs.entry(item.id).or_default().prepare(
                        shrimply_visual_core::obj::Request {
                            project,
                            item,
                            position: content_time,
                            audio_analysis: audio,
                            render_canvas,
                            content_accurate: self.requested_accuracy.content_accurate(),
                            sequence_path: prepared.address.sequence_path(),
                            track_id: prepared.address.track_id(),
                        },
                    )?;
                    let width = plan.width;
                    let height = plan.height;
                    (Source::Obj(Box::new(plan)), width, height)
                }
                _ => {
                    let Some(media::Frame::Image(image)) =
                        self.media.frame(&prepared.address, media::Plane::Content)
                    else {
                        return Err("This visual source is not yet connected to Metal".into());
                    };
                    let width = image.image.width() as u32;
                    let height = image.image.height() as u32;
                    decoded_presentation_time = image.presentation_time;
                    (image.image.source(), width, height)
                }
            };
            raster_transform *= glam::Mat3::from_scale(source_scale);
            let inverse = shrimply_render_core::math::inverse_affine(raster_transform)
                .unwrap_or(shrimply_render_core::math::Mat3::IDENTITY);
            let stabilization = if item.stabilize_video {
                let source_position = decoded_presentation_time
                    .ok_or("Stabilized video frame has no presentation timestamp")?;
                match shrimply_visual_core::stabilization::frame_state(item, source_position) {
                    shrimply_visual_core::stabilization::FrameState::Disabled => None,
                    shrimply_visual_core::stabilization::FrameState::Pending => {
                        self.stabilization_pending = true;
                        return Ok(Vec::new());
                    }
                    shrimply_visual_core::stabilization::FrameState::Ready(warp) => Some(warp),
                    shrimply_visual_core::stabilization::FrameState::Failed(error) => {
                        return Err(error);
                    }
                }
            } else {
                None
            };
            let effects = if let Some(effects) = vector_effects {
                effects
            } else {
                shrimply_visual_core::raster_modifiers::after_source(
                    shrimply_visual_core::raster_modifiers::ChainRequest {
                        project,
                        address: &address,
                        item,
                        position: time,
                        scope_positions: &prepared.scope_positions,
                        require_complete_assets: modifier_input,
                    },
                    &evaluation,
                    &mut self.expressions,
                    self.requested_accuracy.content_accurate(),
                )?
            };
            let parameters = Nv12LayerParams {
                crop: [0.0; 4],
                padding: [0.0; 4],
                y_plane: std::ptr::null(),
                uv_plane: std::ptr::null(),
                rgba: std::ptr::null(),
                y_pitch: 0,
                uv_pitch: 0,
                rgba_pitch: source_width as usize * size_of::<u32>(),
                source_width,
                source_height,
                canvas_width: project.canvas_size.width,
                inverse,
                motion_transform_offset: 0,
                motion_transform_count: 0,
                motion_sample_count: 0,
                opacity,
                blend_mode: item
                    .compositing
                    .blend_mode
                    .value_at(evaluation.local_time()),
                sample_method,
                address_mode: TextureAddressMode::Transparent,
                kind: LayerKind::Rgba,
                _padding_0: [0; 4],
            };
            let layer = Layer {
                stabilization,
                video_mask: prepared
                    .video_mask
                    .map(|size| {
                        let Some(media::Frame::Image(image)) =
                            self.media.frame(&address, media::Plane::Alpha)
                        else {
                            return Err("Alpha video stream did not produce a raster image".into());
                        };
                        Ok::<_, String>(VideoMask {
                            image: image.image.clone(),
                            size: (size.width, size.height),
                            sampling: shrimply_visual_core::generated::sampling(
                                item.sample_method.value_at(evaluation.local_time()),
                                self.requested_accuracy.content_accurate(),
                            ),
                        })
                    })
                    .transpose()?,
                alpha_mask: item
                    .compositing
                    .alpha_mask
                    .as_ref()
                    .filter(|mask| mask.enabled)
                    .map(|mask| {
                        shrimply_visual_core::alpha_mask::resolve(
                            mask,
                            &evaluation,
                            &mut self.expressions,
                        )
                    }),
                transform: raster_transform,
                motion_blur,
                parameters,
                source,
                transitions,
                effects,
                render_size: (render_canvas.width, render_canvas.height),
                output_transform: shrimply_render_core::math::Mat3::from_scale(glam::Vec2::new(
                    project.canvas_size.width as f32 / render_canvas.width as f32,
                    project.canvas_size.height as f32 / render_canvas.height as f32,
                )),
                morph_scene,
            };
            if let Some(transition) = clip_transition.filter(|transition| {
                paired_morph
                    && transition.definition.kind
                        == shrimply_project_document::project::VisualClipTransitionKind::Morph
            }) {
                use shrimply_visual_core::clip_transition::ClipTransitionRole;
                let audio_pending = audio.pending();
                match transition.role {
                    ClipTransitionRole::Outgoing => {
                        if pending_morph.is_some() {
                            return Err("Overlapping outgoing Morph transitions are invalid".into());
                        }
                        pending_morph = Some((
                            transition.progress,
                            MorphEndpoint {
                                address,
                                layer,
                                audio_pending,
                            },
                        ));
                    }
                    ClipTransitionRole::Incoming => {
                        let (progress, outgoing) = pending_morph
                            .take()
                            .ok_or("Morph transition is missing its outgoing clip")?;
                        if outgoing.address.track_id() != address.track_id()
                            || progress != transition.progress
                        {
                            return Err("Morph transition endpoints do not match".into());
                        }
                        layers.push(self.morph_layer(
                            project,
                            outgoing,
                            MorphEndpoint {
                                address,
                                layer,
                                audio_pending,
                            },
                            progress,
                        )?);
                    }
                }
                continue;
            }
            if let Some((_, outgoing)) = pending_morph.take() {
                layers.push(outgoing.layer);
            }
            layers.push(layer);
            if let Some((color, opacity)) =
                clip_transition.and_then(shrimply_visual_core::clip_transition::color_layer)
            {
                let background = shrimply_visual_core::background::solid(
                    project.canvas_size.width,
                    project.canvas_size.height,
                    color,
                );
                layers.push(Layer {
                    stabilization: None,
                    video_mask: None,
                    alpha_mask: None,
                    transform: shrimply_render_core::math::Mat3::IDENTITY,
                    motion_blur: None,
                    parameters: Nv12LayerParams {
                        source_width: project.canvas_size.width,
                        source_height: project.canvas_size.height,
                        rgba_pitch: project.canvas_size.width as usize * size_of::<u32>(),
                        inverse: shrimply_render_core::math::Mat3::IDENTITY,
                        opacity,
                        blend_mode: shrimply_render_core::LayerBlendMode::Normal,
                        sample_method: shrimply_render_core::VideoSampleMethod::Nearest,
                        ..parameters
                    },
                    source: Source::Background(Box::new(background)),
                    effects: Vec::new(),
                    transitions: Vec::new(),
                    render_size: (project.canvas_size.width, project.canvas_size.height),
                    output_transform: shrimply_render_core::math::Mat3::IDENTITY,
                    morph_scene: None,
                });
            }
        }
        if let Some((_, outgoing)) = pending_morph {
            layers.push(outgoing.layer);
        }
        Ok(layers)
    }

    fn morph_layer(
        &mut self,
        project: &Project,
        outgoing: MorphEndpoint,
        incoming: MorphEndpoint,
        progress: f32,
    ) -> Result<Layer, String> {
        let cacheable = !outgoing.audio_pending
            && !incoming.audio_pending
            && !self.manim_loading
            && !self.sanim_loading;
        let outgoing_address = outgoing.address;
        let incoming_address = incoming.address;
        let outgoing = outgoing.layer;
        let incoming = incoming.layer;
        let vector_eligible = outgoing.effects.is_empty()
            && incoming.effects.is_empty()
            && outgoing.alpha_mask.is_none()
            && incoming.alpha_mask.is_none()
            && outgoing.video_mask.is_none()
            && incoming.video_mask.is_none()
            && outgoing
                .transitions
                .iter()
                .all(|stage| stage.effect.is_none())
            && incoming
                .transitions
                .iter()
                .all(|stage| stage.effect.is_none())
            && outgoing.morph_scene.is_some()
            && incoming.morph_scene.is_some()
            && matches!(outgoing.source, Source::Generated(_))
            && matches!(incoming.source, Source::Generated(_));
        let key = MorphCacheKey {
            sequence_path: outgoing_address.sequence_path().to_vec(),
            track_id: outgoing_address.track_id(),
            outgoing_id: outgoing_address.item_id(),
            incoming_id: incoming_address.item_id(),
            width: project.canvas_size.width,
            height: project.canvas_size.height,
            content_revision: self.media.revision(),
            cacheable,
        };
        if vector_eligible {
            return self.vector_morph_layer(project, key, outgoing, incoming, progress, cacheable);
        }
        Ok(Layer {
            stabilization: None,
            video_mask: None,
            alpha_mask: None,
            parameters: Nv12LayerParams {
                source_width: project.canvas_size.width,
                source_height: project.canvas_size.height,
                rgba_pitch: project.canvas_size.width as usize * size_of::<u32>(),
                inverse: shrimply_render_core::math::Mat3::IDENTITY,
                opacity: 1.0,
                blend_mode: shrimply_render_core::LayerBlendMode::Normal,
                sample_method: shrimply_render_core::VideoSampleMethod::Bilinear,
                ..outgoing.parameters
            },
            transform: shrimply_render_core::math::Mat3::IDENTITY,
            source: Source::RasterMorph(Box::new(RasterMorph {
                key,
                outgoing: Box::new(outgoing),
                incoming: Box::new(incoming),
                progress,
                cacheable,
            })),
            transitions: Vec::new(),
            effects: Vec::new(),
            render_size: (project.canvas_size.width, project.canvas_size.height),
            output_transform: shrimply_render_core::math::Mat3::IDENTITY,
            motion_blur: None,
            morph_scene: None,
        })
    }

    fn vector_morph_layer(
        &mut self,
        project: &Project,
        key: MorphCacheKey,
        outgoing: Layer,
        incoming: Layer,
        progress: f32,
        cacheable: bool,
    ) -> Result<Layer, String> {
        if !outgoing.effects.is_empty()
            || !incoming.effects.is_empty()
            || outgoing.alpha_mask.is_some()
            || incoming.alpha_mask.is_some()
            || outgoing.video_mask.is_some()
            || incoming.video_mask.is_some()
            || outgoing
                .transitions
                .iter()
                .any(|stage| stage.effect.is_some())
            || incoming
                .transitions
                .iter()
                .any(|stage| stage.effect.is_some())
        {
            return Err("Morph endpoints must remain vector operations".into());
        }
        let source = outgoing
            .morph_scene
            .clone()
            .ok_or("Morph source requires a vector-only generated clip")?;
        let target = incoming
            .morph_scene
            .clone()
            .ok_or("Morph target requires a vector-only generated clip")?;
        let morph = if let Some(morph) = self.morphs.get(&key) {
            morph.clone()
        } else {
            let morph = std::rc::Rc::new(
                shrimply_visual_core::vector_morph::PreparedVectorMorph::new(source, target),
            );
            if cacheable {
                self.morphs.insert(key, morph.clone());
            }
            morph
        };
        let presentation = morph.presentation(
            progress,
            outgoing.parameters.opacity,
            incoming.parameters.opacity,
        );
        let selected = if presentation.target_side {
            &incoming
        } else {
            &outgoing
        };
        let mut parameters = selected.parameters;
        parameters.source_width = project.canvas_size.width;
        parameters.source_height = project.canvas_size.height;
        parameters.rgba_pitch = project.canvas_size.width as usize * size_of::<u32>();
        parameters.inverse = shrimply_render_core::math::Mat3::IDENTITY;
        parameters.opacity = presentation.opacity;
        let drawing_strategy = match &selected.source {
            Source::Generated(frame) => frame.drawing_strategy,
            _ => return Err("Morph endpoints must remain generated vectors".into()),
        };
        Ok(Layer {
            stabilization: None,
            video_mask: None,
            alpha_mask: None,
            parameters,
            transform: shrimply_render_core::math::Mat3::IDENTITY,
            source: Source::Generated(Box::new(shrimply_visual_core::generated::GeneratedFrame {
                visual: Box::new(morph.frame(progress)),
                evaluation: presentation.scene.evaluation.clone(),
                operations: Vec::new(),
                render_size: project.canvas_size,
                canvas_size: project.canvas_size,
                drawing_strategy,
            })),
            transitions: Vec::new(),
            effects: Vec::new(),
            render_size: (project.canvas_size.width, project.canvas_size.height),
            output_transform: shrimply_render_core::math::Mat3::IDENTITY,
            motion_blur: None,
            morph_scene: None,
        })
    }
}
