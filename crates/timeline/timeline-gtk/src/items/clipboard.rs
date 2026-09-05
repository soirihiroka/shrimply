use super::*;
use crate::project::{ItemAddress, SequenceScopeId, TrackAddress};
use crate::timeline_operation::TimelineOperationContext;

pub(crate) fn selected_item_addresses(
    context: &impl TimelineOperationContext,
    project: &Project,
    selection: TimelineSelection,
) -> Vec<ItemAddress> {
    let mut selected = Vec::new();
    let start = selection.start.min(selection.end);
    let end = selection.start.max(selection.end);
    let items = context.items(project);
    for (row, track) in track_rows(project).into_iter().enumerate() {
        let track = track.address;
        if !context.contains_track(project, &track) || !selection_intersects_row(selection, row) {
            continue;
        }
        selected.extend(items.iter().filter_map(|item| {
            if item.track() != track {
                return None;
            }
            let (item_start, item_end) = context.timeline_item_times(project, item)?;
            (item_start <= end && item_end >= start).then(|| item.clone())
        }));
    }
    selected
}

pub(crate) fn copy_items(
    project: &Project,
    selected_items: &[ItemAddress],
) -> Option<TimelineClipboard> {
    let sequence_scope = project.item_scope(selected_items.first()?)?;
    if selected_items
        .iter()
        .any(|address| project.item_scope(address).as_ref() != Some(&sequence_scope))
    {
        return None;
    }
    let origin = selected_items
        .iter()
        .filter_map(|address| project.projected_item_times(address).map(|times| times.0))
        .min()?;

    let mut items = Vec::new();
    for address in selected_items {
        let item = match project.item(address)? {
            ItemRef::Caption(item) => ProjectItem::Caption(item.clone()),
            ItemRef::Video(item) => ProjectItem::Video(Box::new(item.clone())),
            ItemRef::Audio(item) => ProjectItem::Audio(Box::new(item.clone())),
        };
        let (start, end) = project.projected_item_times(address)?;
        items.push(CopiedItem {
            track_index: scoped_track_index(project, &address.track())?,
            start_offset: start.saturating_sub(origin),
            duration: end.saturating_sub(start),
            item,
        });
    }

    (!items.is_empty()).then_some(TimelineClipboard { items })
}

pub(crate) fn paste_items(
    project: &mut Project,
    clipboard: &TimelineClipboard,
    sequence_scope: &SequenceScopeId,
    timeline_start: Time,
) -> PasteResult {
    let mut result = PasteResult {
        selection: Vec::new(),
        captions: false,
        video: false,
        audio: false,
    };
    let mut remapping = PasteRemapping {
        next_group_id: next_group_id(project),
        group_map: HashMap::new(),
        sequence_instance_map: HashMap::new(),
        item_map: HashMap::new(),
    };
    paste_caption_items(
        project,
        clipboard,
        sequence_scope,
        timeline_start,
        &mut result,
        &mut remapping.next_group_id,
        &mut remapping.group_map,
    );
    paste_video_items(
        project,
        clipboard,
        sequence_scope,
        timeline_start,
        &mut result,
        &mut remapping,
    );
    paste_audio_items(
        project,
        clipboard,
        sequence_scope,
        timeline_start,
        &mut result,
        &mut remapping,
    );
    result
}

struct PasteRemapping {
    next_group_id: u64,
    group_map: HashMap<u64, u64>,
    sequence_instance_map: HashMap<Uuid, Uuid>,
    item_map: HashMap<Uuid, Uuid>,
}

fn scoped_track_index(project: &Project, track: &TrackAddress) -> Option<usize> {
    match track {
        TrackAddress::Caption { track_id } => project
            .caption_tracks
            .iter()
            .position(|track| track.id == *track_id),
        TrackAddress::Video {
            sequence_path,
            track_id,
        } => project
            .video_tracks_for_path(sequence_path)?
            .iter()
            .position(|track| track.id == *track_id),
        TrackAddress::Audio {
            sequence_path,
            track_id,
        } => project
            .audio_tracks_for_path(sequence_path)?
            .iter()
            .position(|track| track.id == *track_id),
    }
}

