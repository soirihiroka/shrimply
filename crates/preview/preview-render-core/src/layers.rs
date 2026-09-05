use super::*;
use crate::items::PreparedItem;

struct MorphEndpoint {
    address: shrimply_project::project::ItemAddress,
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
            item.modifier_output_state()?;
            shrimply_project::project::validate_visual_transitions(item)?;
            let evaluation = VisualEvaluation::for_item_with_audio(project, item, time, audio);
            if item.stabilize_video {
                return Err(format!(
                    "Clip {} requires an effect pipeline that is not yet connected to Metal",
                    item.id
                ));
            }
            let transform = shrimply_evaluation::resolve_item_transform_with_audio(
                project,
                item,
                time,
                audio,
                &mut self.expressions,
            );
            let generated_transition =
                shrimply_video_core::generated::transition(item, time, false);
            let mut motion_blur = shrimply_video_core::motion_blur::sample_transforms(
                shrimply_video_core::motion_blur::Request {
                    project,
                    item,
                    position: time,
                    current: transform.composed(),
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
                shrimply_math_geometry::relative_motion_transforms(transform.composed(), samples)
            });
            let mut vector = matches!(
                item.content,
                VideoItemContent::Shape(_)
                    | VideoItemContent::Text(_)
                    | VideoItemContent::Paint(_)
                    | VideoItemContent::Svg
            )
            .then(|| {
                self.vector(
                    item,
                    evaluation.clone(),
                    project.canvas_size,
                    transform.composed(),
                    generated_transition,
                    match self.media.frame(&prepared.address, media::Plane::Content) {
                        Some(media::Frame::Svg(svg)) => Some(svg.clone()),
                        _ => None,
                    },
                )
            })
            .transpose()?;
            let render_canvas = vector
                .as_ref()
                .map_or(project.canvas_size, |vector| vector.frame.render_size);
            let sample_method = vector.as_ref().map_or_else(
                || item.sample_method.value_at(evaluation.local_time()),
                |vector| vector.sample_method,
            );
            let sample_method = shrimply_video_core::generated::sampling(
                sample_method,
                self.requested_accuracy.content_accurate(),
            );
            // The existing background renderer generates a canvas-sized texture
            // and bakes the source transform before item effects and transitions.
            let raster_transform =
                if vector.is_some() || matches!(item.content, VideoItemContent::Background(_)) {
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
            let inverse = shrimply_render_core::math::inverse_affine(raster_transform)
                .unwrap_or(shrimply_render_core::math::Mat3::IDENTITY);
            let mut transitions = Vec::new();
            if let Some(vector) = vector.as_mut().filter(|vector| vector.is_vector)
                && let Some(samples) = motion_blur.take()
            {
                vector.frame.operations.push(
                    shrimply_video_core::generated::VectorOperation::MotionBlur(samples.into()),
                );
            }
            if let Some((_, transition, visible, _)) =
                shrimply_video_core::transition::active_visual_transition(item, time)
            {
                use shrimply_project::project::VisualTransitionKind;
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
                ) || vector.is_some() && generated_transition.is_some();
                if !supported {
                    return Err(format!(
                        "Clip {} has a transition effect not yet connected to Metal",
                        item.id
                    ));
                }
                let spatial = shrimply_video_core::transition::spatial(
                    transition,
                    visible,
                    transform.position,
                );
                let effect = shrimply_video_core::transition::raster(
                    transition,
                    visible,
                    transform.position,
                );
                opacity *= spatial.opacity;
                let stage_transform =
                    if let Some(vector) = vector.as_mut().filter(|vector| vector.is_vector) {
                        vector.frame.operations.push(
                            shrimply_video_core::generated::VectorOperation::Transform(
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
                use shrimply_video_core::clip_transition::{self, ClipTransitionRole};
                if transition.definition.kind
                    != shrimply_project::project::VisualClipTransitionKind::Morph
                {
                    let spatial = clip_transition::spatial(transition, render_canvas);
                    opacity *= spatial.opacity;
                    let stage_transform =
                        if let Some(vector) = vector.as_mut().filter(|vector| vector.is_vector) {
                            vector.frame.operations.push(
                                shrimply_video_core::generated::VectorOperation::Transform(
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
                            shrimply_video_core::transition::clip_mask(
                                &transition.definition,
                                transition.progress,
                            )
                        })
                        .flatten();
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
                    let background = shrimply_video_core::background::resolve(
                        background,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let local_time = shrimply_project::project::generated_item_time(item, time)
                        .expect("active generated source time");
                    let uniforms = shrimply_video_core::background::uniforms(
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
                VideoItemContent::Manim(_) => {
                    let fps =
                        if item.playback_fps == shrimply_project::project::native_playback_fps() {
                            project.fps
                        } else {
                            item.playback_fps
                        };
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
                    let outcome = source.poll(item, project.canvas_size, fps, time);
                    let source_revision = source.source_revision();
                    let scene = source.scene().to_string();
                    let input_parameters = source.input_parameters().clone();
                    let duration = source.take_duration();
                    let parameters = source.take_parameters();
                    if let Some(duration) = duration {
                        self.manim_updates
                            .push(shrimply_state::manim_status::Update::Duration {
                                item_id: item.id,
                                source_revision,
                                scene: scene.clone(),
                                input_parameters: input_parameters.clone(),
                                duration,
                            });
                    }
                    if let Some((parameters, render_is_current)) = parameters {
                        self.manim_updates
                            .push(shrimply_state::manim_status::Update::Parameters {
                                item_id: item.id,
                                source_revision,
                                scene: scene.clone(),
                                input_parameters: input_parameters.clone(),
                                parameters,
                                render_is_current,
                            });
                    }
                    match outcome {
                        Err(error) => {
                            self.manim_updates
                                .push(shrimply_state::manim_status::Update::Error {
                                    item_id: item.id,
                                    source_revision,
                                    scene: scene.clone(),
                                    input_parameters: input_parameters.clone(),
                                    error: Some(error.clone()),
                                });
                            return Err(error);
                        }
                        Ok(Ok(frame)) => {
                            let source_width = frame.prepared.scene().width.max(1);
                            let source_height = frame.prepared.scene().height.max(1);
                            (
                                Source::Manim(ManimFrame {
                                    item_id: item.id,
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
                                self.manim_updates.push(
                                    shrimply_state::manim_status::Update::Error {
                                        item_id: item.id,
                                        source_revision,
                                        scene: scene.clone(),
                                        input_parameters: input_parameters.clone(),
                                        error: None,
                                    },
                                );
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
                _ => {
                    let Some(media::Frame::Image(image)) =
                        self.media.frame(&prepared.address, media::Plane::Content)
                    else {
                        return Err("This visual source is not yet connected to Metal".into());
                    };
                    let width = image.width() as u32;
                    let height = image.height() as u32;
                    (Source::Image(image.clone()), width, height)
                }
            };
            let effects = if let Some(effects) = vector_effects {
                effects
            } else {
                item.modifiers
                    .iter()
                    .filter(|modifier| modifier.enabled)
                    .map(|modifier| {
                        let effect = shrimply_video_core::raster_modifiers::modifier(
                            modifier,
                            &evaluation,
                            &mut self.expressions,
                            self.requested_accuracy.content_accurate(),
                        )?;
                        effect.ok_or_else(|| {
                            format!(
                                "Clip {} has a modifier or mask that is not yet connected to Metal",
                                item.id
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?
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
                video_mask: prepared
                    .video_mask
                    .map(|size| {
                        let Some(media::Frame::Image(image)) =
                            self.media.frame(&address, media::Plane::Alpha)
                        else {
                            return Err("Alpha video stream did not produce a raster image".into());
                        };
                        Ok::<_, String>(VideoMask {
                            image: image.clone(),
                            size: (size.width, size.height),
                            sampling: shrimply_video_core::generated::sampling(
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
                        shrimply_video_core::alpha_mask::resolve(
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
                        == shrimply_project::project::VisualClipTransitionKind::Morph
            }) {
                use shrimply_video_core::clip_transition::ClipTransitionRole;
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
                        layers.push(self.vector_morph_layer(
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
                clip_transition.and_then(shrimply_video_core::clip_transition::color_layer)
            {
                let background = shrimply_video_core::background::solid(
                    project.canvas_size.width,
                    project.canvas_size.height,
                    color,
                );
                layers.push(Layer {
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

    fn vector_morph_layer(
        &mut self,
        project: &Project,
        outgoing: MorphEndpoint,
        incoming: MorphEndpoint,
        progress: f32,
    ) -> Result<Layer, String> {
        let cacheable = !outgoing.audio_pending && !incoming.audio_pending;
        let outgoing_address = outgoing.address;
        let incoming_address = incoming.address;
        let outgoing = outgoing.layer;
        let incoming = incoming.layer;
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
        let key = MorphCacheKey {
            sequence_path: outgoing_address.sequence_path().to_vec(),
            track_id: outgoing_address.track_id(),
            outgoing_id: outgoing_address.item_id(),
            incoming_id: incoming_address.item_id(),
            width: project.canvas_size.width,
            height: project.canvas_size.height,
        };
        let morph = if let Some(morph) = self.morphs.get(&key) {
            morph.clone()
        } else {
            let morph = std::rc::Rc::new(
                shrimply_video_core::vector_morph::PreparedVectorMorph::new(source, target),
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
            video_mask: None,
            alpha_mask: None,
            parameters,
            transform: shrimply_render_core::math::Mat3::IDENTITY,
            source: Source::Generated(Box::new(shrimply_video_core::generated::GeneratedFrame {
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
