use crate::project::{Project, Time, VisualTrack};
use crate::timeline_search::{self, TimeSlice};

use super::{
    DragCollisionMode, DragIndicator, DraggedGroup, TrackKind, set_track_offset, target_item_times,
    target_track_at_y, target_track_index, track_count,
};

#[derive(Clone, Copy)]
pub(crate) enum NewItemTarget {
    Automatic,
    AtY(f64),
}

#[derive(Clone, Copy)]
pub(crate) struct TrackFootprintItem {
    pub(crate) track_offset: usize,
    pub(crate) start: Time,
    pub(crate) end: Time,
}

pub(crate) struct NewItemGroup {
    pub(crate) kind: TrackKind,
    pub(crate) footprint: Vec<TrackFootprintItem>,
}

#[derive(Default)]
pub(crate) struct NewItemPlacement {
    pub(crate) base: usize,
    pub(crate) new_tracks: std::ops::Range<usize>,
}

pub(crate) fn place_new_items(
    project: &Project,
    items: &NewItemGroup,
    target: NewItemTarget,
    collision_mode: DragCollisionMode,
) -> NewItemPlacement {
    let kind = items.kind;
    let span = track_footprint_span(&items.footprint);
    let NewItemTarget::AtY(y) = target else {
        return place_new_items_automatically(project, items);
    };
    if items.footprint.is_empty() {
        return NewItemPlacement::default();
    }

    let Some((base, new_track)) = target_track_at_y(project, kind, y) else {
        return place_new_items_automatically(project, items);
    };
    if let Some(new_track) = new_track {
        return NewItemPlacement {
            base,
            new_tracks: new_track..base + span,
        };
    }

    if collision_mode == DragCollisionMode::Block && !new_items_fit(project, items, base) {
        place_new_items_automatically(project, items)
    } else {
        place_new_items_at_base(project, items, base, collision_mode)
    }
}

pub(crate) fn place_new_items_at_base(
    project: &Project,
    items: &NewItemGroup,
    base: usize,
    collision_mode: DragCollisionMode,
) -> NewItemPlacement {
    if items.footprint.is_empty() {
        return NewItemPlacement::default();
    }
    let track_count = track_count(project, items.kind);
    let span = track_footprint_span(&items.footprint);
    let occupied = footprint_collides_at(&items.footprint, base, |track, start, end| {
        track < track_count && new_item_collides(project, items.kind, track, start, end)
    });
    let first_new = if occupied && collision_mode != DragCollisionMode::Overwrite {
        base.min(track_count)
    } else {
        track_count
    };
    NewItemPlacement {
        base,
        new_tracks: first_new..base + span,
    }
}

fn place_new_items_automatically(project: &Project, items: &NewItemGroup) -> NewItemPlacement {
    let track_count = track_count(project, items.kind);
    let base = choose_track_base(
        track_count,
        &items.footprint,
        None,
        |track, start, end| new_item_collides(project, items.kind, track, start, end),
        |track, start, end| {
            items.kind == TrackKind::Video
                && visual_track_is_obscured(&project.video_tracks, track, start, end)
        },
    );
    NewItemPlacement {
        base,
        new_tracks: track_count..base + track_footprint_span(&items.footprint),
    }
}

fn new_items_fit(project: &Project, items: &NewItemGroup, base: usize) -> bool {
    base + track_footprint_span(&items.footprint) <= track_count(project, items.kind)
        && !footprint_collides_at(&items.footprint, base, |track, start, end| {
            new_item_collides(project, items.kind, track, start, end)
        })
}

fn new_item_collides(
    project: &Project,
    kind: TrackKind,
    track: usize,
    start: Time,
    end: Time,
) -> bool {
    match kind {
        TrackKind::Caption => true,
        TrackKind::Video => project
            .video_tracks
            .get(track)
            .is_none_or(|track| timeline_search::collides(&track.items, start, end)),
        TrackKind::Audio => project
            .audio_tracks
            .get(track)
            .is_none_or(|track| timeline_search::collides(&track.items, start, end)),
    }
}