pub(crate) fn remap_item_group_id(
    next_group_id: &mut u64,
    group_map: &mut HashMap<u64, u64>,
    group_id: Option<u64>,
) -> Option<u64> {
    group_id.map(|group_id| {
        *group_map.entry(group_id).or_insert_with(|| {
            let id = *next_group_id;
            *next_group_id = next_group_id.saturating_add(1);
            id
        })
    })
}

pub(crate) fn paste_caption_items(
    project: &mut Project,
    clipboard: &TimelineClipboard,
    sequence_scope: &SequenceScopeId,
    start: Time,
    result: &mut PasteResult,
    next_group_id: &mut u64,
    group_map: &mut HashMap<u64, u64>,
) {
    if !sequence_scope.is_root() {
        return;
    }
    let items: Vec<_> = clipboard
        .items
        .iter()
        .filter_map(|copied| match &copied.item {
            ProjectItem::Caption(item) => Some((
                copied.track_index,
                copied.start_offset,
                copied.duration,
                item.clone(),
            )),
            ProjectItem::Video(_) | ProjectItem::Audio(_) => None,
        })
        .map(|(track_index, start_offset, duration, mut item)| {
            item.id = uuid::Uuid::new_v4();
            item.group_id = remap_item_group_id(next_group_id, group_map, item.group_id);
            let item_start = start.saturating_add(start_offset);
            (
                track_index,
                item_start,
                item_start.saturating_add(duration),
                item,
            )
        })
        .collect();
    if items.is_empty() {
        return;
    }

    let (source_base, footprint) = paste_footprint(&items);
    let target_base = choose_track_base(
        project.caption_tracks.len(),
        &footprint,
        Some(source_base),
        |track_index, start, end| {
            timeline_search::collides(&project.caption_tracks[track_index].items, start, end)
        },
        |_, _, _| false,
    );
    ensure_tracks(
        &mut project.caption_tracks,
        target_base + track_footprint_span(&footprint),
    );

    for (source_track, start, end, mut item) in items {
        let track_index = target_base + source_track - source_base;
        item.start = start;
        item.end = end;
        let Some(track) = project.caption_tracks.get_mut(track_index) else {
            continue;
        };
        let item_id = item.id;
        insert_sorted(&mut track.items, item);
        result.selection.push(ItemAddress::Caption {
            track_id: track.id,
            item_id,
        });
    }
    result.captions = true;
}

