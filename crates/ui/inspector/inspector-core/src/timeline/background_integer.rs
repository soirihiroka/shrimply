use shrimply_project_document::project::Time;
use shrimply_property_model::timeline_value::{
    CurveEditPolicy, CurveKeyframeInsert, TimelineBase, TimelineValue, TimelineValueType,
    edit_curve_value,
};

use super::scalar_item_address;
use crate::{
    AudioModifierKeyframeMove, InspectorCommit, InspectorController, InspectorExpressionOutput,
    InspectorTarget,
};

struct BackgroundIntegerUpdate<'a> {
    kind: crate::model::EditKind,
    commit: InspectorCommit<'a>,
    refresh_inspector: bool,
}

impl InspectorController {
    pub fn set_background_integer_keyframes_enabled(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        enabled: bool,
    ) -> Result<(), String> {
        let (mut value, runtime) = self.background_integer_timeline(target, path, timeline_id)?;
        let current = value.value_at(
            runtime
                .local_time
                .ok_or_else(|| "background integer evaluation time is unavailable".to_string())?,
        );
        let keyframe_time = runtime
            .keyframe_playhead
            .ok_or_else(|| "background integer keyframe time is unavailable".to_string())?;
        if !crate::keyframe_model::set_keyframes_enabled(
            &mut value,
            keyframe_time,
            current,
            enabled,
        ) {
            return Ok(());
        }
        let (_, keyframe_commit, _) = integer_commits(path)?;
        self.replace_background_integer(
            target,
            path,
            timeline_id,
            value,
            BackgroundIntegerUpdate {
                kind: crate::model::EditKind::Structural,
                commit: InspectorCommit::Immediate(keyframe_commit),
                refresh_inspector: true,
            },
        )
    }

