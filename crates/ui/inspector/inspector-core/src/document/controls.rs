//! Initialize typed control storage independently of its native widget toolkit.
use crate::{ControlKind, InspectorControl, InspectorController, InspectorTarget};

impl InspectorController {
    pub fn ensure_control_timeline(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
    ) -> Result<(), String> {
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
        let id = control
            .timeline_id
            .ok_or("inspector control timeline ID is unavailable")?;
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
