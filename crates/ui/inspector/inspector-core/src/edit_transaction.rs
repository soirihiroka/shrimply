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
    control: Option<(InspectorTarget, crate::InspectorControl)>,
}

impl InspectorGraphEdit {
    pub fn begin_control(
        &mut self,
        controller: &InspectorController,
        target: &InspectorTarget,
        control: &crate::InspectorControl,
    ) -> Result<(), String> {
        if self
            .control
            .as_ref()
            .is_some_and(|(bound_target, bound_control)| {
                bound_target != target || !same_graph_control(bound_control, control)
            })
        {
            self.cancel(controller)?;
        }
        self.apply(
            controller,
            target,
            FrameGraphAction::EditStarted,
            |_| Ok(()),
        )?;
        self.control = Some((target.clone(), control.clone()));
        Ok(())
    }

    pub fn finish_control(&mut self, controller: &InspectorController) -> Result<(), String> {
        let control = self.control.take();
        self.finish(controller)?;
        if let Some((target, control)) = control {
            controller.apply_control_graph_action(
                &target,
                &control,
                FrameGraphAction::EditFinished,
            )?;
        }
        Ok(())
    }

    /// A native document refresh may retain values, but cannot transfer an in-flight
    /// gesture to a different target, timeline, modifier, or control instance.
    pub fn retain_control(
        &mut self,
        controller: &InspectorController,
        target: &InspectorTarget,
        control: Option<&crate::InspectorControl>,
    ) -> Result<bool, String> {
        let Some((bound_target, bound_control)) = self.control.as_ref() else {
            return Ok(false);
        };
        if bound_target == target
            && control
                .filter(|control| control.scalar_graph.is_some())
                .is_some_and(|control| same_graph_control(bound_control, control))
        {
            return Ok(false);
        }
        self.cancel(controller)?;
        Ok(true)
    }

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
        self.control = None;
        if let Some(transaction) = self.transaction.take() {
            transaction.finish(controller)?;
        }
        Ok(())
    }

    pub fn cancel(&mut self, controller: &InspectorController) -> Result<(), String> {
        self.control = None;
        if let Some(transaction) = self.transaction.take() {
            transaction.cancel(controller)?;
        }
        Ok(())
    }

    pub fn is_editing(&self) -> bool {
        self.transaction.is_some()
    }
}

fn same_graph_control(left: &crate::InspectorControl, right: &crate::InspectorControl) -> bool {
    left.kind == right.kind
        && left.path == right.path
        && left.timeline_path == right.timeline_path
        && left.timeline_id == right.timeline_id
        && left.target_id == right.target_id
        && left.audio_modifier == right.audio_modifier
}

pub struct InspectorEditTransaction {
    baseline: Project,
    target: InspectorTarget,
    path: PathBuf,
    revision: u64,
    position: shrimply_math_core::Time,
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
        let snapshot = player_state::snapshot(&self.player_state);
        Ok(InspectorEditTransaction {
            baseline: self.project.borrow().clone(),
            target: target.clone(),
            path: project::active_project_path(),
            revision: snapshot.revision,
            position: snapshot.position,
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
        self.apply_checked(controller, edit, true)
    }

    /// An advertised native editor may drain queued text after losing selection.
    /// The transaction remains bound to its original project, revision and target.
    pub fn apply_retained<T>(
        &mut self,
        controller: &InspectorController,
        edit: impl FnOnce(&InspectorController) -> Result<T, String>,
    ) -> Result<T, String> {
        let position = self.position;
        self.apply_checked(
            controller,
            |controller| crate::model::with_edit_position(position, || edit(controller)),
            false,
        )
    }

    fn apply_checked<T>(
        &mut self,
        controller: &InspectorController,
        edit: impl FnOnce(&InspectorController) -> Result<T, String>,
        selected: bool,
    ) -> Result<T, String> {
        self.validate(controller)?;
        if selected && controller.target() != self.target {
            return Err("Inspector selection changed".into());
        }
        if crate::model::target_value(&controller.project.borrow(), &self.target).is_none() {
            return Err("Inspector target is unavailable".into());
        }
        if selected {
            self.position = player_state::snapshot(&controller.player_state).position;
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