pub(crate) fn choose_track_base(
    track_count: usize,
    footprint: &[TrackFootprintItem],
    preferred_base: Option<usize>,
    collides: impl Fn(usize, Time, Time) -> bool,
    obscures: impl Fn(usize, Time, Time) -> bool,
) -> usize {
    let span = track_footprint_span(footprint);
    let fits = |base| {
        base + span <= track_count
            && !footprint_collides_at(footprint, base, |track, start, end| {
                collides(track, start, end)
            })
    };
    if let Some(base) = preferred_base
        && fits(base)
    {
        return base;
    }
    track_count
        .checked_sub(span)
        .and_then(|last_base| {
            (0..=last_base)
                .filter(|base| fits(*base))
                .min_by_key(|base| {
                    (
                        footprint_collides_at(footprint, *base, |track, start, end| {
                            obscures(track, start, end)
                        }),
                        *base,
                    )
                })
        })
        .unwrap_or(track_count)
}

pub(crate) fn track_footprint_span(footprint: &[TrackFootprintItem]) -> usize {
    footprint
        .iter()
        .map(|item| item.track_offset + 1)
        .max()
        .unwrap_or(0)
}

fn footprint_collides_at(
    footprint: &[TrackFootprintItem],
    base: usize,
    collides: impl Fn(usize, Time, Time) -> bool,
) -> bool {
    footprint
        .iter()
        .any(|item| collides(base + item.track_offset, item.start, item.end))
}

pub(crate) fn visual_track_is_obscured(
    tracks: &[VisualTrack],
    track_index: usize,
    start: Time,
    end: Time,
) -> bool {
    !tracks[track_index].enabled
        || tracks[track_index + 1..]
            .iter()
            .any(|track| track.enabled && timeline_search::collides(&track.items, start, end))
}

#[derive(Clone, Copy)]
pub(crate) struct ItemPlacement {
    pub(super) key: super::ItemKey,
    pub(super) target_track_index: usize,
    pub(super) start: Time,
    pub(super) end: Time,
}

pub(super) fn dragged_group_placements(
    project: &Project,
    group: &DraggedGroup,
) -> Option<Vec<ItemPlacement>> {
    let mut placements = Vec::with_capacity(group.items.len());
    for item in &group.items {
        let target_track_index = target_track_index(group, item)?;
        let final_track_count =
            track_count(project, item.key.kind) + new_track_indices(group, item.key.kind).len();
        if target_track_index >= final_track_count {
            return None;
        }
        let (start, end) = target_item_times(group, item)?;
        placements.push(ItemPlacement {
            key: item.key,
            target_track_index,
            start,
            end,
        });
    }
    Some(placements)
}

pub(super) fn can_place_dragged_group(project: &Project, group: &DraggedGroup) -> bool {
    let Some(placements) = dragged_group_placements(project, group) else {
        return false;
    };

    !placements_collide(&placements)
        && !placements_collide_with_project(project, group, &placements)
}

pub(super) fn placements_collide(placements: &[ItemPlacement]) -> bool {
    for (index, placement) in placements.iter().enumerate() {
        if placements[index + 1..].iter().any(|other| {
            placement.key.kind == other.key.kind
                && placement.target_track_index == other.target_track_index
                && time_ranges_collide(placement.start, placement.end, other.start, other.end)
        }) {
            return true;
        }
    }

    false
}

pub(super) fn placements_collide_with_project(
    project: &Project,
    group: &DraggedGroup,
    placements: &[ItemPlacement],
) -> bool {
    placements
        .iter()
        .copied()
        .any(|placement| collides_with_track(project, group, placement))
}

fn collides_with_track(project: &Project, group: &DraggedGroup, placement: ItemPlacement) -> bool {
    let Some(existing_track_index) =
        existing_track_index(group, placement.key.kind, placement.target_track_index)
    else {
        return false;
    };

    match placement.key.kind {
        TrackKind::Caption => {
            project
                .caption_tracks
                .get(existing_track_index)
                .is_none_or(|track| {
                    collides_with_remaining(&track.items, group, placement, existing_track_index)
                })
        }
        TrackKind::Video => project
            .video_tracks
            .get(existing_track_index)
            .is_none_or(|track| {
                collides_with_remaining(&track.items, group, placement, existing_track_index)
            }),
        TrackKind::Audio => project
            .audio_tracks
            .get(existing_track_index)
            .is_none_or(|track| {
                collides_with_remaining(&track.items, group, placement, existing_track_index)
            }),
    }
}

