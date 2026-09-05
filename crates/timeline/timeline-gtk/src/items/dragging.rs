use super::*;
use crate::snapping::SnapRepo;

pub(crate) fn dragged_group_for_hit(
    project: &Project,
    selected_items: &[ItemKey],
    hit: ItemKey,
    view: TimelineViewState,
    x: f64,
    collision_mode: DragCollisionMode,
) -> Option<DraggedGroup> {
    let (hit_start, _) = item_times(project, hit)?;
    let pointer_seconds = x_to_time(x, view.scroll_seconds, view.seconds_per_pixel);
    let mut group_items: Vec<_> = selected_items
        .iter()
        .copied()
        .filter_map(|key| {
            let (start, end) = item_times(project, key)?;
            Some(DraggedGroupItem { key, start, end })
        })
        .collect();

    if group_items.iter().all(|item| item.key != hit) {
        let (start, end) = item_times(project, hit)?;
        group_items = vec![DraggedGroupItem {
            key: hit,
            start,
            end,
        }];
    }

    Some(DraggedGroup {
        grabbed: hit,
        grabbed_start: hit_start,
        pointer_offset: Time::from_seconds_f64(pointer_seconds).signed_sub(hit_start),
        target_start: hit_start,
        track_offsets: vec![TrackOffset {
            kind: hit.kind,
            offset: 0,
        }],
        new_tracks: Vec::new(),
        collision_mode,
        valid_drop: true,
        preview_status: DragPreviewStatus::Clear,
        blocked_indicators: Vec::new(),
        overwrite_indicators: Vec::new(),
        items: group_items,
        cross_scope_preview_row: None,
    })
}

pub(crate) fn update_dragged_group(
    group: &mut DraggedGroup,
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
    snap_repository: &SnapRepo,
) {
    let previous_position = drag_position(group);
    let y = y + view.scroll_y;
    let pointer_seconds = x_to_time(x, view.scroll_seconds, view.seconds_per_pixel)
        - group.pointer_offset.as_secs_f64();
    let target = Time::from_seconds_f64(pointer_seconds);
    let target = snap_repository.snap(target).unwrap_or(target);
    let earliest_start = group
        .items
        .iter()
        .map(|item| item.start)
        .min()
        .unwrap_or(group.grabbed_start);
    group.target_start = target.max(group.grabbed_start.signed_sub(earliest_start));
    group.blocked_indicators.clear();
    group.overwrite_indicators.clear();
    group.cross_scope_preview_row = None;

    let Some((target_track_index, new_track_index)) =
        active_new_track_at_y(project, group.grabbed.kind, &group.new_tracks, y)
            .or_else(|| target_track_at_y(project, group.grabbed.kind, y))
    else {
        group.valid_drop = false;
        group.new_tracks.clear();
        group.preview_status = DragPreviewStatus::Blocked;
        return;
    };

    let target_track_index = target_track_index as isize - isize::from(new_track_index == Some(0));
    let offset = target_track_index - group.grabbed.track_index as isize;
    set_track_offsets(project, group, offset);

    resolve_dragged_group(project, group, previous_position);
}

pub(crate) fn drag_position(group: &DraggedGroup) -> DragPosition {
    DragPosition {
        target_start: group.target_start,
        track_offsets: group.track_offsets.clone(),
        new_tracks: group.new_tracks.clone(),
        valid_drop: group.valid_drop,
    }
}

pub(crate) fn restore_drag_position(group: &mut DraggedGroup, position: DragPosition) {
    group.target_start = position.target_start;
    group.track_offsets = position.track_offsets;
    group.new_tracks = position.new_tracks;
    group.valid_drop = position.valid_drop;
}

pub(crate) fn resolve_dragged_group(
    project: &Project,
    group: &mut DraggedGroup,
    previous: DragPosition,
) {
    match group.collision_mode {
        DragCollisionMode::Overwrite => resolve_overwrite_drag(project, group),
        DragCollisionMode::Block => resolve_block_drag(project, group, previous),
        DragCollisionMode::NewTrack => resolve_new_track_drag(project, group),
    }
}

