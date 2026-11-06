use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use shrimply_project_document::project::Project;

use super::SharedAudioLevels;
use super::output::{self, OutputState, PlaybackWindow};
use super::worker::{self, AudioCommand};

const OUTPUT_PERIOD_MS: u32 = 40;

pub(super) struct AudioRuntime {
    stream: cpal::Stream,
    command_tx: Sender<AudioCommand>,
    worker: Option<thread::JoinHandle<()>>,
    pub(super) sample_rate: u32,
    pub(super) output: OutputState,
}

impl AudioRuntime {
    pub(super) fn new(project: &Project, levels: SharedAudioLevels) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No default audio output device".to_string())?;
        let default_config = device
            .default_output_config()
            .map_err(|error| error.to_string())?;
        let output_config = device
            .supported_output_configs()
            .map_err(|error| error.to_string())?
            .find(|range| {
                range.channels() == default_config.channels()
                    && range.sample_format() == default_config.sample_format()
                    && range.min_sample_rate() <= 48_000
                    && range.max_sample_rate() >= 48_000
            })
            .map(|range| range.with_sample_rate(48_000))
            .unwrap_or(default_config);
        let mut config = output_config.config();
        if let cpal::SupportedBufferSize::Range { min, max } = *output_config.buffer_size() {
            let period_frames = config
                .sample_rate
                .saturating_mul(OUTPUT_PERIOD_MS)
                .div_ceil(1_000)
                .clamp(min, max);
            config.buffer_size = cpal::BufferSize::Fixed(period_frames);
        }
        let output_channels = config.channels as usize;
        let sample_rate = config.sample_rate;
        tracing::info!(
            "Building streaming audio output: host={}, device={}, sample_format={:?}, sample_rate={}, channels={}, buffer_size={:?}",
            host.id(),
            device,
            output_config.sample_format(),
            sample_rate,
            output_channels,
            config.buffer_size,
        );

        let window = Arc::new(RwLock::new(Arc::new(PlaybackWindow::new())));
        let playing = Arc::new(AtomicBool::new(false));
        let previewing = Arc::new(AtomicBool::new(false));
        let cursor_frame = Arc::new(AtomicU64::new(0));
        let preview_end_frame = Arc::new(AtomicU64::new(0));
        let duration_frames = Arc::new(AtomicU64::new(
            project.duration().as_sample_frame(sample_rate),
        ));
        let failure = Arc::new(Mutex::new(None));
        let recoverable_error_log = Arc::new(Mutex::new(None));
        let output_state = OutputState {
            window,
            requested: Arc::new(AtomicBool::new(true)),
            playing,
            previewing,
            cursor_frame,
            preview_end_frame,
            duration_frames,
            output_channels,
            levels,
            failure,
            recoverable_error_log,
        };
        let stream = output::build_stream(
            &device,
            &config,
            output_config.sample_format(),
            output_state.clone(),
        )?;
        let (command_tx, worker) =
            worker::spawn(project.clone(), sample_rate, output_state.clone());
        Ok(Self {
            stream,
            command_tx,
            worker: Some(worker),
            sample_rate,
            output: output_state,
        })
    }

    pub(super) fn send(&self, command: AudioCommand) {
        self.command_tx
            .send(command)
            .expect("streaming audio worker disconnected");
    }

    pub(super) fn start_output(&self) {
        {
            let mut failure = self
                .output
                .failure
                .lock()
                .expect("audio failure lock poisoned");
            if let Some(failure) = failure.as_mut() {
                // A device can fail between checking the runtime and requesting playback.
                failure.requested = true;
                self.output.requested.store(false, Ordering::SeqCst);
                self.output.playing.store(false, Ordering::SeqCst);
                self.output.previewing.store(false, Ordering::SeqCst);
                return;
            }
        }
        if let Err(error) = self.stream.play() {
            output::fail_output(format!("Audio output failed: {error}"), &self.output);
        }
    }
}

impl Drop for AudioRuntime {
    fn drop(&mut self) {
        self.output.requested.store(false, Ordering::SeqCst);
        self.output.playing.store(false, Ordering::SeqCst);
        self.output.previewing.store(false, Ordering::SeqCst);
        // The receiver may already be gone if the worker panicked; join reports that failure.
        let _ = self.command_tx.send(AudioCommand::Stop);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("streaming audio worker panicked during shutdown");
        }
    }
}
