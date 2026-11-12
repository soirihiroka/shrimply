use super::*;

impl qobject::InspectorBackend {
    pub fn set_control_value(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        value: &QString,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        if let Err(error) = crate::ensure_control_timeline(&target, &control) {
            self.as_mut().finish(Err(error));
            return;
        }
        let value = value.to_string();
        if let Some(shrimply_inspector_core::InspectorControlAction::SetSam2Model { modifier_id }) =
            control.action
        {
            self.as_mut()
                .finish(crate::set_sam2_model(&target, modifier_id, &value));
            return;
        }
        if let Some(shrimply_inspector_core::InspectorControlAction::SetSam2PointLabel {
            modifier_id,
            point_id,
        }) = control.action
        {
            self.as_mut().finish(crate::set_sam2_point_label(
                &target,
                modifier_id,
                point_id,
                &value,
            ));
            return;
        }
        if control.kind == ControlKind::AudioModifierMenu {
            if value == "__paste__" {
                let result = crate::paste_audio_modifiers(&target).map(|count| {
                    (count > 0).then(|| {
                        if count == 1 {
                            "1 effect pasted".to_string()
                        } else {
                            format!("{count} effects pasted")
                        }
                    })
                });
                self.as_mut().finish_confirmation(result);
            } else {
                self.as_mut()
                    .finish(crate::add_audio_modifier(&target, &value));
            }
            return;
        }
        if control.kind == ControlKind::VisualModifierMenu {
            if value == "__paste__" {
                let result = crate::paste_visual_modifiers(&target).map(|count| {
                    (count > 0).then(|| {
                        if count == 1 {
                            "1 effect pasted".to_string()
                        } else {
                            format!("{count} effects pasted")
                        }
                    })
                });
                self.as_mut().finish_confirmation(result);
            } else {
                let result = crate::add_visual_modifier(&target, &value).map(|id| {
                    self.as_mut().rust_mut().list_state.set_expanded(
                        &target,
                        &format!("modifier:{id}"),
                        true,
                    );
                });
                self.as_mut().finish(result);
            }
            return;
        }
        if control.kind == ControlKind::LayeredText {
            let result = control
                .timeline_id
                .ok_or_else(|| "text timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    crate::set_text_value(
                        &target,
                        &control.path,
                        timeline_id,
                        value,
                        &control.commit_name,
                    )
                });
            self.as_mut().finish(result);
            return;
        }
        if let Some(result) =
            crate::set_manim_text_field(&target, &control.path, &value, &control.commit_name)
        {
            self.as_mut().finish(result);
            return;
        }
        if let Some(result) = crate::named_control_edit(&target, &control, value.clone()) {
            self.as_mut().finish(result);
            return;
        }
        let result = if control.kind == ControlKind::AudioCachePreset {
            control
                .target_id
                .ok_or_else(|| "audio cache control has no modifier target".to_string())
                .and_then(|id| crate::set_audio_cache_preset(&target, id, &value))
        } else if control.kind == ControlKind::VisualCacheQuality {
            control
                .target_id
                .ok_or_else(|| "visual cache control has no modifier target".to_string())
                .and_then(|id| crate::set_visual_cache_quality(&target, id, &value))
        } else if control.kind == ControlKind::LayeredBoolean {
            value
                .parse::<bool>()
                .map_err(|_| format!("invalid timeline boolean: {value}"))
                .and_then(|value| crate::set_bool_value(&target, &control.path, value))
        } else if matches!(
            control.kind,
            ControlKind::LayeredNumber | ControlKind::LayeredSelector
        ) {
            control_value(&control, &value).and_then(|value| {
                if crate::graph_backend::background_integer(&control) {
                    let value = serde_json::from_value::<u32>(value)
                        .map_err(|error| format!("invalid background integer: {error}"))?;
                    crate::set_background_integer_value(
                        &target,
                        &control.path,
                        control.timeline_id.ok_or_else(|| {
                            "background integer timeline ID is unavailable".to_string()
                        })?,
                        value,
                    )
                } else if control.audio_modifier {
                    control
                        .target_id
                        .ok_or_else(|| "audio modifier target is unavailable".to_string())
                        .and_then(|id| {
                            control
                                .timeline_id
                                .ok_or_else(|| {
                                    "audio modifier timeline ID is unavailable".to_string()
                                })
                                .and_then(|timeline_id| {
                                    crate::set_audio_modifier_timeline_base(
                                        &target,
                                        id,
                                        timeline_id,
                                        value,
                                    )
                                })
                        })
                } else {
                    let commit_name = control
                        .keyframe_commits
                        .filter(|_| control.layered.keyframes)
                        .map_or(control.commit_name.as_str(), |commits| commits.edit);
                    crate::set_timeline_base(
                        &target,
                        &control.path,
                        value,
                        commit_name,
                        control.commit_immediately,
                    )
                }
            })
        } else if control.kind == ControlKind::OptionalSelector {
            crate::set_optional_field(
                &target,
                &control.path,
                (!value.is_empty()).then_some(value.as_str()),
            )
        } else if control.kind == ControlKind::OptionalNumberSelector {
            crate::set_optional_number_field(
                &target,
                &control.path,
                (!value.is_empty()).then_some(value.as_str()),
            )
        } else {
            let value = if control.kind == ControlKind::Number
                && (control.store_multiplier != 1.0
                    || control.number_mapping != shrimply_inspector_core::NumberMapping::Linear)
            {
                value
                    .parse::<f64>()
                    .map(|value| control.store_number(value).to_string())
                    .map_err(|_| format!("invalid numeric inspector value: {value}"))
            } else {
                Ok(value)
            };
            value.and_then(|value| {
                if control.audio_modifier {
                    control
                        .target_id
                        .ok_or_else(|| "audio modifier target is unavailable".to_string())
                        .and_then(|id| {
                            if control.kind == ControlKind::Number {
                                crate::set_audio_modifier_live_field(
                                    &target,
                                    id,
                                    &control.path,
                                    &value,
                                )
                            } else {
                                crate::set_audio_modifier_field(&target, id, &control.path, &value)
                            }
                        })
                } else {
                    crate::set_field(&target, &control.path, &value)
                }
            })
        };
        self.as_mut().finish(result);
    }

    pub fn trigger_control_action(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
    ) {
        let control_index = control;
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        if control.action.is_some() && !control.action_sensitive {
            return;
        }
        let result = if let Some(action) = control.action {
            match action {
                shrimply_inspector_core::InspectorControlAction::SelectObject3dModel {
                    modifier_id,
                } => crate::select_object_3d_model(&target, modifier_id),
                shrimply_inspector_core::InspectorControlAction::SelectScene3dEnvironment => {
                    crate::select_scene_3d_environment(&target)
                }
                shrimply_inspector_core::InspectorControlAction::SelectPaintTexture {
                    color_id,
                } => crate::select_paint_texture(&target, color_id),
                shrimply_inspector_core::InspectorControlAction::ToggleSam2Analysis {
                    modifier_id,
                    ..
                } => crate::toggle_sam2_analysis(&target, modifier_id),
                shrimply_inspector_core::InspectorControlAction::ToggleTransparentFillAnalysis {
                    modifier_id,
                } => crate::toggle_transparent_fill_analysis(&target, modifier_id),
                shrimply_inspector_core::InspectorControlAction::ToggleCameraAnalysis => {
                    crate::toggle_camera_analysis(&target)
                }
                action => crate::trigger_video_control_action(&target, action),
            }
        } else {
            match control.kind {
                ControlKind::AudioCache => control
                    .target_id
                    .ok_or_else(|| "audio cache control has no modifier target".to_string())
                    .and_then(|id| crate::toggle_audio_cache(&target, id)),
                ControlKind::VisualCache => control
                    .target_id
                    .ok_or_else(|| "visual cache control has no modifier target".to_string())
                    .and_then(|id| crate::toggle_visual_cache(&target, id)),
                ControlKind::Analysis => Err("analysis control has no action".to_string()),
                ControlKind::Action => Err("inspector action control has no action".to_string()),
                _ => Err("inspector control does not have an action".to_string()),
            }
        };
        let refresh_analysis = result.is_ok() && control.kind == ControlKind::Analysis;
        self.as_mut().finish(result);
        if refresh_analysis {
            self.as_mut()
                .poll_analysis_control(category, item, control_index);
        }
    }

    pub fn trigger_secondary_control_action(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        let result = control
            .secondary_action
            .ok_or_else(|| "inspector control does not have a secondary action".to_string())
            .and_then(|action| crate::trigger_video_control_action(&target, action));
        self.as_mut().finish(result);
    }

    pub fn set_control_fraction(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        numerator: i64,
        denominator: i64,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        let result = crate::set_control_fraction(&target, &control, numerator, denominator);
        self.as_mut().finish(result);
    }

    #[expect(clippy::too_many_arguments, reason = "QML slot signature")]
    pub fn set_control_components(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        first: f64,
        second: f64,
        third: f64,
        _changed: i32,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        if let Err(error) = crate::ensure_control_timeline(&target, &control) {
            self.as_mut().finish(Err(error));
            return;
        }
        if let Some(shrimply_inspector_core::InspectorControlAction::SetSam2PointPosition {
            modifier_id,
            point_id,
        }) = control.action
        {
            self.as_mut().finish(crate::set_sam2_point_position(
                &target,
                modifier_id,
                point_id,
                first,
                second,
            ));
            return;
        }
        let count = if matches!(
            control.kind,
            ControlKind::Vector3 | ControlKind::LayeredVector3
        ) {
            3
        } else {
            2
        };
        if control.kind == ControlKind::LayeredVector2 {
            let commit_name = control
                .keyframe_commits
                .filter(|_| control.layered.keyframes)
                .map_or(control.commit_name.as_str(), |commits| commits.edit);
            self.as_mut().finish(crate::set_vector2_value(
                &target,
                &control.path,
                first * control.store_multiplier,
                second * control.store_multiplier,
                commit_name,
            ));
            return;
        }
        if control.kind == ControlKind::LayeredVector3 {
            self.as_mut().finish(crate::set_vector3_value(
                &target,
                &control.path,
                first * control.store_multiplier,
                second * control.store_multiplier,
                third * control.store_multiplier,
                &control.commit_name,
            ));
            return;
        }
        let values = [first, second, third]
            .into_iter()
            .take(count)
            .enumerate()
            .map(|(component, value)| (component, (value * control.store_multiplier).to_string()))
            .collect::<Vec<_>>();
        self.as_mut()
            .finish(crate::set_components(&target, &control.path, &values));
    }

    #[expect(clippy::too_many_arguments, reason = "QML slot signature")]
    pub fn set_control_color(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        red: f64,
        green: f64,
        blue: f64,
        alpha: f64,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        let alpha = if control.with_alpha { alpha } else { 1.0 };
        let channels =
            [red, green, blue, alpha].map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8);
        let values = channels.map(f64::from);
        let result = crate::with_controller(|controller| {
            controller.set_basic_control_components(&target, &control, &values)
        });
        if result.is_ok() {
            crate::mark_dirty();
        }
        self.as_mut().finish(result);
    }

    pub fn commit_control(mut self: Pin<&mut Self>, category: i32, item: i32, control: i32) {
        if let Some((target, control)) = self.control_target(category, item, control) {
            self.as_mut()
                .finish(crate::commit_control_edit(&target, &control));
        }
        crate::mark_dirty();
    }

    pub fn set_control_keyframes(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        enabled: bool,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        if let Err(error) = crate::ensure_control_timeline(&target, &control) {
            self.as_mut().finish(Err(error));
            return;
        }
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        let keyframe_commit_name = control
            .keyframe_commits
            .map_or(control.keyframe_commit_name.as_str(), |commits| {
                commits.toggle
            });
        let result = if control.kind == ControlKind::LayeredDrawing {
            control
                .timeline_id
                .ok_or_else(|| "paint drawing timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    crate::set_paint_drawing_keyframes_enabled(&target, timeline_id, enabled)
                })
        } else if control.audio_modifier {
            control
                .target_id
                .ok_or_else(|| "audio modifier target is unavailable".to_string())
                .and_then(|id| {
                    control
                        .timeline_id
                        .ok_or_else(|| "audio modifier timeline ID is unavailable".to_string())
                        .and_then(|timeline_id| {
                            crate::set_audio_modifier_timeline_mode(
                                &target,
                                id,
                                timeline_id,
                                path,
                                shrimply_inspector_core::TimelineModeChange {
                                    keyframes: true,
                                    enabled,
                                    current: serde_json::Value::Null,
                                    default_expression: control.kind.default_expression(),
                                },
                            )
                        })
                })
        } else if control.kind == ControlKind::LayeredNumber {
            if crate::graph_backend::background_integer(&control) {
                control
                    .timeline_id
                    .ok_or_else(|| "background integer timeline ID is unavailable".to_string())
                    .and_then(|timeline_id| {
                        crate::set_background_integer_keyframes_enabled(
                            &target,
                            path,
                            timeline_id,
                            enabled,
                        )
                    })
            } else {
                crate::set_scalar_keyframes_enabled(
                    &target,
                    path,
                    enabled,
                    control.number_constraint,
                    keyframe_commit_name,
                )
            }
        } else if control.kind == ControlKind::LayeredVector2 {
            crate::set_vector2_keyframes_enabled(&target, path, enabled, keyframe_commit_name)
        } else if control.kind == ControlKind::LayeredVector3 {
            crate::set_vector3_keyframes_enabled(&target, path, enabled, keyframe_commit_name)
        } else if control.kind == ControlKind::LayeredColor {
            control
                .timeline_id
                .ok_or_else(|| "timeline color ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    crate::set_color_keyframes_enabled(
                        &target,
                        path,
                        timeline_id,
                        enabled,
                        keyframe_commit_name,
                    )
                })
        } else if control.kind == ControlKind::LayeredBoolean {
            crate::set_bool_keyframes_enabled(&target, path, enabled, keyframe_commit_name)
        } else if control.kind == ControlKind::LayeredText {
            control
                .timeline_id
                .ok_or_else(|| "text timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    crate::set_text_keyframes_enabled(
                        &target,
                        path,
                        timeline_id,
                        enabled,
                        keyframe_commit_name,
                    )
                })
        } else if control.kind == ControlKind::LayeredSelector {
            crate::set_step_keyframes_enabled(&target, path, enabled, keyframe_commit_name)
        } else {
            timeline_value(&control).and_then(|current| {
                crate::set_timeline_mode(
                    &target,
                    path,
                    true,
                    enabled,
                    current,
                    control.kind.default_expression(),
                    keyframe_commit_name,
                )
            })
        };
        self.as_mut().finish(result);
    }

    pub fn set_control_expression(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        enabled: bool,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        if let Err(error) = crate::ensure_control_timeline(&target, &control) {
            self.as_mut().finish(Err(error));
            return;
        }
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        let result = if control.kind == ControlKind::LayeredDrawing {
            crate::set_timeline_mode(
                &target,
                path,
                false,
                enabled,
                serde_json::Value::Null,
                control.kind.default_expression(),
                &control.expression_commit_name,
            )
        } else if control.audio_modifier {
            control
                .target_id
                .ok_or_else(|| "audio modifier target is unavailable".to_string())
                .and_then(|id| {
                    control
                        .timeline_id
                        .ok_or_else(|| "audio modifier timeline ID is unavailable".to_string())
                        .and_then(|timeline_id| {
                            crate::set_audio_modifier_timeline_mode(
                                &target,
                                id,
                                timeline_id,
                                path,
                                shrimply_inspector_core::TimelineModeChange {
                                    keyframes: false,
                                    enabled,
                                    current: serde_json::Value::Null,
                                    default_expression: control.kind.default_expression(),
                                },
                            )
                        })
                })
        } else if crate::graph_backend::background_integer(&control) {
            control
                .timeline_id
                .ok_or_else(|| "background integer timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    crate::set_background_integer_expression_enabled(
                        &target,
                        path,
                        timeline_id,
                        enabled,
                    )
                })
        } else if let Some(field) =
            shrimply_inspector_core::transform::TransformField::from_path(path)
        {
            control
                .timeline_id
                .ok_or_else(|| "transform timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    crate::set_transform_expression_enabled(&target, field, timeline_id, enabled)
                })
        } else if control.kind == ControlKind::LayeredVector2 {
            crate::set_vector2_expression_enabled(
                &target,
                path,
                enabled,
                &control.expression_commit_name,
            )
        } else if control.kind == ControlKind::LayeredText {
            control
                .timeline_id
                .ok_or_else(|| "text timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    crate::set_text_expression_enabled(
                        &target,
                        path,
                        timeline_id,
                        enabled,
                        &control.expression_commit_name,
                    )
                })
        } else {
            timeline_value(&control).and_then(|current| {
                crate::set_timeline_mode(
                    &target,
                    path,
                    false,
                    enabled,
                    current,
                    control.kind.default_expression(),
                    &control.expression_commit_name,
                )
            })
        };
        self.as_mut().finish(result);
    }

    pub fn set_control_expression_source(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        source: &QString,
    ) {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return;
        };
        if let Err(error) = crate::ensure_control_timeline(&target, &control) {
            self.as_mut().finish(Err(error));
            return;
        }
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        let source = source.to_string();
        let result = if control.audio_modifier {
            control
                .target_id
                .ok_or_else(|| "audio modifier target is unavailable".to_string())
                .and_then(|id| {
                    crate::set_audio_modifier_expression_source(&target, id, path, &source)
                })
        } else if crate::graph_backend::background_integer(&control) {
            control
                .timeline_id
                .ok_or_else(|| "background integer timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    crate::set_background_integer_expression_source(
                        &target,
                        path,
                        timeline_id,
                        source,
                    )
                })
        } else if let Some(field) =
            shrimply_inspector_core::transform::TransformField::from_path(path)
        {
            control
                .timeline_id
                .ok_or_else(|| "transform timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    crate::set_transform_expression_source(&target, field, timeline_id, source)
                })
        } else if control.kind == ControlKind::LayeredVector2 {
            crate::set_vector2_expression_source(
                &target,
                path,
                source,
                &control.expression_commit_name,
            )
        } else if control.kind == ControlKind::LayeredText {
            control
                .timeline_id
                .ok_or_else(|| "text timeline ID is unavailable".to_string())
                .and_then(|timeline_id| {
                    crate::set_text_expression_source(
                        &target,
                        path,
                        timeline_id,
                        &source,
                        &control.expression_commit_name,
                    )
                })
        } else {
            crate::set_expression_source(&target, path, &source, &control.expression_commit_name)
        };
        self.as_mut().finish(result);
    }

    pub fn apply_project_settings(
        mut self: Pin<&mut Self>,
        width: i32,
        height: i32,
        fps_numerator: &QString,
        fps_denominator: &QString,
    ) {
        self.as_mut().finish(crate::apply_project_settings(
            width,
            height,
            &fps_numerator.to_string(),
            &fps_denominator.to_string(),
        ));
    }
}
