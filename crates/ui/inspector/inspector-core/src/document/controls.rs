//! Initialize typed control storage independently of its native widget toolkit.
use crate::{ControlKind, InspectorControl, InspectorController, InspectorTarget};

impl InspectorController {
    pub fn ensure_control_timeline(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
    ) -> Result<(), String> {
        if control.kind == ControlKind::LayeredBoolean
            && control.path.starts_with("/content/layers/")
        {
            return self.ensure_image_layer_control(target, control);
        }
        if control.audio_modifier {
            return Ok(());
        }
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        let layered = matches!(
            control.kind,
            ControlKind::LayeredNumber
                | ControlKind::LayeredSelector
                | ControlKind::LayeredVector2
                | ControlKind::LayeredVector3
                | ControlKind::LayeredColor
                | ControlKind::LayeredText
                | ControlKind::LayeredDrawing
        );
        let modifier = path.starts_with("/modifiers/");
        let mask = path.contains("/alpha_mask/");
        if (modifier || (mask && layered))
            && let Some(id) = control.target_id
        {
            self.ensure_visual_modifier(target, path, id)?;
        }
        let transform = matches!(
            (
                crate::transform::TransformField::from_path(path),
                control.kind
            ),
            (
                Some(crate::transform::TransformField::Vec2(_)),
                ControlKind::LayeredVector2
            ) | (
                Some(crate::transform::TransformField::Scalar(_)),
                ControlKind::LayeredNumber
            )
        );
        let required = if modifier {
            layered
        } else if path.starts_with("/content/generator/") {
            path != "/content/generator/kind" || layered
        } else {
            crate::generated::is_timeline_control(control)
                || (layered
                    && (mask
                        || transform
                        || crate::paint::is_timeline_path(path)
                        || path.starts_with("/content/model/")
                        || path.starts_with("/content/camera/")
                        || crate::scene_3d::is_timeline_path(path)))
        };
        if !required {
            return Ok(());
        }
        let Some(id) = control.timeline_id else {
            return Ok(());
        };
        if modifier {
            match control.kind {
                ControlKind::LayeredNumber => self.ensure_visual_modifier_number(target, path, id),
                ControlKind::LayeredText => self.ensure_visual_modifier_text(target, path, id),
                ControlKind::LayeredVector2 => {
                    self.ensure_visual_modifier_vector2(target, path, id)
                }
                ControlKind::LayeredVector3 => {
                    self.ensure_visual_modifier_vector3(target, path, id)
                }
                _ => self.ensure_timeline(target, path, id),
            }
        } else {
            self.ensure_timeline(target, path, id)
        }
    }

    pub(super) fn set_basic_alpha_mask(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        value: &str,
    ) -> Option<Result<(), String>> {
        let field = control.path.rsplit_once("/alpha_mask/")?.1;
        let mask = if control.path.starts_with("/compositing/alpha_mask/") {
            shrimply_project_document::project::VisualAlphaMaskTarget::Compositing
        } else if control.path.starts_with("/modifiers/") {
            let Some(id) = control.target_id else {
                return Some(Err("alpha-mask modifier target is unavailable".into()));
            };
            shrimply_project_document::project::VisualAlphaMaskTarget::Modifier(id)
        } else {
            return None;
        };
        match field {
            "shape" => Some(
                crate::alpha_mask::SHAPE_CHOICES
                    .iter()
                    .find_map(|(shape, key, _)| (*key == value).then_some(*shape))
                    .ok_or_else(|| format!("invalid alpha-mask shape: {value}"))
                    .and_then(|shape| self.set_alpha_mask_shape(target, mask, shape)),
            ),
            "invert" => Some(
                value
                    .parse()
                    .map_err(|_| format!("invalid alpha-mask inversion: {value}"))
                    .and_then(|invert| self.set_alpha_mask_inverted(target, mask, invert)),
            ),
            _ => None,
        }
    }
}

impl InspectorController {
    pub fn populate_background_control(
        &self,
        target: &InspectorTarget,
        control: &mut InspectorControl,
        server_url: &str,
    ) -> Result<(), String> {
        match control.kind {
            ControlKind::Analysis => {
                let status = self
                    .control_analysis_presentation(
                        target,
                        control.action.ok_or("analysis control has no action")?,
                        server_url,
                    )?
                    .ok_or("analysis action has no presentation")?;
                if status.active() {
                    self.observe_analysis_transition(
                        target,
                        control.action.expect("analysis action was resolved"),
                        &status,
                    );
                }
                control.sensitive = status.sensitive;
                control.busy = status.active();
                control.value = status.label.clone();
                control.tooltip = status.tooltip.as_str().into();
                control.analysis = Some(status);
            }
            ControlKind::AudioCache | ControlKind::VisualCache => {
                let id = control.target_id.ok_or("cache control has no target")?;
                let status = self.cache_status(target, control.kind, id);
                if control.kind == ControlKind::AudioCache {
                    let mut active = self.audio_cache_transitions.borrow_mut();
                    if active.tracked(target.clone(), id).is_none() {
                        active.observe(target.clone(), id, status.clone());
                    }
                }
                let status = crate::cache_control_presentation(status, "").analysis();
                control.value = status.label.clone();
                control.tooltip = status.tooltip.as_str().into();
                control.busy = status.active();
                control.analysis = Some(status);
            }
            ControlKind::BeatDetection => control.busy = self.audio_beat_detection_loading(target),
            ControlKind::VisualCacheQuality if self.analysis_backend.visual_cache_images => {
                control.kind = ControlKind::ReadOnly;
                control.value = shrimply_i18n::text("Lossless image frames").into_owned();
                control.values.clear();
                control.labels.clear();
                control.editable = false;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn activate_control(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        server_url: &str,
    ) -> Result<(), String> {
        if !control.sensitive || !control.action_sensitive {
            return Err("inspector action is unavailable".into());
        }
        match control.kind {
            ControlKind::AudioCache => {
                return self.toggle_audio_cache(
                    target,
                    control.target_id.ok_or("cache target is required")?,
                );
            }
            ControlKind::VisualCache => {
                return self.toggle_visual_cache(
                    target,
                    control.target_id.ok_or("cache target is required")?,
                );
            }
            _ => {}
        }
        let action = control.action.ok_or("control has no action")?;
        if let Some(status) = self.control_analysis_presentation(target, action, server_url)?
            && !status.sensitive
        {
            return Err("analysis action is unavailable".into());
        }
        match action {
            crate::InspectorControlAction::ToggleSam2Analysis { modifier_id, .. } => {
                self.toggle_sam2_analysis(target, modifier_id, server_url.into())
            }
            crate::InspectorControlAction::ToggleCameraAnalysis => {
                self.toggle_camera_analysis(target, server_url.into())
            }
            _ => self.trigger_video_control_action(target, action),
        }
    }
}
