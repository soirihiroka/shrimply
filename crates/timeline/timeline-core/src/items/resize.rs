use super::*;
use shrimply_timeline_snap::SnapRepo;

pub fn resize_drag_for_hit(
    project: &Project,
    selected_items: &[ItemKey],
    key: ItemKey,
    edge: ItemEdge,
    collision_mode: DragCollisionMode,
) -> Option<ResizeDrag> {
    let (start, end) = item_times(project, key)?;
    let resize_seed = if selected_items.contains(&key) {
        selected_items
    } else {
        std::slice::from_ref(&key)
    };
    let mut members = expand_grouped_selection(project, resize_seed);
    if members.iter().all(|member| *member != key) {
        members.push(key);
    }
    members.sort_by_key(item_key_sort_key);
    members.dedup();
    let items: Vec<_> = members
        .into_iter()
        .filter_map(|key| {
            let (start, end) = item_times(project, key)?;
            Some(ResizeDragItem { key, start, end })
        })
        .collect();
    if items.is_empty() {
        return None;
    }

    Some(ResizeDrag {
        key,
        edge,
        start,
        end,
        target_start: start,
        target_end: end,
        collision_mode,
        valid: true,
        preview_status: DragPreviewStatus::Clear,
        blocked_indicators: Vec::new(),
        overwrite_indicators: Vec::new(),
        items,
    })
}

pub fn update_resize_drag(
    drag: &mut ResizeDrag,
    project: &Project,
    view: TimelineViewState,
    x: f64,
    snap_repository: &SnapRepo,
) {
    let minimum_duration = crate::geometry::frame_step(project);
    let target = Time::from_seconds_f64(x_to_time(x, view.scroll_seconds, view.seconds_per_pixel));
    let target = snap_repository.snap(target).unwrap_or(target);

    match drag.edge {
        ItemEdge::Start => {
            let target_delta = target.signed_sub(drag.start);
            let max_delta = drag
                .items
                .iter()
                .map(|item| {
                    item.end
                        .saturating_sub(minimum_duration)
                        .signed_sub(item.start)
                })
                .min()
                .expect("resize drag has no items");
            let earliest_start = drag
                .items
                .iter()
                .map(|item| item.start)
                .min()
                .expect("resize drag has no items");
            let min_delta = Time::ZERO.signed_sub(earliest_start);
            let delta = target_delta.min(max_delta).max(min_delta);
            drag.target_start = drag.start.saturating_add(delta);
            drag.target_end = drag.end;
        }
        ItemEdge::End => {
            let target_delta = target.signed_sub(drag.end);
            let min_delta = drag
                .items
                .iter()
                .map(|item| {
                    item.start
                        .saturating_add(minimum_duration)
                        .signed_sub(item.end)
                })
                .max()
                .expect("resize drag has no items");
            let delta = target_delta.max(min_delta);
            drag.target_start = drag.start;
            drag.target_end = drag.end.saturating_add(delta);
        }
    }
    resolve_resize_drag(project, drag);
}

pub fn resize_item_times(drag: &ResizeDrag, item: &ResizeDragItem) -> Option<(Time, Time)> {
    let (start, end) = match drag.edge {
        ItemEdge::Start => {
            let delta = drag.target_start.signed_sub(drag.start);
            (item.start.saturating_add(delta), item.end)
        }
        ItemEdge::End => {
            let delta = drag.target_end.signed_sub(drag.end);
            (item.start, item.end.saturating_add(delta))
        }
    };
    (start < end).then_some((start, end))
}

pub fn apply_resize_drag(project: &mut Project, drag: ResizeDrag) -> Option<Vec<ItemKey>> {
    let placements = resize_placements(&drag)?;
    if !drag.valid || placements_collide(&placements) {
        return None;
    }
    let overwrite = drag.collision_mode == DragCollisionMode::Overwrite;
    if !overwrite && resize_collides_with_project(project, &drag, &placements) {
        return None;
    }

    let mut selection = Vec::with_capacity(placements.len());
    resize_caption_items(project, &placements, overwrite, &mut selection)?;
    resize_video_items(project, &placements, overwrite, &mut selection)?;
    resize_audio_items(project, &placements, overwrite, &mut selection)?;
    Some(selection)
}

