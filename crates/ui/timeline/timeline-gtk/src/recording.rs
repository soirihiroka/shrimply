use super::*;

pub(super) fn handle_video_recording(
    area: &gtk::GLArea,
    runtime_shared: &Rc<RefCell<TimelineRuntime>>,
    runtime: &mut TimelineRuntime,
    request: shrimply_timeline_skia::scene::TrackRecordingRequest,
) -> bool {
    if request.key.kind != TrackKind::Video {
        return false;
    }
    if let Err(error) = runtime
        .scene
        .toggle_video_recording_at(request.key, request.position)
    {
        interaction::show_error_dialog(area, "Could not record screen or application", &error);
        return true;
    }
    match apply_video_recording_commands(runtime) {
        Ok(started) => {
            if started {
                poll_video_recording(area, runtime_shared.clone());
            }
        }
        Err(error) => {
            interaction::show_error_dialog(area, "Could not record screen or application", &error);
        }
    }
    area.queue_render();
    true
}

pub(super) fn apply_video_recording_commands(
    runtime: &mut TimelineRuntime,
) -> Result<bool, String> {
    let mut started = false;
    while let Some(command) = runtime.scene.take_video_recording_command() {
        match command {
            shrimply_timeline_skia::recording::VideoRecordingCommand::Start { fps } => {
                if runtime.screen_recording.is_some() {
                    return Err("A screen recording is already active".into());
                }
                match video_recording::ScreenRecording::start(fps) {
                    Ok(recording) => {
                        runtime.screen_recording = Some(recording);
                        started = true;
                    }
                    Err(error) => runtime.scene.handle_video_recording_event(
                        shrimply_timeline_skia::recording::VideoRecordingEvent::Finished(Err(
                            error,
                        )),
                    )?,
                }
            }
            shrimply_timeline_skia::recording::VideoRecordingCommand::Stop => {
                if let Some(recording) = runtime.screen_recording.as_ref() {
                    recording.stop();
                }
            }
        }
    }
    Ok(started)
}

fn poll_video_recording(area: &gtk::GLArea, runtime: Rc<RefCell<TimelineRuntime>>) {
    let area = area.clone();
    glib::timeout_add_local(VIDEO_RECORDING_POLL_INTERVAL, move || {
        let event = match runtime
            .borrow()
            .screen_recording
            .as_ref()
            .map(video_recording::ScreenRecording::try_event)
        {
            Some(Ok(event)) => event,
            Some(Err(std::sync::mpsc::TryRecvError::Empty)) => {
                return glib::ControlFlow::Continue;
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                video_recording::VideoRecordingEvent::Finished(Err(
                    "PipeWire screen capture stopped without a final event".into(),
                ))
            }
            None => return glib::ControlFlow::Break,
        };
        let terminal = matches!(
            event,
            video_recording::VideoRecordingEvent::Cancelled
                | video_recording::VideoRecordingEvent::Finished(_)
        );

        let result = {
            let mut runtime = runtime.borrow_mut();
            let result = runtime.scene.handle_video_recording_event(event);
            if terminal {
                runtime.screen_recording = None;
            }
            result.and_then(|()| apply_video_recording_commands(&mut runtime).map(|_| ()))
        };
        if let Err(error) = result {
            interaction::show_error_dialog(&area, "Could not record screen or application", &error);
        }
        area.queue_render();
        if terminal {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}
