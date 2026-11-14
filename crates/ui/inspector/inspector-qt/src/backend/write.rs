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
        let value = value.to_string();
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
        let result = crate::with_controller(|controller| {
            controller.set_basic_control_value(&target, &control, &value)
        });
        if result.is_ok() {
            crate::EXPRESSION_DIRTY.set(true);
            crate::GRAPH_DIRTY.set(true);
        }
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
        let result = if denominator <= 0 {
            Err("fraction denominator must be positive".to_string())
        } else {
            crate::with_controller(|controller| {
                controller.set_basic_control_fraction(
                    &target,
                    &control,
                    shrimply_math_core::fraction_new(numerator, denominator),
                )
            })
        };
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
        let values = if matches!(
            control.kind,
            ControlKind::Vector3 | ControlKind::LayeredVector3
        ) {
            vec![first, second, third]
        } else {
            vec![first, second]
        };
        let result = crate::with_controller(|controller| {
            controller.set_basic_control_components(&target, &control, &values)
        });
        if result.is_ok() {
            crate::EXPRESSION_DIRTY.set(true);
            crate::GRAPH_DIRTY.set(true);
        }
        self.as_mut().finish(result);
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
            self.as_mut().finish(crate::with_controller(|controller| {
                controller.commit_basic_control(&target, &control)
            }));
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
        let result = crate::with_controller(|controller| {
            controller.set_control_keyframes(&target, &control, enabled)
        });
        if result.is_ok() {
            crate::EXPRESSION_DIRTY.set(true);
            crate::GRAPH_DIRTY.set(true);
        }
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
        let result = crate::with_controller(|controller| {
            controller.set_control_expression(&target, &control, enabled)
        });
        if result.is_ok() {
            crate::EXPRESSION_DIRTY.set(true);
            crate::GRAPH_DIRTY.set(true);
        }
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
        let result = crate::with_controller(|controller| {
            controller.set_control_expression_source(&target, &control, &source.to_string())
        });
        if result.is_ok() {
            crate::EXPRESSION_DIRTY.set(true);
            crate::GRAPH_DIRTY.set(true);
        }
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
