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

#[cfg(all(feature = "capture", any(target_os = "linux", windows)))]
mod camera;
#[cfg(all(feature = "capture", any(target_os = "linux", windows)))]
pub use camera::devices as camera_devices;

#[cfg(all(feature = "capture", any(target_os = "linux", windows)))]
pub enum VideoRecording {
    Screen(ScreenRecording),
    Camera(camera::CameraRecording),
}

#[cfg(all(feature = "capture", any(target_os = "linux", windows)))]
impl VideoRecording {
    pub fn start(
        fps: shrimply_math_core::Fraction,
        source: Option<&str>,
        #[cfg(windows)] native_window: usize,
    ) -> Result<Self, String> {
        if let Some(source) = source {
            return camera::CameraRecording::start(source, fps).map(Self::Camera);
        }
        #[cfg(target_os = "linux")]
        let result = ScreenRecording::start(fps);
        #[cfg(windows)]
        let result = ScreenRecording::start(fps, native_window);
        result.map(Self::Screen)
    }

    pub fn stop(&self) {
        match self {
            Self::Screen(recording) => recording.stop(),
            Self::Camera(recording) => recording.stop(),
        }
    }

    pub fn try_event(&self) -> Result<VideoRecordingEvent, std::sync::mpsc::TryRecvError> {
        match self {
            Self::Screen(recording) => recording.try_event(),
            Self::Camera(recording) => recording.try_event(),
        }
    }
}

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
