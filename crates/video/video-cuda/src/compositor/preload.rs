use std::collections::HashSet;

use shrimply_project::project::{
    CanvasSize, Project, SequenceReference, Time, VideoItem, VideoItemContent,
    VisualClipTransitionKind, VisualTrack, video_source_time_at,
};
use uuid::Uuid;

use crate::decode::{DecodeControl, VideoPlane};
use crate::visual_source::CompositeAccuracy;

use super::{
    ActiveVideoItem, ClipTransitionRole, FrameItemRenderer, RenderSessions, SourcePrepareRequest,
    VideoDecodeRoute, VideoDecodeRoutes, active_video_items,
};

const VIDEO_PRELOAD_SECONDS: i64 = 1;
const IMAGE_PRELOAD_SECONDS: i64 = 5;

#[derive(Clone, Copy, Eq, PartialEq)]
enum PreloadClass {
    Image,
    Video,
}

struct PreloadRequest<'a> {
    sequence_path: &'a [Uuid],
    track_id: Uuid,
    position: Time,
    canvas_size: CanvasSize,
    accuracy: CompositeAccuracy,
    decode_control: Option<&'a DecodeControl>,
    routes: VideoDecodeRoutes,
    preload_color: bool,
    preload_alpha: bool,
}

trait Preload {
    fn class(&self) -> PreloadClass;
    fn lead_time(&self) -> Time;
    fn weight(&self) -> u8;
    fn item(&self) -> &VideoItem;
    fn prepare(&self, request: PreloadRequest<'_>, sessions: &mut RenderSessions, active: bool);
}

struct VisualPreload<'a> {
    item: &'a VideoItem,
    class: PreloadClass,
    lead_time: Time,
    weight: u8,
}

impl Preload for VisualPreload<'_> {
    fn class(&self) -> PreloadClass {
        self.class
    }

    fn lead_time(&self) -> Time {
        self.lead_time
    }

    fn weight(&self) -> u8 {
        self.weight
    }

    fn item(&self) -> &VideoItem {
        self.item
    }

    fn prepare(&self, request: PreloadRequest<'_>, sessions: &mut RenderSessions, _active: bool) {
        if let Err(error) = sessions.prepare_source(SourcePrepareRequest {
            sequence_path: request.sequence_path,
            track_id: request.track_id,
            item: self.item,
            position: request.position,
            canvas_size: request.canvas_size,
            accuracy: request.accuracy,
            decode_control: request.decode_control,
            route: request.routes.route(VideoPlane::Color),
            prefetch: false,
        }) {
            shrimply_benchmarking::increment("Visual source / Preload errors");
            tracing::debug!(item = %self.item.id, file = %self.item.file.display(), %error, "could not preload visual source");
        }
    }
}

struct VideoPreload<'a>(&'a VideoItem);

impl VideoPreload<'_> {
    fn prepare_item(
        &self,
        item: &VideoItem,
        request: &PreloadRequest<'_>,
        sessions: &mut RenderSessions,
        active: bool,
        route: VideoDecodeRoute,
    ) {
        if !active {
            if route.handoff_item_id.is_some() {
                return;
            }
            let owner = sessions.decoders.owner(
                request.sequence_path,
                request.track_id,
                item.id,
                route.plane,
            );
            match sessions.decoders.prepare(item, owner) {
                Ok(false) => return,
                Err(error) => {
                    shrimply_benchmarking::increment("Visual source / Preload errors");
                    tracing::debug!(item = %item.id, file = %item.file.display(), %error, "could not prewarm video decoder");
                    return;
                }
                Ok(true) => {}
            }
        }
        let reserved = usize::from(!active);
        if !sessions.decoders.has_request_capacity(reserved) {
            shrimply_benchmarking::increment(if active {
                "Video decode / Prepare skipped at decoder capacity"
            } else {
                "Temporal decoder / Preload skipped for foreground capacity"
            });
            return;
        }
        if let Err(error) = sessions.prepare_source(SourcePrepareRequest {
            sequence_path: request.sequence_path,
            track_id: request.track_id,
            item,
            position: request.position,
            canvas_size: request.canvas_size,
            accuracy: request.accuracy,
            decode_control: request.decode_control,
            route,
            prefetch: !active,
        }) {
            shrimply_benchmarking::increment("Visual source / Preload errors");
            tracing::debug!(item = %item.id, file = %item.file.display(), %error, "could not preload video source");
        }
    }
}