fn collides_with_remaining<T: Clone + TimeSlice>(
    items: &[T],
    group: &DraggedGroup,
    placement: ItemPlacement,
    existing_track_index: usize,
) -> bool {
    let remaining: Vec<_> = items
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            !group.items.iter().any(|item| {
                item.key.kind == placement.key.kind
                    && item.key.track_index == existing_track_index
                    && item.key.item_index == *index
            })
        })
        .map(|(_, item)| item.clone())
        .collect();

    timeline_search::collides(&remaining, placement.start, placement.end)
}

pub(super) fn new_track_indices(group: &DraggedGroup, kind: TrackKind) -> Vec<usize> {
    let mut indices: Vec<_> = group
        .new_tracks
        .iter()
        .filter_map(|(track_kind, index)| (*track_kind == kind).then_some(*index))
        .collect();
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn existing_track_index(
    group: &DraggedGroup,
    kind: TrackKind,
    target_track_index: usize,
) -> Option<usize> {
    let new_tracks = new_track_indices(group, kind);
    if new_tracks.contains(&target_track_index) {
        return None;
    }
    let inserted_before = new_tracks
        .iter()
        .filter(|index| **index <= target_track_index)
        .count();
    target_track_index.checked_sub(inserted_before)
}

pub(super) fn placement_indicators(placements: &[ItemPlacement]) -> Vec<DragIndicator> {
    placements
        .iter()
        .map(|placement| DragIndicator {
            kind: placement.key.kind,
            track_index: placement.target_track_index,
            start: placement.start,
            end: placement.end,
        })
        .collect()
}

pub(super) fn overwrite_indicators(
    project: &Project,
    group: &DraggedGroup,
    placements: &[ItemPlacement],
) -> Vec<DragIndicator> {
    let mut indicators = Vec::new();
    for placement in placements {
        let Some(existing_track_index) =
            existing_track_index(group, placement.key.kind, placement.target_track_index)
        else {
            continue;
        };
        match placement.key.kind {
            TrackKind::Caption => {
                if let Some(track) = project.caption_tracks.get(existing_track_index) {
                    collect_overwrite_indicators(
                        &track.items,
                        group,
                        *placement,
                        existing_track_index,
                        &mut indicators,
                    );
                }
            }
            TrackKind::Video => {
                if let Some(track) = project.video_tracks.get(existing_track_index) {
                    collect_overwrite_indicators(
                        &track.items,
                        group,
                        *placement,
                        existing_track_index,
                        &mut indicators,
                    );
                }
            }
            TrackKind::Audio => {
                if let Some(track) = project.audio_tracks.get(existing_track_index) {
                    collect_overwrite_indicators(
                        &track.items,
                        group,
                        *placement,
                        existing_track_index,
                        &mut indicators,
                    );
                }
            }
        }
    }
    indicators
}

fn collect_overwrite_indicators<T: TimeSlice>(
    items: &[T],
    group: &DraggedGroup,
    placement: ItemPlacement,
    existing_track_index: usize,
    indicators: &mut Vec<DragIndicator>,
) {
    for (item_index, item) in items.iter().enumerate() {
        if group.items.iter().any(|dragged| {
            dragged.key.kind == placement.key.kind
                && dragged.key.track_index == existing_track_index
                && dragged.key.item_index == item_index
        }) {
            continue;
        }
        let start = item.start().max(placement.start);
        let end = item.end().min(placement.end);
        if start < end {
            indicators.push(DragIndicator {
                kind: placement.key.kind,
                track_index: placement.target_track_index,
                start,
                end,
            });
        }
    }
}

pub(super) fn add_collision_tracks(project: &Project, group: &mut DraggedGroup) {
    let Some(placements) = dragged_group_placements(project, group) else {
        return;
    };
    let mut kinds = Vec::new();
    for placement in &placements {
        if collides_with_track(project, group, *placement) && !kinds.contains(&placement.key.kind) {
            kinds.push(placement.key.kind);
        }
    }

    for kind in kinds {
        let Some((source_base, span)) = group_track_span(group, kind) else {
            continue;
        };
        let target_base = target_base_for_kind(&placements, kind).unwrap_or(source_base);
        clear_kind_new_tracks(group, kind);
        if let Some(existing_base) = closest_empty_track(project, group, kind, target_base, span) {
            set_track_offset(group, kind, existing_base as isize - source_base as isize);
            continue;
        }

        let new_track_base = match kind {
            TrackKind::Caption | TrackKind::Video => target_base
                .saturating_add(span)
                .min(track_count(project, kind)),
            TrackKind::Audio => track_count(project, kind),
        };
        set_track_offset(group, kind, new_track_base as isize - source_base as isize);
        group
            .new_tracks
            .extend((0..span).map(|offset| (kind, new_track_base + offset)));
    }
}

fn group_track_span(group: &DraggedGroup, kind: TrackKind) -> Option<(usize, usize)> {
    let source_base = group
        .items
        .iter()
        .filter(|item| item.key.kind == kind)
        .map(|item| item.key.track_index)
        .min()?;
    let span = group
        .items
        .iter()
        .filter(|item| item.key.kind == kind)
        .map(|item| item.key.track_index - source_base + 1)
        .max()
        .unwrap_or(1);
    Some((source_base, span))
}

fn target_base_for_kind(placements: &[ItemPlacement], kind: TrackKind) -> Option<usize> {
    placements
        .iter()
        .filter(|placement| placement.key.kind == kind)
        .map(|placement| placement.target_track_index)
        .min()
}

fn clear_kind_new_tracks(group: &mut DraggedGroup, kind: TrackKind) {
    group
        .new_tracks
        .retain(|(track_kind, _)| *track_kind != kind);
}

fn closest_empty_track(
    project: &Project,
    group: &mut DraggedGroup,
    kind: TrackKind,
    target_base: usize,
    span: usize,
) -> Option<usize> {
    let track_count = track_count(project, kind);
    if span == 0 || span > track_count {
        return None;
    }

    let (source_base, _) = group_track_span(group, kind)?;
    let start = target_base.saturating_add(1);
    (start..=track_count - span).find(|base| {
        base + span <= track_count && {
            set_track_offset(group, kind, *base as isize - source_base as isize);
            can_place_dragged_kind(project, group, kind)
        }
    })
}

fn can_place_dragged_kind(project: &Project, group: &DraggedGroup, kind: TrackKind) -> bool {
    let Some(placements) = dragged_group_placements(project, group) else {
        return false;
    };
    let placements: Vec<_> = placements
        .into_iter()
        .filter(|placement| placement.key.kind == kind)
        .collect();
    !placements_collide(&placements)
        && !placements
            .iter()
            .copied()
            .any(|placement| collides_with_track(project, group, placement))
}

pub(super) fn insert_new_tracks<T: Default>(tracks: &mut Vec<T>, indices: &[usize]) -> Option<()> {
    let mut indices = indices.to_vec();
    indices.sort_unstable();
    indices.dedup();
    for index in indices {
        if index > tracks.len() {
            return None;
        }
        tracks.insert(index, T::default());
    }
    Some(())
}

pub(super) fn target_existing_track_index(
    new_track_indices: &[usize],
    placement: ItemPlacement,
) -> Option<usize> {
    if new_track_indices.contains(&placement.target_track_index) {
        return None;
    }
    let inserted_before = new_track_indices
        .iter()
        .filter(|index| **index <= placement.target_track_index)
        .count();
    placement.target_track_index.checked_sub(inserted_before)
}

pub(super) fn time_ranges_collide(
    start: Time,
    end: Time,
    other_start: Time,
    other_end: Time,
) -> bool {
    start < other_end && end > other_start
}
