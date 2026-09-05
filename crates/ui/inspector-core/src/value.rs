use serde_json::Value;
use shrimply_project::project::{ItemAddress, VideoStabilizationMethod};

use crate::InspectorTarget;
use crate::model::{EditKind, InspectorCommit, InspectorController, default_commit, duration_path};

impl InspectorController {
    pub fn set_field(
        &self,
        target: &InspectorTarget,
        path: &str,
        text: &str,
    ) -> Result<(), String> {
        if path == "/enabled"
            && let InspectorTarget::Track(address) = target
        {
            let enabled = text
                .parse::<bool>()
                .map_err(|_| format!("invalid track enabled state: {text}"))?;
            self.set_track_enabled(address, enabled)
        } else if path == "/stabilization_method"
            && matches!(target, InspectorTarget::Item(ItemAddress::Video { .. }))
        {
            self.set_video_stabilization_method(target, text)
        } else {
            self.set_regular_field(target, path, text)
        }
    }

    pub fn set_project_name(&self, name: &str) {
        let mut project = self.project.borrow_mut();
        if project.name == name {
            return;
        }
        project.name.clear();
        project.name.push_str(name);
        shrimply_project::project::commit_coalesced_edit(&project, "project-name");
        drop(project);
        shrimply_state::player_state::refresh_project(
            &self.player_state,
            shrimply_state::player_state::ProjectChange::default(),
        );
    }

