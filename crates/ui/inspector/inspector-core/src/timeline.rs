use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use shrimply_project_document::project::{ItemAddress, Time};
use shrimply_property_model::timeline_value::{
    TimelineBool, TimelineExpressionValue, TimelineStep, TimelineValue,
};

use crate::audio_modifiers::{audio_item_address, audio_modifier_evaluation_time};
use crate::{
    AudioModifierKeyframeMove, InspectorCommit, InspectorController, InspectorExpressionOutput,
    InspectorTarget,
};

mod background_integer;
mod text;

// Background enum fields share their concrete type across edits, evaluation and clipboard data.
macro_rules! background_step {
    (@getter $getter:pat, $value:ident, $variant:ident, $field:ident) => {
        let $getter: fn(&shrimply_project_document::project::BackgroundGenerator) -> Option<&TimelineValue<$value>> =
            |generator| match generator {
                shrimply_project_document::project::BackgroundGenerator::$variant(value) => Some(&value.$field),
                _ => None,
            };
    };
    ($project:expr, $target:expr, $path:expr, $value:ident, $getter:pat, $action:expr) => {{
        use shrimply_project_document::project::{
            BackgroundKind, Curve, GradientMode, GridLineStyle, NoiseColorMode, NoiseDistribution,
            PerlinMode, RainbowBands, RainbowFill, VoronoiFill, VoronoiMetric,
        };
        let kind = {
            let project = $project;
            let item = project
                .video_item(video_item_address($target)?)
                .ok_or("background item is no longer available")?;
            crate::background::generator(item)
                .ok_or("background generator is no longer available")?
                .kind()
        };
        match ($path, kind) {
            ("/content/generator/mode", BackgroundKind::ColorGradient) => {
                type $value = GradientMode;
                background_step!(@getter $getter, $value, ColorGradient, mode);
                $action
            }
            ("/content/generator/mode", BackgroundKind::PerlinNoise) => {
                type $value = PerlinMode;
                background_step!(@getter $getter, $value, PerlinNoise, mode);
                $action
            }
            ("/content/generator/curve", _) => {
                type $value = Curve;
                background_step!(@getter $getter, $value, ColorGradient, curve);
                $action
            }
            ("/content/generator/line_style", _) => {
                type $value = GridLineStyle;
                background_step!(@getter $getter, $value, Grid, line_style);
                $action
            }
            ("/content/generator/distribution", _) => {
                type $value = NoiseDistribution;
                background_step!(@getter $getter, $value, WhiteNoise, distribution);
                $action
            }
            ("/content/generator/color_mode", _) => {
                type $value = NoiseColorMode;
                background_step!(@getter $getter, $value, WhiteNoise, color_mode);
                $action
            }
            ("/content/generator/fill", BackgroundKind::Rainbow) => {
                type $value = RainbowFill;
                background_step!(@getter $getter, $value, Rainbow, fill);
                $action
            }
            ("/content/generator/fill", BackgroundKind::Voronoi) => {
                type $value = VoronoiFill;
                background_step!(@getter $getter, $value, Voronoi, fill);
                $action
            }
            ("/content/generator/bands", _) => {
                type $value = RainbowBands;
                background_step!(@getter $getter, $value, Rainbow, bands);
                $action
            }
            ("/content/generator/metric", _) => {
                type $value = VoronoiMetric;
                background_step!(@getter $getter, $value, Voronoi, metric);
                $action
            }
            _ => Err(format!("unknown background step timeline: {}", $path)),
        }
    }};
}

impl InspectorController {
    pub fn current_keyframe_time(&self, target: &InspectorTarget) -> Result<Time, String> {
        let address = match target {
            InspectorTarget::Item(address) | InspectorTarget::Transition { item: address, .. } => {
                address
            }
            InspectorTarget::Project | InspectorTarget::Track(_) => {
                return Err("inspector target has no keyframe time".to_string());
            }
        };
        self.project
            .borrow()
            .keyframe_time(
                address,
                shrimply_editor_state::player_state::current_time(&self.player_state),
            )
            .ok_or_else(|| "the current item keyframe time is no longer available".to_string())
    }

