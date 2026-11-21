use crate::TrackAddAction;
use shrimply_player_core::{self as player_state, ProjectChange, SharedPlayerState};
use shrimply_project_document::Project;
use shrimply_timeline_edit::selection_state::SharedSelectionState;
use shrimply_timeline_edit::{TrackKey, TrackKind, selection_state};
use std::cell::RefCell;
use std::rc::Rc;

pub use shrimply_timeline_edit::TrackAddSettings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackAddOutcome {
    Import,
    Changed,
    Unchanged,
}

pub fn activate_track_add(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    key: TrackKey,
    action: TrackAddAction,
    settings: TrackAddSettings<'_>,
) -> TrackAddOutcome {
    activate_track_add_checked(
        project,
        player_state,
        selection_state,
        key,
        action,
        settings,
    )
    .expect("could not commit generated timeline item")
}

pub fn activate_track_add_checked(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    key: TrackKey,
    action: TrackAddAction,
    settings: TrackAddSettings<'_>,
) -> Result<TrackAddOutcome, String> {
    activate_track_add_checked_at(
        project,
        player_state,
        selection_state,
        key,
        action,
        settings,
        player_state::current_time(player_state),
    )
}

pub fn activate_track_add_checked_at(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    key: TrackKey,
    action: TrackAddAction,
    settings: TrackAddSettings<'_>,
    position: crate::project::Time,
) -> Result<TrackAddOutcome, String> {
    if action == TrackAddAction::Import {
        return Ok(TrackAddOutcome::Import);
    }

    let mut project_state = project.borrow().clone();
    let Some(item) = shrimply_timeline_edit::insert_track_add(
        &mut project_state,
        key,
        action,
        position,
        settings,
        None,
    )?
    else {
        return Ok(TrackAddOutcome::Unchanged);
    };
    let duration = project_state.duration();
    shrimply_project_document::project::commit_edit_checked(
        &project_state,
        "create-generated-item",
    )?;
    *project.borrow_mut() = project_state;

    selection_state::select_item_address(selection_state, &project.borrow(), item);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            audio: key.kind == TrackKind::Audio,
            audio_waveforms: key.kind == TrackKind::Audio,
            video: key.kind == TrackKind::Video,
            inspector: true,
            ..ProjectChange::default()
        },
    );
    Ok(TrackAddOutcome::Changed)
}
