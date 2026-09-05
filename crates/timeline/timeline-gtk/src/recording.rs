use super::*;

pub(super) fn handle_audio_recording(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    runtime: &mut TimelineRuntime,
    key: TrackKey,
) -> bool {
    if key.kind != TrackKind::Audio {
        return false;
    }

    if let Some(active) = runtime.active_audio_recording.take() {
        let same_track = active.key == key;
        if let Err(error) = finish_audio_recording(area, project, player_state, active) {
            interaction::show_error_dialog(area, "Could not record audio", &error);
        }
        if same_track {
            area.queue_render();
            return true;
        }
    }

    let start = {
        let project = project.borrow();
        if project.audio_tracks.get(key.track_index).is_none() {
            interaction::show_error_dialog(
                area,
                "Could not record audio",
                "Audio track no longer exists",
            );
            return false;
        }
        player_state::snapshot(player_state)
            .position
            .snapped(project.frame_step())
    };
    match crate::audio::recording::MicRecording::start() {
        Ok(recording) => {
            ensure_recording_duration(player_state, start);
            player_state::set_playing(player_state, true);
            runtime.active_audio_recording = Some(ActiveAudioRecording {
                key,
                start,
                recording,
            });
            area.queue_render();
        }
        Err(error) => interaction::show_error_dialog(area, "Could not record audio", &error),
    }
    false
}

pub(super) fn handle_video_recording(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    runtime_shared: &Rc<RefCell<TimelineRuntime>>,
    runtime: &mut TimelineRuntime,
    key: TrackKey,
) -> bool {
    if key.kind != TrackKind::Video {
        return false;
    }
    if let Some(active) = runtime.active_video_recording.as_mut() {
        if !active.stopping {
            active.stopping = true;
            active.recording.stop();
            area.queue_render();
        }
        return true;
    }

    let position = player_state::snapshot(player_state).position;
    let (start, fps, stop_at) = {
        let project = project.borrow();
        let start = position.snapped(project.frame_step());
        let Some(track) = project.video_tracks.get(key.track_index) else {
            interaction::show_error_dialog(
                area,
                "Could not record screen or application",
                "Video track no longer exists",
            );
            return false;
        };
        if track
            .items
            .iter()
            .any(|item| item.start <= start && start < item.end)
        {
            interaction::show_error_dialog(
                area,
                "Could not record screen or application",
                "The playhead is inside an existing item on this video track",
            );
            return false;
        }
        (
            start,
            project.fps,
            track
                .items
                .iter()
                .find(|item| item.start > start)
                .map(|item| item.start),
        )
    };
    match video_recording::ScreenRecording::start(fps) {
        Ok(recording) => {
            runtime.active_video_recording = Some(ActiveVideoRecording {
                key,
                start,
                stop_at,
                recording,
                ready: false,
                stopping: false,
            });
            poll_video_recording(
                area,
                project.clone(),
                player_state.clone(),
                runtime_shared.clone(),
            );
            area.queue_render();
            true
        }
        Err(error) => {
            interaction::show_error_dialog(area, "Could not record screen or application", &error);
            false
        }
    }
}

