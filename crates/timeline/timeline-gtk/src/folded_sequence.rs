use crate::Fraction;
use crate::items::{
    DragCollisionMode, DragPreviewStatus, ItemKey, OverwriteItem, TrackKind, fit_audio_transitions,
    fit_visual_transitions, hit_item_at, overwrite_items,
};
use crate::project::{
    AudioItem, AudioSource, ItemAddress, ItemKind, Project, ProjectItem, SequenceReference, Time,
    TrackAddress, TrackMut, VideoItem, VideoItemContent, scaled_time_delta, unscaled_time_delta,
};
use uuid::Uuid;

use super::{TRACK_HEIGHT, TimelineViewState, timeline_x};
use crate::timeline_operation::TimelineOperationContext;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum FoldedDragKind {
    Move,
    ResizeStart,
    ResizeEnd,
}

#[derive(Clone)]
pub(super) struct FoldedDrag {
    pub(super) key: ItemAddress,
    pub(super) target_track: TrackAddress,
    pub(super) kind: FoldedDragKind,
    pub(super) start: Time,
    pub(super) end: Time,
    pub(super) target_start: Time,
    pub(super) target_end: Time,
    pub(super) pointer_offset: Time,
    pub(super) items: Vec<FoldedDragItem>,
    pub(super) cross_scope_preview_row: Option<usize>,
    pub(super) valid_drop: bool,
    pub(super) preview_status: DragPreviewStatus,
}

#[derive(Clone)]
pub(super) struct FoldedDragItem {
    pub(super) key: ItemAddress,
    pub(super) start: Time,
    pub(super) end: Time,
    pub(super) target_track: TrackAddress,
    pub(super) target_start: Time,
    pub(super) target_end: Time,
}

pub(super) struct ProjectedItemHit {
    pub(super) key: ItemAddress,
    pub(super) start: Time,
    pub(super) end: Time,
}