    pub fn set_bool_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        next: bool,
    ) -> Result<(), String> {
        let (mut value, runtime) = self.bool_timeline(target, path)?;
        let time = runtime
            .keyframe_playhead
            .ok_or_else(|| "boolean keyframe time is no longer available".to_string())?;
        if !crate::keyframe_model::set_discrete_value(
            &mut value,
            time,
            next.into(),
            runtime.frame_step,
        ) {
            return Ok(());
        }
        self.set_live_value(target, path, serialize_bool_timeline(value))
    }

    pub fn set_bool_keyframes_enabled(
        &self,
        target: &InspectorTarget,
        path: &str,
        enabled: bool,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.set_video_step_keyframes_enabled::<TimelineBool>(target, path, enabled, commit)
    }

    pub fn set_step_keyframes_enabled(
        &self,
        target: &InspectorTarget,
        path: &str,
        enabled: bool,
    ) -> Result<(), String> {
        self.set_step_keyframes_enabled_with_commit(
            target,
            path,
            enabled,
            InspectorCommit::Immediate(crate::model::INSPECTOR_EDIT_COMMIT),
        )
    }

    pub fn set_step_keyframes_enabled_with_commit(
        &self,
        target: &InspectorTarget,
        path: &str,
        enabled: bool,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        if path.ends_with("/effect/effect/config/method")
            && sampling_method_target(&self.project.borrow(), target, path)?
        {
            return self
                .set_video_step_keyframes_enabled::<shrimply_property_model::VideoSampleMethod>(
                    target, path, enabled, commit,
                );
        }
        if path.ends_with("/effect/effect/config/mode")
            && mask_mode_target(&self.project.borrow(), target, path)?
        {
            return self
                .set_video_step_keyframes_enabled::<shrimply_visual_modifiers::mask::MaskMode>(
                    target, path, enabled, commit,
                );
        }
        if let Some(timeline) = crate::generated::step_timeline(path) {
            return match timeline {
                crate::generated::StepTimeline::Shape => self
                    .set_video_step_keyframes_enabled::<shrimply_project_document::project::ShapeKind>(
                        target, path, enabled, commit,
                    ),
                crate::generated::StepTimeline::Rounding => self
                    .set_video_step_keyframes_enabled::<
                        shrimply_project_document::project::ShapeRoundingStrategy,
                    >(target, path, enabled, commit),
                crate::generated::StepTimeline::HorizontalAlign => self
                    .set_video_step_keyframes_enabled::<
                        shrimply_project_document::project::TextHorizontalAlign,
                    >(target, path, enabled, commit),
                crate::generated::StepTimeline::VerticalAlign => self
                    .set_video_step_keyframes_enabled::<
                        shrimply_project_document::project::VerticalAlign,
                    >(target, path, enabled, commit),
                crate::generated::StepTimeline::Direction => self
                    .set_video_step_keyframes_enabled::<
                        shrimply_project_document::project::TextDirection,
                    >(target, path, enabled, commit),
                crate::generated::StepTimeline::FontStyle => self
                    .set_video_step_keyframes_enabled::<
                        shrimply_project_document::project::TextFontStyle,
                    >(target, path, enabled, commit),
            };
        }
        if path.starts_with("/content/generator/") {
            return background_step!(
                self.project.borrow(),
                target,
                path,
                T,
                _,
                self.set_video_step_keyframes_enabled::<T>(target, path, enabled, commit)
            );
        }
        match path {
            crate::gaussian_3d::MODEL_ROTATION_ORDER_PATH => self
                .set_video_step_keyframes_enabled::<shrimply_3dgs_core::RotationOrder>(
                    target, path, enabled, commit,
                ),
            "/sample_method" => self
                .set_video_step_keyframes_enabled::<shrimply_property_model::VideoSampleMethod>(
                    target, path, enabled, commit,
                ),
            path if path.ends_with("/effect/effect/sample_method") => self
                .set_video_step_keyframes_enabled::<shrimply_property_model::VideoSampleMethod>(
                    target, path, enabled, commit,
                ),
            "/compositing/blend_mode" => self
                .set_video_step_keyframes_enabled::<shrimply_property_model::LayerBlendMode>(
                    target, path, enabled, commit,
                ),
            path if path.ends_with("/effect/effect/config/operation") => self
                .set_video_step_keyframes_enabled::<
                    shrimply_visual_modifiers::erode_dilate::ErodeDilateOperation,
                >(target, path, enabled, commit),
            path if path.ends_with("/effect/effect/config/pattern") => self
                .set_video_step_keyframes_enabled::<
                    shrimply_visual_modifiers::dithering::DitheringPattern,
                >(target, path, enabled, commit),
            path if path.ends_with("/effect/effect/config/color_mode") => self
                .set_video_step_keyframes_enabled::<
                    shrimply_visual_modifiers::dithering::DitheringColorMode,
                >(target, path, enabled, commit),
            path if path.ends_with("/effect/effect/config/version") => self
                .set_video_step_keyframes_enabled::<
                    shrimply_visual_modifiers::kuwahara::KuwaharaVersion,
                >(target, path, enabled, commit),
            path if path.ends_with("/effect/effect/config/row_offset_axis") => self
                .set_video_step_keyframes_enabled::<
                    shrimply_visual_modifiers::repeat::RepeatOffsetAxis,
                >(target, path, enabled, commit),
            path if path.ends_with("/effect/effect/config/address_mode") => self
                .set_video_step_keyframes_enabled::<shrimply_property_model::TextureAddressMode>(
                    target, path, enabled, commit,
                ),
            path if path.ends_with("/effect/effect/config/mode") => self
                .set_video_step_keyframes_enabled::<
                    shrimply_visual_modifiers::halftone::HalftoneMode,
                >(target, path, enabled, commit),
            _ => Err(format!("unknown step timeline: {path}")),
        }
    }

    fn set_video_step_keyframes_enabled<T>(
        &self,
        target: &InspectorTarget,
        path: &str,
        enabled: bool,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String>
    where
        T: TimelineStep + DeserializeOwned + Serialize,
    {
        let snapshot = self.snapshot();
        if &snapshot.target != target {
            return Err("inspector target changed".to_string());
        }
        let mut value: TimelineValue<T> = serde_json::from_value(
            snapshot
                .value
                .pointer(path)
                .cloned()
                .ok_or_else(|| format!("step timeline is no longer available: {path}"))?,
        )
        .map_err(|error| format!("invalid step timeline: {error}"))?;
        let evaluation_time = snapshot
            .runtime
            .local_time
            .ok_or_else(|| "step evaluation time is no longer available".to_string())?;
        let current = value.value_at(evaluation_time);
        let time = snapshot
            .runtime
            .keyframe_playhead
            .ok_or_else(|| "step keyframe time is no longer available".to_string())?;
        if !crate::keyframe_model::set_keyframes_enabled(&mut value, time, current, enabled) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serde_json::to_value(value).expect("step timeline must serialize"),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn bool_expression_output(
        &self,
        target: &InspectorTarget,
        path: &str,
    ) -> Result<InspectorExpressionOutput<bool>, String> {
        let outcome = self.video_expression_output::<TimelineBool>(target, path)?;
        Ok(InspectorExpressionOutput {
            value: outcome.value.get(),
            error: outcome.error,
        })
    }

    pub(crate) fn background_step_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        background_step!(self.project.borrow(), target, path, T, get, {
            let project = self.project.borrow();
            let item = project
                .video_item(video_item_address(target)?)
                .ok_or("background item is no longer available")?;
            let timeline = crate::background::generator(item)
                .and_then(get)
                .filter(|timeline| timeline.id == timeline_id)
                .ok_or("background timeline changed while the inspector was open")?;
            let runtime = crate::model::target_runtime(&project, &self.player_state, target);
            Ok(crate::selector::step_graph(timeline, runtime))
        })
    }

    pub fn step_expression_output(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: Option<uuid::Uuid>,
    ) -> Result<InspectorExpressionOutput<String>, String> {
        if path.starts_with("/content/generator/") {
            if let Some(id) = timeline_id {
                self.ensure_timeline(target, path, id)?;
            }
            return background_step!(
                self.project.borrow(),
                target,
                path,
                T,
                _,
                self.step_expression_output_as::<T>(target, path)
            );
        }
        if path.ends_with("/effect/effect/config/method")
            && sampling_method_target(&self.project.borrow(), target, path)?
        {
            let timeline_id = timeline_id
                .ok_or_else(|| "visual modifier step timeline ID is unavailable".to_string())?;
            return self.visual_modifier_step_expression_output(
                target,
                path,
                timeline_id,
                crate::visual_modifiers::sampling_method_timeline,
            );
        }
        if path.ends_with("/effect/effect/config/mode")
            && mask_mode_target(&self.project.borrow(), target, path)?
        {
            let timeline_id = timeline_id
                .ok_or_else(|| "visual modifier step timeline ID is unavailable".to_string())?;
            return self.visual_modifier_step_expression_output(
                target,
                path,
                timeline_id,
                crate::visual_modifiers::mask_mode,
            );
        }
        match path {
            path if crate::paint::paint_taper_path(path) => {
                self.step_expression_output_as::<shrimply_project_document::project::PaintTaper>(
                    target, path,
                )
            }
            crate::gaussian_3d::MODEL_ROTATION_ORDER_PATH => timeline_id
                .ok_or_else(|| "Gaussian timeline ID is unavailable".to_string())
                .and_then(|timeline_id| self.ensure_timeline(target, path, timeline_id))
                .and_then(|()| {
                    self.step_expression_output_as::<shrimply_3dgs_core::RotationOrder>(
                        target, path,
                    )
                }),
            "/content/shape" => timeline_id
                .ok_or_else(|| "shape timeline ID is unavailable".to_string())
                .and_then(|timeline_id| self.ensure_timeline(target, path, timeline_id))
                .and_then(|()| {
                    self.step_expression_output_as::<shrimply_project_document::project::ShapeKind>(
                        target, path,
                    )
                }),
            "/content/rounding_strategy" => timeline_id
                .ok_or_else(|| "shape timeline ID is unavailable".to_string())
                .and_then(|timeline_id| self.ensure_timeline(target, path, timeline_id))
                .and_then(|()| {
                    self.step_expression_output_as::<
                        shrimply_project_document::project::ShapeRoundingStrategy,
                    >(target, path)
                }),
            "/sample_method" => self
                .step_expression_output_as::<shrimply_property_model::VideoSampleMethod>(
                    target, path,
                ),
            path if path.ends_with("/effect/effect/sample_method") => {
                let timeline_id = timeline_id
                    .ok_or_else(|| "visual modifier step timeline ID is unavailable".to_string())?;
                self.visual_modifier_step_expression_output(
                    target,
                    path,
                    timeline_id,
                    crate::visual_modifiers::rasterize_sample_method_timeline,
                )
            }
            "/compositing/blend_mode" => self
                .step_expression_output_as::<shrimply_property_model::LayerBlendMode>(target, path),
            path if path.ends_with("/effect/effect/config/operation") => {
                let timeline_id = timeline_id
                    .ok_or_else(|| "visual modifier step timeline ID is unavailable".to_string())?;
                self.visual_modifier_step_expression_output(
                    target,
                    path,
                    timeline_id,
                    crate::visual_modifiers::erode_dilate_operation,
                )
            }
            path if path.ends_with("/effect/effect/config/pattern") => {
                let timeline_id = timeline_id
                    .ok_or_else(|| "visual modifier step timeline ID is unavailable".to_string())?;
                self.visual_modifier_step_expression_output(
                    target,
                    path,
                    timeline_id,
                    crate::visual_modifiers::dithering_pattern,
                )
            }
            path if path.ends_with("/effect/effect/config/color_mode") => {
                let timeline_id = timeline_id
                    .ok_or_else(|| "visual modifier step timeline ID is unavailable".to_string())?;
                self.visual_modifier_step_expression_output(
                    target,
                    path,
                    timeline_id,
                    crate::visual_modifiers::dithering_color_mode,
                )
            }
            path if path.ends_with("/effect/effect/config/version") => {
                let timeline_id = timeline_id
                    .ok_or_else(|| "visual modifier step timeline ID is unavailable".to_string())?;
                self.visual_modifier_step_expression_output(
                    target,
                    path,
                    timeline_id,
                    crate::visual_modifiers::kuwahara_version_timeline,
                )
            }
            path if path.ends_with("/effect/effect/config/row_offset_axis") => {
                let timeline_id = timeline_id
                    .ok_or_else(|| "visual modifier step timeline ID is unavailable".to_string())?;
                self.visual_modifier_step_expression_output(
                    target,
                    path,
                    timeline_id,
                    crate::visual_modifiers::repeat_offset_axis_timeline,
                )
            }
            path if path.ends_with("/effect/effect/config/address_mode") => {
                let timeline_id = timeline_id
                    .ok_or_else(|| "visual modifier step timeline ID is unavailable".to_string())?;
                self.visual_modifier_step_expression_output(
                    target,
                    path,
                    timeline_id,
                    crate::visual_modifiers::texture_bounds_address_mode_timeline,
                )
            }
            path if path.ends_with("/effect/effect/config/mode") => {
                let timeline_id = timeline_id
                    .ok_or_else(|| "visual modifier step timeline ID is unavailable".to_string())?;
                self.visual_modifier_step_expression_output(
                    target,
                    path,
                    timeline_id,
                    crate::visual_modifiers::halftone_mode,
                )
            }
            _ => Err(format!("unknown step timeline: {path}")),
        }
    }

    fn visual_modifier_step_expression_output<T>(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        timeline: for<'a> fn(
            &'a shrimply_project_document::project::VideoItem,
            &str,
            uuid::Uuid,
        ) -> Option<&'a TimelineValue<T>>,
    ) -> Result<InspectorExpressionOutput<String>, String>
    where
        T: TimelineExpressionValue + TimelineStep,
    {
        let outcome = self.video_modifier_expression_output(target, path, timeline_id, timeline)?;
        let value = T::variants()
            .iter()
            .find(|variant| variant.value == outcome.value)
            .expect("evaluated modifier step must be a declared variant")
            .key
            .to_string();
        Ok(InspectorExpressionOutput {
            value,
            error: outcome.error,
        })
    }

    fn step_expression_output_as<T>(
        &self,
        target: &InspectorTarget,
        path: &str,
    ) -> Result<InspectorExpressionOutput<String>, String>
    where
        T: TimelineExpressionValue + TimelineStep + DeserializeOwned,
    {
        let outcome = self.video_expression_output::<T>(target, path)?;
        let value = T::variants()
            .iter()
            .find(|variant| variant.value == outcome.value)
            .expect("evaluated timeline step must be one of its declared variants")
            .key
            .to_string();
        Ok(InspectorExpressionOutput {
            value,
            error: outcome.error,
        })
    }

    pub(crate) fn video_expression_output<T>(
        &self,
        target: &InspectorTarget,
        path: &str,
    ) -> Result<InspectorExpressionOutput<T>, String>
    where
        T: TimelineExpressionValue + DeserializeOwned,
    {
        if &self.target() != target {
            return Err("inspector target changed".to_string());
        }
        let player = crate::model::player_snapshot(&self.player_state);
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        let position = project
            .timeline_time_to_sequence(&address.track(), player.position)
            .ok_or_else(|| "expression time is no longer available".to_string())?;
        let item = project
            .video_item(address)
            .ok_or_else(|| "expression item is no longer available".to_string())?;
        let serialized = transform_expression_value(item, path).unwrap_or_else(|| {
            serde_json::to_value(item)
                .expect("video item must serialize")
                .pointer(path)
                .cloned()
                .unwrap_or(Value::Null)
        });
        let value: TimelineValue<T> = serde_json::from_value(serialized)
            .map_err(|error| format!("invalid timeline expression value at {path}: {error}"))?;
        let audio =
            self.audio_sampler
                .borrow_mut()
                .sample(&project, player.position, player.revision);
        let evaluation = shrimply_project_evaluation::VisualEvaluation::for_item_with_audio(
            &project, item, position, &audio,
        );
        let outcome = shrimply_project_evaluation::resolve_with_error(
            &value,
            &evaluation,
            &mut self.expression_cache.borrow_mut(),
        );
        Ok(InspectorExpressionOutput {
            value: outcome.value,
            error: outcome.error,
        })
    }

    pub(crate) fn video_modifier_expression_output<T>(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        timeline: for<'a> fn(
            &'a shrimply_project_document::project::VideoItem,
            &str,
            uuid::Uuid,
        ) -> Option<&'a TimelineValue<T>>,
    ) -> Result<InspectorExpressionOutput<T>, String>
    where
        T: TimelineExpressionValue,
    {
        if &self.target() != target {
            return Err("inspector target changed".to_string());
        }
        let player = crate::model::player_snapshot(&self.player_state);
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        let position = project
            .timeline_time_to_sequence(&address.track(), player.position)
            .ok_or_else(|| "modifier expression time is no longer available".to_string())?;
        let item = project
            .video_item(address)
            .ok_or_else(|| "modifier expression item is no longer available".to_string())?;
        let value = timeline(item, path, timeline_id)
            .ok_or_else(|| format!("modifier expression timeline is unavailable: {path}"))?;
        let audio =
            self.audio_sampler
                .borrow_mut()
                .sample(&project, player.position, player.revision);
        let evaluation = shrimply_project_evaluation::VisualEvaluation::for_item_with_audio(
            &project, item, position, &audio,
        );
        let outcome = shrimply_project_evaluation::resolve_with_error(
            value,
            &evaluation,
            &mut self.expression_cache.borrow_mut(),
        );
        Ok(InspectorExpressionOutput {
            value: outcome.value,
            error: outcome.error,
        })
    }

    pub fn set_color_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        next: shrimply_property_model::Color<u8>,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let (mut value, runtime) = self.color_timeline(target, path, timeline_id)?;
        let time = runtime
            .keyframe_playhead
            .ok_or_else(|| "color keyframe time is no longer available".to_string())?;
        if !crate::timeline_color::set_value(
            &mut value,
            runtime.local_time.unwrap_or(Time::ZERO),
            time,
            next,
        ) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Live,
            path,
            serialize_color_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Live),
            commit,
        )
    }

    pub fn set_color_keyframes_enabled(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        enabled: bool,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let (mut value, runtime) = self.color_timeline(target, path, timeline_id)?;
        let time = runtime
            .keyframe_playhead
            .ok_or_else(|| "color keyframe time is no longer available".to_string())?;
        if !crate::timeline_color::set_keyframes_enabled(
            &mut value,
            runtime.local_time.unwrap_or(Time::ZERO),
            time,
            enabled,
        ) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serialize_color_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn color_expression_output(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<InspectorExpressionOutput<shrimply_property_model::Color<u8>>, String> {
        if &self.target() != target {
            return Err("inspector target changed".to_string());
        }
        let player = crate::model::player_snapshot(&self.player_state);
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        let position = project
            .timeline_time_to_sequence(&address.track(), player.position)
            .ok_or_else(|| "color expression time is no longer available".to_string())?;
        let item = project
            .video_item(address)
            .ok_or_else(|| "color expression item is no longer available".to_string())?;
        let value = video_color_value(item, path, timeline_id)
            .ok_or_else(|| format!("color timeline is no longer available: {path}"))?;
        let audio =
            self.audio_sampler
                .borrow_mut()
                .sample(&project, player.position, player.revision);
        let evaluation = shrimply_project_evaluation::VisualEvaluation::for_item_with_audio(
            &project, item, position, &audio,
        );
        let outcome = shrimply_project_evaluation::resolve_with_error(
            value,
            &evaluation,
            &mut self.expression_cache.borrow_mut(),
        );
        Ok(InspectorExpressionOutput {
            value: outcome.value,
            error: outcome.error,
        })
    }

    pub fn color_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<shrimply_property_model::Color<u8>, String> {
        if &self.target() != target {
            return Err("inspector target changed".to_string());
        }
        let player = crate::model::player_snapshot(&self.player_state);
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        let sequence_time = project
            .timeline_time_to_sequence(&address.track(), player.position)
            .ok_or_else(|| "color time is no longer available".to_string())?;
        let item = project
            .video_item(address)
            .ok_or_else(|| "color item is no longer available".to_string())?;
        let local_time =
            shrimply_project_document::project::generated_item_time(item, sequence_time)
                .ok_or_else(|| "color time is outside the item".to_string())?;
        let value = video_color_value(item, path, timeline_id)
            .ok_or_else(|| format!("color timeline is no longer available: {path}"))?;
        Ok(crate::timeline_color::value_at(value, local_time))
    }

    pub fn move_color_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        moves: &[(Time, Time)],
        commit: InspectorCommit<'_>,
    ) -> Result<Vec<Time>, String> {
        let moves = self.canonical_video_keyframe_moves(target, moves)?;
        let (mut value, _) = self.color_timeline(target, path, timeline_id)?;
        if !crate::timeline_color::move_keyframes(&mut value, &moves) {
            return Err("color keyframe move targets are no longer available".to_string());
        }
        self.set_live_keyframe_graph_value_with_commit(
            target,
            path,
            serialize_color_timeline(value),
            commit,
        )?;
        Ok(moves.into_iter().map(|(_, time)| time).collect())
    }

    pub fn delete_color_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.delete_color_keyframes(
            target,
            path,
            timeline_id,
            std::slice::from_ref(&time),
            commit,
        )
    }

    pub fn delete_color_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        times: &[Time],
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let (mut value, _) = self.color_timeline(target, path, timeline_id)?;
        if !crate::keyframe_model::edit_keyframe_selection(&mut value, times, |value, time| {
            Ok(crate::timeline_color::delete_keyframe(value, time))
        })? {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serialize_color_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn add_color_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let time = self.canonical_video_keyframe_time(target, time)?;
        let (mut value, _) = self.color_timeline(target, path, timeline_id)?;
        if !crate::timeline_color::add_keyframe(&mut value, time) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serialize_color_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn copy_color_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        selected: &[Time],
    ) -> Result<usize, String> {
        let (value, _) = self.color_timeline(target, path, timeline_id)?;
        let Some(mut clipboard) = crate::timeline_color::copy_keyframes(&value, selected) else {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        if !crate::keyframe_model::normalize_clipboard_times(
            &project,
            Some(address),
            &mut clipboard,
        ) {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        }
        let count = clipboard.len();
        self.keyframe_clipboard.replace(Some(clipboard));
        Ok(count)
    }

    pub fn paste_color_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<usize, String> {
        let Some(clipboard) = self.keyframe_clipboard.borrow().clone() else {
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        let times =
            crate::keyframe_model::clipboard_paste_times(&project, Some(address), &clipboard, time);
        drop(project);
        let Some(times) = times else {
            return Err("color keyframes cannot be pasted at this time".to_string());
        };
        let (mut value, _) = self.color_timeline(target, path, timeline_id)?;
        let Some(pasted) = crate::timeline_color::paste_keyframes(&mut value, &clipboard, &times)
        else {
            return Ok(0);
        };
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serialize_color_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )?;
        Ok(pasted.len())
    }

    pub fn set_color_interpolation(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        owner_id: uuid::Uuid,
        interpolation_index: usize,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let interpolation = crate::keyframe_model::interpolation(interpolation_index)?;
        let (mut value, _) = self.color_timeline(target, path, timeline_id)?;
        if !crate::timeline_color::set_interpolation(&mut value, owner_id, interpolation)? {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serialize_color_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    fn color_timeline(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<
        (
            TimelineValue<shrimply_property_model::Color<u8>>,
            crate::InspectorRuntime,
        ),
        String,
    > {
        if &self.target() != target {
            return Err("inspector target changed".to_string());
        }
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        let item = project
            .video_item(address)
            .ok_or_else(|| "color item is no longer available".to_string())?;
        let value = video_color_value(item, path, timeline_id)
            .cloned()
            .ok_or_else(|| format!("color timeline is no longer available: {path}"))?;
        crate::timeline_color::validate_timeline(&value, timeline_id)?;
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        Ok((value, runtime))
    }

    pub fn move_bool_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        moves: &[(Time, Time)],
    ) -> Result<Vec<Time>, String> {
        let moves = self.canonical_video_keyframe_moves(target, moves)?;
        let (mut value, _) = self.bool_timeline(target, path)?;
        if !crate::keyframe_model::move_discrete_keyframes(&mut value, &moves) {
            return Err("boolean keyframe move targets are no longer available".to_string());
        }
        self.set_live_keyframe_graph_value(target, path, serialize_bool_timeline(value))?;
        Ok(moves.into_iter().map(|(_, time)| time).collect())
    }

    pub fn delete_bool_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
    ) -> Result<(), String> {
        self.delete_bool_keyframes(target, path, std::slice::from_ref(&time))
    }

    pub fn delete_bool_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        times: &[Time],
    ) -> Result<(), String> {
        let (mut value, runtime) = self.bool_timeline(target, path)?;
        if !crate::keyframe_model::edit_keyframe_selection(&mut value, times, |value, time| {
            Ok(crate::keyframe_model::delete_discrete_keyframe(
                value,
                time,
                runtime.frame_step,
            ))
        })? {
            return Ok(());
        }
        self.set_value(target, path, serialize_bool_timeline(value))
    }

    pub fn add_bool_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
    ) -> Result<(), String> {
        let time = self.canonical_video_keyframe_time(target, time)?;
        let (mut value, runtime) = self.bool_timeline(target, path)?;
        if !crate::keyframe_model::add_discrete_keyframe(&mut value, time, runtime.frame_step) {
            return Ok(());
        }
        self.set_value(target, path, serialize_bool_timeline(value))
    }

    pub fn copy_bool_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        selected: &[Time],
    ) -> Result<usize, String> {
        let (value, _) = self.bool_timeline(target, path)?;
        let Some(mut clipboard) = crate::keyframe_model::copy_keyframes(&value, selected) else {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        if !crate::keyframe_model::normalize_clipboard_times(
            &project,
            Some(address),
            &mut clipboard,
        ) {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        }
        let count = clipboard.len();
        self.keyframe_clipboard.replace(Some(clipboard));
        Ok(count)
    }

    pub fn paste_bool_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
    ) -> Result<usize, String> {
        let Some(clipboard) = self.keyframe_clipboard.borrow().clone() else {
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        let times =
            crate::keyframe_model::clipboard_paste_times(&project, Some(address), &clipboard, time);
        drop(project);
        let Some(times) = times else {
            return Err("boolean keyframes cannot be pasted at this time".to_string());
        };
        let (mut value, _) = self.bool_timeline(target, path)?;
        let Some(pasted) = crate::keyframe_model::paste_keyframes(&mut value, &clipboard, &times)
        else {
            return Ok(0);
        };
        self.set_value(target, path, serialize_bool_timeline(value))?;
        Ok(pasted.len())
    }

    pub fn seek_discrete_keyframe(
        &self,
        target: &InspectorTarget,
        time: Time,
    ) -> Result<(), String> {
        let address = video_item_address(target)?;
        let project = self.project.borrow();
        let position = project
            .keyframe_timeline_time(address, time)
            .ok_or_else(|| "boolean keyframe time is no longer available".to_string())?;
        drop(project);
        shrimply_editor_state::player_state::seek_time(&self.player_state, position);
        Ok(())
    }

    pub fn move_step_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        moves: &[(Time, Time)],
        commit: InspectorCommit<'_>,
    ) -> Result<Vec<Time>, String> {
        let moves = self.canonical_video_keyframe_moves(target, moves)?;
        let (mut value, _) = self.json_step_timeline(target, path)?;
        if !crate::keyframe_model::move_json_discrete_keyframes(&mut value, &moves)? {
            return Err("selector keyframe move targets are no longer available".to_string());
        }
        self.set_live_keyframe_graph_value_with_commit(target, path, value, commit)?;
        Ok(moves.into_iter().map(|(_, time)| time).collect())
    }

    pub fn delete_step_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.delete_step_keyframes(target, path, std::slice::from_ref(&time), commit)
    }

    pub fn delete_step_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        times: &[Time],
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let (mut value, runtime) = self.json_step_timeline(target, path)?;
        if !crate::keyframe_model::edit_keyframe_selection(&mut value, times, |value, time| {
            crate::keyframe_model::delete_json_discrete_keyframe(value, time, runtime.frame_step)
        })? {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            value,
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn add_step_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let time = self.canonical_video_keyframe_time(target, time)?;
        let (mut value, runtime) = self.json_step_timeline(target, path)?;
        if !crate::keyframe_model::add_json_discrete_keyframe(&mut value, time, runtime.frame_step)?
        {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            value,
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn copy_step_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        selected: &[Time],
    ) -> Result<usize, String> {
        let (value, _) = self.json_step_timeline(target, path)?;
        let Some(mut clipboard) = crate::keyframe_model::copy_json_discrete_keyframes(
            &value,
            selected,
            step_timeline_type(&self.project.borrow(), target, path)?,
        )?
        else {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        if !crate::keyframe_model::normalize_clipboard_times(
            &project,
            Some(address),
            &mut clipboard,
        ) {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        }
        let count = clipboard.len();
        self.keyframe_clipboard.replace(Some(clipboard));
        Ok(count)
    }

    pub fn paste_step_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<usize, String> {
        let Some(clipboard) = self.keyframe_clipboard.borrow().clone() else {
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = video_item_address(target)?;
        let times =
            crate::keyframe_model::clipboard_paste_times(&project, Some(address), &clipboard, time);
        drop(project);
        let Some(times) = times else {
            return Err("step keyframes cannot be pasted at this time".to_string());
        };
        let (mut value, _) = self.json_step_timeline(target, path)?;
        let Some(pasted) = crate::keyframe_model::paste_json_discrete_keyframes(
            &mut value,
            &clipboard,
            &times,
            step_timeline_type(&self.project.borrow(), target, path)?,
        )?
        else {
            return Ok(0);
        };
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            value,
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )?;
        Ok(pasted.len())
    }

    fn json_step_timeline(
        &self,
        target: &InspectorTarget,
        path: &str,
    ) -> Result<(Value, crate::InspectorRuntime), String> {
        let snapshot = self.snapshot();
        if &snapshot.target != target {
            return Err("inspector target changed".to_string());
        }
        let value = snapshot
            .value
            .pointer(path)
            .cloned()
            .ok_or_else(|| format!("step timeline is no longer available: {path}"))?;
        Ok((value, snapshot.runtime))
    }

    fn canonical_video_keyframe_time(
        &self,
        target: &InspectorTarget,
        time: Time,
    ) -> Result<Time, String> {
        let address = video_item_address(target)?;
        let project = self.project.borrow();
        project
            .keyframe_timeline_time(address, time)
            .and_then(|time| project.keyframe_time(address, time))
            .ok_or_else(|| "keyframe time is no longer available".to_string())
    }

    fn canonical_video_keyframe_moves(
        &self,
        target: &InspectorTarget,
        moves: &[(Time, Time)],
    ) -> Result<Vec<(Time, Time)>, String> {
        moves
            .iter()
            .map(|&(old_time, time)| {
                Ok((old_time, self.canonical_video_keyframe_time(target, time)?))
            })
            .collect()
    }

    fn bool_timeline(
        &self,
        target: &InspectorTarget,
        path: &str,
    ) -> Result<(TimelineValue<TimelineBool>, crate::InspectorRuntime), String> {
        let snapshot = self.snapshot();
        if &snapshot.target != target {
            return Err("inspector target changed".to_string());
        }
        let value: TimelineValue<TimelineBool> = serde_json::from_value(
            snapshot
                .value
                .pointer(path)
                .cloned()
                .ok_or_else(|| format!("boolean timeline is no longer available: {path}"))?,
        )
        .map_err(|error| format!("invalid boolean timeline: {error}"))?;
        Ok((value, snapshot.runtime))
    }

    pub fn set_scalar_keyframes_enabled(
        &self,
        target: &InspectorTarget,
        path: &str,
        enabled: bool,
        constraint: crate::NumberConstraint,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let snapshot = self.snapshot();
        if &snapshot.target != target {
            return Err("inspector target changed".to_string());
        }
        let mut value: TimelineValue<f32> = serde_json::from_value(
            snapshot
                .value
                .pointer(path)
                .cloned()
                .ok_or_else(|| format!("inspector timeline is no longer available: {path}"))?,
        )
        .map_err(|error| format!("invalid inspector timeline: {error}"))?;
        let evaluation_time = snapshot.runtime.local_time.unwrap_or(Time::ZERO);
        let keyframe_time = snapshot
            .runtime
            .keyframe_playhead
            .ok_or_else(|| "scalar keyframe time is no longer available".to_string())?;
        if !crate::timeline_value::scalar::set_keyframes_enabled(
            &mut value,
            evaluation_time,
            keyframe_time,
            enabled,
            constraint.into(),
        ) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serialize_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn scalar_timeline(
        &self,
        target: &InspectorTarget,
        path: &str,
    ) -> Result<TimelineValue<f32>, String> {
        let snapshot = self.snapshot();
        if &snapshot.target != target {
            return Err("inspector target changed".to_string());
        }
        serde_json::from_value(
            snapshot
                .value
                .pointer(path)
                .cloned()
                .ok_or_else(|| format!("inspector timeline is no longer available: {path}"))?,
        )
        .map_err(|error| format!("invalid inspector timeline: {error}"))
    }

    pub fn scalar_expression_output(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: Option<uuid::Uuid>,
    ) -> Result<InspectorExpressionOutput, String> {
        if matches!(target, InspectorTarget::Item(ItemAddress::Video { .. })) {
            if path.starts_with("/modifiers/")
                && let Some(timeline_id) = timeline_id
            {
                return self.video_modifier_expression_output(
                    target,
                    path,
                    timeline_id,
                    crate::visual_modifiers::visual_modifier_number,
                );
            }
            if let Some(timeline_id) = timeline_id {
                self.ensure_timeline(target, path, timeline_id)?;
            }
            return self.video_expression_output(target, path);
        }
        let value = self.scalar_timeline(target, path)?;
        let project = self.project.borrow();
        let address = audio_item_address(target)?;
        let local_time = audio_modifier_evaluation_time(&project, &self.player_state, target)?;
        let item = project
            .audio_item(address)
            .ok_or_else(|| "audio item is no longer available".to_string())?;
        let evaluation = shrimply_project_evaluation::VisualEvaluation::for_audio_item_local_time(
            &project, item, local_time,
        );
        let outcome = shrimply_project_evaluation::resolve_with_error(
            &value,
            &evaluation,
            &mut self.expression_cache.borrow_mut(),
        );
        Ok(InspectorExpressionOutput {
            value: outcome.value,
            error: outcome.error,
        })
    }

    pub fn move_scalar_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        change: AudioModifierKeyframeMove,
        constraint: crate::NumberConstraint,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.move_scalar_keyframes(
            target,
            path,
            std::slice::from_ref(&change),
            constraint,
            commit,
        )
        .map(|_| ())
    }

    pub fn move_scalar_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        changes: &[AudioModifierKeyframeMove],
        constraint: crate::NumberConstraint,
        commit: InspectorCommit<'_>,
    ) -> Result<Vec<Time>, String> {
        if changes.is_empty() {
            return Ok(Vec::new());
        }
        let changes = changes
            .iter()
            .map(|change| {
                Ok((
                    change.old_time,
                    self.canonical_scalar_keyframe_time(target, change.time)?,
                    (change.displayed_value * change.store_multiplier) as f32,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut value = self.scalar_timeline(target, path)?;
        if !crate::timeline_value::scalar::move_stored_keyframes(&mut value, &changes, constraint) {
            return Err("scalar keyframe move contains invalid values or stale source keys".into());
        }
        self.set_live_keyframe_graph_value_with_commit(
            target,
            path,
            serialize_timeline(value),
            commit,
        )?;
        Ok(changes.iter().map(|change| change.1).collect())
    }

    pub fn delete_scalar_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.delete_scalar_keyframes(target, path, std::slice::from_ref(&time), commit)
    }

    pub fn delete_scalar_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        times: &[Time],
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let mut value = self.scalar_timeline(target, path)?;
        if !crate::keyframe_model::edit_keyframe_selection(&mut value, times, |value, time| {
            Ok(crate::keyframe_model::delete_scalar_keyframe(value, time))
        })? {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serialize_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn add_scalar_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        constraint: crate::NumberConstraint,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let time = self.canonical_scalar_keyframe_time(target, time)?;
        let mut value = self.scalar_timeline(target, path)?;
        if !crate::timeline_value::scalar::add_keyframe(&mut value, time, constraint.into()) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serialize_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn set_scalar_keyframe_interpolation(
        &self,
        target: &InspectorTarget,
        path: &str,
        owner_id: uuid::Uuid,
        interpolation_index: usize,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        let interpolation = crate::keyframe_model::interpolation(interpolation_index)?;
        let mut value = self.scalar_timeline(target, path)?;
        if !crate::keyframe_model::set_scalar_interpolation(&mut value, owner_id, interpolation) {
            return Ok(());
        }
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serialize_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )
    }

    pub fn copy_scalar_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        selected: &[Time],
    ) -> Result<usize, String> {
        let value = self.scalar_timeline(target, path)?;
        let Some(mut clipboard) = crate::keyframe_model::copy_keyframes(&value, selected) else {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = scalar_item_address(target)?;
        if !crate::keyframe_model::normalize_clipboard_times(
            &project,
            Some(address),
            &mut clipboard,
        ) {
            self.keyframe_clipboard.replace(None);
            return Ok(0);
        }
        let count = clipboard.len();
        self.keyframe_clipboard.replace(Some(clipboard));
        Ok(count)
    }

    pub fn paste_scalar_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        time: Time,
        constraint: crate::NumberConstraint,
        commit: InspectorCommit<'_>,
    ) -> Result<usize, String> {
        let Some(clipboard) = self.keyframe_clipboard.borrow().clone() else {
            return Ok(0);
        };
        let project = self.project.borrow();
        let address = scalar_item_address(target)?;
        let times =
            crate::keyframe_model::clipboard_paste_times(&project, Some(address), &clipboard, time);
        drop(project);
        let Some(times) = times else {
            return Err("keyframes cannot be pasted at this time".to_string());
        };
        let mut value = self.scalar_timeline(target, path)?;
        let Some(pasted) = crate::keyframe_model::paste_keyframes(&mut value, &clipboard, &times)
        else {
            return Ok(0);
        };
        crate::timeline_value::scalar::constrain_keyframes(&mut value, &pasted, constraint.into());
        self.replace_value_with_commit(
            target,
            crate::model::EditKind::Structural,
            path,
            serialize_timeline(value),
            crate::refresh::audio_path_change(target, path, crate::model::EditKind::Structural),
            commit,
        )?;
        Ok(pasted.len())
    }

    pub fn seek_scalar_keyframe(&self, target: &InspectorTarget, time: Time) -> Result<(), String> {
        let address = scalar_item_address(target)?;
        let project = self.project.borrow();
        let position = project
            .keyframe_timeline_time(address, time)
            .ok_or_else(|| "keyframe time is no longer available".to_string())?;
        drop(project);
        shrimply_editor_state::player_state::seek_time(&self.player_state, position);
        Ok(())
    }

    fn canonical_scalar_keyframe_time(
        &self,
        target: &InspectorTarget,
        time: Time,
    ) -> Result<Time, String> {
        let project = self.project.borrow();
        let address = scalar_item_address(target)?;
        project
            .keyframe_timeline_time(address, time)
            .and_then(|timeline_time| project.keyframe_time(address, timeline_time))
            .ok_or_else(|| "scalar keyframe time is no longer available".to_string())
    }
}

fn transform_expression_value(
    item: &shrimply_project_document::project::VideoItem,
    path: &str,
) -> Option<Value> {
    let value = match path {
        "/transform/position" => serde_json::to_value(&item.transform.position),
        "/transform/anchor" => serde_json::to_value(&item.transform.anchor),
        "/transform/scale" => serde_json::to_value(&item.transform.scale),
        "/transform/shear" => serde_json::to_value(&item.transform.shear),
        "/transform/rotation_degrees" => serde_json::to_value(&item.transform.rotation_degrees),
        _ => return None,
    };
    Some(value.expect("transform timeline must serialize"))
}

fn serialize_timeline(value: TimelineValue<f32>) -> Value {
    serde_json::to_value(value).expect("scalar timeline must serialize")
}

fn video_color_value<'a>(
    item: &'a shrimply_project_document::project::VideoItem,
    path: &str,
    timeline_id: uuid::Uuid,
) -> Option<&'a TimelineValue<shrimply_property_model::Color<u8>>> {
    if path.starts_with("/content/generator/") {
        crate::background::color_value(item, timeline_id)
    } else if path.starts_with("/content/") {
        crate::paint::color_value(item, path, timeline_id)
            .or_else(|| crate::generated::color_value(item, path, timeline_id))
            .or_else(|| {
                let shrimply_project_document::project::VideoItemContent::Obj(scene) =
                    &item.content
                else {
                    return None;
                };
                crate::scene_3d::color(scene, timeline_id)
            })
    } else {
        crate::visual_modifiers::visual_modifier_color(item, path, timeline_id)
    }
}

