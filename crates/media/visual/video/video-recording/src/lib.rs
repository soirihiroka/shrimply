//! Native screen capture reports the same recording events on every platform.
use shrimply_math_core::Time;
use std::path::PathBuf;

#[cfg(all(feature = "capture", target_os = "linux"))]
mod wayland;
#[cfg(all(feature = "capture", any(target_os = "linux", windows)))]
mod writer;
#[cfg(all(feature = "capture", target_os = "linux"))]
pub use wayland::ScreenRecording;
#[cfg(all(feature = "capture", windows))]
#[path = "windows.rs"]
mod windows_capture;
#[cfg(all(feature = "capture", windows))]
pub use windows_capture::ScreenRecording;

pub enum VideoRecordingEvent {
    Ready { width: u32, height: u32 },
    Cancelled,
    Finished(Result<FinishedVideoRecording, String>),
}

pub struct FinishedVideoRecording {
    path: Option<PathBuf>,
    pub duration: Time,
    pub width: u32,
    pub height: u32,
    pub alpha_track_id: Option<u32>,
}

impl FinishedVideoRecording {
    pub fn new(
        path: PathBuf,
        duration: Time,
        width: u32,
        height: u32,
        alpha_track_id: Option<u32>,
    ) -> Self {
        Self {
            path: Some(path),
            duration,
            width,
            height,
            alpha_track_id,
        }
    }

    pub fn path(&self) -> &std::path::Path {
        self.path.as_deref().expect("recording file is owned")
    }

    pub fn keep(mut self) {
        self.path.take();
    }
}

impl Drop for FinishedVideoRecording {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            remove_incomplete_recording(&path);
        }
    }
}

fn remove_incomplete_recording(path: &std::path::Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "could not remove incomplete recording")
        }
    }
}
