use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use cpal::traits::DeviceTrait;

use super::CHANNELS;
use super::SharedAudioLevels;

const RECOVERABLE_ERROR_LOG_INTERVAL: Duration = Duration::from_secs(1);
const STREAM_INIT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub(super) struct PlaybackWindow {
    generation: u64,
    start_frame: u64,
    samples: Vec<f32>,
}

pub(super) type SharedPlaybackWindow = Arc<RwLock<Arc<PlaybackWindow>>>;

#[derive(Clone)]
pub(super) struct PlaybackFailure {
    pub(super) message: String,
    pub(super) requested: bool,
}

#[derive(Clone)]
pub(super) struct OutputState {
    pub(super) window: SharedPlaybackWindow,
    pub(super) shutting_down: Arc<AtomicBool>,
    pub(super) requested: Arc<AtomicBool>,
    pub(super) playing: Arc<AtomicBool>,
    pub(super) previewing: Arc<AtomicBool>,
    pub(super) cursor_frame: Arc<AtomicU64>,
    pub(super) preview_end_frame: Arc<AtomicU64>,
    pub(super) duration_frames: Arc<AtomicU64>,
    pub(super) output_channels: usize,
    pub(super) levels: SharedAudioLevels,
    pub(super) failure: Arc<Mutex<Option<PlaybackFailure>>>,
    pub(super) recoverable_error_log: Arc<Mutex<Option<Instant>>>,
}

impl PlaybackWindow {
    pub(super) fn new() -> Self {
        Self {
            generation: 0,
            start_frame: 0,
            samples: Vec::new(),
        }
    }

    pub(super) fn clear(&mut self, generation: u64) {
        self.generation = generation;
        self.samples.clear();
    }

    pub(super) fn store_chunk(
        &mut self,
        generation: u64,
        start_frame: u64,
        samples: Vec<f32>,
        keep_before_frame: u64,
        max_frames: usize,
    ) {
        if samples.is_empty() {
            return;
        }

        if self.generation != generation || self.samples.is_empty() {
            self.generation = generation;
            self.start_frame = start_frame;
            self.samples = samples;
        } else {
            self.merge_chunk(start_frame, samples);
        }

        self.trim_before(keep_before_frame);
        self.trim_to_max_frames(max_frames);
    }

    fn merge_chunk(&mut self, start_frame: u64, samples: Vec<f32>) {
        let current_end = self.end_frame();
        if start_frame == current_end {
            self.samples.extend(samples);
            return;
        }
        if start_frame > current_end || start_frame < self.start_frame {
            self.start_frame = start_frame;
            self.samples = samples;
            return;
        }

        let overlap_samples = start_frame.saturating_sub(self.start_frame) as usize * CHANNELS;
        let replace_len = samples
            .len()
            .min(self.samples.len().saturating_sub(overlap_samples));
        self.samples[overlap_samples..overlap_samples + replace_len]
            .copy_from_slice(&samples[..replace_len]);
        if replace_len < samples.len() {
            self.samples.extend_from_slice(&samples[replace_len..]);
        }
    }

    fn sample_at(&self, frame: u64) -> Option<(f32, f32)> {
        if frame < self.start_frame || frame >= self.end_frame() {
            return None;
        }
        let index = frame.saturating_sub(self.start_frame) as usize * CHANNELS;
        Some((
            self.samples.get(index).copied().unwrap_or(0.0),
            self.samples.get(index + 1).copied().unwrap_or(0.0),
        ))
    }

    fn end_frame(&self) -> u64 {
        self.start_frame
            .saturating_add((self.samples.len() / CHANNELS) as u64)
    }

    fn trim_before(&mut self, frame: u64) {
        if frame <= self.start_frame {
            return;
        }
        let drain_frames = frame
            .saturating_sub(self.start_frame)
            .min((self.samples.len() / CHANNELS) as u64) as usize;
        self.samples.drain(..drain_frames * CHANNELS);
        self.start_frame = self.start_frame.saturating_add(drain_frames as u64);
    }

    fn trim_to_max_frames(&mut self, max_frames: usize) {
        let frame_count = self.samples.len() / CHANNELS;
        if frame_count <= max_frames {
            return;
        }
        let drain_frames = frame_count - max_frames;
        self.samples.drain(..drain_frames * CHANNELS);
        self.start_frame = self.start_frame.saturating_add(drain_frames as u64);
    }
}

