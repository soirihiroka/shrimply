use super::*;

pub(crate) use crate::selection::select_item_in_context;
pub(in crate::scene) fn activate_track_button(
    runtime: &mut Scene,
    selection_state: &SharedSelectionState,
    (key, action): TrackButtonId,
) {
    match action {
        TrackLabelAction::Toggle => runtime.pending_track_toggle = Some(key),
        TrackLabelAction::AudioRecord => runtime.pending_audio_record = Some(key),
        TrackLabelAction::VideoRecord => runtime.pending_video_record = Some(key),
        TrackLabelAction::Add => {
            let selected_tracks = selected_timeline_tracks(selection_state);
            runtime.pending_track_add_menu = Some(TrackAddMenuRequest {
                key,
                import_targets: if selected_tracks.contains(&key) {
                    selected_tracks
                } else {
                    vec![key]
                },
            });
        }
        TrackLabelAction::Select => unreachable!("track selection is not a button"),
    }
}

pub fn set_timeline_selection(
    project: &Project,
    selection_state: &SharedSelectionState,
    selected_items: Vec<ItemKey>,
    focused_item: Option<ItemKey>,
) {
    set_selection(project, selection_state, selected_items, focused_item, true);
}

pub(in crate::scene) use crate::cutting::timeline_cut;

pub(in crate::scene) fn set_selection(
    project: &Project,
    selection_state: &SharedSelectionState,
    selected_items: Vec<ItemKey>,
    focused_item: Option<ItemKey>,
    expand_groups: bool,
) {
    let previous_item_count = selected_timeline_items(selection_state).len();
    let previous_track_count = selected_timeline_tracks(selection_state).len();
    let focused_kind = focused_item.map(|key| key.kind.label()).unwrap_or("none");
    tracing::debug!(
        "timeline item selection begin requested_count={} focused_kind={} focused_track={:?} focused_item={:?} expand_groups={} previous_item_count={} previous_track_count={}",
        selected_items.len(),
        focused_kind,
        focused_item.map(|key| key.track_index),
        focused_item.map(|key| key.item_index),
        expand_groups,
        previous_item_count,
        previous_track_count
    );
    shrimply_support::crash::set_context(format!(
        "timeline item selection begin requested_count={} focused_kind={} focused_track={:?} focused_item={:?}",
        selected_items.len(),
        focused_kind,
        focused_item.map(|key| key.track_index),
        focused_item.map(|key| key.item_index)
    ));
    let mut selected_items = if expand_groups {
        expand_grouped_selection(project, &selected_items)
    } else {
        selected_items
    };
    selected_items.sort_by_key(item_key_sort_key);
    selected_items.dedup();
    let selected_for_state = selected_items.clone();
    let focused_item = focused_item.filter(|item| selected_items.contains(item));
    let focused_for_state = focused_item;
    let focused_kind = focused_item.map(|key| key.kind.label()).unwrap_or("none");
    tracing::debug!(
        "timeline item selection commit selected_count={} focused_kind={} focused_track={:?} focused_item={:?} previous_item_count={} previous_track_count={}",
        selected_items.len(),
        focused_kind,
        focused_item.map(|key| key.track_index),
        focused_item.map(|key| key.item_index),
        previous_item_count,
        previous_track_count
    );
    shrimply_support::crash::set_context(format!(
        "timeline item selection commit selected_count={} focused_kind={} focused_track={:?} focused_item={:?}",
        selected_items.len(),
        focused_kind,
        focused_item.map(|key| key.track_index),
        focused_item.map(|key| key.item_index)
    ));
    selection_state::set_selected_items(selection_state, selected_for_state, focused_for_state);
}

pub(in crate::scene) use crate::track_controls::select_track;

pub(in crate::scene) fn item_key_sort_key(key: &ItemKey) -> (u8, usize, usize) {
    let kind = match key.kind {
        TrackKind::Caption => 0_u8,
        TrackKind::Video => 1_u8,
        TrackKind::Audio => 2_u8,
    };
    (kind, key.track_index, key.item_index)
}
