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
    if !(0.0..timeline_x()).contains(&x) || y < RULER_HEIGHT {
        return None;
    }
    let content_y = y + view.scroll_y;
    let (kind, track_index, row) = items::track_at_y(project, content_y)?;
    let row_y = row_screen_y(row, view);
    let button_y = track_label_button_y(row_y);
    let key = TrackKey { kind, track_index };
    if hit_track_label_button(x, y, TRACK_LABEL_TOGGLE_X, button_y) {
        Some((key, TrackLabelAction::Toggle))
    } else if hit_track_label_button(x, y, TRACK_LABEL_ADD_X, button_y) {
        Some((key, TrackLabelAction::Add))
    } else if hit_track_label_button(x, y, TRACK_LABEL_RECORD_X, button_y) {
        match kind {
            TrackKind::Audio => Some((key, TrackLabelAction::AudioRecord)),
            TrackKind::Video => Some((key, TrackLabelAction::VideoRecord)),
            TrackKind::Caption => Some((key, TrackLabelAction::Select)),
        }
    } else {
        Some((key, TrackLabelAction::Select))
    }
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

fn hit_track_label_button(x: f64, y: f64, button_x: f64, button_y: f64) -> bool {
    shrimply_skia_adw_core::button::Button::hit_test(
        shrimply_skia_adw_core::Rect::from_xywh(
            button_x as f32,
            button_y as f32,
            TRACK_LABEL_BUTTON_SIZE as f32,
            TRACK_LABEL_BUTTON_SIZE as f32,
        ),
        glam::Vec2::new(x as f32, y as f32),
    )
}

pub fn track_label_button_y(row_y: f64) -> f64 {
    row_y + ((TRACK_HEIGHT - TRACK_LABEL_BUTTON_SIZE) / 2.0).max(0.0)
}

pub fn track_enabled(project: &Project, key: TrackKey) -> bool {
    match key.kind {
        TrackKind::Caption => project
            .caption_tracks
            .get(key.track_index)
            .is_some_and(|track| track.enabled),
        TrackKind::Video => project
            .video_tracks
            .get(key.track_index)
            .is_some_and(|track| track.enabled),
        TrackKind::Audio => project
            .audio_tracks
            .get(key.track_index)
            .is_some_and(|track| track.enabled),
    }
}

pub fn toggle_track_enabled(project: &mut Project, key: TrackKey) -> bool {
    let enabled = match key.kind {
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