fn paste_video_items(
    project: &mut Project,
    clipboard: &TimelineClipboard,
    sequence_scope: &SequenceScopeId,
    timeline_start: Time,
    result: &mut PasteResult,
    remapping: &mut PasteRemapping,
) {
    let Some(sequence_path) =
        project.sequence_path_for_scope(crate::project::ItemKind::Video, sequence_scope)
    else {
        return;
    };
    let mut items: Vec<_> = clipboard
        .items
        .iter()
        .filter_map(|copied| match &copied.item {
            ProjectItem::Video(item) => Some((
                copied.track_index,
                copied.start_offset,
                copied.duration,
                item.as_ref().clone(),
            )),
            ProjectItem::Caption(_) | ProjectItem::Audio(_) => None,
        })
        .filter_map(|(track_index, start_offset, duration, mut item)| {
            let source_id = item.id;
            item.id = uuid::Uuid::new_v4();
            remapping.item_map.insert(source_id, item.id);
            item.group_id = remap_item_group_id(
                &mut remapping.next_group_id,
                &mut remapping.group_map,
                item.group_id,
            );
            if let VideoItemContent::FoldedSequence(reference) = &mut item.content {
                reference.instance_id = *remapping
                    .sequence_instance_map
                    .entry(reference.instance_id)
                    .or_insert_with(Uuid::new_v4);
            }
            let timeline_item_start = timeline_start.saturating_add(start_offset);
            let timeline_item_end = timeline_item_start.saturating_add(duration);
            let first = project
                .timeline_time_to_sequence_path(
                    crate::project::ItemKind::Video,
                    &sequence_path,
                    timeline_item_start,
                )?
                .snapped(project.frame_step());
            let second = project
                .timeline_time_to_sequence_path(
                    crate::project::ItemKind::Video,
                    &sequence_path,
                    timeline_item_end,
                )?
                .snapped(project.frame_step());
            (first != second).then_some((track_index, first.min(second), first.max(second), item))
        })
        .collect();
    for (_, _, _, item) in &mut items {
        if let Some(transition) = item.transitions.to_next.as_mut() {
            match remapping.item_map.get(&transition.target_item_id) {
                Some(target) => transition.target_item_id = *target,
                None => item.transitions.to_next = None,
            }
        }
    }
    if items.is_empty() {
        return;
    }

    let Some(tracks) = project.video_tracks_for_path(&sequence_path) else {
        return;
    };
    let (source_base, footprint) = paste_footprint(&items);
    let target_base = choose_track_base(
        tracks.len(),
        &footprint,
        None,
        |track_index, start, end| timeline_search::collides(&tracks[track_index].items, start, end),
        |track_index, start, end| visual_track_is_obscured(tracks, track_index, start, end),
    );
    let required_tracks = target_base + track_footprint_span(&footprint);
    let Some(tracks) = video_tracks_mut_for_path(project, &sequence_path) else {
        return;
    };
    ensure_tracks(tracks, required_tracks);

    for (source_track, start, end, mut item) in items {
        let track_index = target_base + source_track - source_base;
        item.start = start;
        item.end = end;
        let Some(track) = tracks.get_mut(track_index) else {
            continue;
        };
        let item_id = item.id;
        insert_sorted(&mut track.items, item);
        result.selection.push(ItemAddress::Video {
            sequence_path: sequence_path.clone(),
            track_id: track.id,
            item_id,
        });
    }
    result.video = true;
}

fn paste_audio_items(
    project: &mut Project,
    clipboard: &TimelineClipboard,
    sequence_scope: &SequenceScopeId,
    timeline_start: Time,
    result: &mut PasteResult,
    remapping: &mut PasteRemapping,
) {
    let Some(sequence_path) =
        project.sequence_path_for_scope(crate::project::ItemKind::Audio, sequence_scope)
    else {
        return;
    };
    let mut items: Vec<_> = clipboard
        .items
        .iter()
        .filter_map(|copied| match &copied.item {
            ProjectItem::Audio(item) => Some((
                copied.track_index,
                copied.start_offset,
                copied.duration,
                item.as_ref().clone(),
            )),
            ProjectItem::Caption(_) | ProjectItem::Video(_) => None,
        })
        .filter_map(|(track_index, start_offset, duration, mut item)| {
            let source_id = item.id;
            item.id = uuid::Uuid::new_v4();
            remapping.item_map.insert(source_id, item.id);
            item.group_id = remap_item_group_id(
                &mut remapping.next_group_id,
                &mut remapping.group_map,
                item.group_id,
            );
            if let AudioSource::FoldedSequence(reference) = &mut item.source {
                reference.instance_id = *remapping
                    .sequence_instance_map
                    .entry(reference.instance_id)
                    .or_insert_with(Uuid::new_v4);
            }
            let timeline_item_start = timeline_start.saturating_add(start_offset);
            let timeline_item_end = timeline_item_start.saturating_add(duration);
            let first = project
                .timeline_time_to_sequence_path(
                    crate::project::ItemKind::Audio,
                    &sequence_path,
                    timeline_item_start,
                )?
                .snapped(project.frame_step());
            let second = project
                .timeline_time_to_sequence_path(
                    crate::project::ItemKind::Audio,
                    &sequence_path,
                    timeline_item_end,
                )?
                .snapped(project.frame_step());
            (first != second).then_some((track_index, first.min(second), first.max(second), item))
        })
        .collect();
    for (_, _, _, item) in &mut items {
        if let Some(transition) = item.transitions.to_next.as_mut() {
            match remapping.item_map.get(&transition.target_item_id) {
                Some(target) => transition.target_item_id = *target,
                None => item.transitions.to_next = None,
            }
        }
    }
    if items.is_empty() {
        return;
    }

    let Some(tracks) = project.audio_tracks_for_path(&sequence_path) else {
        return;
    };
    let (source_base, footprint) = paste_footprint(&items);
    let target_base = choose_track_base(
        tracks.len(),
        &footprint,
        Some(source_base),
        |track_index, start, end| timeline_search::collides(&tracks[track_index].items, start, end),
        |_, _, _| false,
    );
    let required_tracks = target_base + track_footprint_span(&footprint);
    let Some(tracks) = audio_tracks_mut_for_path(project, &sequence_path) else {
        return;
    };
    ensure_tracks(tracks, required_tracks);

    for (source_track, start, end, mut item) in items {
        let track_index = target_base + source_track - source_base;
        item.start = start;
        item.end = end;
        let Some(track) = tracks.get_mut(track_index) else {
            continue;
        };
        let item_id = item.id;
        insert_sorted(&mut track.items, item);
        result.selection.push(ItemAddress::Audio {
            sequence_path: sequence_path.clone(),
            track_id: track.id,
            item_id,
        });
    }
    result.audio = true;
}

