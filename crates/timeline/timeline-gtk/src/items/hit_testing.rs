use super::*;
use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};

pub(crate) const TRANSITION_TOP_STRIP_HEIGHT: f64 = 12.0;
pub(crate) const TRANSITION_HANDLE_RADIUS: f64 = 7.0;

pub(crate) fn is_item_selected(
    selected_items: &[ItemKey],
    kind: TrackKind,
    track_index: usize,
    item_index: usize,
) -> bool {
    selected_items.contains(&ItemKey {
        kind,
        track_index,
        item_index,
    })
}

pub(crate) fn is_item_dragged(
    dragged_group: Option<&DraggedGroup>,
    kind: TrackKind,
    track_index: usize,
    item_index: usize,
) -> bool {
    dragged_group.is_some_and(|group| {
        group.items.iter().any(|item| {
            item.key
                == (ItemKey {
                    kind,
                    track_index,
                    item_index,
                })
        })
    })
}

pub(crate) fn hit_item_at(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
) -> Option<ItemKey> {
    if x < super::timeline_x() || y < super::RULER_HEIGHT {
        return None;
    }
    let y = y + view.scroll_y;
    let (kind, track_index, _) = track_at_y(project, y)?;
    let time = Time::from_seconds_f64(x_to_time(x, view.scroll_seconds, view.seconds_per_pixel));

    match kind {
        TrackKind::Caption => timeline_search::overlapping(
            &project.caption_tracks.get(track_index)?.items,
            time,
            time,
        )
        .next()
        .map(|(item_index, _)| ItemKey {
            kind,
            track_index,
            item_index,
        }),
        TrackKind::Video => {
            timeline_search::overlapping(&project.video_tracks.get(track_index)?.items, time, time)
                .next()
                .map(|(item_index, _)| ItemKey {
                    kind,
                    track_index,
                    item_index,
                })
        }
        TrackKind::Audio => {
            timeline_search::overlapping(&project.audio_tracks.get(track_index)?.items, time, time)
                .next()
                .map(|(item_index, _)| ItemKey {
                    kind,
                    track_index,
                    item_index,
                })
        }
    }
}

pub(crate) fn hit_gap_at(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
) -> Option<shrimply_timeline::TrackGap> {
    if x < super::timeline_x() || y < super::RULER_HEIGHT {
        return None;
    }
    let (kind, track_index, _) = track_at_y(project, y + view.scroll_y)?;
    track_gap_at(
        project,
        TrackKey { kind, track_index },
        Time::from_seconds_f64(x_to_time(x, view.scroll_seconds, view.seconds_per_pixel).max(0.0)),
    )
}