impl Preload for VideoPreload<'_> {
    fn class(&self) -> PreloadClass {
        PreloadClass::Video
    }

    fn lead_time(&self) -> Time {
        Time::from_seconds(VIDEO_PRELOAD_SECONDS)
    }

    fn weight(&self) -> u8 {
        0
    }

    fn item(&self) -> &VideoItem {
        self.0
    }

    fn prepare(&self, request: PreloadRequest<'_>, sessions: &mut RenderSessions, active: bool) {
        if request.preload_color {
            self.prepare_item(
                self.0,
                &request,
                sessions,
                active,
                request.routes.route(VideoPlane::Color),
            );
        }
        if request.preload_alpha
            && let Some(media_track_id) = self.0.alpha_mask_video
        {
            let (alpha_item, _) = shrimply_video_core::alpha_mask::video_source(
                self.0,
                media_track_id,
                request.canvas_size,
            );
            self.prepare_item(
                &alpha_item,
                &request,
                sessions,
                active,
                request.routes.route(VideoPlane::Alpha),
            );
        }
    }
}

pub(super) fn predecessor(items: &[VideoItem], item_id: Uuid) -> Option<&VideoItem> {
    let item_index = items
        .iter()
        .position(|item| item.id == item_id)
        .expect("video item must belong to its track");
    item_index.checked_sub(1).map(|index| &items[index])
}

fn decode_routes(
    sessions: &RenderSessions,
    sequence_path: &[Uuid],
    track_id: Uuid,
    previous: Option<&VideoItem>,
    item: &VideoItem,
) -> VideoDecodeRoutes {
    let Some(previous) = previous else {
        return VideoDecodeRoutes::default();
    };
    VideoDecodeRoutes {
        color_handoff_item_id: sessions
            .decoders
            .can_handoff(sequence_path, track_id, previous, item, VideoPlane::Color)
            .then_some(previous.id),
        alpha_handoff_item_id: sessions
            .decoders
            .can_handoff(sequence_path, track_id, previous, item, VideoPlane::Alpha)
            .then_some(previous.id),
    }
}

fn preload_for(item: &VideoItem) -> Option<Box<dyn Preload + '_>> {
    match &item.content {
        VideoItemContent::LayeredImage(_) => Some(Box::new(VisualPreload {
            item,
            class: PreloadClass::Image,
            lead_time: Time::from_seconds(IMAGE_PRELOAD_SECONDS),
            weight: 0,
        })),
        VideoItemContent::Image => Some(Box::new(VisualPreload {
            item,
            class: PreloadClass::Image,
            lead_time: Time::from_seconds(IMAGE_PRELOAD_SECONDS),
            weight: 1,
        })),
        VideoItemContent::Media => Some(Box::new(VideoPreload(item))),
        VideoItemContent::Gif => Some(Box::new(VisualPreload {
            item,
            class: PreloadClass::Video,
            lead_time: Time::from_seconds(VIDEO_PRELOAD_SECONDS),
            weight: 1,
        })),
        _ => None,
    }
}

struct SequenceCursor<'a> {
    tracks: &'a [VisualTrack],
    sequence_path: Vec<Uuid>,
    position: Time,
    ancestors: Vec<Uuid>,
    future: bool,
}

pub(super) fn upcoming_images(
    project: &Project,
    sessions: &mut RenderSessions,
    position: Time,
    accuracy: CompositeAccuracy,
) {
    upcoming(project, sessions, position, accuracy, PreloadClass::Image);
}

pub(super) fn upcoming_videos(
    project: &Project,
    sessions: &mut RenderSessions,
    position: Time,
    accuracy: CompositeAccuracy,
) {
    upcoming(project, sessions, position, accuracy, PreloadClass::Video);
}

