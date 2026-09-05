use crate::{VideoFrameSelection, selection_state};
use shrimply_math_core::Time;
use shrimply_project::project::Project;
use shrimply_state::player_state::{self, SharedPlayerState};
use shrimply_timeline::{TrackKind, selection_state::SharedSelectionState};
use std::{cell::RefCell, rc::Rc};

pub fn prepare_selected_video_frame(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection: &SharedSelectionState,
    kind: VideoFrameSelection,
) -> (Project, Time, Vec<uuid::Uuid>) {
    let project = project.borrow().clone();
    let item_ids = match kind {
        VideoFrameSelection::Items => selection_state::selected_items(selection)
            .iter()
            .filter(|key| key.kind == TrackKind::Video)
            .filter_map(|key| {
                project
                    .video_tracks
                    .get(key.track_index)
                    .and_then(|track| track.items.get(key.item_index))
                    .map(|item| item.id)
            })
            .collect(),
        VideoFrameSelection::Tracks => selection_state::selected_tracks(selection)
            .iter()
            .filter(|key| key.kind == TrackKind::Video)
            .filter_map(|key| project.video_tracks.get(key.track_index))
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect(),
    };
    (
        project,
        player_state::snapshot(player_state).position,
        item_ids,
    )
}

/// Preserve track ordering, enabled state, canvas settings and nested sequence dependencies.
pub fn selected_video_project(
    project: &Project,
    item_ids: &[uuid::Uuid],
) -> Result<Project, String> {
    if item_ids.is_empty() {
        return Err("No video is selected.".into());
    }
    let mut selected = project.clone();
    for track in &mut selected.video_tracks {
        track.items.retain(|item| item_ids.contains(&item.id));
    }
    selected.caption_tracks.clear();
    Ok(selected)
}
