use super::*;
use crate::project::{AudioSource, ItemAddress};
use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};

const UNPLAYED_OVERLAY_ALPHA: f64 = 0.48;

pub(super) fn draw_expanded_video_tracks(
    draw: &TimelineDraw<'_>,
    input: super::tracks::TrackDrawInput<'_>,
    track_index: usize,
    parent_row: usize,
    content_height: f64,
) {
    let project = input.project;
    let selected_items = input.selected_nested_items;
    let folded_drag = input.folded_drag;
    let transition_drag = input.transition_drag;
    let clip_transition_drag = input.clip_transition_drag;
    let focused_transition = input.focused_transition;
    let (first_visible_row, last_visible_row) = visible_row_range(draw.view, content_height);
    let mut tracks = folded_sequence::projected_video_tracks(project, track_index, parent_row);
    let drag_played_range = folded_drag.and_then(|drag| {
        tracks
            .iter()
            .find(|track| {
                track.sequence_path == drag.key.sequence_path()
                    && track.track_id == drag.key.track_id()
            })
            .and_then(|track| {
                track
                    .items
                    .iter()
                    .find(|item| item.item.id == drag.key.item_id())
            })
            .and_then(|item| item.played_range)
    });
    for track in &mut tracks {
        let track_address = crate::project::TrackAddress::Video {
            sequence_path: track.sequence_path.clone(),
            track_id: track.track_id,
        };
        for preview in folded_drag
            .into_iter()
            .flat_map(|drag| &drag.items)
            .filter(|item| item.target_track == track_address && item.key.track() != track_address)
        {
            let Some(source) = project.video_item(&preview.key) else {
                continue;
            };
            let mut item = source.clone();
            item.start = preview.target_start;
            item.end = preview.target_end;
            let sequence_path =
                matches!(item.content, VideoItemContent::FoldedSequence(_)).then(|| {
                    let mut path = track.sequence_path.clone();
                    path.push(item.id);
                    path
                });
            track.items.push(folded_sequence::ProjectedVideoItem {
                item,
                sequence_path,
                played_range: drag_played_range,
            });
        }
    }
    for track in tracks {
        if track.row < first_visible_row || track.row >= last_visible_row {
            continue;
        }
        let y = row_screen_y(track.row, draw.view);
        let track_address = crate::project::TrackAddress::Video {
            sequence_path: track.sequence_path.clone(),
            track_id: track.track_id,
        };
        draw_expanded_row_background(draw.painter, y, draw.timeline_x, draw.timeline_width);
        draw_selected_track_fill(
            draw.painter,
            input.selected_tracks,
            &track_address,
            draw.timeline_x,
            y,
            draw.timeline_width,
        );
        let mut clip_transitions = Vec::new();
        for projected in track.items {
            let played_range = projected.played_range;
            let mut item = projected.item;
            let address = ItemAddress::Video {
                sequence_path: track.sequence_path.clone(),
                track_id: track.track_id,
                item_id: item.id,
            };
            let selected = selected_items.contains(&address);
            if folded_drag.is_some_and(|drag| {
                drag.kind == folded_sequence::FoldedDragKind::Move
                    && drag.preview(&address).is_some_and(|item| {
                        item.target_track != address.track()
                            || drag.cross_scope_preview_row.is_some()
                    })
            }) {
                continue;
            }
            if let Some(preview) = folded_drag.and_then(|drag| drag.preview_at(&address)) {
                item.start = preview.target_start;
                item.end = preview.target_end;
            }
            let item = &item;
            let sequence_expanded = projected
                .sequence_path
                .as_ref()
                .is_some_and(|path| folded_sequence::expanded(project, path));
            let (item_x, item_width) = item_rect(item.start, item.end, draw.timeline_x, draw.view);
            if item_x + item_width <= draw.timeline_x
                || item_x >= draw.timeline_x + draw.timeline_width
            {
                continue;
            }
            draw_video_item(
                draw.painter,
                NaturalEndMarker {
                    start: item.start,
                    end: item.end,
                    position: video_natural_end_marker(item),
                    repeat_interval: video_natural_end_interval(item),
                    real_start: video_real_start(item),
                    real_end: video_real_end(item),
                },
                if sequence_expanded {
                    Icon("folder-open-symbolic")
                } else {
                    video_item_icon(&item.content)
                },
                draw.timeline_x,
                y,
                draw.view,
                selected,
            );
            if !matches!(
                item.content,
                VideoItemContent::Obj(_) | VideoItemContent::Gaussian(_)
            ) {
                let (intro, outro) = transition_durations(project, &address).unwrap_or_default();
                draw_item_transitions(
                    draw.painter,
                    &address,
                    item.start,
                    item.end,
                    intro,
                    outro,
                    focused_transition,
                    transition_drag,
                    draw.timeline_x,
                    y,
                    draw.view,
                    Color::BLUE1,
                );
            }
            if let Some(transition) = item.transitions.to_next.as_ref()
                && let Some(context) = SequenceTimeline::for_item(project, &address)
                && let Some(duration) = context.timeline_clip_transition_duration(
                    project,
                    &address,
                    transition.duration,
                )
            {
                let drag_duration = clip_transition_drag
                    .filter(|drag| drag.outgoing == address)
                    .map(|drag| {
                        drag.target_duration.and_then(|duration| {
                            context.timeline_clip_transition_duration(project, &address, duration)
                        })
                    });
                clip_transitions.push((address.clone(), item.end, duration, drag_duration));
            }
            draw_unplayed_item_ranges(
                draw.painter,
                item.start,
                item.end,
                played_range,
                y,
                draw.timeline_x,
                draw.view,
            );
            if let Some(drag) = folded_drag.filter(|drag| drag.preview_at(&address).is_some()) {
                draw_item_drag_status_outline(
                    draw.painter,
                    (item.start, item.end),
                    draw.timeline_x,
                    y,
                    draw.view,
                    drag.valid_drop,
                    drag.preview_status,
                );
            }
        }
        for (address, cut, duration, drag_duration) in clip_transitions {
            draw_clip_transition(
                draw.painter,
                &address,
                cut,
                duration,
                focused_transition,
                drag_duration,
                draw.timeline_x,
                y,
                draw.view,
                Color::BLUE1,
            );
        }
        draw_track_divider(draw.painter, 0.0, y, draw.timeline_x + draw.timeline_width);
    }
}

