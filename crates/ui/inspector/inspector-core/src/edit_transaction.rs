//! A live control gesture previews mutations and advances history exactly once.
use std::path::PathBuf;

use shrimply_editor_state::player_state;
use shrimply_project_document::project::{self, Project};

use crate::keyframe_graph::FrameGraphAction;
use crate::{InspectorController, InspectorTarget};

/// A graph drag uses the same provisional edit and rollback on every host.
#[derive(Default)]
pub struct InspectorGraphEdit {
    transaction: Option<InspectorEditTransaction>,
}

impl InspectorGraphEdit {
    pub fn apply(
        &mut self,
        controller: &InspectorController,
        target: &InspectorTarget,
        action: FrameGraphAction,
        edit: impl FnOnce(FrameGraphAction) -> Result<(), String>,
    ) -> Result<(), String> {
        match action {
            FrameGraphAction::EditStarted => {
                if self.transaction.is_none() {
                    self.transaction = Some(controller.begin_control_edit(target)?);
                }
                Ok(())
            }
            FrameGraphAction::KeysMoved(_) => {
                if let Some(transaction) = self.transaction.as_mut() {
                    transaction.apply(controller, |_| edit(action))
                } else {
                    edit(action)
                }
            }
            FrameGraphAction::EditCancelled => self.cancel(controller),
            FrameGraphAction::EditFinished => {
                self.finish(controller)?;
                edit(action)
            }
            FrameGraphAction::HistoryRequested { redo } => {
                self.finish(controller)?;
                player_state::change_history(
                    &controller.project,
                    &controller.player_state,
                    if redo { project::redo } else { project::undo },
                );
                Ok(())
            }
            action => {
                self.finish(controller)?;
                edit(action)
            }
        }
    }

    pub fn finish(&mut self, controller: &InspectorController) -> Result<(), String> {
        if let Some(transaction) = self.transaction.take() {
            transaction.finish(controller)?;
        }
        Ok(())
    }

    pub fn cancel(&mut self, controller: &InspectorController) -> Result<(), String> {
        if let Some(transaction) = self.transaction.take() {
            transaction.cancel(controller)?;
        }
        Ok(())
    }

    pub fn is_editing(&self) -> bool {
        self.transaction.is_some()
    }
}

pub struct InspectorEditTransaction {
    baseline: Project,
    target: InspectorTarget,
    path: PathBuf,
    revision: u64,
    history_revision: u64,
    changed: bool,
}

impl InspectorController {
    pub fn begin_control_edit(
        &self,
        target: &InspectorTarget,
    ) -> Result<InspectorEditTransaction, String> {
        if self.target() != *target {
            return Err("Inspector selection changed".into());
        }
        crate::model::target_value(&self.project.borrow(), target)
            .ok_or("Inspector target is unavailable")?;
        self.finish_live_edit();
        Ok(InspectorEditTransaction {
            baseline: self.project.borrow().clone(),
            target: target.clone(),
            path: project::active_project_path(),
            revision: player_state::snapshot(&self.player_state).revision,
            history_revision: project::history_revision(),
            changed: false,
        })
    }
}

impl InspectorEditTransaction {
    fn validate(&self, controller: &InspectorController) -> Result<(), String> {
        if project::active_project_path() != self.path
            || project::history_revision() != self.history_revision
            || player_state::snapshot(&controller.player_state).revision != self.revision
        {
            return Err("The project changed during the control gesture".into());
        }
        Ok(())
    }

    pub fn apply<T>(
        &mut self,
        controller: &InspectorController,
        edit: impl FnOnce(&InspectorController) -> Result<T, String>,
    ) -> Result<T, String> {
        self.validate(controller)?;
        if controller.target() != self.target {
            return Err("Inspector selection changed".into());
        }
        let result = project::with_provisional_edit(|| edit(controller));
        let revision = player_state::snapshot(&controller.player_state).revision;
        self.changed |= revision != self.revision;
        self.revision = revision;
        if let Err(error) = &result {
            self.changed = true;
            self.restore(controller)
                .map_err(|cleanup| format!("{error}; {cleanup}"))?;
        }
        result
    }

    pub fn finish(mut self, controller: &InspectorController) -> Result<(), String> {
        self.validate(controller)?;
        let project = controller.project.borrow();
        if let Err(error) = project.validate() {
            drop(project);
            self.restore(controller)?;
            return Err(format!("Invalid control edit: {error}"));
        }
        if self.changed {
            // Comparing the canonical model also avoids an undo entry for a drag
            // that returned to its starting value.
            let mut before =
                serde_json::to_value(&self.baseline).map_err(|error| error.to_string())?;
            let mut after = serde_json::to_value(&*project).map_err(|error| error.to_string())?;
            for field in [
                "cursor_position",
                "timeline_zoom",
                "expanded_sequence_paths",
            ] {
                before
                    .as_object_mut()
                    .expect("project is an object")
                    .remove(field);
                after
                    .as_object_mut()
                    .expect("project is an object")
                    .remove(field);
            }
            if before != after
                && let Err(error) = project::commit_edit_checked(&project, "inspector-control-drag")
            {
                drop(project);
                self.restore(controller)?;
                return Err(error);
            }
        }
        drop(project);
        controller.finish_live_edit();
        Ok(())
    }

    pub fn cancel(mut self, controller: &InspectorController) -> Result<(), String> {
        self.restore(controller)
    }

    fn restore(&mut self, controller: &InspectorController) -> Result<(), String> {
        self.validate(controller)?;
        if self.changed {
            let mut project = controller.project.borrow_mut();
            let cursor_position = project.cursor_position;
            let timeline_zoom = project.timeline_zoom;
            let expanded_sequence_paths = project.expanded_sequence_paths.clone();
            *project = self.baseline.clone();
            project.cursor_position = cursor_position;
            project.timeline_zoom = timeline_zoom;
            project.expanded_sequence_paths = expanded_sequence_paths;
            drop(project);
            player_state::refresh_project(
                &controller.player_state,
                player_state::ProjectChange {
                    duration: Some(self.baseline.duration()),
                    frame_rate: Some(self.baseline.fps),
                    video: true,
                    audio: true,
                    audio_waveforms: true,
                    audio_beats: true,
                    captions: true,
                    inspector: true,
                    ..Default::default()
                },
            );
            self.revision = player_state::snapshot(&controller.player_state).revision;
            self.changed = false;
        }
        Ok(())
    }
}