fn upcoming(
    project: &Project,
    sessions: &mut RenderSessions,
    position: Time,
    accuracy: CompositeAccuracy,
    class: PreloadClass,
) {
    let mut cursors = vec![SequenceCursor {
        tracks: &project.video_tracks,
        sequence_path: Vec::new(),
        position,
        ancestors: Vec::new(),
        future: false,
    }];
    while let Some(cursor) = cursors.pop() {
        let traversal_end = cursor
            .position
            .saturating_add(Time::from_seconds(match class {
                PreloadClass::Image => IMAGE_PRELOAD_SECONDS,
                PreloadClass::Video => VIDEO_PRELOAD_SECONDS,
            }));
        for item in cursor
            .tracks
            .iter()
            .filter(|track| track.enabled)
            .flat_map(|track| &track.items)
            .filter(|item| item.end > cursor.position && item.start <= traversal_end)
        {
            let VideoItemContent::FoldedSequence(reference) = &item.content else {
                continue;
            };
            if cursor.ancestors.contains(&reference.sequence_id) {
                continue;
            }
            let Some(sequence) = project.folded_sequence(reference.sequence_id) else {
                continue;
            };
            let Some(nested_position) = video_source_time_at(item, cursor.position.max(item.start))
            else {
                continue;
            };
            let mut sequence_path = cursor.sequence_path.clone();
            sequence_path.push(item.id);
            let mut ancestors = cursor.ancestors.clone();
            ancestors.push(reference.sequence_id);
            cursors.push(SequenceCursor {
                tracks: &sequence.video_tracks,
                sequence_path,
                position: nested_position,
                ancestors,
                future: cursor.future || item.start > cursor.position,
            });
        }

        let cursor_position = cursor.position;
        let mut preloads = cursor
            .tracks
            .iter()
            .filter(|track| track.enabled)
            .flat_map(|track| {
                track.items.iter().filter_map(move |item| {
                    if crate::modifier_cache::effective_item(item, project.canvas_size)
                        .ok()
                        .flatten()
                        .is_some()
                    {
                        return None;
                    }
                    let preload = preload_for(item)?;
                    (preload.class() == class
                        && item.end > cursor_position
                        && item.start <= cursor_position.saturating_add(preload.lead_time())
                        && (class == PreloadClass::Image
                            || cursor.future
                            || item.start > cursor_position))
                        .then(|| (track.id, predecessor(&track.items, item.id), preload))
                })
            })
            .collect::<Vec<_>>();
        preloads.sort_by_key(|(_, _, preload)| match class {
            PreloadClass::Image => (preload.weight(), preload.item().start),
            PreloadClass::Video => (0, preload.item().start),
        });
        let mut selected_video_tracks = HashSet::new();
        for (track_id, previous, preload) in preloads {
            let item = preload.item();
            if class == PreloadClass::Video && !selected_video_tracks.insert(track_id) {
                continue;
            }
            let (preload_color, preload_alpha) = if class == PreloadClass::Video {
                (true, item.alpha_mask_video.is_some())
            } else {
                (true, true)
            };
            let routes = decode_routes(sessions, &cursor.sequence_path, track_id, previous, item);
            preload.prepare(
                PreloadRequest {
                    sequence_path: &cursor.sequence_path,
                    track_id,
                    position: cursor_position.max(item.start),
                    canvas_size: project.canvas_size,
                    accuracy,
                    decode_control: None,
                    routes,
                    preload_color,
                    preload_alpha,
                },
                sessions,
                false,
            );
        }
    }
}

