use super::*;

pub fn for_each_visible_track_row(
    project: &Project,
    view: TimelineViewState,
    height: f64,
    mut f: impl FnMut(&items::TrackRow, usize),
) {
    let (start_row, end_row) = visible_row_range(view, height);
    let rows = items::track_rows(project);
    for (row, track) in rows
        .iter()
        .enumerate()
        .take(end_row.min(rows.len()))
        .skip(start_row)
    {
        f(track, row);
    }
}

pub fn visible_row_range(view: TimelineViewState, height: f64) -> (usize, usize) {
    let top = view.scroll_y.max(0.0);
    let bottom = (height - RULER_HEIGHT + view.scroll_y).max(top);
    (
        (top / TRACK_HEIGHT).floor().max(0.0) as usize,
        (bottom / TRACK_HEIGHT).ceil().max(0.0) as usize,
    )
}

pub fn track_label_action_at(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
) -> Option<(TrackKey, TrackLabelAction)> {
    shrimply_timeline_paint_skia::track_controls::track_label_action_at(project, view.into(), x, y)
}

pub fn track_button_at(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
) -> Option<TrackButtonId> {
    let (key, action) = track_label_action_at(project, view, x, y)?;
    match action {
        TrackLabelAction::Select => None,
        _ => Some((key, action)),
    }
}

pub use shrimply_timeline_paint_skia::math::track_label_button_y;
pub use shrimply_timeline_paint_skia::track_controls::track_enabled;

pub fn toggle_track_enabled(project: &mut Project, key: TrackKey) -> bool {
    let enabled = match key.kind {
        TrackKind::Comment => project
            .comment_tracks
            .get_mut(key.track_index)
            .map(|track| &mut track.enabled),
        TrackKind::Caption => project
            .caption_tracks
            .get_mut(key.track_index)
            .map(|track| &mut track.enabled),
        TrackKind::Video => project
            .video_tracks
            .get_mut(key.track_index)
            .map(|track| &mut track.enabled),
        TrackKind::Audio => project
            .audio_tracks
            .get_mut(key.track_index)
            .map(|track| &mut track.enabled),
    };
    if let Some(enabled) = enabled {
        *enabled = !*enabled;
        true
    } else {
        false
    }
}

pub fn select_track(
    selection: &selection_state::SharedSelectionState,
    key: TrackKey,
    toggle: bool,
    extend: bool,
) {
    let mut selected = if toggle || extend {
        selection_state::selected_tracks(selection)
            .into_iter()
            .filter(|track| track.kind == key.kind)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if extend {
        let anchor = selection_state::focused_track(selection)
            .filter(|track| track.kind == key.kind)
            .unwrap_or(key);
        for track_index in
            anchor.track_index.min(key.track_index)..=anchor.track_index.max(key.track_index)
        {
            selected.push(TrackKey {
                kind: key.kind,
                track_index,
            });
        }
    } else if toggle {
        if let Some(index) = selected.iter().position(|track| *track == key) {
            selected.remove(index);
        } else {
            selected.push(key);
        }
    } else {
        selected.push(key);
    }
    selected.sort_by_key(|track| track.track_index);
    selected.dedup();
    let focused = selected
        .contains(&key)
        .then_some(key)
        .or(selected.last().copied());
    selection_state::set_selected_tracks(selection, selected, focused);
}
