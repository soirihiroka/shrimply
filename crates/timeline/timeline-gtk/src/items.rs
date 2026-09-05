use hashbrown::{HashMap, HashSet};

pub(super) use crate::DragCollisionMode;
use crate::project::{
    AudioItem, AudioSource, AudioTrack, CaptionItem, FoldedSequence, ItemAddress, ItemKind,
    ItemMut, ItemRef, Project, ProjectItem, RepeatStrategy, SequenceReference, Time, Transform,
    VideoItem, VideoItemContent, VisualTrack, default_playback_speed,
    generated_item_natural_end_position, generated_item_natural_span,
    media_item_natural_end_position, media_natural_end_interval, media_real_span,
    scaled_time_delta, video_natural_end_interval,
};
use crate::timeline_search::{self, TimeSlice};
use shrimply_timeline::TrackKey;
pub(super) use shrimply_timeline::edit::{
    advanced_media_source_offset, fit_audio_transitions, fit_visual_transitions,
    fitted_transition_durations, shifted_media_source_offset,
};
pub(super) use shrimply_timeline::{ItemKey, TrackKind};
pub(super) use shrimply_timeline::{insert_sorted, next_group_id};
use uuid::Uuid;

use super::x_to_time;
use super::{RULER_HEIGHT, TRACK_HEIGHT, TimelineSelection, TimelineViewState, row_y, timeline_x};

mod clipboard;
mod dragging;
mod grouping;
mod hit_testing;
mod mutation;
mod placement;
mod resize;
mod ripple;
mod tracks;
pub(super) use clipboard::*;
pub(super) use dragging::*;
pub(super) use grouping::*;
pub(super) use hit_testing::*;
pub(super) use mutation::*;
use placement::{
    ItemPlacement, add_collision_tracks, can_place_dragged_group, dragged_group_placements,
    insert_new_tracks, new_track_indices, overwrite_indicators, placement_indicators,
    placements_collide, placements_collide_with_project, target_existing_track_index,
    time_ranges_collide,
};
pub(super) use placement::{
    NewItemGroup, NewItemTarget, TrackFootprintItem, choose_track_base, place_new_items,
    place_new_items_at_base, track_footprint_span, visual_track_is_obscured,
};
pub(super) use resize::*;
pub(super) use ripple::*;
use tracks::active_new_track_at_y;
pub(super) use tracks::{
    TrackRow, color as track_color, projected_row_for_track, projected_row_for_virtual_track,
    row_for_address, row_for_track, rows as track_rows, target_track_at_y, track_count,
};

pub(super) fn track_at_y(project: &Project, y: f64) -> Option<(TrackKind, usize, usize)> {
    tracks::track_at_y(project, y)
}

pub(super) fn item_key_sort_key(key: &ItemKey) -> (u8, usize, usize) {
    let kind = match key.kind {
        TrackKind::Caption => 0_u8,
        TrackKind::Video => 1_u8,
        TrackKind::Audio => 2_u8,
    };
    (kind, key.track_index, key.item_index)
}

#[derive(Clone, Copy)]
pub(super) struct ItemIdentity {
    kind: TrackKind,
    id: Uuid,
}

pub(super) fn item_identity(project: &Project, key: ItemKey) -> Option<ItemIdentity> {
    let id = match key.kind {
        TrackKind::Caption => project
            .caption_tracks
            .get(key.track_index)?
            .items
            .get(key.item_index)
            .map(|item| item.id),
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
    }?;
    Some(ItemIdentity { kind: key.kind, id })
}

pub(super) fn item_key_for_identity(project: &Project, identity: ItemIdentity) -> Option<ItemKey> {
    match identity.kind {
        TrackKind::Caption => {
            project
                .caption_tracks
                .iter()
                .enumerate()
                .find_map(|(track_index, track)| {
                    track
                        .items
                        .iter()
                        .position(|item| item.id == identity.id)
                        .map(|item_index| ItemKey {
                            kind: identity.kind,
                            track_index,
                            item_index,
                        })
                })
        }
        TrackKind::Video => {
            project
                .video_tracks
                .iter()
                .enumerate()
                .find_map(|(track_index, track)| {
                    track
                        .items
                        .iter()
                        .position(|item| item.id == identity.id)
                        .map(|item_index| ItemKey {
                            kind: identity.kind,
                            track_index,
                            item_index,
                        })
                })
        }
        TrackKind::Audio => {
            project
                .audio_tracks
                .iter()
                .enumerate()
                .find_map(|(track_index, track)| {
                    track
                        .items
                        .iter()
                        .position(|item| item.id == identity.id)
                        .map(|item_index| ItemKey {
                            kind: identity.kind,
                            track_index,
                            item_index,
                        })
                })
        }
    }
}