    pub fn set_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        replacement: Value,
    ) -> Result<(), String> {
        self.replace_value(
            target,
            path,
            replacement,
            EditKind::Structural,
            crate::refresh::audio_path_change(target, path, EditKind::Structural),
        )
    }

    pub fn set_live_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        replacement: Value,
    ) -> Result<(), String> {
        self.replace_value(
            target,
            path,
            replacement,
            EditKind::Live,
            crate::refresh::audio_path_change(target, path, EditKind::Live),
        )
    }

    pub fn set_values(
        &self,
        target: &InspectorTarget,
        replacements: &[(String, Value)],
    ) -> Result<(), String> {
        self.edit_value_if_changed_with_refresh(
            target,
            EditKind::Structural,
            replacements.iter().any(|(path, _)| duration_path(path)),
            crate::refresh::audio_paths_change(
                target,
                replacements.iter().map(|(path, _)| path.as_str()),
                EditKind::Structural,
            ),
            |root| {
                let mut changed = false;
                for (path, replacement) in replacements {
                    let current = root
                        .pointer_mut(path)
                        .ok_or_else(|| format!("inspector field is no longer available: {path}"))?;
                    if current != replacement {
                        *current = replacement.clone();
                        changed = true;
                    }
                }
                Ok(changed)
            },
        )
    }

    pub(crate) fn set_live_keyframe_graph_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        replacement: Value,
    ) -> Result<(), String> {
        self.replace_value(
            target,
            path,
            replacement,
            EditKind::Live,
            crate::refresh::audio_scalar_graph_change(target),
        )
    }

    pub(crate) fn set_live_keyframe_graph_value_with_commit(
        &self,
        target: &InspectorTarget,
        path: &str,
        replacement: Value,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.replace_value_with_commit(
            target,
            EditKind::Live,
            path,
            replacement,
            crate::refresh::audio_scalar_graph_change(target),
            commit,
        )
    }

    pub fn set_optional_field(
        &self,
        target: &InspectorTarget,
        path: &str,
        value: Option<&str>,
    ) -> Result<(), String> {
        if path == "/language"
            && let InspectorTarget::Track(address) = target
        {
            self.set_caption_track_language(address, value)
        } else {
            self.set_value(
                target,
                path,
                value.map_or(Value::Null, |value| Value::String(value.to_string())),
            )
        }
    }

    pub fn set_optional_number_field(
        &self,
        target: &InspectorTarget,
        path: &str,
        value: Option<&str>,
    ) -> Result<(), String> {
        if path == "/alpha_mask_video"
            && matches!(target, InspectorTarget::Item(ItemAddress::Video { .. }))
        {
            let value = value
                .map(|value| {
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("invalid video alpha-mask stream: {value}"))
                })
                .transpose()?;
            return self.set_video_alpha_mask_stream(target, value);
        }
        let value = value
            .map(|value| {
                value
                    .parse::<u64>()
                    .map(serde_json::Number::from)
                    .map(Value::Number)
                    .map_err(|_| format!("invalid numeric inspector value: {value}"))
            })
            .transpose()?
            .unwrap_or(Value::Null);
        self.set_value(target, path, value)
    }

    pub fn set_video_stream(&self, target: &InspectorTarget, track_id: u32) -> Result<(), String> {
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("inspector target is not a video item".to_string());
        };
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        if item.track_id == track_id {
            return Ok(());
        }
        item.track_id = track_id;
        if item.alpha_mask_video == Some(track_id) {
            item.alpha_mask_video = None;
        }
        shrimply_project::project::commit_edit(&project, "video-stream");
        drop(project);
        shrimply_state::player_state::refresh_project(
            &self.player_state,
            crate::refresh::target_change(target, None, true),
        );
        Ok(())
    }

    fn set_video_alpha_mask_stream(
        &self,
        target: &InspectorTarget,
        stream: Option<u32>,
    ) -> Result<(), String> {
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("inspector target is not a video item".to_string());
        };
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        if stream == Some(item.track_id) {
            return Err("alpha-mask stream must differ from the video stream".to_string());
        }
        if item.alpha_mask_video == stream {
            return Ok(());
        }
        item.alpha_mask_video = stream;
        shrimply_project::project::commit_edit(&project, "video-alpha-mask-stream");
        drop(project);
        shrimply_state::player_state::refresh_project(
            &self.player_state,
            crate::refresh::target_change(target, None, true),
        );
        Ok(())
    }

    pub(crate) fn set_video_stabilization_method(
        &self,
        target: &InspectorTarget,
        text: &str,
    ) -> Result<(), String> {
        let InspectorTarget::Item(address) = target else {
            return Err("inspector target is not a video item".to_string());
        };
        let method: VideoStabilizationMethod =
            serde_json::from_value(Value::String(text.to_string()))
                .map_err(|error| format!("invalid inspector value: {error}"))?;
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        if item.stabilization_method() == method {
            return Ok(());
        }
        shrimply_video_cuda::video_stabilization::cancel(item);
        item.stabilize_video = !matches!(method, VideoStabilizationMethod::Off);
        item.stabilization_method = method;
        if item.stabilize_video {
            shrimply_video_cuda::video_stabilization::request(item);
        }
        shrimply_project::project::commit_edit(&project, "video-stabilization-method");
        drop(project);
        shrimply_state::player_state::refresh_project(
            &self.player_state,
            crate::refresh::target_change(target, None, true),
        );
        Ok(())
    }

    pub fn finish_live_edit(&self) {
        shrimply_project::project::finish_coalesced_edit();
    }

    pub fn finish_live_inspector_edit(&self, target: &InspectorTarget) -> Result<(), String> {
        let project = self.project.borrow();
        crate::model::target_value(&project, target)
            .ok_or_else(|| "inspector target is no longer available".to_string())?;
        drop(project);
        shrimply_project::project::finish_coalesced_edit();
        shrimply_state::player_state::refresh_project(
            &self.player_state,
            shrimply_state::player_state::ProjectChange {
                inspector: true,
                ..Default::default()
            },
        );
        Ok(())
    }

    fn replace_value(
        &self,
        target: &InspectorTarget,
        path: &str,
        replacement: Value,
        kind: EditKind,
        refresh: Option<shrimply_state::player_state::ProjectChange>,
    ) -> Result<(), String> {
        self.replace_value_with_commit(
            target,
            kind,
            path,
            replacement,
            refresh,
            default_commit(kind),
        )
    }

    pub(crate) fn replace_value_with_commit(
        &self,
        target: &InspectorTarget,
        kind: EditKind,
        path: &str,
        replacement: Value,
        refresh: Option<shrimply_state::player_state::ProjectChange>,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.edit_value_if_changed_with_refresh_and_commit(
            target,
            kind,
            duration_path(path),
            refresh,
            commit,
            |value| {
                let current = match path {
                    "" => value,
                    _ => value
                        .pointer_mut(path)
                        .ok_or_else(|| format!("inspector field is no longer available: {path}"))?,
                };
                if *current == replacement {
                    return Ok(false);
                }
                *current = replacement;
                Ok(true)
            },
        )
    }
}
