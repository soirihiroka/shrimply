pub use shrimply_timeline_core::import::*;

use crate::project::{
    AudioItem, AudioTrack, Project, ResolvedTransform, Time, Transform, VideoItem, VideoTrack,
    default_playback_speed,
};
use shrimply_core::timeline_value::*;

use super::items::{self, ItemKey, TrackKind};

#[derive(Clone)]
pub struct ImportPreview {
    pub(super) start: Time,
    pub(super) end: Time,
    pub(super) video_streams: usize,
    pub(super) audio_streams: usize,
    pub(super) video_base: usize,
    pub(super) audio_base: usize,
    pub(super) virtual_tracks: Vec<(TrackKind, usize)>,
    pub(super) collision_mode: items::DragCollisionMode,
}

pub(super) fn preview(
    project: &Project,
    duration: Time,
    video_streams: usize,
    audio_streams: usize,
    start: Time,
    target: items::NewItemTarget,
    collision_mode: items::DragCollisionMode,
) -> ImportPreview {
    let step = project.frame_step();
    let start = start.max(Time::ZERO).snapped(step);
    let end = start
        .saturating_add(duration)
        .snapped(step)
        .max(start.saturating_add(step));
    let collision_mode = match target {
        items::NewItemTarget::Automatic => items::DragCollisionMode::NewTrack,
        items::NewItemTarget::AtY(_) => collision_mode,
    };
    let group = |kind, stream_count| items::NewItemGroup {
        kind,
        footprint: (0..stream_count)
            .map(|track_offset| items::TrackFootprintItem {
                track_offset,
                start,
                end,
            })
            .collect(),
    };
    let place = |items| items::place_new_items(project, items, target, collision_mode);
    let video_items = group(TrackKind::Video, video_streams);
    let audio_items = group(TrackKind::Audio, audio_streams);
    let video = place(&video_items);
    let audio = if video_streams > 0 && audio_streams > 0 {
        items::place_new_items_at_base(project, &audio_items, video.base, collision_mode)
    } else {
        place(&audio_items)
    };
    let mut virtual_tracks: Vec<_> = video
        .new_tracks
        .map(|index| (TrackKind::Video, index))
        .collect();
    virtual_tracks.extend(audio.new_tracks.map(|index| (TrackKind::Audio, index)));

    ImportPreview {
        start,
        end,
        video_streams,
        audio_streams,
        video_base: video.base,
        audio_base: audio.base,
        virtual_tracks,
        collision_mode,
    }
}

pub fn apply(project: &mut Project, info: &MediaInfo, preview: &ImportPreview) -> ImportResult {
    for (kind, index) in &preview.virtual_tracks {
        match kind {
            TrackKind::Caption => {}
            TrackKind::Video if *index <= project.video_tracks.len() => {
                project.video_tracks.insert(*index, VideoTrack::default());
            }
            TrackKind::Audio if *index <= project.audio_tracks.len() => {
                project.audio_tracks.insert(*index, AudioTrack::default());
            }
            _ => {}
        }
    }

    let mut selection = Vec::new();
    let group_id = Some(items::next_group_id(project));
    for stream_index in 0..preview.video_streams {
        let track_index = preview.video_base + stream_index;
        let source_size = info
            .video_sizes
            .get(stream_index)
            .copied()
            .filter(|size| size.x > 0 && size.y > 0);
        let transform = if matches!(
            info.visual_kind,
            Some(VisualMediaKind::Obj | VisualMediaKind::Gaussian)
        ) {
            Transform::from_resolved(ResolvedTransform::IDENTITY)
        } else {
            source_size
                .map(|size| Transform::natural_size(project.canvas_size, size.x, size.y))
                .unwrap_or_else(|| Transform::fill(project.canvas_size))
        };
        let source_size = source_size.unwrap_or_default();
        let item = VideoItem {
            id: uuid::Uuid::new_v4(),
            start: preview.start,
            end: preview.end,
            time_offset: Time::ZERO,
            source_duration: info.duration,
            playback_speed: default_playback_speed(),
            playback_fps: shrimply_project::project::native_playback_fps(),
            repeat_strategy: repeat_strategy_for_import(info),
            stabilize_video: false,
            stabilization_method: Default::default(),
            stabilization_crop_ratio:
                shrimply_project::project::default_video_stabilization_crop_ratio(),
            stabilization_first_derivative_weight:
                shrimply_project::project::default_video_stabilization_first_derivative_weight(),
            stabilization_second_derivative_weight:
                shrimply_project::project::default_video_stabilization_second_derivative_weight(),
            stabilization_third_derivative_weight:
                shrimply_project::project::default_video_stabilization_third_derivative_weight(),
            mesh_flow_rows: shrimply_project::project::default_mesh_flow_rows(),
            mesh_flow_columns: shrimply_project::project::default_mesh_flow_columns(),
            mesh_flow_smoothing_radius:
                shrimply_project::project::default_mesh_flow_smoothing_radius(),
            mesh_flow_iterations: shrimply_project::project::default_mesh_flow_iterations(),
            mesh_flow_adaptive_weights: Default::default(),
            animation_time_offset: Time::ZERO,
            motion_blur: Default::default(),
            transform: transform.clone(),
            modifiers: modifiers_for_import(info),
            sample_method: TimelineValue::new_const(
                if matches!(info.visual_kind, Some(VisualMediaKind::LayeredImage)) {
                    shrimply_core::VideoSampleMethod::Nearest
                } else {
                    Default::default()
                },
            ),
            skia_drawing_strategy: Default::default(),
            compositing: Default::default(),
            visibility: TimelineValue::new_const(TimelineBool::True),
            alpha_mask_video: None,
            transitions: Default::default(),
            svg_color_overrides: Vec::new(),
            source_width: source_size.x,
            source_height: source_size.y,
            default_transform: Some(transform),
            content: video_content_for_import(info),
            video_generation: None,
            group_id,
            render_canvas_size: None,
            track_id: stream_index as u32,
            file: info.source.clone(),
        };
        let Some(track) = project.video_tracks.get_mut(track_index) else {
            continue;
        };
        if preview.collision_mode == items::DragCollisionMode::Overwrite {
            items::overwrite_items(&mut track.items, preview.start, preview.end);
        }
        let item_index = items::insert_sorted(&mut track.items, item);
        selection.push(ItemKey {
            kind: TrackKind::Video,
            track_index,
            item_index,
        });
    }

    for stream_index in 0..preview.audio_streams {
        let track_index = preview.audio_base + stream_index;
        let item = AudioItem::builder(preview.start, preview.end)
            .source_duration(info.duration)
            .repeat_strategy(repeat_strategy_for_import(info))
            .group_id(group_id)
            .track_id(stream_index as u32)
            .file(info.source.clone())
            .build();
        let Some(track) = project.audio_tracks.get_mut(track_index) else {
            continue;
        };
        if preview.collision_mode == items::DragCollisionMode::Overwrite {
            items::overwrite_items(&mut track.items, preview.start, preview.end);
        }
        let item_index = items::insert_sorted(&mut track.items, item);
        selection.push(ItemKey {
            kind: TrackKind::Audio,
            track_index,
            item_index,
        });
    }

    ImportResult {
        selection,
        video: preview.video_streams > 0,
        audio: preview.audio_streams > 0,
        captions: false,
    }
}