pub(super) fn draw_expanded_audio_tracks(
    draw: &TimelineDraw<'_>,
    input: super::tracks::TrackDrawInput<'_>,
    track_index: usize,
    parent_row: usize,
    content_height: f64,
) {
    let project = input.project;
    let selected_items = input.selected_nested_items;
    let folded_drag = input.folded_drag;
    let transition_drag = input.transition_drag;
    let clip_transition_drag = input.clip_transition_drag;
    let focused_transition = input.focused_transition;
    let (first_visible_row, last_visible_row) = visible_row_range(draw.view, content_height);
    let mut tracks = folded_sequence::projected_audio_tracks(project, track_index, parent_row);
    let drag_played_range = folded_drag.and_then(|drag| {
        tracks
            .iter()
            .find(|track| {
                track.sequence_path == drag.key.sequence_path()
                    && track.track_id == drag.key.track_id()
            })
            .and_then(|track| {
                track
                    .items
                    .iter()
                    .find(|item| item.item.id == drag.key.item_id())
            })
            .and_then(|item| item.played_range)
    });
    for track in &mut tracks {
        let track_address = crate::project::TrackAddress::Audio {
            sequence_path: track.sequence_path.clone(),
            track_id: track.track_id,
        };
        for preview in folded_drag
            .into_iter()
            .flat_map(|drag| &drag.items)
            .filter(|item| item.target_track == track_address && item.key.track() != track_address)
        {
            let Some(source) = project.audio_item(&preview.key) else {
                continue;
            };
            let mut item = source.clone();
            item.start = preview.target_start;
            item.end = preview.target_end;
            let sequence_path = matches!(item.source, AudioSource::FoldedSequence(_)).then(|| {
                let mut path = track.sequence_path.clone();
                path.push(item.id);
                path
            });
            track.items.push(folded_sequence::ProjectedAudioItem {
                item,
                sequence_path,
                played_range: drag_played_range,
            });
        }
    }
    for track in tracks {
        if track.row < first_visible_row || track.row >= last_visible_row {
            continue;
        }
        let y = row_screen_y(track.row, draw.view);
        let track_address = crate::project::TrackAddress::Audio {
            sequence_path: track.sequence_path.clone(),
            track_id: track.track_id,
        };
        draw_expanded_row_background(draw.painter, y, draw.timeline_x, draw.timeline_width);
        draw_selected_track_fill(
            draw.painter,
            input.selected_tracks,
            &track_address,
            draw.timeline_x,
            y,
            draw.timeline_width,
        );
        let mut waveform_columns = vec![0.0; draw.timeline_width.ceil() as usize];
        let mut unplayed_ranges = Vec::new();
        let mut transitions = Vec::new();
        let mut clip_transitions = Vec::new();
        for projected in track.items {
            let played_range = projected.played_range;
            let mut item = projected.item;
            let address = ItemAddress::Audio {
                sequence_path: track.sequence_path.clone(),
                track_id: track.track_id,
                item_id: item.id,
            };
            let selected = selected_items.contains(&address);
            if folded_drag.is_some_and(|drag| {
                drag.kind == folded_sequence::FoldedDragKind::Move
                    && drag.preview(&address).is_some_and(|item| {
                        item.target_track != address.track()
                            || drag.cross_scope_preview_row.is_some()
                    })
            }) {
                continue;
            }
            if let Some(preview) = folded_drag.and_then(|drag| drag.preview_at(&address)) {
                item.start = preview.target_start;
                item.end = preview.target_end;
            }
            let item = &item;
            let sequence_expanded = projected
                .sequence_path
                .as_ref()
                .is_some_and(|path| folded_sequence::expanded(project, path));
            let (item_x, item_width) = item_rect(item.start, item.end, draw.timeline_x, draw.view);
            if item_x + item_width <= draw.timeline_x
                || item_x >= draw.timeline_x + draw.timeline_width
            {
                continue;
            }
            let detailed = item_width >= MIN_DETAILED_ITEM_WIDTH;
            if projected.sequence_path.is_some() {
                draw_audio_item_with_waveform(
                    draw,
                    item,
                    item.start,
                    y,
                    WaveformState::Loaded(None),
                    Color::GREEN5.alpha_multiply(0.55),
                    Color::GREEN1,
                    selected,
                    Color::GREEN1,
                    detailed || selected,
                    &mut waveform_columns,
                );
                draw_item_icon(
                    draw.painter,
                    rect(item_x, y, item_width, TRACK_HEIGHT),
                    if sequence_expanded {
                        Icon("folder-open-symbolic")
                    } else {
                        Icon("folder-symbolic")
                    },
                    Color::GREEN1,
                );
            } else {
                draw_audio_item_into(
                    draw,
                    item,
                    item.start,
                    y,
                    selected,
                    detailed || selected,
                    &mut waveform_columns,
                );
            }
            let (intro, outro) = transition_durations(project, &address).unwrap_or_default();
            transitions.push((address.clone(), item.start, item.end, intro, outro));
            if let Some(transition) = item.transitions.to_next.as_ref()
                && let Some(context) = SequenceTimeline::for_item(project, &address)
                && let Some(duration) = context.timeline_clip_transition_duration(
                    project,
                    &address,
                    transition.duration,
                )
            {
                let drag_duration = clip_transition_drag
                    .filter(|drag| drag.outgoing == address)
                    .map(|drag| {
                        drag.target_duration.and_then(|duration| {
                            context.timeline_clip_transition_duration(project, &address, duration)
                        })
                    });
                clip_transitions.push((address.clone(), item.end, duration, drag_duration));
            }
            unplayed_ranges.push((item.start, item.end, played_range));
            if let Some(drag) = folded_drag.filter(|drag| drag.preview_at(&address).is_some()) {
                draw_item_drag_status_outline(
                    draw.painter,
                    (item.start, item.end),
                    draw.timeline_x,
                    y,
                    draw.view,
                    drag.valid_drop,
                    drag.preview_status,
                );
            }
        }
        draw_waveform_columns(
            draw.painter,
            &waveform_columns,
            draw.timeline_x,
            y,
            Color::GREEN1,
        );
        for (address, start, end, intro, outro) in transitions {
            draw_item_transitions(
                draw.painter,
                &address,
                start,
                end,
                intro,
                outro,
                focused_transition,
                transition_drag,
                draw.timeline_x,
                y,
                draw.view,
                Color::GREEN1,
            );
        }
        for (address, cut, duration, drag_duration) in clip_transitions {
            draw_clip_transition(
                draw.painter,
                &address,
                cut,
                duration,
                focused_transition,
                drag_duration,
                draw.timeline_x,
                y,
                draw.view,
                Color::GREEN1,
            );
        }
        for (start, end, played_range) in unplayed_ranges {
            draw_unplayed_item_ranges(
                draw.painter,
                start,
                end,
                played_range,
                y,
                draw.timeline_x,
                draw.view,
            );
        }
        draw_track_divider(draw.painter, 0.0, y, draw.timeline_x + draw.timeline_width);
    }
}