pub fn resize_caption_items(
    project: &mut Project,
    placements: &[ItemPlacement],
    overwrite: bool,
    selection: &mut Vec<ItemKey>,
) -> Option<()> {
    let mut resized = Vec::new();
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
        resized.push((placement, item));
    }

    remove_caption_items(project, placements)?;
    if overwrite {
        overwrite_caption_items(project, placements, &[])?;
    }
    for (placement, mut item) in resized {
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

pub fn resize_video_items(
    project: &mut Project,
    placements: &[ItemPlacement],
    overwrite: bool,
    selection: &mut Vec<ItemKey>,
) -> Option<()> {
    let mut resized = Vec::new();
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
        let source_offset = item_source_offset(project, placement.key);
        resized.push((placement, item, source_offset));
    }

    remove_video_items(project, placements)?;
    if overwrite {
        overwrite_video_items(project, placements, &[])?;
    }
    for (placement, mut item, source_offset) in resized {
        let animation_delta = placement.start.signed_sub(item.start);
        if let Some(source_offset) = source_offset {
            item.time_offset = shifted_media_source_offset(
                source_offset,
                item.start,
                placement.start,
                item.playback_speed,
                item.repeat_strategy,
                item.source_duration,
            );
        }
        item.animation_time_offset = Time {
            seconds: item.animation_time_offset.seconds + animation_delta.seconds,
        };
        item.start = placement.start;
        item.end = placement.end;
        fit_visual_transitions(&mut item);
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

pub fn resize_audio_items(
    project: &mut Project,
    placements: &[ItemPlacement],
    overwrite: bool,
    selection: &mut Vec<ItemKey>,
) -> Option<()> {
    let mut resized = Vec::new();
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
        let source_offset = item_source_offset(project, placement.key);
        resized.push((placement, item, source_offset));
    }

    remove_audio_items(project, placements)?;
    if overwrite {
        overwrite_audio_items(project, placements, &[])?;
    }
    for (placement, mut item, source_offset) in resized {
        if let Some(source_offset) = source_offset {
            item.time_offset = shifted_media_source_offset(
                source_offset,
                item.start,
                placement.start,
                item.playback_speed,
                item.repeat_strategy,
                item.source_duration,
            );
        }
        item.start = placement.start;
        item.end = placement.end;
        fit_audio_transitions(&mut item);
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

pub fn cut_time_for_address(
    project: &Project,
    view: TimelineViewState,
    address: &crate::project::ItemAddress,
    x: f64,
    snap_repository: &SnapRepo,
) -> Option<Time> {
    use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};

    let context = SequenceTimeline::for_item(project, address)?;
    let (start, end) = context.timeline_item_times(project, address)?;
    let minimum_duration = crate::geometry::frame_step(project);
    let cut = Time::from_seconds_f64(x_to_time(x, view.scroll_seconds, view.seconds_per_pixel));
    let cut = snap_repository.snap(cut).unwrap_or(cut);
    (start.saturating_add(minimum_duration)..=end.saturating_sub(minimum_duration))
        .contains(&cut)
        .then_some(cut)
}

pub fn split_item_address(
    project: &mut Project,
    address: &crate::project::ItemAddress,
    timeline_cut: Time,
) -> Option<(crate::project::ItemAddress, crate::project::ItemAddress)> {
    use crate::project::{ItemRef, ProjectItem};

    let track = address.track();
    let cut = project
        .timeline_time_to_sequence(&track, timeline_cut)?
        .snapped(project.frame_step());
    let source = match project.item(address)? {
        ItemRef::Caption(item) => ProjectItem::Caption(item.clone()),
        ItemRef::Video(item) => ProjectItem::Video(Box::new(item.clone())),
        ItemRef::Audio(item) => ProjectItem::Audio(Box::new(item.clone())),
    };
    let (start, end) = source.times();
    if !(start < cut && cut < end) {
        return None;
    }

    let mut left = source.clone();
    let mut right = source;
    match (&mut left, &mut right) {
        (ProjectItem::Caption(left), ProjectItem::Caption(right)) => {
            left.end = cut;
            right.start = cut;
            right.id = uuid::Uuid::new_v4();
        }
        (ProjectItem::Video(left), ProjectItem::Video(right)) => {
            let source_start = right.start;
            let source_offset = right.time_offset;
            left.end = cut;
            left.transitions.outro = None;
            left.transitions.to_next = None;
            right.start = cut;
            right.transitions.intro = None;
            right.id = uuid::Uuid::new_v4();
            Project::regenerate_video_property_ids(right);
            right.time_offset = advanced_media_source_offset(
                source_offset,
                scaled_time_delta(cut.saturating_sub(source_start), right.playback_speed),
                right.repeat_strategy,
                right.source_duration,
            );
            right.animation_time_offset = right
                .animation_time_offset
                .saturating_add(cut.saturating_sub(source_start));
            fit_visual_transitions(left);
            fit_visual_transitions(right);
        }
        (ProjectItem::Audio(left), ProjectItem::Audio(right)) => {
            let source_start = right.start;
            let source_offset = right.time_offset;
            left.end = cut;
            left.transitions.outro = None;
            left.transitions.to_next = None;
            right.start = cut;
            right.transitions.intro = None;
            right.id = uuid::Uuid::new_v4();
            Project::regenerate_audio_property_ids(right);
            right.time_offset = advanced_media_source_offset(
                source_offset,
                scaled_time_delta(cut.saturating_sub(source_start), right.playback_speed),
                right.repeat_strategy,
                right.source_duration,
            );
            fit_audio_transitions(left);
            fit_audio_transitions(right);
        }
        _ => unreachable!("cloned timeline item kinds must match"),
    }

    project
        .take_item(address)
        .expect("validated timeline item must still exist");
    let left = project
        .insert_item(&track, left)
        .expect("split item must return to its source track");
    let right = project
        .insert_item(&track, right)
        .expect("split item must return to its source track");
    Some((left, right))
}

pub fn item_times(project: &Project, key: ItemKey) -> Option<(Time, Time)> {
    match key.kind {
        TrackKind::Caption => project
            .caption_tracks
            .get(key.track_index)?
            .items
            .get(key.item_index)
            .map(|item| (item.start, item.end)),
        TrackKind::Video => project
            .video_tracks
            .get(key.track_index)?
            .items
            .get(key.item_index)
            .map(|item| (item.start, item.end)),
        TrackKind::Audio => project
            .audio_tracks
            .get(key.track_index)?
            .items
            .get(key.item_index)
            .map(|item| (item.start, item.end)),
    }
}

pub fn item_source_offset(project: &Project, key: ItemKey) -> Option<Time> {
    match key.kind {
        TrackKind::Caption => None,
        TrackKind::Video => project
            .video_tracks
            .get(key.track_index)
            .and_then(|track| track.items.get(key.item_index))
            .map(|item| item.time_offset),
        TrackKind::Audio => project
            .audio_tracks
            .get(key.track_index)
            .and_then(|track| track.items.get(key.item_index))
            .map(|item| item.time_offset),
    }
}

pub fn resolve_resize_drag(project: &Project, drag: &mut ResizeDrag) {
    drag.blocked_indicators.clear();
    drag.overwrite_indicators.clear();

    let Some(placements) = resize_placements(drag) else {
        drag.valid = false;
        drag.preview_status = DragPreviewStatus::Blocked;
        return;
    };
    if placements_collide(&placements) {
        drag.valid = false;
        drag.blocked_indicators = placement_indicators(&placements);
        drag.preview_status = DragPreviewStatus::Blocked;
        return;
    }

    match drag.collision_mode {
        DragCollisionMode::Overwrite => {
            drag.overwrite_indicators = resize_collision_indicators(project, drag, &placements);
            drag.valid = true;
            drag.preview_status = if drag.overwrite_indicators.is_empty() {
                DragPreviewStatus::Clear
            } else {
                DragPreviewStatus::Overwrite
            };
        }
        DragCollisionMode::Block | DragCollisionMode::NewTrack => {
            drag.blocked_indicators = resize_collision_indicators(project, drag, &placements);
            drag.valid = drag.blocked_indicators.is_empty();
            drag.preview_status = if drag.valid {
                DragPreviewStatus::Clear
            } else {
                DragPreviewStatus::Blocked
            };
        }
    }
}

pub fn resize_collides_with_project(
    project: &Project,
    drag: &ResizeDrag,
    placements: &[ItemPlacement],
) -> bool {
    placements
        .iter()
        .copied()
        .any(|placement| resize_collides_with_track(project, drag, placement))
}

pub fn resize_placements(drag: &ResizeDrag) -> Option<Vec<ItemPlacement>> {
    if drag.items.iter().all(|item| item.key != drag.key) {
        return None;
    }
    let mut placements = Vec::with_capacity(drag.items.len());
    for item in &drag.items {
        let (start, end) = resize_item_times(drag, item)?;
        placements.push(ItemPlacement {
            key: item.key,
            target_track_index: item.key.track_index,
            start,
            end,
        });
    }
    Some(placements)
}

pub fn resize_collides_with_track(
    project: &Project,
    drag: &ResizeDrag,
    placement: ItemPlacement,
) -> bool {
    match placement.key.kind {
        TrackKind::Caption => project
            .caption_tracks
            .get(placement.target_track_index)
            .is_none_or(|track| resize_collides_with_items(&track.items, drag, placement)),
        TrackKind::Video => project
            .video_tracks
            .get(placement.target_track_index)
            .is_none_or(|track| resize_collides_with_items(&track.items, drag, placement)),
        TrackKind::Audio => project
            .audio_tracks
            .get(placement.target_track_index)
            .is_none_or(|track| resize_collides_with_items(&track.items, drag, placement)),
    }
}

pub fn resize_collides_with_items<T: Clone + TimeSlice>(
    items: &[T],
    drag: &ResizeDrag,
    placement: ItemPlacement,
) -> bool {
    let remaining: Vec<_> = items
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            !drag.items.iter().any(|item| {
                item.key.kind == placement.key.kind
                    && item.key.track_index == placement.target_track_index
                    && item.key.item_index == *index
            })
        })
        .map(|(_, item)| item.clone())
        .collect();
    timeline_search::collides(&remaining, placement.start, placement.end)
}

