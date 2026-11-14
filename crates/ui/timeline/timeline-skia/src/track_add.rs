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
    if action == TrackAddAction::Import {
        return Ok(TrackAddOutcome::Import);
    }

    let mut project_state = project.borrow().clone();
    let Some((track_address, item)) = shrimply_timeline_edit::prepare_track_add(
        &project_state,
        key,
        action,
        player_state::snapshot(player_state).position,
        settings,
    )?
    else {
        return Ok(TrackAddOutcome::Unchanged);
    };
    let Some(item_address) = project_state.insert_item(&track_address, item) else {
        return Ok(TrackAddOutcome::Unchanged);
    };
    let selected = selection_state::item_key(&project_state, &item_address)
        .expect("inserted track item must have a root item key");
    let duration = project_state.duration();
    shrimply_project_document::project::commit_edit_checked(
        &project_state,
        "create-generated-item",
    )?;
    *project.borrow_mut() = project_state;

    selection_state::set_selected_items(selection_state, vec![selected], Some(selected));
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
