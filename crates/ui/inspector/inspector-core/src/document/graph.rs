use super::layered::{owner, timeline};
use crate::keyframe_graph::{
    FrameGraphAction, FrameGraphKeyMove, FrameGraphState, KeyframeGraph, KeyframePoint, RawSegment,
    SpeedSegment,
};
use crate::{
    AudioModifierKeyframeMove, ControlKind, InspectorCommit, InspectorControl, InspectorController,
    InspectorGraphKind, InspectorTarget,
};
use shrimply_project_document::project::Time;

pub fn frame_graph(control: &InspectorControl) -> Result<FrameGraphState, String> {
    let graph = control
        .scalar_graph
        .as_ref()
        .ok_or("inspector keyframe graph is unavailable")?;
    let points = graph
        .points
        .iter()
        .map(|p| KeyframePoint {
            time: p.time,
            value: p.value,
        })
        .collect();
    let model =
        match InspectorGraphKind::for_control(control.kind).ok_or("control is not a timeline")? {
            InspectorGraphKind::Raw => KeyframeGraph::RawValue {
                points,
                segments: graph
                    .segments
                    .iter()
                    .map(|s| {
                        Ok(RawSegment {
                            owner_id: s.owner_id,
                            start: s.start,
                            end: s.end,
                            start_value: s.start_value,
                            end_value: s.end_value,
                            interpolation: crate::keyframe_model::interpolation(s.interpolation)?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
                static_value: control.value.parse().unwrap_or_default(),
            },
            InspectorGraphKind::Step => KeyframeGraph::Step { points },
            InspectorGraphKind::Speed => KeyframeGraph::Speed {
                keys: graph.points.iter().map(|p| p.time).collect(),
                segments: graph
                    .segments
                    .iter()
                    .map(|s| {
                        Ok(SpeedSegment {
                            owner_id: s.owner_id,
                            start: s.start,
                            end: s.end,
                            value: s.start_value,
                            interpolation: crate::keyframe_model::interpolation(s.interpolation)?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
                static_value: 0.0,
            },
        };
    let mut state = FrameGraphState::new(model, graph.range, graph.frame_step, graph.playhead);
    state.set_external_clipboard(true);
    state.set_text_interpolation(control.kind == ControlKind::LayeredText);
    Ok(state)
}

impl InspectorController {
    pub fn control_text_interpolation_menu(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        owner_id: uuid::Uuid,
    ) -> Result<InspectorControl, String> {
        self.ensure_control_timeline(target, control)?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        let selected =
            self.text_keyframe_text_interpolation(target, path, timeline(control)?, owner_id)?;
        Ok(crate::selector::selector(
            "",
            "Text interpolation",
            selected.to_string(),
            shrimply_property_model::timeline_value::TextInterpolation::ALL
                .into_iter()
                .enumerate()
                .map(|(index, mode)| (index.to_string(), mode.label().to_string())),
        ))
    }

    pub fn set_control_text_interpolation(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        owner_id: uuid::Uuid,
        selected: usize,
    ) -> Result<(), String> {
        self.ensure_control_timeline(target, control)?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        self.set_text_keyframe_text_interpolation(
            target,
            path,
            timeline(control)?,
            owner_id,
            selected,
            text_commits(control)?.text_interpolation,
        )
    }
    pub fn move_control_graph_keys(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        moves: &[FrameGraphKeyMove],
    ) -> Result<Vec<Time>, String> {
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        self.ensure_control_timeline(target, control)?;
        let times = moves
            .iter()
            .map(|m| (m.old_time, m.time))
            .collect::<Vec<_>>();
        let commit = InspectorCommit::Coalesced(control.keyframe_commits.map_or_else(
            || {
                if control.keyframe_commit_name.is_empty() {
                    "inspector-keyframe-move"
                } else {
                    &control.keyframe_commit_name
                }
            },
            |c| c.move_keyframe,
        ));
        if control.kind == ControlKind::LayeredNumber {
            let changes = moves
                .iter()
                .map(|m| AudioModifierKeyframeMove {
                    old_time: m.old_time,
                    time: m.time,
                    displayed_value: if control.audio_modifier || control.background_integer() {
                        control.store_number(if control.integer {
                            m.value.round()
                        } else {
                            m.value
                        })
                    } else {
                        control.map_number_for_storage(m.value)
                    },
                    store_multiplier: 1.0,
                })
                .collect::<Vec<_>>();
            return if control.audio_modifier {
                self.move_audio_modifier_keyframes(
                    target,
                    owner(control)?,
                    timeline(control)?,
                    &changes,
                )
            } else if control.scalar_storage == crate::section::ScalarStorage::UnsignedInteger {
                self.move_background_integer_keyframes(target, path, timeline(control)?, &changes)
            } else {
                self.move_scalar_keyframes(
                    target,
                    path,
                    &changes,
                    crate::NumberConstraint {
                        integer: control.integer || control.number_constraint.integer,
                        ..control.number_constraint
                    },
                    commit,
                )
            };
        }
        match control.kind {
            ControlKind::LayeredVector2 => {
                self.move_vector2_keyframes(target, path, &times, commit)
            }
            ControlKind::LayeredVector3 => {
                self.move_vector3_keyframes(target, path, &times, commit)
            }
            ControlKind::LayeredColor => {
                self.move_color_keyframes(target, path, timeline(control)?, &times, commit)
            }
            ControlKind::LayeredText => self.move_text_keyframes(
                target,
                path,
                timeline(control)?,
                &times,
                text_commits(control)?,
            ),
            ControlKind::LayeredDrawing => {
                self.move_paint_drawing_keyframes(target, timeline(control)?, &times)
            }
            ControlKind::LayeredBoolean => self.move_bool_keyframes(target, path, &times),
            ControlKind::LayeredSelector => self.move_step_keyframes(target, path, &times, commit),
            _ => Err("control has no movable keyframes".into()),
        }
    }

    pub fn apply_control_graph_action(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        action: FrameGraphAction,
    ) -> Result<(), String> {
        self.ensure_control_timeline(target, control)?;
        match action {
            FrameGraphAction::PlayheadChanged(time) => {
                if control.audio_modifier {
                    self.seek_audio_modifier_keyframe(target, time)
                } else if InspectorGraphKind::uses_discrete_seek(control.kind) {
                    self.seek_discrete_keyframe(target, time)
                } else {
                    self.seek_scalar_keyframe(target, time)
                }
            }
            FrameGraphAction::TogglePlayback => {
                self.toggle_keyframe_playback();
                Ok(())
            }
            FrameGraphAction::EditFinished => self.finish_live_inspector_edit(target),
            FrameGraphAction::KeysMoved(moves) => self
                .move_control_graph_keys(target, control, &moves)
                .map(|_| ()),
            FrameGraphAction::KeyAdded(point) => {
                self.add_control_keyframe(target, control, point.time)
            }
            FrameGraphAction::KeysDeleted(times) => {
                self.delete_control_keyframes(target, control, &times)
            }
            FrameGraphAction::CopyRequested(times) => self
                .copy_control_keyframes(target, control, &times)
                .map(|_| ()),
            FrameGraphAction::PasteRequested(time) => self
                .paste_control_keyframes(target, control, time)
                .map(|_| ()),
            FrameGraphAction::InterpolationRequested {
                owner_id,
                interpolation,
                ..
            } => {
                let index = shrimply_property_model::timeline_value::Interpolation::KEYFRAME
                    .iter()
                    .position(|i| *i == interpolation)
                    .ok_or("unsupported interpolation")?;
                self.set_control_interpolation(target, control, owner_id, index)
            }
            FrameGraphAction::TextInterpolationRequested { .. } => {
                Err("text interpolation requires selecting an interpolation".into())
            }
            FrameGraphAction::KeysChanged(_) | FrameGraphAction::KeysPasted(_) => {
                Err("inspector graphs use semantic keyframe actions".into())
            }
        }
    }

    pub fn add_control_keyframe(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        time: Time,
    ) -> Result<(), String> {
        self.ensure_control_timeline(target, control)?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        let name = control.keyframe_commits.map_or_else(
            || {
                if control.keyframe_commit_name.is_empty() {
                    "inspector-keyframe-add"
                } else {
                    &control.keyframe_commit_name
                }
            },
            |c| c.add,
        );
        let commit = InspectorCommit::Immediate(name);
        if control.audio_modifier {
            return self.add_audio_modifier_keyframe(
                target,
                owner(control)?,
                timeline(control)?,
                time,
            );
        }
        if control.scalar_storage == crate::section::ScalarStorage::UnsignedInteger {
            return self.add_background_integer_keyframe(target, path, timeline(control)?, time);
        }
        match control.kind {
            ControlKind::LayeredNumber => {
                self.add_scalar_keyframe(target, path, time, control.number_constraint, commit)
            }
            ControlKind::LayeredVector2 => self.add_vector2_keyframe(target, path, time, commit),
            ControlKind::LayeredVector3 => self.add_vector3_keyframe(target, path, time, commit),
            ControlKind::LayeredColor => {
                self.add_color_keyframe(target, path, timeline(control)?, time, commit)
            }
            ControlKind::LayeredText => self.add_text_keyframe(
                target,
                path,
                timeline(control)?,
                time,
                text_commits(control)?,
            ),
            ControlKind::LayeredDrawing => {
                self.add_paint_drawing_keyframe(target, timeline(control)?, time)
            }
            ControlKind::LayeredBoolean => self.add_bool_keyframe(target, path, time),
            ControlKind::LayeredSelector => self.add_step_keyframe(target, path, time, commit),
            _ => Err("control does not support this keyframe operation".into()),
        }
    }

    pub fn delete_control_keyframe(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        time: Time,
    ) -> Result<(), String> {
        self.delete_control_keyframes(target, control, std::slice::from_ref(&time))
    }

    pub fn delete_control_keyframes(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        times: &[Time],
    ) -> Result<(), String> {
        self.ensure_control_timeline(target, control)?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        let name = control.keyframe_commits.map_or_else(
            || {
                if control.keyframe_commit_name.is_empty() {
                    "inspector-keyframe-delete"
                } else {
                    &control.keyframe_commit_name
                }
            },
            |c| c.delete,
        );
        let commit = InspectorCommit::Immediate(name);
        if control.audio_modifier {
            return self.delete_audio_modifier_keyframes(
                target,
                owner(control)?,
                timeline(control)?,
                times,
            );
        }
        if control.scalar_storage == crate::section::ScalarStorage::UnsignedInteger {
            return self.delete_background_integer_keyframes(
                target,
                path,
                timeline(control)?,
                times,
            );
        }
        match control.kind {
            ControlKind::LayeredNumber => self.delete_scalar_keyframes(target, path, times, commit),
            ControlKind::LayeredVector2 => {
                self.delete_vector2_keyframes(target, path, times, commit)
            }
            ControlKind::LayeredVector3 => {
                self.delete_vector3_keyframes(target, path, times, commit)
            }
            ControlKind::LayeredColor => {
                self.delete_color_keyframes(target, path, timeline(control)?, times, commit)
            }
            ControlKind::LayeredText => self.delete_text_keyframes(
                target,
                path,
                timeline(control)?,
                times,
                text_commits(control)?,
            ),
            ControlKind::LayeredDrawing => {
                self.delete_paint_drawing_keyframes(target, timeline(control)?, times)
            }
            ControlKind::LayeredBoolean => self.delete_bool_keyframes(target, path, times),
            ControlKind::LayeredSelector => self.delete_step_keyframes(target, path, times, commit),
            _ => Err("control does not support this keyframe operation".into()),
        }
    }

    pub fn copy_control_keyframes(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        times: &[Time],
    ) -> Result<usize, String> {
        self.ensure_control_timeline(target, control)?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        if control.audio_modifier {
            return self.copy_audio_modifier_keyframes(
                target,
                owner(control)?,
                timeline(control)?,
                times,
            );
        }
        if control.scalar_storage == crate::section::ScalarStorage::UnsignedInteger {
            return self.copy_background_integer_keyframes(target, path, timeline(control)?, times);
        }
        match control.kind {
            ControlKind::LayeredNumber => self.copy_scalar_keyframes(target, path, times),
            ControlKind::LayeredVector2 => self.copy_vector2_keyframes(target, path, times),
            ControlKind::LayeredVector3 => self.copy_vector3_keyframes(target, path, times),
            ControlKind::LayeredColor => {
                self.copy_color_keyframes(target, path, timeline(control)?, times)
            }
            ControlKind::LayeredText => {
                self.copy_text_keyframes(target, path, timeline(control)?, times)
            }
            ControlKind::LayeredDrawing => {
                self.copy_paint_drawing_keyframes(target, timeline(control)?, times)
            }
            ControlKind::LayeredBoolean => self.copy_bool_keyframes(target, path, times),
            ControlKind::LayeredSelector => self.copy_step_keyframes(target, path, times),
            _ => Err("control does not support this keyframe operation".into()),
        }
    }

    pub fn paste_control_keyframes(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        time: Time,
    ) -> Result<usize, String> {
        self.ensure_control_timeline(target, control)?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        let name = control.keyframe_commits.map_or_else(
            || {
                if control.keyframe_commit_name.is_empty() {
                    "inspector-keyframe-paste"
                } else {
                    &control.keyframe_commit_name
                }
            },
            |c| c.paste,
        );
        let commit = InspectorCommit::Immediate(name);
        if control.audio_modifier {
            return self.paste_audio_modifier_keyframes(
                target,
                owner(control)?,
                timeline(control)?,
                time,
            );
        }
        if control.scalar_storage == crate::section::ScalarStorage::UnsignedInteger {
            return self.paste_background_integer_keyframes(target, path, timeline(control)?, time);
        }
        match control.kind {
            ControlKind::LayeredNumber => {
                self.paste_scalar_keyframes(target, path, time, control.number_constraint, commit)
            }
            ControlKind::LayeredVector2 => self.paste_vector2_keyframes(target, path, time, commit),
            ControlKind::LayeredVector3 => self.paste_vector3_keyframes(target, path, time, commit),
            ControlKind::LayeredColor => {
                self.paste_color_keyframes(target, path, timeline(control)?, time, commit)
            }
            ControlKind::LayeredText => self.paste_text_keyframes(
                target,
                path,
                timeline(control)?,
                time,
                text_commits(control)?,
            ),
            ControlKind::LayeredDrawing => {
                self.paste_paint_drawing_keyframes(target, timeline(control)?, time)
            }
            ControlKind::LayeredBoolean => self.paste_bool_keyframes(target, path, time),
            ControlKind::LayeredSelector => self.paste_step_keyframes(target, path, time, commit),
            _ => Err("control does not support this keyframe operation".into()),
        }
    }

    pub fn set_control_interpolation(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        owner_id: uuid::Uuid,
        index: usize,
    ) -> Result<(), String> {
        self.ensure_control_timeline(target, control)?;
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        let name = control.keyframe_commits.map_or_else(
            || {
                if control.keyframe_commit_name.is_empty() {
                    "inspector-keyframe-interpolation"
                } else {
                    &control.keyframe_commit_name
                }
            },
            |c| c.interpolation,
        );
        let commit = InspectorCommit::Immediate(name);
        if control.audio_modifier {
            return self.set_audio_modifier_keyframe_interpolation(
                target,
                owner(control)?,
                timeline(control)?,
                owner_id,
                index,
            );
        }
        if control.scalar_storage == crate::section::ScalarStorage::UnsignedInteger {
            return self.set_background_integer_interpolation(
                target,
                path,
                timeline(control)?,
                owner_id,
                index,
            );
        }
        match control.kind {
            ControlKind::LayeredNumber => {
                self.set_scalar_keyframe_interpolation(target, path, owner_id, index, commit)
            }
            ControlKind::LayeredVector2 => {
                self.set_vector2_interpolation(target, path, owner_id, index, commit)
            }
            ControlKind::LayeredVector3 => {
                self.set_vector3_interpolation(target, path, owner_id, index, commit)
            }
            ControlKind::LayeredColor => self.set_color_interpolation(
                target,
                path,
                timeline(control)?,
                owner_id,
                index,
                commit,
            ),
            ControlKind::LayeredText => self.set_text_keyframe_interpolation(
                target,
                path,
                timeline(control)?,
                owner_id,
                index,
                text_commits(control)?.interpolation,
            ),
            ControlKind::LayeredDrawing => {
                self.set_paint_drawing_interpolation(target, timeline(control)?, owner_id, index)
            }
            _ => Err("control does not support this keyframe operation".into()),
        }
    }
}

fn text_commits(control: &InspectorControl) -> Result<crate::TextKeyframeCommits, String> {
    control
        .text_keyframe_commits
        .ok_or_else(|| "text keyframe commits are unavailable".into())
}

/// Localized feedback for native keyframe clipboard operations.
pub fn confirmation(count: usize, pasted: bool) -> Option<String> {
    (count > 0).then(|| match (count, pasted) {
        (1, false) => shrimply_i18n::text("1 keyframe copied").into_owned(),
        (1, true) => shrimply_i18n::text("1 keyframe pasted").into_owned(),
        (_, false) => {
            shrimply_i18n::text_args("%{count} keyframes copied", &[("count", count.to_string())])
        }
        (_, true) => {
            shrimply_i18n::text_args("%{count} keyframes pasted", &[("count", count.to_string())])
        }
    })
}