pub(crate) fn resolve_overwrite_drag(project: &Project, group: &mut DraggedGroup) {
    group.overwrite_indicators.clear();
    group.blocked_indicators.clear();
    let Some(placements) = dragged_group_placements(project, group) else {
        group.valid_drop = false;
        group.preview_status = DragPreviewStatus::Blocked;
        return;
    };
    if placements_collide(&placements) {
        group.valid_drop = false;
        group.blocked_indicators = placement_indicators(&placements);
        group.preview_status = DragPreviewStatus::Blocked;
        return;
    }

    group.overwrite_indicators = overwrite_indicators(project, group, &placements);
    group.valid_drop = true;
    group.preview_status = if !group.overwrite_indicators.is_empty() {
        DragPreviewStatus::Overwrite
    } else if group.new_tracks.is_empty() {
        DragPreviewStatus::Clear
    } else {
        DragPreviewStatus::NewTrack
    };
}

pub(crate) fn resolve_block_drag(
    project: &Project,
    group: &mut DraggedGroup,
    previous: DragPosition,
) {
    group.overwrite_indicators.clear();
    group.blocked_indicators.clear();
    if can_place_dragged_group(project, group) {
        group.valid_drop = true;
        group.preview_status = if group.new_tracks.is_empty() {
            DragPreviewStatus::Clear
        } else {
            DragPreviewStatus::NewTrack
        };
        return;
    }

    if let Some(placements) = dragged_group_placements(project, group) {
        group.blocked_indicators = placement_indicators(&placements);
    }
    restore_drag_position(group, previous);
    group.preview_status = DragPreviewStatus::Blocked;
}

pub(crate) fn resolve_new_track_drag(project: &Project, group: &mut DraggedGroup) {
    group.overwrite_indicators.clear();
    group.blocked_indicators.clear();
    if !can_place_dragged_group(project, group) {
        add_collision_tracks(project, group);
    }

    let Some(placements) = dragged_group_placements(project, group) else {
        group.valid_drop = false;
        group.preview_status = DragPreviewStatus::Blocked;
        return;
    };
    if placements_collide(&placements)
        || placements_collide_with_project(project, group, &placements)
    {
        group.valid_drop = false;
        group.blocked_indicators = placement_indicators(&placements);
        group.preview_status = DragPreviewStatus::Blocked;
        return;
    }

    group.valid_drop = true;
    group.preview_status = if group.new_tracks.is_empty() {
        DragPreviewStatus::Clear
    } else {
        DragPreviewStatus::NewTrack
    };
}

pub(crate) fn item_natural_end_edges_at_address(
    project: &Project,
    address: &ItemAddress,
) -> Vec<Time> {
    let mut local = Vec::new();
    match project.item(address) {
        Some(ItemRef::Video(item)) => {
            let first = if item.repeats_keyframes() {
                generated_item_natural_end_position(item)
            } else {
                media_item_natural_end_position(
                    item.start,
                    item.time_offset,
                    item.source_duration,
                    item.playback_speed,
                    item.repeat_strategy,
                )
            }
            .filter(|marker| *marker > item.start);
            push_natural_end_times(
                &mut local,
                item.start,
                item.end,
                first,
                video_natural_end_interval(item),
            );
        }
        Some(ItemRef::Audio(item)) => push_natural_end_times(
            &mut local,
            item.start,
            item.end,
            media_item_natural_end_position(
                item.start,
                item.time_offset,
                item.source_duration,
                item.playback_speed,
                item.repeat_strategy,
            )
            .filter(|marker| *marker > item.start),
            media_natural_end_interval(
                item.source_duration,
                item.playback_speed,
                item.repeat_strategy,
            ),
        ),
        Some(ItemRef::Caption(_)) | None => {}
    }
    let track = address.track();
    local
        .into_iter()
        .filter_map(|time| project.sequence_time_to_timeline(&track, time))
        .collect()
}

fn item_natural_span_at_address(project: &Project, address: &ItemAddress) -> Option<(Time, Time)> {
    let (start, end) = match project.item(address)? {
        ItemRef::Video(item) if item.repeats_keyframes() => generated_item_natural_span(item)?,
        ItemRef::Video(item) => media_real_span(
            item.start,
            item.time_offset,
            item.source_duration,
            item.playback_speed,
            item.repeat_strategy,
        )?,
        ItemRef::Audio(item) => media_real_span(
            item.start,
            item.time_offset,
            item.source_duration,
            item.playback_speed,
            item.repeat_strategy,
        )?,
        ItemRef::Caption(_) => return None,
    };
    let track = address.track();
    let start = project.sequence_time_to_timeline(&track, start)?;
    let end = project.sequence_time_to_timeline(&track, end)?;
    Some((start.min(end), start.max(end)))
}

pub(crate) fn item_natural_snap_targets_at_address(
    project: &Project,
    address: &ItemAddress,
) -> Vec<Time> {
    let mut targets = item_natural_end_edges_at_address(project, address);
    if let Some((start, end)) = item_natural_span_at_address(project, address) {
        targets.extend([start, end]);
    }
    targets
}