fn draw_unplayed_item_ranges(
    painter: &TimelinePainter,
    item_start: Time,
    item_end: Time,
    played_range: Option<(Time, Time)>,
    y: f64,
    timeline_x: f64,
    view: TimelineViewState,
) {
    let ranges = match played_range {
        Some((played_start, played_end)) => [
            (item_start, item_end.min(played_start)),
            (item_start.max(played_end), item_end),
        ],
        None => [(item_start, item_end), (Time::ZERO, Time::ZERO)],
    };
    let overlay = Color::BLACK.with_alpha(UNPLAYED_OVERLAY_ALPHA as f32);
    for (start, end) in ranges {
        if end <= start {
            continue;
        }
        let (x, width) = item_rect(start, end, timeline_x, view);
        painter.rect_filled(rect(x, y, width, TRACK_HEIGHT - 1.0), 0, overlay);
    }
}

fn draw_expanded_row_background(
    painter: &TimelinePainter,
    y: f64,
    timeline_x: f64,
    timeline_width: f64,
) {
    painter.rect_filled(
        rect(0.0, y, timeline_x + timeline_width, TRACK_HEIGHT),
        0,
        crate::theme::current().sidebar_shade.alpha_multiply(0.22),
    );
    let branch_x = timeline_x - 18.0;
    painter.line_segment(
        [
            vec2(branch_x as f32, y as f32),
            vec2(branch_x as f32, (y + TRACK_HEIGHT * 0.5) as f32),
        ],
        Stroke::new(1.0, crate::theme::current().sidebar_border),
    );
    painter.line_segment(
        [
            vec2(branch_x as f32, (y + TRACK_HEIGHT * 0.5) as f32),
            vec2((timeline_x - 6.0) as f32, (y + TRACK_HEIGHT * 0.5) as f32),
        ],
        Stroke::new(1.0, crate::theme::current().sidebar_border),
    );
}