fn video_tracks_mut_for_path<'a>(
    project: &'a mut Project,
    sequence_path: &[Uuid],
) -> Option<&'a mut Vec<VisualTrack>> {
    let Some((host_id, parent_path)) = sequence_path.split_last() else {
        return Some(&mut project.video_tracks);
    };
    let sequence_id = project
        .video_tracks_for_path(parent_path)?
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == *host_id)
        .and_then(|item| match &item.content {
            VideoItemContent::FoldedSequence(reference) => Some(reference.sequence_id),
            _ => None,
        })?;
    Some(&mut project.folded_sequence_mut(sequence_id)?.video_tracks)
}

fn audio_tracks_mut_for_path<'a>(
    project: &'a mut Project,
    sequence_path: &[Uuid],
) -> Option<&'a mut Vec<AudioTrack>> {
    let Some((host_id, parent_path)) = sequence_path.split_last() else {
        return Some(&mut project.audio_tracks);
    };
    let sequence_id = project
        .audio_tracks_for_path(parent_path)?
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == *host_id)
        .and_then(|item| match item.source {
            AudioSource::FoldedSequence(reference) => Some(reference.sequence_id),
            AudioSource::Media | AudioSource::Tts(_) | AudioSource::Generator(_) => None,
        })?;
    Some(&mut project.folded_sequence_mut(sequence_id)?.audio_tracks)
}

fn paste_footprint<T>(items: &[(usize, Time, Time, T)]) -> (usize, Vec<TrackFootprintItem>) {
    let source_base = items
        .iter()
        .map(|(track_index, _, _, _)| *track_index)
        .min()
        .unwrap_or(0);
    let footprint = items
        .iter()
        .map(|(source_track, start, end, _)| TrackFootprintItem {
            track_offset: source_track - source_base,
            start: *start,
            end: *end,
        })
        .collect();
    (source_base, footprint)
}

pub(crate) fn ensure_tracks<T: Default>(tracks: &mut Vec<T>, count: usize) {
    if tracks.len() < count {
        tracks.resize_with(count, T::default);
    }
}

pub(crate) fn push_moved_selection(selection: &mut Vec<ItemKey>, key: ItemKey) {
    for selected in selection.iter_mut() {
        if selected.kind == key.kind
            && selected.track_index == key.track_index
            && selected.item_index >= key.item_index
        {
            selected.item_index += 1;
        }
    }
    selection.push(key);
}

pub(crate) fn selection_intersects_row(selection: TimelineSelection, row: usize) -> bool {
    let selection_top = selection.start_y.min(selection.end_y);
    let selection_bottom = selection.start_y.max(selection.end_y);
    let row_top = row_y(row);
    let row_bottom = row_top + TRACK_HEIGHT;
    selection_top <= row_bottom && selection_bottom >= row_top
}