fn poll_video_recording(
    area: &gtk::GLArea,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    runtime: Rc<RefCell<TimelineRuntime>>,
) {
    let area = area.clone();
    glib::timeout_add_local(VIDEO_RECORDING_POLL_INTERVAL, move || {
        let event = runtime
            .borrow()
            .active_video_recording
            .as_ref()
            .and_then(|active| active.recording.try_event().ok());
        let Some(event) = event else {
            return if runtime.borrow().active_video_recording.is_some() {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            };
        };
        match event {
            video_recording::ScreenRecordingEvent::Ready { width, height } => {
                tracing::info!(width, height, "screen recording ready");
                let mut timeline = runtime.borrow_mut();
                let Some(active) = timeline.active_video_recording.as_mut() else {
                    return glib::ControlFlow::Break;
                };
                if active.stopping {
                    return glib::ControlFlow::Continue;
                }
                let start_occupied = project
                    .borrow()
                    .video_tracks
                    .get(active.key.track_index)
                    .is_none_or(|track| {
                        track
                            .items
                            .iter()
                            .any(|item| item.start <= active.start && active.start < item.end)
                    });
                if start_occupied {
                    active.stopping = true;
                    active.recording.stop();
                    drop(timeline);
                    interaction::show_error_dialog(
                        &area,
                        "Could not record screen or application",
                        "The recording position became occupied while selecting a screen or application",
                    );
                } else {
                    active.ready = true;
                    let start = active.start;
                    drop(timeline);
                    ensure_recording_duration(&player_state, start);
                    player_state::set_playing(&player_state, true);
                }
                area.queue_render();
            }
            video_recording::ScreenRecordingEvent::Cancelled => {
                runtime.borrow_mut().active_video_recording = None;
                area.queue_render();
                return glib::ControlFlow::Break;
            }
            video_recording::ScreenRecordingEvent::Finished(result) => {
                let active = runtime.borrow_mut().active_video_recording.take();
                if let Some(active) = active {
                    let was_ready = active.ready;
                    if let Err(error) =
                        finish_video_recording(&area, &project, &player_state, active, result)
                    {
                        interaction::show_error_dialog(
                            &area,
                            "Could not record screen or application",
                            &error,
                        );
                    }
                    if was_ready {
                        player_state::set_playing(&player_state, false);
                    }
                }
                area.queue_render();
                return glib::ControlFlow::Break;
            }
        }
        glib::ControlFlow::Continue
    });
}

fn finish_video_recording(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    active: ActiveVideoRecording,
    result: Result<video_recording::FinishedScreenRecording, String>,
) -> Result<(), String> {
    let finished = result?;
    if !active.ready || finished.duration <= Time::ZERO {
        let _ = std::fs::remove_file(&finished.path);
        return Ok(());
    }
    let mut project_state = project.borrow_mut();
    let recorded_end = active
        .start
        .saturating_add(finished.duration)
        .snapped(project_state.frame_step());
    let end = active
        .stop_at
        .map_or(recorded_end, |stop_at| recorded_end.min(stop_at));
    if end <= active.start {
        drop(project_state);
        let _ = std::fs::remove_file(&finished.path);
        return Err("The recording has no free space on the video track".to_string());
    }
    let canvas_size = project_state.canvas_size;
    let rgb_track = project_state
        .video_tracks
        .get(active.key.track_index)
        .ok_or_else(|| "Video track no longer exists".to_string());
    let Ok(rgb_track) = rgb_track else {
        drop(project_state);
        let _ = std::fs::remove_file(&finished.path);
        return Err("Video track no longer exists".to_string());
    };
    if timeline_search::collides(&rgb_track.items, active.start, end) {
        drop(project_state);
        let _ = std::fs::remove_file(&finished.path);
        return Err("The recording position overlaps another video item".to_string());
    }
    let transform = Transform::natural_size(canvas_size, finished.width, finished.height);
    let item = VideoItem {
        id: uuid::Uuid::new_v4(),
        start: active.start,
        end,
        time_offset: Time::ZERO,
        source_duration: finished.duration,
        playback_speed: default_playback_speed(),
        playback_fps: project::native_playback_fps(),
        repeat_strategy: RepeatStrategy::Hold,
        stabilize_video: false,
        stabilization_method: Default::default(),
        stabilization_crop_ratio: project::default_video_stabilization_crop_ratio(),
        stabilization_first_derivative_weight:
            project::default_video_stabilization_first_derivative_weight(),
        stabilization_second_derivative_weight:
            project::default_video_stabilization_second_derivative_weight(),
        stabilization_third_derivative_weight:
            project::default_video_stabilization_third_derivative_weight(),
        mesh_flow_rows: project::default_mesh_flow_rows(),
        mesh_flow_columns: project::default_mesh_flow_columns(),
        mesh_flow_smoothing_radius: project::default_mesh_flow_smoothing_radius(),
        mesh_flow_iterations: project::default_mesh_flow_iterations(),
        mesh_flow_adaptive_weights: Default::default(),
        animation_time_offset: Time::ZERO,
        motion_blur: Default::default(),
        transform: transform.clone(),
        modifiers: Vec::new(),
        sample_method: TimelineValue::new_const(VideoSampleMethod::Xbrz),
        skia_drawing_strategy: Default::default(),
        compositing: Default::default(),
        visibility: TimelineValue::new_const(TimelineBool::True),
        alpha_mask_video: Some(1),
        transitions: Default::default(),
        svg_color_overrides: Vec::new(),
        source_width: finished.width,
        source_height: finished.height,
        default_transform: Some(transform),
        content: VideoItemContent::Media,
        video_generation: None,
        group_id: None,
        render_canvas_size: None,
        track_id: 0,
        file: finished.path.into(),
    };
    items::insert_sorted(
        &mut project_state.video_tracks[active.key.track_index].items,
        item,
    );
    crate::project::commit_edit(&project_state, "record-video");
    let duration = project_state.duration();
    drop(project_state);
    player_state::refresh_project(
        player_state,
        player_state::ProjectChange {
            duration: Some(duration),
            video: true,
            inspector: true,
            ..player_state::ProjectChange::default()
        },
    );
    area.queue_render();
    Ok(())
}