pub(super) fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    output_state: OutputState,
) -> Result<cpal::Stream, String> {
    let error_state = output_state.clone();
    match sample_format {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                *config,
                move |data: &mut [f32], _| write_f32(data, &output_state),
                move |error| handle_output_error(error, &error_state),
                Some(STREAM_INIT_TIMEOUT),
            )
            .map_err(|error| error.to_string()),
        cpal::SampleFormat::I16 => device
            .build_output_stream(
                *config,
                move |data: &mut [i16], _| write_i16(data, &output_state),
                move |error| handle_output_error(error, &error_state),
                Some(STREAM_INIT_TIMEOUT),
            )
            .map_err(|error| error.to_string()),
        cpal::SampleFormat::U16 => device
            .build_output_stream(
                *config,
                move |data: &mut [u16], _| write_u16(data, &output_state),
                move |error| handle_output_error(error, &error_state),
                Some(STREAM_INIT_TIMEOUT),
            )
            .map_err(|error| error.to_string()),
        other => Err(format!("Unsupported audio output sample format {other:?}")),
    }
}

fn write_f32(output: &mut [f32], output_state: &OutputState) {
    let window = playback_window(output_state);
    for frame in output.chunks_mut(output_state.output_channels) {
        let (left, right) = next_frame(output_state, &window);
        write_frame(frame, left, right, |sample| sample);
    }
}

fn write_i16(output: &mut [i16], output_state: &OutputState) {
    let window = playback_window(output_state);
    for frame in output.chunks_mut(output_state.output_channels) {
        let (left, right) = next_frame(output_state, &window);
        write_frame(frame, left, right, |sample| {
            (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
        });
    }
}

fn write_u16(output: &mut [u16], output_state: &OutputState) {
    let window = playback_window(output_state);
    for frame in output.chunks_mut(output_state.output_channels) {
        let (left, right) = next_frame(output_state, &window);
        write_frame(frame, left, right, |sample| {
            ((sample.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16
        });
    }
}

fn playback_window(output_state: &OutputState) -> Arc<PlaybackWindow> {
    match output_state.window.read() {
        Ok(window) => window.clone(),
        Err(error) => error.into_inner().clone(),
    }
}

fn next_frame(output_state: &OutputState, window: &PlaybackWindow) -> (f32, f32) {
    if !output_state.requested.load(Ordering::SeqCst) {
        return (0.0, 0.0);
    }
    let playing = output_state.playing.load(Ordering::SeqCst);
    let previewing = output_state.previewing.load(Ordering::SeqCst);
    if !playing && !previewing {
        return (0.0, 0.0);
    }

    let frame = output_state.cursor_frame.load(Ordering::SeqCst);
    let duration_frames = output_state.duration_frames.load(Ordering::SeqCst);
    if frame >= duration_frames {
        output_state.requested.store(false, Ordering::SeqCst);
        output_state.playing.store(false, Ordering::SeqCst);
        output_state.previewing.store(false, Ordering::SeqCst);
        return (0.0, 0.0);
    }
    if !playing && frame >= output_state.preview_end_frame.load(Ordering::SeqCst) {
        output_state.requested.store(false, Ordering::SeqCst);
        output_state.previewing.store(false, Ordering::SeqCst);
        return (0.0, 0.0);
    }

    let Some(samples) = window.sample_at(frame) else {
        return (0.0, 0.0);
    };
    output_state.cursor_frame.fetch_add(1, Ordering::SeqCst);
    output_state.levels.record(samples.0, samples.1);
    samples
}

fn write_frame<T: Copy>(frame: &mut [T], left: f32, right: f32, convert: impl Fn(f32) -> T) {
    for (index, sample) in frame.iter_mut().enumerate() {
        *sample = match index {
            0 => convert(left),
            1 => convert(right),
            _ => convert((left + right) * 0.5),
        };
    }
}

fn handle_output_error(error: cpal::Error, output_state: &OutputState) {
    if output_state.shutting_down.load(Ordering::SeqCst) {
        return;
    }
    if matches!(
        error.kind(),
        cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied | cpal::ErrorKind::DeviceChanged
    ) {
        let now = Instant::now();
        let mut last_log = match output_state.recoverable_error_log.lock() {
            Ok(last_log) => last_log,
            Err(error) => error.into_inner(),
        };
        if last_log
            .as_ref()
            .is_none_or(|last| now.duration_since(*last) >= RECOVERABLE_ERROR_LOG_INTERVAL)
        {
            tracing::warn!("Streaming audio output recovered from: {error}");
            *last_log = Some(now);
        }
        return;
    }
    fail_output(format!("Audio output failed: {error}"), output_state);
}

pub(super) fn fail_output(message: String, output_state: &OutputState) {
    let mut failure = output_state
        .failure
        .lock()
        .expect("audio failure lock poisoned");
    if failure.is_some() || output_state.shutting_down.load(Ordering::SeqCst) {
        return;
    }
    tracing::error!("{message}");
    let requested = output_state.requested.swap(false, Ordering::SeqCst);
    output_state.playing.store(false, Ordering::SeqCst);
    output_state.previewing.store(false, Ordering::SeqCst);
    *failure = Some(PlaybackFailure { message, requested });
}
