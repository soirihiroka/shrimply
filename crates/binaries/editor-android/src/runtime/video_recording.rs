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

pub(super) struct Recording {
    id: String,
    path: PathBuf,
    fps: Fraction,
    stopping: bool,
    discard: bool,
}

impl Drop for Recording {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "Could not remove incomplete screen recording");
        }
    }
}

impl Runtime {
    pub fn poll_video_recording(&mut self) -> Result<bool, String> {
        let mut changed = false;
        while let Some(command) = self.timeline.as_mut().and_then(|scene| scene.take_video_recording_command()) {
            changed = true;
            match command {
                VideoRecordingCommand::Start { fps } => {
                    if self.video_recording.is_some() {
                        self.timeline.as_mut().expect("recording scene").handle_video_recording_event(
                            VideoRecordingEvent::Finished(Err("A screen recording is already active".into())),
                        )?;
                        continue;
                    }
                    let directory = shrimply_path_core::cache_directory()?.join("recordings");
                    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
                    let id = uuid::Uuid::new_v4().to_string();
                    self.video_recording = Some(Recording {
                        path: directory.join(format!("{id}.mp4.partial")),
                        id, fps, stopping: false, discard: false,
                    });
                }
                VideoRecordingCommand::Stop => {
                    if let Some(recording) = &mut self.video_recording { recording.stopping = true; }
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

    pub(super) fn video_recording_snapshot(&self) -> Value {
        self.video_recording.as_ref().map_or(Value::Null, |recording| json!({
            "id":recording.id, "path":recording.path,
            "state":if recording.stopping { "stop" } else { "start" },
            "fps_numerator":fraction_numerator(recording.fps),
            "fps_denominator":fraction_denominator(recording.fps),
            "mime":"video/avc", "bitrate":SCREEN_RECORDING_BITRATE,
            "keyframe_seconds":SCREEN_RECORDING_KEYFRAME_SECONDS,
            "notification_title":shrimply_i18n::text("Screen Recording"),
            "notification_text":shrimply_i18n::text("Recording screen or application"),
            "stop_label":shrimply_i18n::text("Stop"),
        }))
    }

    pub(super) fn video_recording_event(&mut self, request: &Value) -> Result<Value, String> {
        let Some(recording) = self.video_recording.as_ref() else { return Ok(json!({})); };
        if request["id"].as_str() != Some(&recording.id) { return Ok(json!({})); }
        let event = request["event"].as_str().ok_or("Screen recording event is required")?;
        if event == "ready" {
            if recording.stopping { return Ok(json!({})); }
            let dimension = |key: &str| request[key].as_u64().and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0).ok_or("Invalid screen recording dimensions");
            self.timeline.as_mut().ok_or("The recording project is closed")?
                .handle_video_recording_event(VideoRecordingEvent::Ready { width: dimension("width")?, height: dimension("height")? })?;
        } else if matches!(event, "finished" | "cancelled" | "failed") {
            let recording = self.video_recording.take().expect("validated recording");
            if recording.discard || self.timeline.is_none() { return Ok(json!({})); }
            let event = match event {
                "cancelled" => VideoRecordingEvent::Cancelled,
                "failed" => VideoRecordingEvent::Finished(Err(request["error"].as_str().unwrap_or("Screen capture failed").to_owned())),
                _ => {
                    let result = (|| {
                        let last = request["last_timestamp_us"].as_i64().filter(|value| *value >= 0)
                            .ok_or("The screen recording has no frames")?;
                        let duration = Time::from_fraction(last, 1_000_000).saturating_add(Time::from_fraction(
                            fraction_denominator(recording.fps), fraction_numerator(recording.fps)));
                        let width = request["width"].as_u64().and_then(|value| u32::try_from(value).ok()).filter(|value| *value > 0).ok_or("Invalid recording width")?;
                        let height = request["height"].as_u64().and_then(|value| u32::try_from(value).ok()).filter(|value| *value > 0).ok_or("Invalid recording height")?;
                        let path = recording.path.with_extension("");
                        std::fs::rename(&recording.path, &path).map_err(|error| error.to_string())?;
                        Ok(FinishedVideoRecording::new(path, duration, width, height, None))
                    })();
                    VideoRecordingEvent::Finished(result)
                }
            };
            self.timeline.as_mut().expect("recording scene").handle_video_recording_event(event)?;
        } else { return Err("Unknown screen recording event".into()); }
        self.timeline_changed()?;
        Ok(json!({}))
    }
}