pub(super) fn hit_folded_item(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
) -> Option<Vec<Uuid>> {
    if let Some(key) = hit_item_at(project, view, x, y)
        && reference(project, key).is_some()
    {
        return Some(vec![item_id(project, key)?]);
    }

    let row = ((y + view.scroll_y - super::RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize;
    for track_index in 0..project.video_tracks.len() {
        let parent_row = crate::items::row_for_track(project, TrackKind::Video, track_index)?;
        for track in projected_video_tracks(project, track_index, parent_row) {
            if track.row == row {
                for item in track.items {
                    if let Some(path) = item.sequence_path
                        && hits_item(x, item.item.start, item.item.end, view)
                    {
                        return Some(path);
                    }
                }
            }
        }
    }
    for track_index in 0..project.audio_tracks.len() {
        let parent_row = crate::items::row_for_track(project, TrackKind::Audio, track_index)?;
        for track in projected_audio_tracks(project, track_index, parent_row) {
            if track.row == row {
                for item in track.items {
                    if let Some(path) = item.sequence_path
                        && hits_item(x, item.item.start, item.item.end, view)
                    {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

pub(super) fn hit_projected_item(
    project: &Project,
    view: TimelineViewState,
    x: f64,
    y: f64,
) -> Option<ProjectedItemHit> {
    if x < timeline_x() || y < super::RULER_HEIGHT {
        return None;
    }
    let row = ((y + view.scroll_y - super::RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize;
    for track_index in 0..project.video_tracks.len() {
        let parent_row = crate::items::row_for_track(project, TrackKind::Video, track_index)?;
        for track in projected_video_tracks(project, track_index, parent_row) {
            if track.row != row {
                continue;
            }
            let item = track
                .items
                .iter()
                .find(|item| hits_item(x, item.item.start, item.item.end, view))?;
            return Some(ProjectedItemHit {
                key: ItemAddress::Video {
                    sequence_path: track.sequence_path,
                    track_id: track.track_id,
                    item_id: item.item.id,
                },
                start: item.item.start,
                end: item.item.end,
            });
        }
    }
    for track_index in 0..project.audio_tracks.len() {
        let parent_row = crate::items::row_for_track(project, TrackKind::Audio, track_index)?;
        for track in projected_audio_tracks(project, track_index, parent_row) {
            if track.row != row {
                continue;
            }
            let item = track
                .items
                .iter()
                .find(|item| hits_item(x, item.item.start, item.item.end, view))?;
            return Some(ProjectedItemHit {
                key: ItemAddress::Audio {
                    sequence_path: track.sequence_path,
                    track_id: track.track_id,
                    item_id: item.item.id,
                },
                start: item.item.start,
                end: item.item.end,
            });
        }
    }
    None
}

pub(super) fn begin_drag(
    project: &Project,
    hit: ProjectedItemHit,
    kind: FoldedDragKind,
    pointer_seconds: f64,
    selected: &[ItemAddress],
) -> Option<FoldedDrag> {
    project.item(&hit.key)?;
    let scope = project.item_scope(&hit.key)?;
    let target_track = hit.key.track();
    let mut addresses = std::iter::once(hit.key.clone())
        .chain(selected.iter().cloned())
        .filter(|address| project.item_scope(address).as_ref() == Some(&scope))
        .filter(|address| project.item(address).is_some())
        .collect::<Vec<_>>();
    let mut identities = Vec::new();
    addresses.retain(|address| {
        let identity = (address.kind(), address.track_id(), address.item_id());
        if identities.contains(&identity) {
            false
        } else {
            identities.push(identity);
            true
        }
    });
    let items = addresses
        .into_iter()
        .filter_map(|key| {
            let (start, end) = project.timeline_item_times(&key)?;
            Some(FoldedDragItem {
                target_track: key.track(),
                key,
                start,
                end,
                target_start: start,
                target_end: end,
            })
        })
        .collect::<Vec<_>>();
    if items.iter().all(|item| item.key != hit.key) {
        return None;
    }
    Some(FoldedDrag {
        key: hit.key,
        target_track,
        kind,
        start: hit.start,
        end: hit.end,
        target_start: hit.start,
        target_end: hit.end,
        pointer_offset: Time::from_seconds_f64(pointer_seconds).signed_sub(hit.start),
        items,
        cross_scope_preview_row: None,
        valid_drop: true,
        preview_status: DragPreviewStatus::Clear,
    })
}

impl FoldedDrag {
    pub(super) fn preview(&self, address: &ItemAddress) -> Option<&FoldedDragItem> {
        self.items.iter().find(|item| &item.key == address)
    }

    pub(super) fn preview_at(&self, address: &ItemAddress) -> Option<&FoldedDragItem> {
        self.items.iter().find(|item| {
            item.key.item_id() == address.item_id() && item.target_track == address.track()
        })
    }

    pub(super) fn set_target_track(
        &mut self,
        context: &dyn TimelineOperationContext,
        project: &Project,
        target: TrackAddress,
    ) -> bool {
        if !context.allows_track_drop(project, &self.key, &target) {
            return false;
        }
        let source_tracks = concrete_tracks(project, self.key.kind(), self.key.sequence_path());
        let Some(source_index) = source_tracks
            .iter()
            .position(|track| *track == self.key.track())
        else {
            return false;
        };
        let Some(destination_scope) = project.track_scope(&target) else {
            return false;
        };
        let destination_tracks = concrete_tracks(project, target.kind(), target.sequence_path());
        let Some(target_index) = destination_tracks.iter().position(|track| *track == target)
        else {
            return false;
        };
        let track_offset = target_index as isize - source_index as isize;
        for item in &mut self.items {
            let source_kind_tracks =
                concrete_tracks(project, item.key.kind(), item.key.sequence_path());
            let Some(index) = source_kind_tracks
                .iter()
                .position(|track| *track == item.key.track())
            else {
                return false;
            };
            let destination_path = if item.key.kind() == target.kind() {
                Some(target.sequence_path().to_vec())
            } else {
                project.sequence_path_for_scope(item.key.kind(), &destination_scope)
            };
            let Some(destination_path) = destination_path else {
                return false;
            };
            let destination_kind_tracks =
                concrete_tracks(project, item.key.kind(), &destination_path);
            let target_index = index as isize + track_offset;
            if target_index < 0 || target_index as usize >= destination_kind_tracks.len() {
                return false;
            }
            item.target_track = destination_kind_tracks[target_index as usize].clone();
        }
        self.target_track = target;
        true
    }

    fn update_item_times(&mut self) {
        let start_delta = self.target_start.signed_sub(self.start);
        let end_delta = self.target_end.signed_sub(self.end);
        for item in &mut self.items {
            match self.kind {
                FoldedDragKind::Move => {
                    item.target_start = item.start.saturating_add(start_delta);
                    item.target_end = item.end.saturating_add(start_delta);
                }
                FoldedDragKind::ResizeStart => {
                    item.target_start = item.start.saturating_add(start_delta);
                    item.target_end = item.end;
                }
                FoldedDragKind::ResizeEnd => {
                    item.target_start = item.start;
                    item.target_end = item.end.saturating_add(end_delta);
                }
            }
        }
    }
}

fn concrete_tracks(project: &Project, kind: ItemKind, path: &[Uuid]) -> Vec<TrackAddress> {
    match kind {
        ItemKind::Caption => project
            .caption_tracks
            .iter()
            .filter(|_| path.is_empty())
            .map(|track| TrackAddress::Caption { track_id: track.id })
            .collect(),
        ItemKind::Video => project
            .video_tracks_for_path(path)
            .into_iter()
            .flatten()
            .map(|track| TrackAddress::Video {
                sequence_path: path.to_vec(),
                track_id: track.id,
            })
            .collect(),
        ItemKind::Audio => project
            .audio_tracks_for_path(path)
            .into_iter()
            .flatten()
            .map(|track| TrackAddress::Audio {
                sequence_path: path.to_vec(),
                track_id: track.id,
            })
            .collect(),
    }
}

pub(super) fn apply_resize_drag(
    project: &mut Project,
    drag: &FoldedDrag,
    collision_mode: DragCollisionMode,
) -> Option<Vec<ItemAddress>> {
    if drag.kind == FoldedDragKind::Move {
        return None;
    }
    apply_group_drag(project, drag, collision_mode)
}

pub(super) fn apply_move_drag(
    project: &mut Project,
    drag: &FoldedDrag,
    collision_mode: DragCollisionMode,
) -> Option<Vec<ItemAddress>> {
    (drag.kind == FoldedDragKind::Move).then(|| apply_group_drag(project, drag, collision_mode))?
}

struct FoldedPlacement {
    source: ItemAddress,
    target: TrackAddress,
    start: Time,
    end: Time,
    item: ProjectItem,
}

fn apply_group_drag(
    project: &mut Project,
    drag: &FoldedDrag,
    collision_mode: DragCollisionMode,
) -> Option<Vec<ItemAddress>> {
    let scope = project.item_scope(&drag.key)?;
    let target_scope = project.track_scope(&drag.target_track)?;
    if drag.items.is_empty()
        || drag
            .items
            .iter()
            .any(|item| project.item_scope(&item.key).as_ref() != Some(&scope))
    {
        return None;
    }
    let mut candidate = project.clone();
    let mut placements = Vec::with_capacity(drag.items.len());
    for member in &drag.items {
        if project.track_scope(&member.target_track).as_ref() != Some(&target_scope)
            || member.target_track.kind() != member.key.kind()
            || !project
                .can_move_item_to_sequence_path(&member.key, member.target_track.sequence_path())
        {
            return None;
        }
        let first = project
            .timeline_time_to_sequence(&member.target_track, member.target_start)?
            .snapped(project.frame_step());
        let second = project
            .timeline_time_to_sequence(&member.target_track, member.target_end)?
            .snapped(project.frame_step());
        let (start, end) = (first.min(second), first.max(second));
        if start >= end {
            return None;
        }
        let mut item = candidate.take_item(&member.key)?;
        match &mut item {
            ProjectItem::Caption(_) => return None,
            ProjectItem::Video(item) => apply_video_drag(item, drag.kind, start, end),
            ProjectItem::Audio(item) => apply_audio_drag(item, drag.kind, start, end),
        }
        placements.push(FoldedPlacement {
            source: member.key.clone(),
            target: member.target_track.clone(),
            start,
            end,
            item,
        });
    }
    if placements.iter().enumerate().any(|(index, placement)| {
        placements[index + 1..].iter().any(|other| {
            placement.target == other.target
                && placement.start < other.end
                && placement.end > other.start
        })
    }) {
        return None;
    }

    let mut new_tracks = Vec::<(TrackAddress, TrackAddress)>::new();
    if collision_mode == DragCollisionMode::NewTrack {
        for placement in &placements {
            if new_tracks
                .iter()
                .any(|(target, _)| target == &placement.target)
                || !track_collides(
                    &candidate,
                    &placement.target,
                    placement.start,
                    placement.end,
                )
            {
                continue;
            }
            let mut item = placement.item.clone();
            item.set_times(placement.start, placement.end);
            let address =
                candidate.insert_item_on_new_track(placement.target.sequence_path(), item)?;
            new_tracks.push((placement.target.clone(), address.track()));
            candidate
                .take_item(&address)
                .expect("new-track placement must be removable");
        }
    }

    if collision_mode == DragCollisionMode::Block
        && placements.iter().any(|placement| {
            track_collides(
                &candidate,
                &placement.target,
                placement.start,
                placement.end,
            )
        })
    {
        return None;
    }
    if collision_mode == DragCollisionMode::Overwrite {
        for placement in &placements {
            overwrite_track(
                &mut candidate,
                &placement.target,
                placement.start,
                placement.end,
            )?;
        }
    }

    let mut moved = Vec::with_capacity(placements.len());
    for placement in placements {
        let target = new_tracks
            .iter()
            .find_map(|(source, target)| (source == &placement.target).then_some(target))
            .unwrap_or(&placement.target);
        if track_collides(&candidate, target, placement.start, placement.end) {
            return None;
        }
        let address = candidate
            .insert_item(target, placement.item)
            .expect("prevalidated folded group insertion must succeed");
        debug_assert_eq!(address.item_id(), placement.source.item_id());
        moved.push(address);
    }
    *project = candidate;
    Some(moved)
}

fn track_collides(project: &Project, track: &TrackAddress, start: Time, end: Time) -> bool {
    match project.track(track) {
        Some(crate::project::TrackRef::Caption(track)) => track
            .items
            .iter()
            .any(|item| item.start < end && item.end > start),
        Some(crate::project::TrackRef::Video(track)) => track
            .items
            .iter()
            .any(|item| item.start < end && item.end > start),
        Some(crate::project::TrackRef::Audio(track)) => track
            .items
            .iter()
            .any(|item| item.start < end && item.end > start),
        None => true,
    }
}

fn overwrite_track(
    project: &mut Project,
    track: &TrackAddress,
    start: Time,
    end: Time,
) -> Option<()> {
    match project.track_mut(track)? {
        TrackMut::Caption(track) => overwrite_items(&mut track.items, start, end),
        TrackMut::Video(track) => overwrite_items(&mut track.items, start, end),
        TrackMut::Audio(track) => overwrite_items(&mut track.items, start, end),
    }
    Some(())
}

pub(super) fn can_apply_drag(project: &Project, drag: &FoldedDrag) -> bool {
    let mut candidate = project.clone();
    apply_group_drag(&mut candidate, drag, DragCollisionMode::Block).is_some()
}

pub(super) fn move_item_with_collision(
    project: &mut Project,
    source: &ItemAddress,
    target: &TrackAddress,
    start: Time,
    end: Time,
    collision_mode: DragCollisionMode,
) -> Option<ItemAddress> {
    match collision_mode {
        DragCollisionMode::Overwrite => {
            overwrite_drag(project, source, target, FoldedDragKind::Move, start, end)
        }
        DragCollisionMode::Block => project.move_item(source, target, start, end),
        DragCollisionMode::NewTrack => project
            .move_item(source, target, start, end)
            .or_else(|| project.move_item_to_new_track(source, target.sequence_path(), start, end)),
    }
}

fn overwrite_drag(
    project: &mut Project,
    address: &ItemAddress,
    track: &TrackAddress,
    kind: FoldedDragKind,
    start: Time,
    end: Time,
) -> Option<ItemAddress> {
    if start >= end
        || !project.can_insert_item(track, address.kind())
        || !project.can_move_item_to_sequence_path(address, track.sequence_path())
    {
        return None;
    }
    if address.kind() == ItemKind::Caption && kind != FoldedDragKind::Move {
        panic!("caption overwrite resizing must use the root resize operation");
    }
    let mut item = project.take_item(address)?;
    match &mut item {
        ProjectItem::Caption(item) => {
            item.start = start;
            item.end = end;
        }
        ProjectItem::Video(item) => apply_video_drag(item, kind, start, end),
        ProjectItem::Audio(item) => apply_audio_drag(item, kind, start, end),
    }
    match (project.track_mut(track), &item) {
        (Some(TrackMut::Caption(track)), ProjectItem::Caption(_)) => {
            overwrite_items(&mut track.items, start, end);
        }
        (Some(TrackMut::Video(track)), ProjectItem::Video(_)) => {
            overwrite_items(&mut track.items, start, end);
        }
        (Some(TrackMut::Audio(track)), ProjectItem::Audio(_)) => {
            overwrite_items(&mut track.items, start, end);
        }
        _ => panic!("prevalidated folded item insertion must succeed"),
    }
    let address = project
        .insert_item(track, item)
        .expect("prevalidated folded item insertion must succeed");
    Some(address)
}

fn apply_video_drag(item: &mut VideoItem, kind: FoldedDragKind, start: Time, end: Time) {
    match kind {
        FoldedDragKind::Move => {
            item.start = start;
            item.end = end;
        }
        FoldedDragKind::ResizeStart => item.trim_start(start),
        FoldedDragKind::ResizeEnd => item.set_end(end),
    }
    fit_visual_transitions(item);
}

fn apply_audio_drag(item: &mut AudioItem, kind: FoldedDragKind, start: Time, end: Time) {
    match kind {
        FoldedDragKind::Move => {
            item.start = start;
            item.end = end;
        }
        FoldedDragKind::ResizeStart => item.trim_start(start),
        FoldedDragKind::ResizeEnd => item.set_end(end),
    }
    fit_audio_transitions(item);
}

pub(super) fn track_kind(address: &ItemAddress) -> TrackKind {
    match address.kind() {
        ItemKind::Caption => TrackKind::Caption,
        ItemKind::Video => TrackKind::Video,
        ItemKind::Audio => TrackKind::Audio,
    }
}

pub(super) fn update_drag(drag: &mut FoldedDrag, target: Time, minimum_duration: Time) {
    match drag.kind {
        FoldedDragKind::Move => {
            let earliest_offset = drag
                .items
                .iter()
                .map(|item| item.start.signed_sub(drag.start))
                .min()
                .unwrap_or(Time::ZERO)
                .min(Time::ZERO);
            let start = target.max(Time::ZERO.signed_sub(earliest_offset));
            let duration = drag.end.saturating_sub(drag.start);
            drag.target_start = start;
            drag.target_end = start.saturating_add(duration);
        }
        FoldedDragKind::ResizeStart => {
            let max_delta = drag
                .items
                .iter()
                .map(|item| {
                    item.end
                        .saturating_sub(item.start)
                        .signed_sub(minimum_duration)
                })
                .min()
                .unwrap_or(Time::ZERO);
            let delta = target.signed_sub(drag.start).min(max_delta);
            drag.target_start = drag.start.saturating_add(delta);
        }
        FoldedDragKind::ResizeEnd => {
            let min_delta = drag
                .items
                .iter()
                .map(|item| minimum_duration.signed_sub(item.end.saturating_sub(item.start)))
                .max()
                .unwrap_or(Time::ZERO);
            let delta = target.signed_sub(drag.end).max(min_delta);
            drag.target_end = drag.end.saturating_add(delta);
        }
    }
    drag.update_item_times();
}

fn hits_item(x: f64, start: Time, end: Time, view: TimelineViewState) -> bool {
    let start_x =
        timeline_x() + (start.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    let end_x = timeline_x() + (end.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    x >= start_x && x <= end_x
}

pub(super) fn reference(project: &Project, key: ItemKey) -> Option<SequenceReference> {
    match key.kind {
        TrackKind::Video => match &project
            .video_tracks
            .get(key.track_index)?
            .items
            .get(key.item_index)?
            .content
        {
            VideoItemContent::FoldedSequence(reference) => Some(*reference),
            _ => None,
        },
        TrackKind::Audio => match project
            .audio_tracks
            .get(key.track_index)?
            .items
            .get(key.item_index)?
            .source
        {
            AudioSource::FoldedSequence(reference) => Some(reference),
            AudioSource::Media | AudioSource::Tts(_) | AudioSource::Generator(_) => None,
        },
        TrackKind::Caption => None,
    }
}

fn item_id(project: &Project, key: ItemKey) -> Option<Uuid> {
    match key.kind {
        TrackKind::Video => project
            .video_tracks
            .get(key.track_index)?
            .items
            .get(key.item_index)
            .map(|item| item.id),
        TrackKind::Audio => project
            .audio_tracks
            .get(key.track_index)?
            .items
            .get(key.item_index)
            .map(|item| item.id),
        TrackKind::Caption => None,
    }
}

pub(super) struct ProjectedVideoTrack {
    pub(super) row: usize,
    pub(super) sequence_path: Vec<Uuid>,
    pub(super) track_id: Uuid,
    pub(super) items: Vec<ProjectedVideoItem>,
}

pub(super) struct ProjectedVideoItem {
    pub(super) item: VideoItem,
    pub(super) sequence_path: Option<Vec<Uuid>>,
    pub(super) played_range: Option<(Time, Time)>,
}

pub(super) struct ProjectedAudioTrack {
    pub(super) row: usize,
    pub(super) sequence_path: Vec<Uuid>,
    pub(super) track_id: Uuid,
    pub(super) items: Vec<ProjectedAudioItem>,
}

pub(super) struct ProjectedAudioItem {
    pub(super) item: AudioItem,
    pub(super) sequence_path: Option<Vec<Uuid>>,
    pub(super) played_range: Option<(Time, Time)>,
}

#[derive(Clone, Copy)]
struct VideoProjectionHost<'a> {
    item: &'a VideoItem,
    played_range: Option<(Time, Time)>,
}

#[derive(Clone, Copy)]
struct AudioProjectionHost<'a> {
    item: &'a AudioItem,
    played_range: Option<(Time, Time)>,
}

pub(super) fn expanded(project: &Project, path: &[Uuid]) -> bool {
    project
        .expanded_sequence_paths
        .iter()
        .any(|expanded| expanded == path)
}

pub(super) fn expanded_timeline_end(project: &Project) -> Time {
    let mut end = project.duration();
    for track in &project.video_tracks {
        for host in &track.items {
            if let VideoItemContent::FoldedSequence(reference) = host.content
                && expanded(project, &[host.id])
            {
                end = end.max(expanded_video_end(
                    project,
                    reference,
                    host,
                    &[host.id],
                    &mut Vec::new(),
                ));
            }
        }
    }
    for track in &project.audio_tracks {
        for host in &track.items {
            if let AudioSource::FoldedSequence(reference) = host.source
                && expanded(project, &[host.id])
            {
                end = end.max(expanded_audio_end(
                    project,
                    reference,
                    host,
                    &[host.id],
                    &mut Vec::new(),
                ));
            }
        }
    }
    end
}

fn expanded_video_end(
    project: &Project,
    reference: SequenceReference,
    host: &VideoItem,
    path: &[Uuid],
    stack: &mut Vec<Uuid>,
) -> Time {
    if stack.contains(&reference.sequence_id) {
        return Time::ZERO;
    }
    let Some(sequence) = project.folded_sequence(reference.sequence_id) else {
        return Time::ZERO;
    };
    stack.push(reference.sequence_id);
    let mut end = Time::ZERO;
    for track in &sequence.video_tracks {
        for item in &track.items {
            end = end.max(
                map_time(
                    host.start,
                    host.time_offset,
                    host.playback_speed,
                    item.start,
                )
                .max(map_time(
                    host.start,
                    host.time_offset,
                    host.playback_speed,
                    item.end,
                )),
            );
            if let VideoItemContent::FoldedSequence(reference) = item.content {
                let mut nested_path = path.to_vec();
                nested_path.push(item.id);
                if expanded(project, &nested_path)
                    && let Some(mapped) = map_video_item(host, item)
                {
                    end = end.max(expanded_video_end(
                        project,
                        reference,
                        &mapped,
                        &nested_path,
                        stack,
                    ));
                }
            }
        }
    }
    stack.pop();
    end
}

fn expanded_audio_end(
    project: &Project,
    reference: SequenceReference,
    host: &AudioItem,
    path: &[Uuid],
    stack: &mut Vec<Uuid>,
) -> Time {
    if stack.contains(&reference.sequence_id) {
        return Time::ZERO;
    }
    let Some(sequence) = project.folded_sequence(reference.sequence_id) else {
        return Time::ZERO;
    };
    stack.push(reference.sequence_id);
    let mut end = Time::ZERO;
    for track in &sequence.audio_tracks {
        for item in &track.items {
            end = end.max(
                map_time(
                    host.start,
                    host.time_offset,
                    host.playback_speed,
                    item.start,
                )
                .max(map_time(
                    host.start,
                    host.time_offset,
                    host.playback_speed,
                    item.end,
                )),
            );
            if let AudioSource::FoldedSequence(reference) = item.source {
                let mut nested_path = path.to_vec();
                nested_path.push(item.id);
                if expanded(project, &nested_path)
                    && let Some(mapped) = map_audio_item(host, item)
                {
                    end = end.max(expanded_audio_end(
                        project,
                        reference,
                        &mapped,
                        &nested_path,
                        stack,
                    ));
                }
            }
        }
    }
    stack.pop();
    end
}

pub(super) fn child_tracks_before(project: &Project, kind: TrackKind, end: usize) -> usize {
    let mut count = 0;
    for row in crate::items::track_rows(project) {
        if row
            .root_key
            .is_some_and(|key| key.kind == kind && key.track_index >= end)
        {
            break;
        }
        if row.root_key.is_none()
            && matches!(
                (&row.address, kind),
                (TrackAddress::Caption { .. }, TrackKind::Caption)
                    | (TrackAddress::Video { .. }, TrackKind::Video)
                    | (TrackAddress::Audio { .. }, TrackKind::Audio)
            )
        {
            count += 1;
        }
    }
    count
}

pub(super) fn projected_video_tracks(
    project: &Project,
    track_index: usize,
    parent_row: usize,
) -> Vec<ProjectedVideoTrack> {
    let Some(track) = project.video_tracks.get(track_index) else {
        return Vec::new();
    };
    let mut row = parent_row + 1;
    let mut projected = Vec::new();
    for host in &track.items {
        let VideoItemContent::FoldedSequence(reference) = host.content else {
            continue;
        };
        let path = vec![host.id];
        if expanded(project, &path) {
            project_video_sequence(
                project,
                reference,
                VideoProjectionHost {
                    item: host,
                    played_range: Some((host.start, host.end)),
                },
                &path,
                &mut row,
                &mut projected,
                &mut Vec::new(),
            );
        }
    }
    projected
}

pub(super) fn projected_audio_tracks(
    project: &Project,
    track_index: usize,
    parent_row: usize,
) -> Vec<ProjectedAudioTrack> {
    let Some(track) = project.audio_tracks.get(track_index) else {
        return Vec::new();
    };
    let mut row = parent_row + 1;
    let mut projected = Vec::new();
    for host in &track.items {
        let AudioSource::FoldedSequence(reference) = host.source else {
            continue;
        };
        let path = vec![host.id];
        if expanded(project, &path) {
            project_audio_sequence(
                project,
                reference,
                AudioProjectionHost {
                    item: host,
                    played_range: Some((host.start, host.end)),
                },
                &path,
                &mut row,
                &mut projected,
                &mut Vec::new(),
            );
        }
    }
    projected
}

fn project_video_sequence(
    project: &Project,
    reference: SequenceReference,
    host: VideoProjectionHost<'_>,
    path: &[Uuid],
    row: &mut usize,
    projected: &mut Vec<ProjectedVideoTrack>,
    stack: &mut Vec<Uuid>,
) {
    if stack.contains(&reference.sequence_id) {
        return;
    }
    let Some(sequence) = project.folded_sequence(reference.sequence_id) else {
        return;
    };
    stack.push(reference.sequence_id);
    for track in sequence.video_tracks.iter().rev() {
        let items = track
            .items
            .iter()
            .filter_map(|item| map_video_item(host.item, item))
            .map(|item| {
                let sequence_path = match item.content {
                    VideoItemContent::FoldedSequence(_) => {
                        let mut nested = path.to_vec();
                        nested.push(item.id);
                        Some(nested)
                    }
                    _ => None,
                };
                ProjectedVideoItem {
                    item,
                    sequence_path,
                    played_range: host.played_range,
                }
            })
            .collect();
        projected.push(ProjectedVideoTrack {
            row: *row,
            sequence_path: path.to_vec(),
            track_id: track.id,
            items,
        });
        *row += 1;

        for nested_host in &track.items {
            let VideoItemContent::FoldedSequence(nested_reference) = nested_host.content else {
                continue;
            };
            let mut nested_path = path.to_vec();
            nested_path.push(nested_host.id);
            if !expanded(project, &nested_path) {
                continue;
            }
            let Some(mapped_host) = map_video_item(host.item, nested_host) else {
                continue;
            };
            let nested_played_range =
                intersect_range(host.played_range, mapped_host.start, mapped_host.end);
            project_video_sequence(
                project,
                nested_reference,
                VideoProjectionHost {
                    item: &mapped_host,
                    played_range: nested_played_range,
                },
                &nested_path,
                row,
                projected,
                stack,
            );
        }
    }
    stack.pop();
}

fn project_audio_sequence(
    project: &Project,
    reference: SequenceReference,
    host: AudioProjectionHost<'_>,
    path: &[Uuid],
    row: &mut usize,
    projected: &mut Vec<ProjectedAudioTrack>,
    stack: &mut Vec<Uuid>,
) {
    if stack.contains(&reference.sequence_id) {
        return;
    }
    let Some(sequence) = project.folded_sequence(reference.sequence_id) else {
        return;
    };
    stack.push(reference.sequence_id);
    for track in &sequence.audio_tracks {
        let items = track
            .items
            .iter()
            .filter_map(|item| map_audio_item(host.item, item))
            .map(|item| {
                let sequence_path = match item.source {
                    AudioSource::FoldedSequence(_) => {
                        let mut nested = path.to_vec();
                        nested.push(item.id);
                        Some(nested)
                    }
                    AudioSource::Media | AudioSource::Tts(_) | AudioSource::Generator(_) => None,
                };
                ProjectedAudioItem {
                    item,
                    sequence_path,
                    played_range: host.played_range,
                }
            })
            .collect();
        projected.push(ProjectedAudioTrack {
            row: *row,
            sequence_path: path.to_vec(),
            track_id: track.id,
            items,
        });
        *row += 1;

        for nested_host in &track.items {
            let AudioSource::FoldedSequence(nested_reference) = nested_host.source else {
                continue;
            };
            let mut nested_path = path.to_vec();
            nested_path.push(nested_host.id);
            if !expanded(project, &nested_path) {
                continue;
            }
            let Some(mapped_host) = map_audio_item(host.item, nested_host) else {
                continue;
            };
            let nested_played_range =
                intersect_range(host.played_range, mapped_host.start, mapped_host.end);
            project_audio_sequence(
                project,
                nested_reference,
                AudioProjectionHost {
                    item: &mapped_host,
                    played_range: nested_played_range,
                },
                &nested_path,
                row,
                projected,
                stack,
            );
        }
    }
    stack.pop();
}

fn map_video_item(host: &VideoItem, nested: &VideoItem) -> Option<VideoItem> {
    let mut item = nested.clone();
    let start = map_time(
        host.start,
        host.time_offset,
        host.playback_speed,
        nested.start,
    );
    let end = map_time(
        host.start,
        host.time_offset,
        host.playback_speed,
        nested.end,
    );
    item.start = start.min(end);
    item.end = start.max(end);
    (item.end > item.start).then(|| {
        let nested_speed = item.playback_speed;
        let sequence_time = host.time_offset.saturating_add(scaled_time_delta(
            item.start.signed_sub(host.start),
            host.playback_speed,
        ));
        item.time_offset = nested.time_offset.saturating_add(scaled_time_delta(
            sequence_time.signed_sub(nested.start),
            nested_speed,
        ));
        item.playback_speed *= host.playback_speed;
        item
    })
}

fn map_audio_item(host: &AudioItem, nested: &AudioItem) -> Option<AudioItem> {
    let mut item = nested.clone();
    let start = map_time(
        host.start,
        host.time_offset,
        host.playback_speed,
        nested.start,
    );
    let end = map_time(
        host.start,
        host.time_offset,
        host.playback_speed,
        nested.end,
    );
    item.start = start.min(end);
    item.end = start.max(end);
    (item.end > item.start).then(|| {
        let nested_speed = item.playback_speed;
        let sequence_time = host.time_offset.saturating_add(scaled_time_delta(
            item.start.signed_sub(host.start),
            host.playback_speed,
        ));
        item.time_offset = nested.time_offset.saturating_add(scaled_time_delta(
            sequence_time.signed_sub(nested.start),
            nested_speed,
        ));
        item.playback_speed *= host.playback_speed;
        item
    })
}

fn intersect_range(range: Option<(Time, Time)>, start: Time, end: Time) -> Option<(Time, Time)> {
    let (range_start, range_end) = range?;
    let start = start.max(range_start);
    let end = end.min(range_end);
    (end > start).then_some((start, end))
}

fn map_time(host_start: Time, host_offset: Time, host_speed: Fraction, nested_time: Time) -> Time {
    host_start.saturating_add(unscaled_time_delta(
        nested_time.signed_sub(host_offset),
        host_speed,
    ))
}