pub(super) fn ensure_recording_duration(player_state: &SharedPlayerState, position: Time) {
    let snapshot = player_state::snapshot(player_state);
    let headroom = Time::from_seconds(RECORDING_DURATION_HEADROOM_SECONDS);
    let duration = position.saturating_add(headroom);
    if duration > snapshot.duration {
        player_state::set_duration(player_state, duration);
    }
}

pub(super) fn stop_before_backward_seek(
    runtime: &mut TimelineRuntime,
    player_state: &SharedPlayerState,
    position: Time,
) {
    let snapshot = player_state::snapshot(player_state);
    if position >= snapshot.position
        || (runtime.active_audio_recording.is_none() && runtime.active_video_recording.is_none())
    {
        return;
    }

    if let Some(active) = runtime.active_video_recording.as_mut()
        && !active.stopping
    {
        active.stopping = true;
        active.recording.stop();
    }
    player_state::set_playing(player_state, false);
}

pub(super) fn finish_audio_recording(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    active: ActiveAudioRecording,
) -> Result<(), String> {
    let finished = active.recording.finish()?;
    if finished.duration <= Time::ZERO {
        return Ok(());
    }

    let mut project_state = project.borrow_mut();
    let end = active
        .start
        .saturating_add(finished.duration)
        .snapped(project_state.frame_step());
    if end <= active.start {
        drop(project_state);
        let _ = std::fs::remove_file(&finished.path);
        return Err("The recording is shorter than one project frame".to_string());
    }
    let Some(track) = project_state.audio_tracks.get_mut(active.key.track_index) else {
        drop(project_state);
        let _ = std::fs::remove_file(&finished.path);
        return Err("Audio track no longer exists".to_string());
    };
    let item = crate::project::AudioItem::builder(active.start, end)
        .source_duration(finished.duration)
        .file(finished.path)
        .build();
    items::overwrite_items(&mut track.items, active.start, end);
    items::insert_sorted(&mut track.items, item);
    crate::project::commit_edit(&project_state, "record-audio");
    let duration = project_state.duration();
    drop(project_state);

    player_state::refresh_project(
        player_state,
        player_state::ProjectChange {
            duration: Some(duration),
            audio: true,
            audio_waveforms: true,
            inspector: true,
            ..player_state::ProjectChange::default()
        },
    );
    area.queue_render();
    Ok(())
}

pub(super) fn live_recording_draw(
    active: &ActiveAudioRecording,
    waveform_chunks_per_second: u32,
) -> Option<LiveRecordingDraw> {
    let snapshot = active.recording.snapshot();
    let frames = snapshot.samples.len() / 2;
    if frames == 0 {
        return None;
    }

    let duration = Time::from_nanos(
        ((frames as u128 * 1_000_000_000_u128) / snapshot.sample_rate.max(1) as u128)
            .min(u64::MAX as u128) as u64,
    );
    let item = AudioItem::builder(active.start, active.start.saturating_add(duration))
        .source_duration(duration)
        .file(PathBuf::from("live-recording.opus"))
        .build();
    let waveform = waveform::from_stereo_samples(
        &snapshot.samples,
        snapshot.sample_rate,
        waveform_chunks_per_second,
    );
    Some(LiveRecordingDraw {
        key: active.key,
        item,
        waveform,
    })
}
