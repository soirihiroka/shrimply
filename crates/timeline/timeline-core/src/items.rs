use hashbrown::{HashMap, HashSet};

pub use crate::DragCollisionMode;
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
pub use shrimply_timeline::edit::{
    advanced_media_source_offset, fit_audio_transitions, fit_visual_transitions,
    fitted_transition_durations, shifted_media_source_offset,
};
pub use shrimply_timeline::{ItemKey, TrackKind};
pub use shrimply_timeline::{insert_sorted, next_group_id};
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
pub use clipboard::*;
pub use dragging::*;
pub use grouping::*;
pub use hit_testing::*;
pub use mutation::*;
use placement::{
    ItemPlacement, add_collision_tracks, can_place_dragged_group, dragged_group_placements,
    insert_new_tracks, new_track_indices, overwrite_indicators, placement_indicators,
    placements_collide, placements_collide_with_project, target_existing_track_index,
    time_ranges_collide,
};
pub use placement::{
    NewItemGroup, NewItemTarget, TrackFootprintItem, choose_track_base, place_new_items,
    place_new_items_at_base, track_footprint_span, visual_track_is_obscured,
};
pub use resize::*;
pub use ripple::*;
use tracks::active_new_track_at_y;
pub use tracks::{
    TrackRow, color as track_color, projected_row_for_track, projected_row_for_virtual_track,
    row_for_address, row_for_track, rows as track_rows, target_track_at_y, track_count,
};

pub fn track_at_y(project: &Project, y: f64) -> Option<(TrackKind, usize, usize)> {
    tracks::track_at_y(project, y)
}

pub fn item_key_sort_key(key: &ItemKey) -> (u8, usize, usize) {
    let kind = match key.kind {
        TrackKind::Caption => 0_u8,
        TrackKind::Video => 1_u8,
        TrackKind::Audio => 2_u8,
    };
    (kind, key.track_index, key.item_index)
}

#[derive(Clone, Copy)]
pub struct ItemIdentity {
    kind: TrackKind,
    id: Uuid,
}

pub fn item_identity(project: &Project, key: ItemKey) -> Option<ItemIdentity> {
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

pub fn item_key_for_identity(project: &Project, identity: ItemIdentity) -> Option<ItemKey> {
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

pub fn track_gap_at(
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
pub struct DraggedGroup {
    pub grabbed: ItemKey,
    pub grabbed_start: Time,
    pointer_offset: Time,
    target_start: Time,
    track_offsets: Vec<TrackOffset>,
    pub new_tracks: Vec<(TrackKind, usize)>,
    collision_mode: DragCollisionMode,
    pub valid_drop: bool,
    pub preview_status: DragPreviewStatus,
    pub blocked_indicators: Vec<DragIndicator>,
    pub overwrite_indicators: Vec<DragIndicator>,
    pub items: Vec<DraggedGroupItem>,
    pub cross_scope_preview_row: Option<usize>,
}

#[derive(Clone, Copy)]
struct TrackOffset {
    kind: TrackKind,
    offset: isize,
}

#[derive(Clone)]
pub struct DragPosition {
    target_start: Time,
    track_offsets: Vec<TrackOffset>,
    new_tracks: Vec<(TrackKind, usize)>,
    valid_drop: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum DragPreviewStatus {
    Clear,
    Overwrite,
    Blocked,
    NewTrack,
}

#[derive(Clone, Copy)]
pub struct DragIndicator {
    pub kind: TrackKind,
    pub track_index: usize,
    pub start: Time,
    pub end: Time,
}

#[derive(Clone, Copy)]
pub struct DraggedGroupItem {
    pub key: ItemKey,
    pub start: Time,
    pub end: Time,
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
pub enum ItemEdge {
    Start,
    End,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum TransitionHitAction {
    Create,
    Handle,
    Body,
}

#[derive(Clone)]
pub struct TransitionHit {
    pub key: crate::project::ItemAddress,
    pub side: crate::project::TransitionSide,
    pub action: TransitionHitAction,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ClipTransitionHitAction {
    Create,
    Body,
    CenterHandle,
    StartHandle,
    EndHandle,
}

#[derive(Clone)]
pub struct ClipTransitionHit {
    pub outgoing: crate::project::ItemAddress,
    pub incoming: crate::project::ItemAddress,
    pub cut: Time,
    pub duration: Option<Time>,
    pub action: ClipTransitionHitAction,
}

#[derive(Clone)]
pub struct ResizeDrag {
    pub key: ItemKey,
    pub edge: ItemEdge,
    pub start: Time,
    pub end: Time,
    pub target_start: Time,
    pub target_end: Time,
    collision_mode: DragCollisionMode,
    pub valid: bool,
    pub preview_status: DragPreviewStatus,
    pub blocked_indicators: Vec<DragIndicator>,
    pub overwrite_indicators: Vec<DragIndicator>,
    pub items: Vec<ResizeDragItem>,
}

#[derive(Clone, Copy)]
pub struct ResizeDragItem {
    pub key: ItemKey,
    pub start: Time,
    pub end: Time,
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
pub struct TimelineClipboard {
    items: Vec<CopiedItem>,
}

#[derive(Clone)]
struct CopiedItem {
    track_index: usize,
    start_offset: Time,
    duration: Time,
    item: ProjectItem,
}

pub struct PasteResult {
    pub selection: Vec<ItemAddress>,
    pub captions: bool,
    pub video: bool,
    pub audio: bool,
}