fn serialize_bool_timeline(value: TimelineValue<TimelineBool>) -> Value {
    serde_json::to_value(value).expect("boolean timeline must serialize")
}

fn serialize_color_timeline(value: TimelineValue<shrimply_property_model::Color<u8>>) -> Value {
    serde_json::to_value(value).expect("color timeline must serialize")
}

fn video_item_address(target: &InspectorTarget) -> Result<&ItemAddress, String> {
    let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
        return Err("boolean keyframe target is not a video item".to_string());
    };
    Ok(address)
}

fn scalar_item_address(target: &InspectorTarget) -> Result<&ItemAddress, String> {
    match target {
        InspectorTarget::Item(
            address @ (ItemAddress::Audio { .. } | ItemAddress::Video { .. }),
        ) => Ok(address),
        _ => Err("scalar keyframe target is not an audio or video item".to_string()),
    }
}

fn step_timeline_type(
    project: &shrimply_project_document::project::Project,
    target: &InspectorTarget,
    path: &str,
) -> Result<&'static str, String> {
    if path.starts_with("/content/generator/") {
        return background_step!(project, target, path, T, _, Ok(std::any::type_name::<T>()));
    }
    if path.ends_with("/effect/effect/config/method")
        && sampling_method_target(project, target, path)?
    {
        return Ok(std::any::type_name::<
            shrimply_property_model::VideoSampleMethod,
        >());
    }
    if path.ends_with("/effect/effect/config/mode") && mask_mode_target(project, target, path)? {
        return Ok(std::any::type_name::<
            shrimply_visual_modifiers::mask::MaskMode,
        >());
    }
    if let Some(timeline_type) = crate::generated::step_timeline_type(path) {
        return Ok(timeline_type);
    }
    if crate::paint::paint_taper_path(path) {
        return Ok(std::any::type_name::<
            shrimply_project_document::project::PaintTaper,
        >());
    }
    match path {
        crate::gaussian_3d::MODEL_ROTATION_ORDER_PATH => {
            Ok(std::any::type_name::<shrimply_3dgs_core::RotationOrder>())
        }
        "/sample_method" => Ok(std::any::type_name::<
            shrimply_property_model::VideoSampleMethod,
        >()),
        path if path.ends_with("/effect/effect/sample_method") => Ok(std::any::type_name::<
            shrimply_property_model::VideoSampleMethod,
        >()),
        "/compositing/blend_mode" => {
            Ok(std::any::type_name::<shrimply_property_model::LayerBlendMode>())
        }
        path if path.ends_with("/effect/effect/config/operation") => Ok(std::any::type_name::<
            shrimply_visual_modifiers::erode_dilate::ErodeDilateOperation,
        >()),
        path if path.ends_with("/effect/effect/config/pattern") => Ok(std::any::type_name::<
            shrimply_visual_modifiers::dithering::DitheringPattern,
        >()),
        path if path.ends_with("/effect/effect/config/color_mode") => Ok(std::any::type_name::<
            shrimply_visual_modifiers::dithering::DitheringColorMode,
        >()),
        path if path.ends_with("/effect/effect/config/version") => Ok(std::any::type_name::<
            shrimply_visual_modifiers::kuwahara::KuwaharaVersion,
        >()),
        path if path.ends_with("/effect/effect/config/row_offset_axis") => {
            Ok(std::any::type_name::<
                shrimply_visual_modifiers::repeat::RepeatOffsetAxis,
            >())
        }
        path if path.ends_with("/effect/effect/config/address_mode") => Ok(std::any::type_name::<
            shrimply_property_model::TextureAddressMode,
        >()),
        path if path.ends_with("/effect/effect/config/mode") => Ok(std::any::type_name::<
            shrimply_visual_modifiers::halftone::HalftoneMode,
        >()),
        _ => Err(format!("unknown step timeline: {path}")),
    }
}

fn sampling_method_target(
    project: &shrimply_project_document::project::Project,
    target: &InspectorTarget,
    path: &str,
) -> Result<bool, String> {
    let item = project
        .video_item(video_item_address(target)?)
        .ok_or_else(|| "video item is no longer available".to_string())?;
    Ok(crate::visual_modifiers::is_sampling_method(item, path))
}

fn mask_mode_target(
    project: &shrimply_project_document::project::Project,
    target: &InspectorTarget,
    path: &str,
) -> Result<bool, String> {
    let item = project
        .video_item(video_item_address(target)?)
        .ok_or_else(|| "video item is no longer available".to_string())?;
    Ok(crate::visual_modifiers::is_mask_mode(item, path))
}