pub(crate) fn hit_resize_handle_at(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
    handle_width: f64,
) -> Option<(ItemKey, ItemEdge)> {
    if x < super::timeline_x() || y < super::RULER_HEIGHT {
        return None;
    }
    let y = y + view.scroll_y;
    let (kind, track_index, row) = track_at_y(project, y)?;
    if y < row_y(row) || y > row_y(row) + TRACK_HEIGHT {
        return None;
    }

    let items: Vec<_> = match kind {
        TrackKind::Caption => project
            .caption_tracks
            .get(track_index)?
            .items
            .iter()
            .enumerate()
            .map(|(item_index, item)| (item_index, item.start, item.end))
            .collect(),
        TrackKind::Video => project
            .video_tracks
            .get(track_index)?
            .items
            .iter()
            .enumerate()
            .map(|(item_index, item)| (item_index, item.start, item.end))
            .collect(),
        TrackKind::Audio => project
            .audio_tracks
            .get(track_index)?
            .items
            .iter()
            .enumerate()
            .map(|(item_index, item)| (item_index, item.start, item.end))
            .collect(),
    };

    let mut hit: Option<(ItemKey, ItemEdge, f64, f64)> = None;
    for (item_index, start, end) in items {
        let start_x =
            timeline_x() + (start.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
        let end_x =
            timeline_x() + (end.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
        let start_distance = (x - start_x).abs();
        let end_distance = (x - end_x).abs();
        let (edge, edge_distance, edge_x) = if start_distance <= end_distance {
            (ItemEdge::Start, start_distance, start_x)
        } else {
            (ItemEdge::End, end_distance, end_x)
        };
        if (matches!(edge, ItemEdge::Start) && x < edge_x)
            || (matches!(edge, ItemEdge::End) && x > edge_x)
        {
            continue;
        }
        if edge_distance > handle_width || x < start_x - handle_width || x > end_x + handle_width {
            continue;
        }
        let key = ItemKey {
            kind,
            track_index,
            item_index,
        };
        let replace = match hit {
            None => true,
            Some((_, hit_edge, hit_edge_x, hit_distance)) => {
                let same_distance = (edge_distance - hit_distance).abs() <= f64::EPSILON;
                let same_edge_x = (edge_x - hit_edge_x).abs() <= f64::EPSILON;
                edge_distance < hit_distance
                    || (same_distance
                        && same_edge_x
                        && match (hit_edge, edge) {
                            (ItemEdge::End, ItemEdge::Start) => x > edge_x,
                            (ItemEdge::Start, ItemEdge::End) => x < edge_x,
                            _ => false,
                        })
            }
        };
        if replace {
            hit = Some((key, edge, edge_x, edge_distance));
        }
    }
    hit.map(|(key, edge, _, _)| (key, edge))
}

pub(crate) fn hit_transition_at(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
) -> Option<TransitionHit> {
    let content_y = y + view.scroll_y;
    let (key, start, end, row) =
        if let Some(hit) = crate::folded_sequence::hit_projected_item(project, view, x, y) {
            let row = ((content_y - RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize;
            (hit.key, hit.start, hit.end, row)
        } else {
            let item_key = hit_item_at(project, view, x, y)?;
            let key = crate::selection_state::item_address(project, item_key)?;
            let (start, end) = item_times(project, item_key)?;
            let (_, _, row) = track_at_y(project, content_y)?;
            (key, start, end, row)
        };
    if matches!(key.kind(), crate::project::ItemKind::Caption) {
        return None;
    }
    let local_y = content_y - row_y(row);
    if !(0.0..=TRACK_HEIGHT).contains(&local_y) {
        return None;
    }
    let start_x =
        timeline_x() + (start.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    let end_x = timeline_x() + (end.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    let (intro, outro) = transition_durations(project, &key)?;
    let intro_x = intro.map(|duration| {
        timeline_x()
            + (start.saturating_add(duration).as_secs_f64() - view.scroll_seconds)
                / view.seconds_per_pixel
    });
    let outro_x = outro.map(|duration| {
        timeline_x()
            + (end.saturating_sub(duration).as_secs_f64() - view.scroll_seconds)
                / view.seconds_per_pixel
    });

    for (side, handle_x) in [
        (crate::project::TransitionSide::Intro, intro_x),
        (crate::project::TransitionSide::Outro, outro_x),
    ] {
        if local_y <= TRANSITION_TOP_STRIP_HEIGHT
            && handle_x.is_some_and(|handle_x| (x - handle_x).abs() <= TRANSITION_HANDLE_RADIUS)
        {
            return Some(TransitionHit {
                key,
                side,
                action: TransitionHitAction::Handle,
            });
        }
    }

    if let Some(handle_x) = intro_x
        && x > start_x + crate::ITEM_RESIZE_HANDLE_WIDTH
        && x <= handle_x
    {
        let diagonal = TRACK_HEIGHT * (handle_x - x) / (handle_x - start_x).max(f64::EPSILON);
        if local_y <= diagonal {
            return Some(TransitionHit {
                key,
                side: crate::project::TransitionSide::Intro,
                action: TransitionHitAction::Body,
            });
        }
    }
    if let Some(handle_x) = outro_x
        && x >= handle_x
        && x < end_x - crate::ITEM_RESIZE_HANDLE_WIDTH
    {
        let diagonal = TRACK_HEIGHT * (x - handle_x) / (end_x - handle_x).max(f64::EPSILON);
        if local_y <= diagonal {
            return Some(TransitionHit {
                key,
                side: crate::project::TransitionSide::Outro,
                action: TransitionHitAction::Body,
            });
        }
    }

    if local_y <= TRANSITION_TOP_STRIP_HEIGHT
        && intro.is_none()
        && (x - start_x).abs() <= TRANSITION_HANDLE_RADIUS
    {
        return Some(TransitionHit {
            key,
            side: crate::project::TransitionSide::Intro,
            action: TransitionHitAction::Create,
        });
    }
    if local_y <= TRANSITION_TOP_STRIP_HEIGHT
        && outro.is_none()
        && (x - end_x).abs() <= TRANSITION_HANDLE_RADIUS
    {
        return Some(TransitionHit {
            key,
            side: crate::project::TransitionSide::Outro,
            action: TransitionHitAction::Create,
        });
    }
    None
}

pub(crate) fn hit_clip_transition_at(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
) -> Option<ClipTransitionHit> {
    let content_y = y + view.scroll_y;
    let key = if let Some(hit) = crate::folded_sequence::hit_projected_item(project, view, x, y) {
        hit.key
    } else {
        let item = hit_item_at(project, view, x, y)?;
        crate::selection_state::item_address(project, item)?
    };
    if key.kind() == crate::project::ItemKind::Caption {
        return None;
    }
    let row = row_for_address(project, &key.track())?;
    let local_y = content_y - row_y(row);
    if !(0.0..=TRACK_HEIGHT).contains(&local_y) {
        return None;
    }
    let top_strip = local_y <= TRANSITION_TOP_STRIP_HEIGHT;
    let track = key.track();
    let items = match project.track(&track)? {
        crate::project::TrackRef::Video(track) => track
            .items
            .iter()
            .map(|item| ClipItemInfo {
                id: item.id,
                start: item.start,
                end: item.end,
                intro_empty: item.transitions.intro.is_none(),
                outro_empty: item.transitions.outro.is_none(),
                to_next: item
                    .transitions
                    .to_next
                    .as_ref()
                    .map(|value| (value.target_item_id, value.duration)),
            })
            .collect::<Vec<_>>(),
        crate::project::TrackRef::Audio(track) => track
            .items
            .iter()
            .map(|item| ClipItemInfo {
                id: item.id,
                start: item.start,
                end: item.end,
                intro_empty: item.transitions.intro.is_none(),
                outro_empty: item.transitions.outro.is_none(),
                to_next: item
                    .transitions
                    .to_next
                    .as_ref()
                    .map(|value| (value.target_item_id, value.duration)),
            })
            .collect::<Vec<_>>(),
        crate::project::TrackRef::Caption(_) => return None,
    };
    let index = items.iter().position(|item| item.id == key.item_id())?;
    for outgoing_index in [index.checked_sub(1), Some(index)].into_iter().flatten() {
        let Some(outgoing) = items.get(outgoing_index) else {
            continue;
        };
        let Some(incoming) = items.get(outgoing_index + 1) else {
            continue;
        };
        if outgoing.end != incoming.start {
            continue;
        }
        let outgoing_address = track.item(outgoing.id);
        let incoming_address = track.item(incoming.id);
        let context = SequenceTimeline::for_item(project, &outgoing_address)?;
        let (_, timeline_cut) = context.timeline_item_times(project, &outgoing_address)?;
        let cut_x = timeline_x()
            + (timeline_cut.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
        let duration = outgoing
            .to_next
            .filter(|(target, _)| *target == incoming.id)
            .map(|(_, duration)| duration);
        if let Some(duration) = duration {
            let half = crate::math::clip_transition_half_duration(duration);
            let start =
                project.sequence_time_to_timeline(&track, outgoing.end.saturating_sub(half))?;
            let end =
                project.sequence_time_to_timeline(&track, incoming.start.saturating_add(half))?;
            let start_x =
                timeline_x() + (start.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
            let end_x =
                timeline_x() + (end.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
            let action = if top_strip && (x - start_x).abs() <= TRANSITION_HANDLE_RADIUS {
                ClipTransitionHitAction::StartHandle
            } else if top_strip && (x - end_x).abs() <= TRANSITION_HANDLE_RADIUS {
                ClipTransitionHitAction::EndHandle
            } else if (x - cut_x).abs() <= crate::ITEM_RESIZE_HANDLE_WIDTH
                || top_strip && x >= start_x && x <= end_x
            {
                ClipTransitionHitAction::CenterHandle
            } else if x >= start_x && x <= end_x {
                ClipTransitionHitAction::Body
            } else {
                continue;
            };
            return Some(ClipTransitionHit {
                outgoing: outgoing_address,
                incoming: incoming_address,
                cut: outgoing.end,
                duration: Some(duration),
                action,
            });
        }
        if top_strip
            && outgoing.outro_empty
            && outgoing.to_next.is_none()
            && incoming.intro_empty
            && (x - cut_x).abs() <= TRANSITION_HANDLE_RADIUS
        {
            return Some(ClipTransitionHit {
                outgoing: outgoing_address,
                incoming: incoming_address,
                cut: outgoing.end,
                duration: None,
                action: ClipTransitionHitAction::Create,
            });
        }
    }
    None
}

#[derive(Clone, Copy)]
struct ClipItemInfo {
    id: uuid::Uuid,
    start: Time,
    end: Time,
    intro_empty: bool,
    outro_empty: bool,
    to_next: Option<(uuid::Uuid, Time)>,
}

pub(crate) fn transition_durations(
    project: &Project,
    key: &crate::project::ItemAddress,
) -> Option<(Option<Time>, Option<Time>)> {
    SequenceTimeline::for_item(project, key)?.timeline_transition_durations(project, key)
}