pub(crate) fn item_natural_resize_candidates_at_address(
    project: &Project,
    address: &ItemAddress,
) -> Vec<Time> {
    let mut targets = item_natural_snap_targets_at_address(project, address);
    if let (Some((start, end)), Some((current_start, current_end))) = (
        item_natural_span_at_address(project, address),
        project.timeline_item_times(address),
    ) {
        let duration = end.saturating_sub(start);
        targets.extend([
            current_end.saturating_sub(duration),
            current_start.saturating_add(duration),
        ]);
    }
    targets
}

fn push_natural_end_times(
    edges: &mut Vec<Time>,
    start: Time,
    end: Time,
    first: Option<Time>,
    interval: Option<Time>,
) {
    let Some(mut position) = first else {
        return;
    };
    if let Some(interval) = interval {
        if interval <= Time::ZERO {
            return;
        }
        while position > start {
            let previous = Time {
                seconds: position.seconds - interval.seconds,
            };
            if previous <= start {
                break;
            }
            position = previous;
        }
        while position <= start {
            position = position.saturating_add(interval);
        }
    } else if position <= start {
        return;
    }
    while position < end {
        edges.push(position);
        let Some(interval) = interval else {
            break;
        };
        position = position.saturating_add(interval);
    }
}

pub(crate) fn target_track_index(group: &DraggedGroup, item: &DraggedGroupItem) -> Option<usize> {
    let index = item.key.track_index as isize + track_offset(group, item.key.kind);
    (index >= 0).then_some(index as usize)
}

pub(crate) fn track_offset(group: &DraggedGroup, kind: TrackKind) -> isize {
    group
        .track_offsets
        .iter()
        .find(|offset| offset.kind == kind)
        .map(|offset| offset.offset)
        .unwrap_or(0)
}

pub(crate) fn set_track_offset(group: &mut DraggedGroup, kind: TrackKind, offset: isize) {
    if let Some(existing) = group
        .track_offsets
        .iter_mut()
        .find(|existing| existing.kind == kind)
    {
        existing.offset = offset;
    } else {
        group.track_offsets.push(TrackOffset { kind, offset });
    }
}

pub(crate) fn set_track_offsets(project: &Project, group: &mut DraggedGroup, track_offset: isize) {
    group.track_offsets.clear();
    group.new_tracks.clear();
    let mut kinds = Vec::new();
    for item in &group.items {
        if kinds.contains(&item.key.kind) {
            continue;
        }
        kinds.push(item.key.kind);
    }
    for kind in kinds {
        let Some((source_min, source_max)) = group_track_bounds(group, kind) else {
            continue;
        };
        let existing_track_count = track_count(project, kind) as isize;
        let span = (source_max - source_min + 1) as usize;
        let needed_before = (-(source_min + track_offset)).max(0) as usize;
        let needed_after = (source_max + track_offset + 1 - existing_track_count).max(0) as usize;
        let new_before = needed_before.min(span);
        let new_after = needed_after.min(span);
        let min_offset = -source_min;
        let max_offset = existing_track_count + new_after as isize - 1 - source_max;
        let offset = (track_offset + new_before as isize).clamp(min_offset, max_offset);
        set_track_offset(group, kind, offset);
        group
            .new_tracks
            .extend((0..new_before).map(|index| (kind, index)));
        let existing_track_count = track_count(project, kind);
        group
            .new_tracks
            .extend((0..new_after).map(|index| (kind, existing_track_count + index)));
    }
}

pub(crate) fn group_track_bounds(group: &DraggedGroup, kind: TrackKind) -> Option<(isize, isize)> {
    let mut tracks = group
        .items
        .iter()
        .filter(|item| item.key.kind == kind)
        .map(|item| item.key.track_index as isize);
    let first = tracks.next()?;
    let mut min = first;
    let mut max = first;
    for track in tracks {
        min = min.min(track);
        max = max.max(track);
    }
    Some((min, max))
}

pub(crate) fn target_item_times(
    group: &DraggedGroup,
    item: &DraggedGroupItem,
) -> Option<(Time, Time)> {
    let delta = group.target_start.signed_sub(group.grabbed_start);
    let start = item.start.saturating_add(delta);
    let end = start.saturating_add(item.end.signed_sub(item.start));
    Some((start, end))
}

