use super::*;
use crate::drawing::{TimelineInput, active_virtual_tracks, draw_timeline};
use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};

pub(super) fn draw(scene: &mut Scene, painter: &TimelinePainter, size: Vec2, frame: Frame<'_>) {
    let playback_performance = scene.performance.snapshot();
    let project = scene.project.clone();
    let player_state = scene.player.clone();
    let selection_state = scene.selection.clone();
    let project = &project;
    let player_state = &player_state;
    let selection_state = &selection_state;
    let runtime = scene;
    let width = f64::from(size.x);
    let height = f64::from(size.y);
    let accent_color = frame.accent_color;
    let timeline_width = timeline_width(width);
    if timeline_width <= 0.0 {
        return;
    }
    let (duration_seconds, frame_step_seconds) = {
        let project = project.borrow();
        let player = player_state::snapshot(player_state);
        (
            folded_sequence::expanded_timeline_end(&project)
                .max(player.duration)
                .max(player.position)
                .max(Time::from_seconds(1))
                .as_secs_f64(),
            frame_step_seconds(&project),
        )
    };
    let min_seconds_per_pixel = min_seconds_per_pixel(frame_step_seconds);
    runtime
        .view
        .initialize(duration_seconds, timeline_width, min_seconds_per_pixel);
    let track_content_height = {
        let project = project.borrow();
        let virtual_tracks = active_virtual_tracks(
            runtime.dragged_group.as_ref(),
            runtime.import_preview.as_ref(),
        );
        timeline_track_content_height(&project, &virtual_tracks)
    };
    if let Some(time) = runtime.initial_center.take() {
        runtime.view.center_time(
            time,
            duration_seconds,
            timeline_width,
            min_seconds_per_pixel,
            track_content_height,
            height,
        );
    }
    runtime.view.clamp(
        duration_seconds,
        timeline_width,
        min_seconds_per_pixel,
        track_content_height,
        height,
    );

    {
        let project = project.borrow();
        let player = player_state::snapshot(player_state);
        let beats = if runtime.beat_grid_enabled {
            crate::beat_grid::snap_targets(&project, &runtime.beats, runtime.view)
        } else {
            Vec::new()
        };
        runtime.snap_repository = crate::snapping::repository(
            &project,
            crate::snapping::Request {
                folded_drag: runtime.folded_drag.as_ref(),
                dragged_group: runtime.dragged_group.as_ref(),
                resize_drag: runtime.resize_drag.as_ref(),
                beats,
                playhead: player.position,
                distance: runtime
                    .snap_enabled
                    .then(|| crate::math::snap_distance(runtime.view, runtime.snap_radius_px)),
            },
        );
    }

    pointer::handle_timeline_input(
        project,
        player_state,
        selection_state,
        runtime,
        width,
        timeline_width,
        height,
        track_content_height,
        duration_seconds,
        frame_step_seconds,
    );
    apply_scrollbar_scroll_animation(runtime);

    let project = project.borrow();
    let preview_project = runtime
        .clip_transition_drag
        .as_ref()
        .filter(|drag| drag.center_resize && drag.target_cut != drag.cut)
        .and_then(|drag| {
            let mut preview = project.clone();
            SequenceTimeline::for_item(&preview, &drag.outgoing)?
                .apply_clip_transition_cut(
                    &mut preview,
                    &drag.outgoing,
                    &drag.incoming,
                    drag.target_cut,
                )
                .then_some(preview)
        });
    let project = preview_project.as_ref().unwrap_or(&project);
    let virtual_tracks = active_virtual_tracks(
        runtime.dragged_group.as_ref(),
        runtime.import_preview.as_ref(),
    );
    let track_content_height = timeline_track_content_height(project, &virtual_tracks);
    runtime.view.clamp(
        duration_seconds,
        timeline_width,
        min_seconds_per_pixel,
        track_content_height,
        height,
    );
    let player_snapshot = player_state::snapshot(player_state);
    let current_time = player_state::current_time(player_state);
    let playhead_position_changed = runtime
        .last_playhead_position
        .is_some_and(|position| position != current_time);
    runtime.last_playhead_position = Some(current_time);
    let playhead_visibility_requested = runtime.playhead_visibility_requested.replace(false);
    let explicit_paused_seek =
        playhead_visibility_requested && playhead_position_changed && !player_snapshot.playing;
    let keep_playhead_visible = player_snapshot.playing
        || matches!(runtime.view.drag_mode, DragMode::Seek)
        || explicit_paused_seek;
    if keep_playhead_visible {
        runtime.horizontal_scrollbar.cancel_scroll();
        runtime.vertical_scrollbar.cancel_scroll();
        runtime.view.keep_time_visible(
            current_time,
            duration_seconds,
            timeline_width,
            min_seconds_per_pixel,
            track_content_height,
            height,
        );
    }
    let waveform_chunks_per_second = waveform_chunks_per_second_from_frame_step(frame_step_seconds);
    let active_audio_recording_key = frame.active_audio_recording_key;
    let active_video_recording_key = frame.active_video_recording_key;
    let live_recording = frame.live_recording;
    let live_video_recording = frame.live_video_recording;
    let overscroll = runtime.overscroll.and_then(|overscroll| {
        let distance = shrimply_skia_adw_core::overshoot_distance(
            overscroll.distance,
            overscroll.started_at.elapsed(),
        );
        (distance > shrimply_skia_adw_core::OVERSHOOT_VISIBLE_DISTANCE)
            .then_some((overscroll.edge, distance))
    });
    if overscroll.is_none() {
        runtime.overscroll = None;
    }
    let horizontal_scrollbar_frame = runtime.horizontal_scrollbar.frame(
        Some(horizontal_scrollbar(
            runtime.view,
            timeline_width,
            height,
            duration_seconds,
            shrimply_skia_adw_core::slider::idle_state(),
        )),
        runtime.pointer_pos,
    );
    let vertical_scrollbar_frame = runtime.vertical_scrollbar.frame(
        vertical_scrollbar(
            runtime.view,
            width,
            height,
            track_content_height,
            shrimply_skia_adw_core::slider::idle_state(),
        ),
        runtime.pointer_pos,
    );
    if horizontal_scrollbar_frame.animating || vertical_scrollbar_frame.animating {
        runtime.track_controls_animating = true;
    }
    let mut track_control_draw = TrackControlDraw {
        animation_active: &mut runtime.track_controls_animating,
        buttons: &mut runtime.track_buttons,
        active_audio_recording_key,
        active_video_recording_key,
    };
    let selected_items = selected_timeline_items(selection_state);
    let selected_nested_items = selection_state::selected_nested_items(selection_state);
    let selected_tracks = selection_state::selected_track_addresses(selection_state, project);
    let selected_gap = selection_state::selected_gap(selection_state);
    draw_timeline(TimelineInput {
        painter,
        project,
        playback_performance: &playback_performance,
        current_time,
        waveforms: &runtime.waveforms,
        beats: &runtime.beats,
        beat_grid_enabled: runtime.beat_grid_enabled,
        selected_items: &selected_items,
        selected_nested_items: &selected_nested_items,
        selected_tracks: &selected_tracks,
        selected_gap,
        track_control_draw: &mut track_control_draw,
        dragged_group: runtime.dragged_group.as_ref(),
        folded_drag: runtime.folded_drag.as_ref(),
        resize_drag: runtime.resize_drag.as_ref(),
        transition_drag: runtime.transition_drag.as_ref(),
        clip_transition_drag: runtime.clip_transition_drag.as_ref(),
        focused_transition: focused_timeline_transition(selection_state, project),
        import_preview: runtime.import_preview.as_ref(),
        text_drop_preview: runtime.text_drop_preview.as_ref(),
        cut_preview: runtime.cut_preview.as_ref(),
        live_recording,
        live_video_recording,
        view: runtime.view,
        virtual_tracks: &virtual_tracks,
        width,
        height,
        timeline_width,
        frame_step_seconds,
        animation_seconds: runtime.started_at.elapsed().as_secs_f64(),
        waveform_chunks_per_second,
        accent_color,
        overscroll,
        horizontal_scrollbar: horizontal_scrollbar_frame.scrollbar,
        vertical_scrollbar: vertical_scrollbar_frame.scrollbar,
        software_cursor: runtime.software_cursor.as_ref(),
    });
}

fn apply_scrollbar_scroll_animation(runtime: &mut Scene) {
    let mut scroll_seconds = runtime.view.scroll_seconds;
    if runtime.horizontal_scrollbar.apply_scroll(|value| {
        scroll_seconds = value;
    }) {
        runtime.track_controls_animating = true;
    }
    runtime.view.scroll_seconds = scroll_seconds;

    let mut scroll_y = runtime.view.scroll_y;
    if runtime.vertical_scrollbar.apply_scroll(|value| {
        scroll_y = value;
    }) {
        runtime.track_controls_animating = true;
    }
    runtime.view.scroll_y = scroll_y;
}
/// Optional host data for the shared drawing pass; the component owns interaction state.
pub struct Frame<'a> {
    pub before_seek: Option<&'a mut dyn FnMut(Time)>,
    pub accent_color: Color,
    pub active_audio_recording_key: Option<TrackKey>,
    pub active_video_recording_key: Option<TrackKey>,
    pub live_recording: Option<&'a LiveRecordingDraw>,
    pub live_video_recording: Option<LiveVideoRecordingDraw>,
}
