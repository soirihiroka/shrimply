use crate::{
    ControlKind, InspectorCommit, InspectorControl, InspectorController, InspectorTarget,
    TimelineModeChange,
};

impl InspectorController {
    pub fn set_control_keyframes(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        enabled: bool,
    ) -> Result<(), String> {
        self.ensure_control_timeline(target, control)?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        let name = control
            .keyframe_commits
            .map_or(control.keyframe_commit_name.as_str(), |c| c.toggle);
        let commit = InspectorCommit::Immediate(if name.is_empty() {
            "inspector-keyframes"
        } else {
            name
        });
        if control.audio_modifier {
            return self.set_audio_modifier_keyframes_enabled(
                target,
                owner(control)?,
                timeline(control)?,
                enabled,
            );
        }
        if control.background_integer() {
            return self.set_background_integer_keyframes_enabled(
                target,
                path,
                timeline(control)?,
                enabled,
            );
        }
        match control.kind {
            ControlKind::LayeredDrawing => {
                self.set_paint_drawing_keyframes_enabled(target, timeline(control)?, enabled)
            }
            ControlKind::LayeredNumber
                if control.scalar_storage == crate::section::ScalarStorage::UnsignedInteger =>
            {
                self.set_timeline_mode_with_commit(
                    target,
                    path,
                    TimelineModeChange {
                        keyframes: true,
                        enabled,
                        current: serde_json::Value::from(
                            control
                                .value
                                .parse::<u32>()
                                .map_err(|_| "invalid integer")?,
                        ),
                        default_expression: control.kind.default_expression(),
                    },
                    commit,
                )
            }
            ControlKind::LayeredNumber => self.set_scalar_keyframes_enabled(
                target,
                path,
                enabled,
                control.number_constraint,
                commit,
            ),
            ControlKind::LayeredVector2 => {
                self.set_vector2_keyframes_enabled(target, path, enabled, commit)
            }
            ControlKind::LayeredVector3 => {
                self.set_vector3_keyframes_enabled(target, path, enabled, commit)
            }
            ControlKind::LayeredColor => {
                self.set_color_keyframes_enabled(target, path, timeline(control)?, enabled, commit)
            }
            ControlKind::LayeredText => {
                self.set_text_keyframes_enabled(target, path, timeline(control)?, enabled, commit)
            }
            ControlKind::LayeredBoolean => {
                self.set_bool_keyframes_enabled(target, path, enabled, commit)
            }
            ControlKind::LayeredSelector => {
                self.set_step_keyframes_enabled_with_commit(target, path, enabled, commit)
            }
            _ => Err("control does not support keyframes".into()),
        }
    }

    pub fn set_control_expression(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        enabled: bool,
    ) -> Result<(), String> {
        self.ensure_control_timeline(target, control)?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        if control.audio_modifier {
            return self.set_audio_modifier_expression_enabled(
                target,
                owner(control)?,
                path,
                enabled,
                control.kind.default_expression(),
            );
        }
        if control.background_integer() {
            return self.set_background_integer_expression_enabled(
                target,
                path,
                timeline(control)?,
                enabled,
            );
        }
        let name = expression_commit(control);
        let commit = InspectorCommit::Immediate(name);
        if control.kind == ControlKind::LayeredText {
            return self.set_text_expression_enabled(
                target,
                path,
                timeline(control)?,
                enabled,
                commit,
            );
        }
        if control.kind == ControlKind::LayeredDrawing {
            return self.set_paint_drawing_expression_enabled(target, timeline(control)?, enabled);
        }
        if let Some(field) = crate::transform::TransformField::from_path(path) {
            return self.set_transform_expression_enabled(
                target,
                field,
                timeline(control)?,
                enabled,
                commit,
            );
        }
        if control.kind == ControlKind::LayeredVector2 {
            return self.set_vector2_expression_enabled(target, path, enabled, commit);
        }
        self.set_timeline_mode_with_commit(
            target,
            path,
            TimelineModeChange {
                keyframes: false,
                enabled,
                current: serde_json::Value::Null,
                default_expression: control.kind.default_expression(),
            },
            commit,
        )
    }

    pub fn set_control_expression_source(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        source: &str,
    ) -> Result<(), String> {
        self.ensure_control_timeline(target, control)?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        if control.audio_modifier {
            return self.set_audio_modifier_expression_source(
                target,
                owner(control)?,
                path,
                source,
            );
        }
        if control.background_integer() {
            return self.set_background_integer_expression_source(
                target,
                path,
                timeline(control)?,
                source.to_string(),
            );
        }
        if control.kind == ControlKind::LayeredText {
            return self.set_text_expression_source(
                target,
                path,
                timeline(control)?,
                source.to_string(),
                InspectorCommit::Coalesced(expression_commit(control)),
            );
        }
        if control.kind == ControlKind::LayeredDrawing {
            return self.set_paint_drawing_expression_source(target, timeline(control)?, source);
        }
        if let Some(field) = crate::transform::TransformField::from_path(path) {
            return self.set_transform_expression_source(
                target,
                field,
                timeline(control)?,
                source.to_string(),
                InspectorCommit::Coalesced(expression_commit(control)),
            );
        }
        if control.kind == ControlKind::LayeredVector2 {
            return self.set_vector2_expression_source(
                target,
                path,
                source.to_string(),
                InspectorCommit::Coalesced(expression_commit(control)),
            );
        }
        self.set_expression_source_with_commit(
            target,
            path,
            source,
            InspectorCommit::Coalesced(expression_commit(control)),
        )
    }
}

fn expression_commit(control: &InspectorControl) -> &str {
    if control.expression_commit_name.is_empty() {
        "inspector-expression"
    } else {
        &control.expression_commit_name
    }
}

pub(super) fn timeline(control: &InspectorControl) -> Result<uuid::Uuid, String> {
    control
        .timeline_id
        .ok_or_else(|| "inspector timeline is unavailable".into())
}

pub(super) fn owner(control: &InspectorControl) -> Result<uuid::Uuid, String> {
    control
        .target_id
        .ok_or_else(|| "inspector control owner is unavailable".into())
}
