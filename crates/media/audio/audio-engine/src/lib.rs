use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use shrimply_math_core::Fraction;

use shrimply_project_document::project::{Project, Time};

pub use shrimply_project_document::project;

use shrimply_math_media as math;

/// Initialize the application resources used by CPAL's native AAudio host.
///
/// # Safety
/// Both pointers must belong to this process and remain valid for its lifetime.
/// `context` must be a retained global reference to the Android application.
#[cfg(target_os = "android")]
pub unsafe fn initialize_android_context(
    vm: *mut std::ffi::c_void,
    context: *mut std::ffi::c_void,
) {
    static INITIALIZED: std::sync::Once = std::sync::Once::new();
    assert!(
        !vm.is_null() && !context.is_null(),
        "Android audio host requires live application resources"
    );
    INITIALIZED.call_once(|| unsafe { ndk_context::initialize_android_context(vm, context) });
    assert_eq!(
        ndk_context::android_context().vm(),
        vm,
        "Android audio VM changed"
    );
}

pub mod beat;
mod beat_cache;
mod beat_math;
mod effects;
mod host;
pub mod modifier_cache;
mod opus_cache;
mod output;
pub mod pneuma;
pub mod recording;
mod runtime;
pub mod streaming;
pub mod waveform;
mod worker;

use runtime::AudioRuntime;
use worker::AudioCommand;

const CHANNELS: usize = 2;
const SCRUB_PREVIEW_MS: u64 = 120;

#[derive(Default)]
pub struct AudioLevels {
    peaks: [AtomicU32; CHANNELS],
}

pub type SharedAudioLevels = Arc<AudioLevels>;

impl AudioLevels {
    pub fn take_peaks(&self) -> [f32; CHANNELS] {
        self.peaks
            .each_ref()
            .map(|peak| f32::from_bits(peak.swap(0, Ordering::Relaxed)))
    }

    fn record(&self, left: f32, right: f32) {
        for (peak, sample) in self.peaks.iter().zip([left, right]) {
            let sample = sample.abs();
            if sample.is_finite() {
                peak.fetch_max(sample.to_bits(), Ordering::Relaxed);
            }
        }
    }
}

pub struct AudioPlayer {
    state: RefCell<PlayerState>,
    levels: SharedAudioLevels,
}

struct PlayerState {
    project: Project,
    position: Time,
    playback_speed: Fraction,
    runtime: Option<AudioRuntime>,
    failure: Option<String>,
    stopped: bool,
}

impl PlayerState {
    fn collect_failure(&mut self) {
        let failure = self.runtime.as_ref().and_then(|runtime| {
            // Keep the failure latched until dropping the stream has stopped its callbacks.
            runtime
                .output
                .failure
                .lock()
                .expect("audio failure lock poisoned")
                .clone()
        });
        if let Some(failure) = failure {
            let runtime = self.runtime.as_ref().expect("failed audio runtime exists");
            self.position = shrimply_math_core::time_from_sample_frame(
                runtime.output.cursor_frame.load(Ordering::SeqCst),
                runtime.sample_rate,
            );
            self.runtime = None;
            if failure.requested {
                self.failure.get_or_insert(failure.message);
            }
        }
    }

    fn runtime(&mut self, levels: &SharedAudioLevels) -> Option<&AudioRuntime> {
        self.collect_failure();
        if self.stopped || self.failure.is_some() {
            return None;
        }
        if self.runtime.is_none() {
            match AudioRuntime::new(&self.project, levels.clone()) {
                Ok(runtime) => {
                    let frame = self
                        .position
                        .as_sample_frame(runtime.sample_rate)
                        .min(runtime.output.duration_frames.load(Ordering::SeqCst));
                    runtime.output.cursor_frame.store(frame, Ordering::SeqCst);
                    runtime.send(AudioCommand::SetPlaybackSpeed(self.playback_speed));
                    self.runtime = Some(runtime);
                }
                Err(error) => {
                    tracing::error!(%error, "Could not initialize audio playback");
                    self.failure = Some(error);
                    return None;
                }
            }
        }
        self.runtime.as_ref()
    }
}