impl FrameItemRenderer<'_> {
    pub(super) fn decode_routes(
        &self,
        track_id: Uuid,
        previous: Option<&VideoItem>,
        item: &VideoItem,
    ) -> VideoDecodeRoutes {
        decode_routes(self.sessions, &self.sequence_path, track_id, previous, item)
    }

    pub(super) fn preload_active_sources(&mut self, active_items: &[ActiveVideoItem<'_>]) {
        let outer_clip_transition = self.clip_transition;
        for active in active_items {
            abort_render_if_superseded!(self.decode_control, break);
            self.clip_transition = active.clip_transition;
            let morph_position = active.clip_transition.and_then(|transition| {
                (transition.definition.kind == VisualClipTransitionKind::Morph).then_some(
                    match transition.role {
                        ClipTransitionRole::Outgoing => {
                            shrimply_math_media::clip_transition_bounds(
                                active.item.end,
                                transition.definition.duration,
                            )
                            .0
                        }
                        ClipTransitionRole::Incoming => {
                            shrimply_math_media::clip_transition_bounds(
                                active.item.start,
                                transition.definition.duration,
                            )
                            .1
                        }
                    },
                )
            });
            let outer_position = self.position;
            if let Some(position) = morph_position {
                self.position = position;
            }
            let mut held_item = None;
            if active.clip_transition.is_some()
                && (self.position < active.item.start || self.position >= active.item.end)
            {
                let mut item = active.item.clone();
                item.repeat_strategy = shrimply_project::project::RepeatStrategy::Hold;
                held_item = Some(item);
            }
            let item = held_item.as_ref().unwrap_or(active.item);
            let cached_item = crate::modifier_cache::effective_item(item, self.project.canvas_size)
                .ok()
                .flatten();
            let item = cached_item.as_ref().unwrap_or(item);
            let previous = cached_item.is_none().then_some(active.previous).flatten();
            let routes = self.decode_routes(active.track_id, previous, item);
            self.preload_item(active.track_id, item, routes);
            self.position = outer_position;
        }
        self.clip_transition = outer_clip_transition;
    }

    pub(super) fn preload_item(
        &mut self,
        track_id: Uuid,
        item: &VideoItem,
        routes: VideoDecodeRoutes,
    ) {
        abort_render_if_superseded!(self.decode_control, return);
        if let VideoItemContent::FoldedSequence(reference) = &item.content {
            self.prepare_folded_item(item, *reference);
            return;
        }
        let Some(preload) = preload_for(item) else {
            return;
        };
        let cache_item = self.cache_item.as_ref().is_some_and(|address| {
            address.sequence_path() == self.sequence_path
                && address.track_id() == track_id
                && address.item_id() == item.id
        });
        preload.prepare(
            PreloadRequest {
                sequence_path: &self.sequence_path,
                track_id,
                position: if cache_item && self.snap_cache_item {
                    crate::modifiers::transparent_fill::snapped_transparent_fill_position(
                        self.project,
                        item,
                        self.position,
                    )
                } else {
                    crate::modifiers::transparent_fill::render_position(
                        self.project,
                        item,
                        self.position,
                    )
                },
                canvas_size: self.project.canvas_size,
                accuracy: self.mode.accuracy(),
                decode_control: self.decode_control,
                routes,
                preload_color: true,
                preload_alpha: true,
            },
            self.sessions,
            true,
        );
    }

    fn prepare_folded_item(&mut self, item: &VideoItem, reference: SequenceReference) {
        abort_render_if_superseded!(self.decode_control, return);
        let Some(position) = video_source_time_at(item, self.position) else {
            return;
        };
        if self.sequence_stack.contains(&reference.sequence_id) {
            return;
        }
        let Some(sequence) = self.project.folded_sequence(reference.sequence_id) else {
            return;
        };
        let active = sequence
            .video_tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| track.enabled)
            .flat_map(|(track_index, track)| {
                active_video_items(track_index, track.id, &track.items, position, None)
                    .into_iter()
                    .map(|active| {
                        (
                            active.track_id,
                            active.item.clone(),
                            active.clip_transition,
                            active.previous.cloned(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        self.sequence_stack.push(reference.sequence_id);
        self.sequence_path.push(item.id);
        let outer_position = self.position;
        let outer_clip_transition = self.clip_transition;
        self.position = position;
        for (track_id, mut child, transition, previous) in active {
            abort_render_if_superseded!(self.decode_control, break);
            self.clip_transition = transition;
            if let Some(transition) = transition
                && transition.definition.kind == VisualClipTransitionKind::Morph
            {
                self.position = match transition.role {
                    ClipTransitionRole::Outgoing => {
                        shrimply_math_media::clip_transition_bounds(
                            child.end,
                            transition.definition.duration,
                        )
                        .0
                    }
                    ClipTransitionRole::Incoming => {
                        shrimply_math_media::clip_transition_bounds(
                            child.start,
                            transition.definition.duration,
                        )
                        .1
                    }
                };
            }
            if transition.is_some() && (self.position < child.start || self.position >= child.end) {
                child.repeat_strategy = shrimply_project::project::RepeatStrategy::Hold;
            }
            let cached_child =
                crate::modifier_cache::effective_item(&child, self.project.canvas_size)
                    .ok()
                    .flatten();
            let child = cached_child.as_ref().unwrap_or(&child);
            let previous = cached_child
                .is_none()
                .then_some(previous.as_ref())
                .flatten();
            let routes = self.decode_routes(track_id, previous, child);
            self.preload_item(track_id, child, routes);
            self.position = position;
        }
        self.clip_transition = outer_clip_transition;
        self.position = outer_position;
        self.sequence_path.pop();
        self.sequence_stack.pop();
    }
}