pub(crate) fn move_dragged_group(
    project: &mut Project,
    group: &DraggedGroup,
) -> Option<Vec<ItemKey>> {
    let placements = dragged_group_placements(project, group)?;
    if placements_collide(&placements)
        || (group.collision_mode != DragCollisionMode::Overwrite
            && placements_collide_with_project(project, group, &placements))
    {
        return None;
    }

    let mut selection = Vec::with_capacity(placements.len());
    let overwrite = group.collision_mode == DragCollisionMode::Overwrite;
    move_caption_items(
        project,
        &placements,
        &new_track_indices(group, TrackKind::Caption),
        overwrite,
        &mut selection,
    )?;
    move_video_items(
        project,
        &placements,
        &new_track_indices(group, TrackKind::Video),
        overwrite,
        &mut selection,
    )?;
    move_audio_items(
        project,
        &placements,
        &new_track_indices(group, TrackKind::Audio),
        overwrite,
        &mut selection,
    )?;
    Some(selection)
}

pub(crate) fn move_caption_items(
    project: &mut Project,
    placements: &[ItemPlacement],
    new_track_indices: &[usize],
    overwrite: bool,
    selection: &mut Vec<ItemKey>,
) -> Option<()> {
    let mut moved = Vec::new();
    for placement in placements
        .iter()
        .copied()
        .filter(|placement| placement.key.kind == TrackKind::Caption)
    {
        let item = project
            .caption_tracks
            .get(placement.key.track_index)?
            .items
            .get(placement.key.item_index)?
            .clone();
        moved.push((placement, item));
    }

    remove_caption_items(project, placements)?;
    if overwrite {
        overwrite_caption_items(project, placements, new_track_indices)?;
    }
    insert_new_tracks(&mut project.caption_tracks, new_track_indices)?;
    for (placement, mut item) in moved {
        item.start = placement.start;
        item.end = placement.end;
        let target_items = &mut project
            .caption_tracks
            .get_mut(placement.target_track_index)?
            .items;
        let item_index = insert_sorted(target_items, item);
        push_moved_selection(
            selection,
            ItemKey {
                kind: TrackKind::Caption,
                track_index: placement.target_track_index,
                item_index,
            },
        );
    }
    Some(())
}

pub(crate) fn move_video_items(
    project: &mut Project,
    placements: &[ItemPlacement],
    new_track_indices: &[usize],
    overwrite: bool,
    selection: &mut Vec<ItemKey>,
) -> Option<()> {
    let mut moved = Vec::new();
    for placement in placements
        .iter()
        .copied()
        .filter(|placement| placement.key.kind == TrackKind::Video)
    {
        let item = project
            .video_tracks
            .get(placement.key.track_index)?
            .items
            .get(placement.key.item_index)?
            .clone();
        moved.push((placement, item));
    }

    remove_video_items(project, placements)?;
    if overwrite {
        overwrite_video_items(project, placements, new_track_indices)?;
    }
    insert_new_tracks(&mut project.video_tracks, new_track_indices)?;
    for (placement, mut item) in moved {
        item.start = placement.start;
        item.end = placement.end;
        let target_items = &mut project
            .video_tracks
            .get_mut(placement.target_track_index)?
            .items;
        let item_index = insert_sorted(target_items, item);
        push_moved_selection(
            selection,
            ItemKey {
                kind: TrackKind::Video,
                track_index: placement.target_track_index,
                item_index,
            },
        );
    }
    Some(())
}

pub(crate) fn move_audio_items(
    project: &mut Project,
    placements: &[ItemPlacement],
    new_track_indices: &[usize],
    overwrite: bool,
    selection: &mut Vec<ItemKey>,
) -> Option<()> {
    let mut moved = Vec::new();
    for placement in placements
        .iter()
        .copied()
        .filter(|placement| placement.key.kind == TrackKind::Audio)
    {
        let item = project
            .audio_tracks
            .get(placement.key.track_index)?
            .items
            .get(placement.key.item_index)?
            .clone();
        moved.push((placement, item));
    }

    remove_audio_items(project, placements)?;
    if overwrite {
        overwrite_audio_items(project, placements, new_track_indices)?;
    }
    insert_new_tracks(&mut project.audio_tracks, new_track_indices)?;
    for (placement, mut item) in moved {
        item.start = placement.start;
        item.end = placement.end;
        let target_items = &mut project
            .audio_tracks
            .get_mut(placement.target_track_index)?
            .items;
        let item_index = insert_sorted(target_items, item);
        push_moved_selection(
            selection,
            ItemKey {
                kind: TrackKind::Audio,
                track_index: placement.target_track_index,
                item_index,
            },
        );
    }
    Some(())
}