impl AudioPlayer {
    pub fn new(project: &Project, levels: SharedAudioLevels) -> Result<Self, String> {
        Ok(Self {
            state: RefCell::new(PlayerState {
                project: project.clone(),
                position: Time::ZERO,
                playback_speed: project::default_playback_speed(),
                runtime: None,
                failure: None,
                stopped: false,
            }),
            levels,
        })
    }

    pub fn set_project(&self, project: &Project) {
        let mut state = self.state.borrow_mut();
        state.project = project.clone();
        if let Some(runtime) = &state.runtime {
            runtime.output.duration_frames.store(
                project.duration().as_sample_frame(runtime.sample_rate),
                Ordering::SeqCst,
            );
            runtime.send(AudioCommand::SetProject(Box::new(project.clone())));
        }
    }

    pub fn seek(&self, position: Time) {
        let mut state = self.state.borrow_mut();
        state.position = position;
        if let Some(runtime) = &state.runtime {
            let frame = position
                .as_sample_frame(runtime.sample_rate)
                .min(runtime.output.duration_frames.load(Ordering::SeqCst));
            runtime.output.cursor_frame.store(frame, Ordering::SeqCst);
            runtime.output.previewing.store(false, Ordering::SeqCst);
            runtime.output.requested.store(
                runtime.output.playing.load(Ordering::SeqCst),
                Ordering::SeqCst,
            );
            runtime.send(AudioCommand::Seek { frame });
        }
    }

    pub fn preview_from(&self, position: Time) {
        let mut state = self.state.borrow_mut();
        state.position = position;
        if position >= state.project.duration() {
            return;
        }
        let Some(runtime) = state.runtime(&self.levels) else {
            return;
        };
        let frame = position
            .as_sample_frame(runtime.sample_rate)
            .min(runtime.output.duration_frames.load(Ordering::SeqCst));
        let preview_frames = runtime.sample_rate as u64 * SCRUB_PREVIEW_MS / 1_000;
        let end_frame = frame
            .saturating_add(preview_frames)
            .min(runtime.output.duration_frames.load(Ordering::SeqCst));
        if frame >= end_frame {
            runtime.output.requested.store(false, Ordering::SeqCst);
            return;
        }
        runtime.output.cursor_frame.store(frame, Ordering::SeqCst);
        runtime
            .output
            .preview_end_frame
            .store(end_frame, Ordering::SeqCst);
        runtime.output.playing.store(false, Ordering::SeqCst);
        runtime.output.previewing.store(false, Ordering::SeqCst);
        runtime.output.requested.store(true, Ordering::SeqCst);
        runtime.send(AudioCommand::Preview {
            frame,
            frames: end_frame.saturating_sub(frame) as usize,
        });
        runtime.start_output();
    }

    pub fn set_playback_speed(&self, playback_speed: Fraction) {
        let mut state = self.state.borrow_mut();
        state.playback_speed = playback_speed;
        if let Some(runtime) = &state.runtime {
            runtime.send(AudioCommand::SetPlaybackSpeed(playback_speed));
        }
    }

    pub fn set_playing(&self, playing: bool) {
        let mut state = self.state.borrow_mut();
        if playing {
            let Some(runtime) = state.runtime(&self.levels) else {
                return;
            };
            runtime.output.previewing.store(false, Ordering::SeqCst);
            runtime.output.requested.store(true, Ordering::SeqCst);
            runtime.output.playing.store(true, Ordering::SeqCst);
            runtime.send(AudioCommand::PlayFrom {
                frame: runtime.output.cursor_frame.load(Ordering::SeqCst),
            });
            runtime.start_output();
        } else if let Some(runtime) = &state.runtime {
            runtime.output.requested.store(false, Ordering::SeqCst);
            runtime.output.playing.store(false, Ordering::SeqCst);
            runtime.output.previewing.store(false, Ordering::SeqCst);
            let position = shrimply_math_core::time_from_sample_frame(
                runtime.output.cursor_frame.load(Ordering::SeqCst),
                runtime.sample_rate,
            );
            runtime.send(AudioCommand::Pause);
            state.position = position;
        }
    }

    pub fn stop(&self) {
        let mut state = self.state.borrow_mut();
        state.stopped = true;
        state.runtime = None;
    }

    pub fn take_failure(&self) -> Option<String> {
        let mut state = self.state.borrow_mut();
        state.collect_failure();
        state.failure.take()
    }
}