    pub fn set_background_integer_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        next: u32,
    ) -> Result<(), String> {
        let (mut value, runtime) = self.background_integer_timeline(target, path, timeline_id)?;
        let time = runtime
            .keyframe_playhead
            .ok_or_else(|| "background integer keyframe time is unavailable".to_string())?;
        let changed = match &mut value.base {
            TimelineBase::Const(current) if *current == next => false,
            TimelineBase::Const(current) => {
                *current = next;
                true
            }
            TimelineBase::Keyframes(keyframes) => {
                if let Some(keyframe) = keyframes.iter_mut().find(|keyframe| {
                    crate::keyframe_model::same_frame(keyframe.time, time, runtime.frame_step)
                }) {
                    let changed = keyframe.time != time || keyframe.value != next;
                    keyframe.time = time;
                    keyframe.value = next;
                    keyframes.sort_by_key(|keyframe| keyframe.time);
                    changed
                } else {
                    keyframes.push(u32::keyframe(time, next));
                    keyframes.sort_by_key(|keyframe| keyframe.time);
                    true
                }
            }
        };
        if !changed {
            return Ok(());
        }
        let (edit_commit, _, _) = integer_commits(path)?;
        self.replace_background_integer(
            target,
            path,
            timeline_id,
            value,
            BackgroundIntegerUpdate {
                kind: crate::model::EditKind::Live,
                commit: InspectorCommit::Coalesced(edit_commit),
                refresh_inspector: false,
            },
        )
    }

    pub fn commit_background_integer_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<(), String> {
        self.background_integer_timeline(target, path, timeline_id)?;
        let (edit_commit, _, _) = integer_commits(path)?;
        let project = self.project.borrow();
        shrimply_project_document::project::commit_edit(&project, edit_commit);
        Ok(())
    }

    pub fn set_background_integer_expression_enabled(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        enabled: bool,
    ) -> Result<(), String> {
        let (mut value, _) = self.background_integer_timeline(target, path, timeline_id)?;
        if !crate::keyframe_model::set_expression_enabled(&mut value, enabled, "x") {
            return Ok(());
        }
        let (_, _, expression_commit) = integer_commits(path)?;
        self.replace_background_integer(
            target,
            path,
            timeline_id,
            value,
            BackgroundIntegerUpdate {
                kind: crate::model::EditKind::Structural,
                commit: InspectorCommit::Immediate(expression_commit),
                refresh_inspector: true,
            },
        )
    }

    pub fn set_background_integer_expression_source(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        source: String,
    ) -> Result<(), String> {
        let (mut value, _) = self.background_integer_timeline(target, path, timeline_id)?;
        let Some(expression) = &mut value.expression else {
            return Err("background integer expression is not enabled".to_string());
        };
        if !expression.update(Some(source), None) {
            return Ok(());
        }
        let (_, _, expression_commit) = integer_commits(path)?;
        self.replace_background_integer(
            target,
            path,
            timeline_id,
            value,
            BackgroundIntegerUpdate {
                kind: crate::model::EditKind::Live,
                commit: InspectorCommit::Coalesced(expression_commit),
                refresh_inspector: false,
            },
        )
    }

    pub fn background_integer_expression_output(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<InspectorExpressionOutput<u32>, String> {
        self.background_integer_timeline(target, path, timeline_id)?;
        self.video_modifier_expression_output(target, path, timeline_id, video_integer_value)
    }

    pub fn background_integer_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<u32, String> {
        let (value, runtime) = self.background_integer_timeline(target, path, timeline_id)?;
        let time = runtime
            .local_time
            .ok_or_else(|| "background integer evaluation time is unavailable".to_string())?;
        Ok(value.value_at(time))
    }

    pub fn background_integer_graph(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<crate::ScalarGraph>, String> {
        let (value, runtime) = self.background_integer_timeline(target, path, timeline_id)?;
        Ok(crate::background::integer_graph(&value, runtime))
    }

    pub fn move_background_integer_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        change: AudioModifierKeyframeMove,
    ) -> Result<(), String> {
        self.move_background_integer_keyframes(
            target,
            path,
            timeline_id,
            std::slice::from_ref(&change),
        )
        .map(|_| ())
    }

    pub fn move_background_integer_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        changes: &[AudioModifierKeyframeMove],
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
                    background_integer(change.displayed_value * change.store_multiplier)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let (mut value, _) = self.background_integer_timeline(target, path, timeline_id)?;
        if !crate::keyframe_model::update_keyframes(&mut value, &changes) {
            return Err("integer keyframe move contains stale source keys".into());
        }
        let (_, keyframe_commit, _) = integer_commits(path)?;
        self.replace_background_integer(
            target,
            path,
            timeline_id,
            value,
            BackgroundIntegerUpdate {
                kind: crate::model::EditKind::Live,
                commit: InspectorCommit::Coalesced(keyframe_commit),
                refresh_inspector: false,
            },
        )?;
        Ok(changes.iter().map(|change| change.1).collect())
    }

    pub fn delete_background_integer_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        time: Time,
    ) -> Result<(), String> {
        self.delete_background_integer_keyframes(
            target,
            path,
            timeline_id,
            std::slice::from_ref(&time),
        )
    }

    pub fn delete_background_integer_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        times: &[Time],
    ) -> Result<(), String> {
        let (mut value, _) = self.background_integer_timeline(target, path, timeline_id)?;
        if !crate::keyframe_model::edit_keyframe_selection(&mut value, times, |value, time| {
            Ok(crate::keyframe_model::delete_discrete_keyframe(
                value,
                time,
                Time::ZERO,
            ))
        })? {
            return Ok(());
        }
        let (_, keyframe_commit, _) = integer_commits(path)?;
        self.replace_background_integer(
            target,
            path,
            timeline_id,
            value,
            BackgroundIntegerUpdate {
                kind: crate::model::EditKind::Structural,
                commit: InspectorCommit::Immediate(keyframe_commit),
                refresh_inspector: true,
            },
        )
    }

    pub fn add_background_integer_keyframe(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        time: Time,
    ) -> Result<(), String> {
        let time = self.canonical_scalar_keyframe_time(target, time)?;
        let (mut value, _) = self.background_integer_timeline(target, path, timeline_id)?;
        if !matches!(value.base, TimelineBase::Keyframes(_)) {
            return Ok(());
        }
        let current = value.value_at(time);
        edit_curve_value(
            &mut value,
            time,
            current,
            |_, _| false,
            CurveEditPolicy {
                unchanged_keyframe_is_noop: false,
                insert: CurveKeyframeInsert::InheritPreviousInterpolation,
            },
        );
        let (_, keyframe_commit, _) = integer_commits(path)?;
        self.replace_background_integer(
            target,
            path,
            timeline_id,
            value,
            BackgroundIntegerUpdate {
                kind: crate::model::EditKind::Structural,
                commit: InspectorCommit::Immediate(keyframe_commit),
                refresh_inspector: true,
            },
        )
    }

    pub fn set_background_integer_interpolation(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        owner_id: uuid::Uuid,
        interpolation_index: usize,
    ) -> Result<(), String> {
        let interpolation = crate::keyframe_model::interpolation(interpolation_index)?;
        let (mut value, _) = self.background_integer_timeline(target, path, timeline_id)?;
        if !crate::keyframe_model::set_interpolation(&mut value, owner_id, interpolation) {
            return Ok(());
        }
        let (_, keyframe_commit, _) = integer_commits(path)?;
        self.replace_background_integer(
            target,
            path,
            timeline_id,
            value,
            BackgroundIntegerUpdate {
                kind: crate::model::EditKind::Structural,
                commit: InspectorCommit::Immediate(keyframe_commit),
                refresh_inspector: true,
            },
        )
    }

    pub fn copy_background_integer_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        selected: &[Time],
    ) -> Result<usize, String> {
        let (value, _) = self.background_integer_timeline(target, path, timeline_id)?;
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

    pub fn paste_background_integer_keyframes(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        time: Time,
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
        let (mut value, _) = self.background_integer_timeline(target, path, timeline_id)?;
        let Some(pasted) = crate::keyframe_model::paste_keyframes(&mut value, &clipboard, &times)
        else {
            return Ok(0);
        };
        let (_, keyframe_commit, _) = integer_commits(path)?;
        self.replace_background_integer(
            target,
            path,
            timeline_id,
            value,
            BackgroundIntegerUpdate {
                kind: crate::model::EditKind::Structural,
                commit: InspectorCommit::Immediate(keyframe_commit),
                refresh_inspector: true,
            },
        )?;
        Ok(pasted.len())
    }

    fn background_integer_timeline(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
    ) -> Result<(TimelineValue<u32>, crate::InspectorRuntime), String> {
        if integer_commits(path).is_err() {
            return Err("background integer path is invalid".to_string());
        }
        let snapshot = self.snapshot();
        if &snapshot.target != target {
            return Err("inspector target changed".to_string());
        }
        let value: TimelineValue<u32> = serde_json::from_value(
            snapshot
                .value
                .pointer(path)
                .cloned()
                .ok_or_else(|| format!("background integer is no longer available: {path}"))?,
        )
        .map_err(|error| format!("invalid background integer: {error}"))?;
        if value.id != timeline_id {
            return Err(format!("background integer is no longer available: {path}"));
        }
        Ok((value, snapshot.runtime))
    }

    fn replace_background_integer(
        &self,
        target: &InspectorTarget,
        path: &str,
        timeline_id: uuid::Uuid,
        value: TimelineValue<u32>,
        update: BackgroundIntegerUpdate<'_>,
    ) -> Result<(), String> {
        self.background_integer_timeline(target, path, timeline_id)?;
        self.replace_value_with_commit(
            target,
            update.kind,
            path,
            serde_json::to_value(value).expect("background integer timeline must serialize"),
            Some(shrimply_editor_state::player_state::ProjectChange {
                video: true,
                inspector: update.refresh_inspector,
                ..Default::default()
            }),
            update.commit,
        )
    }
}

fn background_integer(value: f64) -> Result<u32, String> {
    if value.is_finite() && value.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&value) {
        Ok(value as u32)
    } else {
        Err(format!("invalid background integer: {value}"))
    }
}

fn integer_commits(path: &str) -> Result<(&'static str, &'static str, &'static str), String> {
    if path.starts_with("/content/generator/") {
        Ok((
            crate::background::INTEGER_EDIT_COMMIT,
            crate::background::INTEGER_KEYFRAME_COMMIT,
            crate::background::INTEGER_EXPRESSION_COMMIT,
        ))
    } else if let Some(commits) = crate::generated::integer_commits(path) {
        Ok(commits)
    } else {
        Err(format!("unknown integer timeline: {path}"))
    }
}

fn video_integer_value<'a>(
    item: &'a shrimply_project_document::project::VideoItem,
    path: &str,
    timeline_id: uuid::Uuid,
) -> Option<&'a TimelineValue<u32>> {
    if path.starts_with("/content/generator/") {
        crate::background::integer_value(item, timeline_id)
    } else {
        crate::generated::integer_value(item, path, timeline_id)
    }
}
