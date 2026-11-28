//! Translate native screen-capture lifecycle to the shared timeline recording owner.
use super::Runtime;
use serde_json::{Value, json};
use shrimply_math_core::{Fraction, Time, fraction_denominator, fraction_numerator};
use shrimply_timeline_skia::recording::{
    FinishedVideoRecording, VideoRecordingCommand, VideoRecordingEvent,
};
use std::path::PathBuf;

const SCREEN_RECORDING_BITRATE: u32 = 20_000_000;
const SCREEN_RECORDING_KEYFRAME_SECONDS: u32 = 1;
const MICROSECONDS_PER_SECOND: i64 = 1_000_000;

pub(super) struct Recording {
    id: String,
    path: PathBuf,
    fps: Fraction,
    camera_id: Option<String>,
    stopping: bool,
    discard: bool,
}

impl Drop for Recording {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("Could not remove incomplete screen recording: {error}");
        }
    }
}

impl Runtime {
    pub fn request_recording(
        &mut self,
        request: shrimply_timeline_skia::scene::TrackRecordingRequest,
    ) -> Result<(), String> {
        let key = request.key;
        let operation = match key.kind {
            shrimply_timeline_edit::TrackKind::Audio => "show_audio_record",
            shrimply_timeline_edit::TrackKind::Video => "record_video",
            _ => return Err("This track cannot record media".into()),
        };
        let track_id = self
            .backend
            .project()
            .and_then(|project| {
                shrimply_timeline_edit::selection_state::track_address(&project, key)
            })
            .ok_or("The recording track no longer exists")?
            .track_id()
            .to_string();
        self.request(&json!({"op":operation,"project_path":self.backend.path(),
            "kind":key.kind.label().to_ascii_lowercase(),"track":key.track_index,
            "track_id":track_id,"position":request.position}))?;
        Ok(())
    }

    pub fn poll_video_recording(&mut self) -> Result<bool, String> {
        let mut changed = false;
        while let Some(command) = self
            .timeline
            .as_mut()
            .and_then(|scene| scene.take_video_recording_command())
        {
            changed = true;
            match command {
                VideoRecordingCommand::Start { fps, source } => {
                    if self.video_recording.is_some() {
                        self.timeline
                            .as_mut()
                            .expect("recording scene")
                            .handle_video_recording_event(VideoRecordingEvent::Finished(Err(
                                "A video recording is already active".into(),
                            )))?;
                        continue;
                    }
                    let result = (|| {
                        let directory = shrimply_path_core::recording_directory()?;
                        let id = uuid::Uuid::new_v4().to_string();
                        Ok(Recording {
                            path: directory.join(format!(".incoming-{id}")),
                            id,
                            fps,
                            camera_id: source,
                            stopping: false,
                            discard: false,
                        })
                    })();
                    match result {
                        Ok(recording) => self.video_recording = Some(recording),
                        Err(error) => self
                            .timeline
                            .as_mut()
                            .expect("recording scene")
                            .handle_video_recording_event(VideoRecordingEvent::Finished(Err(
                                error,
                            )))?,
                    }
                }
                VideoRecordingCommand::Stop => {
                    if let Some(recording) = &mut self.video_recording {
                        recording.stopping = true;
                    }
                }
            }
        }
        Ok(changed)
    }

    pub(super) fn discard_video_recording(&mut self) {
        if let Some(recording) = &mut self.video_recording {
            recording.stopping = true;
            recording.discard = true;
        }
    }

    pub fn screen_recording_clock_active(&self) -> bool {
        self.video_recording
            .as_ref()
            .is_some_and(|recording| !recording.stopping && !recording.discard)
            && self
                .timeline
                .as_ref()
                .is_some_and(|scene| scene.video_recording_clock_active())
    }

    pub(crate) fn screen_recording_pending(&self) -> bool {
        self.video_recording
            .as_ref()
            .is_some_and(|recording| !recording.stopping && !recording.discard)
            && self
                .timeline
                .as_ref()
                .is_some_and(|scene| scene.video_recording_in_progress())
    }

    pub(super) fn video_recording_snapshot(&self) -> Value {
        self.video_recording
            .as_ref()
            .map_or(Value::Null, |recording| {
                json!({
                    "id":recording.id, "path":recording.path, "camera_id":recording.camera_id,
                    "state":if recording.stopping { "stop" } else { "start" },
                    "fps_numerator":fraction_numerator(recording.fps),
                    "fps_denominator":fraction_denominator(recording.fps),
                    "mime":"video/avc", "bitrate":SCREEN_RECORDING_BITRATE,
                    "keyframe_seconds":SCREEN_RECORDING_KEYFRAME_SECONDS,
                    "notification_title":shrimply_i18n::text("Screen Recording"),
                    "notification_text":shrimply_i18n::text("Recording screen or application"),
                    "stop_label":shrimply_i18n::text("Stop"),
                })
            })
    }

    pub(super) fn video_recording_event(&mut self, request: &Value) -> Result<Value, String> {
        let Some(recording) = self.video_recording.as_ref() else {
            return Ok(json!({}));
        };
        if request["id"].as_str() != Some(&recording.id) {
            return Ok(json!({}));
        }
        let event = request["event"]
            .as_str()
            .ok_or("Video recording event is required")?;
        if event == "ready" {
            if recording.stopping {
                return Ok(json!({}));
            }
            let dimension = |key: &str| {
                request[key]
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .filter(|value| *value > 0)
                    .ok_or("Invalid recording dimensions")
            };
            self.timeline
                .as_mut()
                .ok_or("The recording project is closed")?
                .handle_video_recording_event(VideoRecordingEvent::Ready {
                    width: dimension("width")?,
                    height: dimension("height")?,
                })?;
        } else if matches!(event, "finished" | "cancelled" | "failed") {
            let recording = self.video_recording.take().expect("validated recording");
            if recording.discard || self.timeline.is_none() {
                return Ok(json!({}));
            }
            let event = match event {
                "cancelled" => VideoRecordingEvent::Cancelled,
                "failed" => VideoRecordingEvent::Finished(Err(request["error"]
                    .as_str()
                    .unwrap_or("Video capture failed")
                    .to_owned())),
                _ => {
                    let result = (|| {
                        let duration = request["duration_us"]
                            .as_i64()
                            .filter(|value| *value > 0)
                            .map(|value| Time::from_fraction(value, MICROSECONDS_PER_SECOND))
                            .ok_or("The video recording has no duration")?;
                        let width = request["width"]
                            .as_u64()
                            .and_then(|value| u32::try_from(value).ok())
                            .filter(|value| *value > 0)
                            .ok_or("Invalid recording width")?;
                        let height = request["height"]
                            .as_u64()
                            .and_then(|value| u32::try_from(value).ok())
                            .filter(|value| *value > 0)
                            .ok_or("Invalid recording height")?;
                        let path = recording
                            .path
                            .with_file_name(format!("{}.mp4", recording.id));
                        std::fs::rename(&recording.path, &path)
                            .map_err(|error| error.to_string())?;
                        Ok(FinishedVideoRecording::new(
                            path, duration, width, height, None,
                        ))
                    })();
                    VideoRecordingEvent::Finished(result)
                }
            };
            self.timeline
                .as_mut()
                .expect("recording scene")
                .handle_video_recording_event(event)?;
        } else {
            return Err("Unknown screen recording event".into());
        }
        self.timeline_changed()?;
        Ok(json!({}))
    }
}