pub(crate) fn track_gap_at(
    project: &Project,
    track: TrackKey,
    time: Time,
) -> Option<shrimply_timeline::TrackGap> {
    fn in_items<T: TimeSlice>(items: &[T], time: Time) -> Option<(Time, Time)> {
        let mut gap_start = Time::ZERO;
        for item in items {
            if item.start() > time {
                return (gap_start < item.start() && time >= gap_start)
                    .then_some((gap_start, item.start()));
            }
            if item.end() > time {
                return None;
            }
            gap_start = gap_start.max(item.end());
        }
        None
    }

    let (start, end) = match track.kind {
        TrackKind::Caption => in_items(&project.caption_tracks.get(track.track_index)?.items, time),
        TrackKind::Video => in_items(&project.video_tracks.get(track.track_index)?.items, time),
        TrackKind::Audio => in_items(&project.audio_tracks.get(track.track_index)?.items, time),
    }?;
    Some(shrimply_timeline::TrackGap { track, start, end })
}

#[derive(Clone)]
pub(super) struct DraggedGroup {
    pub(super) grabbed: ItemKey,
    pub(super) grabbed_start: Time,
    pointer_offset: Time,
    target_start: Time,
    track_offsets: Vec<TrackOffset>,
    pub(super) new_tracks: Vec<(TrackKind, usize)>,
    collision_mode: DragCollisionMode,
    pub(super) valid_drop: bool,
    pub(super) preview_status: DragPreviewStatus,
    pub(super) blocked_indicators: Vec<DragIndicator>,
    pub(super) overwrite_indicators: Vec<DragIndicator>,
    pub(super) items: Vec<DraggedGroupItem>,
    pub(super) cross_scope_preview_row: Option<usize>,
}

#[derive(Clone, Copy)]
struct TrackOffset {
    kind: TrackKind,
    offset: isize,
}

#[derive(Clone)]
pub(crate) struct DragPosition {
    target_start: Time,
    track_offsets: Vec<TrackOffset>,
    new_tracks: Vec<(TrackKind, usize)>,
    valid_drop: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum DragPreviewStatus {
    Clear,
    Overwrite,
    Blocked,
    NewTrack,
}

#[derive(Clone, Copy)]
pub(super) struct DragIndicator {
    pub(super) kind: TrackKind,
    pub(super) track_index: usize,
    pub(super) start: Time,
    pub(super) end: Time,
}

#[derive(Clone, Copy)]
pub(super) struct DraggedGroupItem {
    pub(super) key: ItemKey,
    pub(super) start: Time,
    pub(super) end: Time,
}

impl TimeSlice for DraggedGroupItem {
    fn start(&self) -> Time {
        self.start
    }

    fn end(&self) -> Time {
        self.end
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ItemEdge {
    Start,
    End,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum TransitionHitAction {
    Create,
    Handle,
    Body,
}

#[derive(Clone)]
pub(super) struct TransitionHit {
    pub(super) key: crate::project::ItemAddress,
    pub(super) side: crate::project::TransitionSide,
    pub(super) action: TransitionHitAction,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ClipTransitionHitAction {
    Create,
    Body,
    CenterHandle,
    StartHandle,
    EndHandle,
}

#[derive(Clone)]
pub(super) struct ClipTransitionHit {
    pub(super) outgoing: crate::project::ItemAddress,
    pub(super) incoming: crate::project::ItemAddress,
    pub(super) cut: Time,
    pub(super) duration: Option<Time>,
    pub(super) action: ClipTransitionHitAction,
}

#[derive(Clone)]
pub(super) struct ResizeDrag {
    pub(super) key: ItemKey,
    pub(super) edge: ItemEdge,
    pub(super) start: Time,
    pub(super) end: Time,
    pub(super) target_start: Time,
    pub(super) target_end: Time,
    collision_mode: DragCollisionMode,
    pub(super) valid: bool,
    pub(super) preview_status: DragPreviewStatus,
    pub(super) blocked_indicators: Vec<DragIndicator>,
    pub(super) overwrite_indicators: Vec<DragIndicator>,
    pub(super) items: Vec<ResizeDragItem>,
}

#[derive(Clone, Copy)]
pub(super) struct ResizeDragItem {
    pub(super) key: ItemKey,
    pub(super) start: Time,
    pub(super) end: Time,
}

impl TimeSlice for ResizeDragItem {
    fn start(&self) -> Time {
        self.start
    }

    fn end(&self) -> Time {
        self.end
    }
}

#[derive(Clone)]
pub(super) struct TimelineClipboard {
    items: Vec<CopiedItem>,
}

#[derive(Clone)]
struct CopiedItem {
    track_index: usize,
    start_offset: Time,
    duration: Time,
    item: ProjectItem,
}

pub(super) struct PasteResult {
    pub(super) selection: Vec<ItemAddress>,
    pub(super) captions: bool,
    pub(super) video: bool,
    pub(super) audio: bool,
}
