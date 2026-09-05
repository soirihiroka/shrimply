use super::*;
use crate::project::ItemAddress;
use crate::timeline_operation::TimelineOperationContext;

pub(crate) fn select_item_in_context(
    context: &dyn TimelineOperationContext,
    project: &Project,
    selection_state: &SharedSelectionState,
    hit: ItemAddress,
    ctrl: bool,
    shift: bool,
) -> bool {
    assert!(
        context.contains_item(project, &hit),
        "selected item must belong to its operation context"
    );
    let mut selected = selection_state::selected_item_addresses(selection_state, project)
        .into_iter()
        .filter(|item| context.contains_item(project, item))
        .collect::<Vec<_>>();
    let members = if shift {
        vec![hit.clone()]
    } else {
        crate::items::expand_grouped_item_addresses(context, project, std::slice::from_ref(&hit))
    };

    if ctrl {
        if members.iter().all(|item| selected.contains(item)) {
            selected.retain(|item| !members.contains(item));
        } else {
            selected.extend(members);
        }
    } else if !selected.contains(&hit) || shift {
        selected = members;
    }

    let context_items = context.items(project);
    selected.sort_by_key(|item| {
        context_items
            .iter()
            .position(|candidate| candidate == item)
            .unwrap_or(usize::MAX)
    });
    selected.dedup();
    let focused = selected.contains(&hit).then_some(hit.clone());
    let hit_selected = focused.is_some();
    selection_state::set_selected_item_addresses(selection_state, project, selected, focused);
    hit_selected
}
pub(in crate::interaction) fn activate_track_button(
    runtime: &mut TimelineRuntime,
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

pub(crate) fn set_timeline_selection(
    project: &Project,
    selection_state: &SharedSelectionState,
    selected_items: Vec<ItemKey>,
    focused_item: Option<ItemKey>,
) {
    set_selection(project, selection_state, selected_items, focused_item, true);
}

pub(in crate::interaction) fn timeline_cut(
    project: &Project,
    selected_items: &[crate::project::ItemAddress],
    key: crate::project::ItemAddress,
    time: Time,
) -> TimelineCut {
    let seed = if selected_items.contains(&key) {
        selected_items
    } else {
        std::slice::from_ref(&key)
    };
    let context = SequenceTimeline::for_item(project, &key)
        .expect("cut item must have a valid operation scope");
    let mut keys = crate::items::expand_grouped_item_addresses(&context, project, seed);
    if !keys.contains(&key) {
        keys.push(key.clone());
    }
    keys.sort_by_key(|address| (address.track_id(), address.item_id()));
    keys.dedup();
    // tracing::debug!(
    // "timeline cut preview keys key={}#{}:{} time={:.3} keys={} elapsed_us={}",
    // key.kind.label(),
    // key.track_index,
    // key.item_index,
    // time,
    // keys.len(),
    // started.elapsed().as_micros()
    // );
    TimelineCut { key, time, keys }
}

pub(in crate::interaction) fn set_selection(
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

pub(in crate::interaction) fn select_track(
    selection_state: &SharedSelectionState,
    key: TrackKey,
    ctrl: bool,
    shift: bool,
) {
    let current_tracks = selected_timeline_tracks(selection_state);
    let current_item_count = selected_timeline_items(selection_state).len();
    tracing::debug!(
        "timeline track selection begin key_kind={} key_track={} ctrl={} shift={} previous_track_count={} previous_item_count={}",
        key.kind.label(),
        key.track_index,
        ctrl,
        shift,
        current_tracks.len(),
        current_item_count
    );
    shrimply_support::crash::set_context(format!(
        "timeline track selection begin key_kind={} key_track={} ctrl={} shift={}",
        key.kind.label(),
        key.track_index,
        ctrl,
        shift
    ));
    let mut selected_tracks = if ctrl || shift {
        current_tracks
            .iter()
            .copied()
            .filter(|track| track.kind == key.kind)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    if shift {
        let anchor = focused_timeline_track(selection_state)
            .filter(|track| track.kind == key.kind)
            .unwrap_or(key);
        let start = anchor.track_index.min(key.track_index);
        let end = anchor.track_index.max(key.track_index);
        for track_index in start..=end {
            selected_tracks.push(TrackKey {
                kind: key.kind,
                track_index,
            });
        }
    } else if ctrl {
        if let Some(index) = selected_tracks.iter().position(|track| *track == key) {
            selected_tracks.remove(index);
        } else {
            selected_tracks.push(key);
        }
    } else {
        selected_tracks.push(key);
    }

    selected_tracks.sort_by_key(track_key_sort_key);
    selected_tracks.dedup();
    let focused = selected_tracks.contains(&key).then_some(key);
    let focused_track = focused.or(selected_tracks.last().copied());
    let selected_for_state = selected_tracks.clone();
    let focused_for_state = focused_track;
    let focused_kind = focused_track.map(|key| key.kind.label()).unwrap_or("none");
    tracing::debug!(
        "timeline track selection commit selected_count={} focused_kind={} focused_track={:?} ctrl={} shift={}",
        selected_tracks.len(),
        focused_kind,
        focused_track.map(|key| key.track_index),
        ctrl,
        shift
    );
    shrimply_support::crash::set_context(format!(
        "timeline track selection commit selected_count={} focused_kind={} focused_track={:?} ctrl={} shift={}",
        selected_tracks.len(),
        focused_kind,
        focused_track.map(|key| key.track_index),
        ctrl,
        shift
    ));
    selection_state::set_selected_tracks(selection_state, selected_for_state, focused_for_state);
}

pub(in crate::interaction) fn focused_timeline_track(
    selection_state: &SharedSelectionState,
) -> Option<TrackKey> {
    selection_state::focused_track(selection_state)
}

pub(in crate::interaction) fn track_key_sort_key(key: &TrackKey) -> (u8, usize) {
    let kind = match key.kind {
        TrackKind::Caption => 0_u8,
        TrackKind::Video => 1_u8,
        TrackKind::Audio => 2_u8,
    };
    (kind, key.track_index)
}

pub(in crate::interaction) fn item_key_sort_key(key: &ItemKey) -> (u8, usize, usize) {
    let kind = match key.kind {
        TrackKind::Caption => 0_u8,
        TrackKind::Video => 1_u8,
        TrackKind::Audio => 2_u8,
    };
    (kind, key.track_index, key.item_index)
}
