use super::*;
#[allow(clippy::too_many_arguments)]
pub(super) fn timeline_gtk(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    _selection_state: &SharedSelectionState,
    runtime: &mut TimelineRuntime,
    painter: &TimelinePainter,
    width: f64,
    height: f64,
    accent_color: Color,
) {
    let current_time = player_state::current_time(player_state);
    let waveform_chunks_per_second =
        waveform_chunks_per_second_from_frame_step(frame_step_seconds(&project.borrow()));
    let active_audio_recording_key = runtime
        .active_audio_recording
        .as_ref()
        .map(|recording| recording.key);
    let active_video_recording_key = runtime
        .active_video_recording
        .as_ref()
        .map(|recording| recording.key);
    let live_recording = runtime
        .active_audio_recording
        .as_ref()
        .and_then(|recording| live_recording_draw(recording, waveform_chunks_per_second));
    let live_video_recording = runtime
        .active_video_recording
        .as_ref()
        .filter(|recording| recording.ready)
        .and_then(|recording| {
            let end = recording
                .stop_at
                .map_or(current_time, |stop_at| current_time.min(stop_at));
            (end > recording.start).then_some(LiveVideoRecordingDraw {
                key: recording.key,
                start: recording.start,
                end,
            })
        });

    let mut before_seek = |position| {
        recording::stop_before_backward_seek(
            &runtime.active_audio_recording,
            &mut runtime.active_video_recording,
            player_state,
            position,
        )
    };
    runtime.scene.draw_frame(
        painter.canvas(),
        vec2(width as f32, height as f32),
        shrimply_timeline_core::scene::Frame {
            before_seek: Some(&mut before_seek),
            accent_color,
            active_audio_recording_key,
            active_video_recording_key,
            live_recording: live_recording.as_ref(),
            live_video_recording,
        },
    );
}