pub fn resize_collision_indicators(
    project: &Project,
    drag: &ResizeDrag,
    placements: &[ItemPlacement],
) -> Vec<DragIndicator> {
    let mut indicators = Vec::new();
    for placement in placements {
        match placement.key.kind {
            TrackKind::Caption => {
                if let Some(track) = project.caption_tracks.get(placement.target_track_index) {
                    collect_resize_collision_indicators(
                        &track.items,
                        drag,
                        *placement,
                        &mut indicators,
                    );
                }
            }
            TrackKind::Video => {
                if let Some(track) = project.video_tracks.get(placement.target_track_index) {
                    collect_resize_collision_indicators(
                        &track.items,
                        drag,
                        *placement,
                        &mut indicators,
                    );
                }
            }
            TrackKind::Audio => {
                if let Some(track) = project.audio_tracks.get(placement.target_track_index) {
                    collect_resize_collision_indicators(
                        &track.items,
                        drag,
                        *placement,
                        &mut indicators,
                    );
                }
            }
        }
    }
    indicators
}

pub fn collect_resize_collision_indicators<T: TimeSlice>(
    items: &[T],
    drag: &ResizeDrag,
    placement: ItemPlacement,
    indicators: &mut Vec<DragIndicator>,
) {
    for (item_index, item) in items.iter().enumerate() {
        if drag.items.iter().any(|resized| {
            resized.key.kind == placement.key.kind
                && resized.key.track_index == placement.target_track_index
                && resized.key.item_index == item_index
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
