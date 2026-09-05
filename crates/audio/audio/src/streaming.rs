use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};

use ffmpeg_next as ffmpeg;
use hashbrown::{HashMap, HashSet, hash_map::Entry};
use libc::EAGAIN;
use shrimply_asset::AssetSnapshot;
use shrimply_math_core::{Fraction, fraction_floor_i64, time_from_sample_frame};
use shrimply_project::project::{
    AUDIO_TRACK_GAIN_MAX_DB, AUDIO_TRACK_GAIN_MIN_DB, AudioClipTransitionCurve, AudioGenerator,
    AudioItem, AudioSource, AudioSpeedMethod, AudioWaveform, Project, RepeatStrategy,
    SequenceReference, Time, audio_source_time_at, fraction_as_f64, fraction_denominator,
    fraction_numerator, playback_speed_is_negative, playback_speed_is_zero,
    playback_speed_or_default, scaled_time_delta,
};

use super::CHANNELS;

mod analysis;

pub use analysis::{
    EXPRESSION_SAMPLE_RATE_HZ, FrameAudioSampler, FrameMouthSampler, FrameVolumeSampler,
};

const AV_TIME_BASE: u128 = 1_000_000;
const BUFFER_LOOKBEHIND_FRAMES: u64 = 48_000;
const OFFLINE_MIX_CHUNK_FRAMES: u64 = 48_000;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AudioSourceKey {
    item_id: uuid::Uuid,
    source: Option<AssetSnapshot>,
    track_id: u32,
}

pub struct AudioRenderSession {
    kind: AudioRenderSessionKind,
    sample_rate: u32,
}

enum AudioRenderSessionKind {
    Decoder(Box<AudioDecoderSession>),
    Generator(GeneratedAudioSession),
}

struct GeneratedAudioSession {
    sample_rate: u32,
    source_hash: u64,
    modifier_hash: u64,
    effects: super::effects::Processor,
    processed_until_frame: Option<u64>,
    effect_input_until_frame: Option<u64>,
    next_source_frame: Option<u64>,
    phase: f32,
}

struct AudioDecoderSession {
    source: AssetSnapshot,
    input: ffmpeg::format::context::Input,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    stream_start_time: i64,
    decoder: ffmpeg::decoder::Audio,
    resampler: ffmpeg::software::resampling::Context,
    sample_rate: u32,
    buffer_start_frame: u64,
    next_resampled_frame: Option<u64>,
    samples: VecDeque<f32>,
    eof: bool,
    modifier_hash: u64,
    effects: super::effects::Processor,
    processed_until_frame: Option<u64>,
    effect_input_until_frame: Option<u64>,
}

pub fn mix_project_range(
    project: &Project,
    sessions: &mut HashMap<AudioSourceKey, AudioRenderSession>,
    start_frame: u64,
    frame_count: usize,
    sample_rate: u32,
) -> Vec<f32> {
    mix_project_range_result(project, sessions, start_frame, frame_count, sample_rate)
        .unwrap_or_else(|error| {
            tracing::warn!("Could not mix project audio: {error}");
            vec![0.0; frame_count * CHANNELS]
        })
}

pub(crate) fn mix_project_range_result(
    project: &Project,
    sessions: &mut HashMap<AudioSourceKey, AudioRenderSession>,
    start_frame: u64,
    frame_count: usize,
    sample_rate: u32,
) -> Result<Vec<f32>, String> {
    let mut mixed = vec![0.0; frame_count * CHANNELS];
    for samples in
        mix_project_tracks_range_result(project, sessions, start_frame, frame_count, sample_rate)?
    {
        mix_samples(&mut mixed, 0, &samples);
    }
    Ok(mixed)
}

fn mix_project_tracks_range_result(
    project: &Project,
    sessions: &mut HashMap<AudioSourceKey, AudioRenderSession>,
    start_frame: u64,
    frame_count: usize,
    sample_rate: u32,
) -> Result<Vec<Vec<f32>>, String> {
    let mut tracks = vec![vec![0.0; frame_count * CHANNELS]; project.audio_tracks.len()];
    if frame_count == 0 {
        return Ok(tracks);
    }

    let end_frame = start_frame.saturating_add(frame_count as u64);
    for (track, mixed) in project.audio_tracks.iter().zip(&mut tracks) {
        mix_track_range_result(
            project,
            track,
            sessions,
            mixed,
            start_frame,
            end_frame,
            sample_rate,
            &mut Vec::new(),
        )?;
    }

    Ok(tracks)
}

fn mix_project_selected_range(
    project: &Project,
    sessions: &mut HashMap<AudioSourceKey, AudioRenderSession>,
    indices: &[usize],
    start_frame: u64,
    frame_count: usize,
    sample_rate: u32,
) -> Vec<f32> {
    let mut mixed = vec![0.0; frame_count * CHANNELS];
    let end_frame = start_frame.saturating_add(frame_count as u64);
    for &index in indices {
        let mut track_samples = vec![0.0; frame_count * CHANNELS];
        mix_track_range(
            project,
            &project.audio_tracks[index],
            sessions,
            &mut track_samples,
            start_frame,
            end_frame,
            sample_rate,
            &mut Vec::new(),
        );
        mix_samples(&mut mixed, 0, &track_samples);
    }
    mixed
}

#[allow(clippy::too_many_arguments)]
fn mix_track_range(
    project: &Project,
    track: &shrimply_project::project::AudioTrack,
    sessions: &mut HashMap<AudioSourceKey, AudioRenderSession>,
    mixed: &mut [f32],
    start_frame: u64,
    end_frame: u64,
    sample_rate: u32,
    sequence_stack: &mut Vec<uuid::Uuid>,
) {
    if let Err(error) = mix_track_range_result(
        project,
        track,
        sessions,
        mixed,
        start_frame,
        end_frame,
        sample_rate,
        sequence_stack,
    ) {
        tracing::warn!("Could not mix audio track: {error}");
    }
}

#[allow(clippy::too_many_arguments)]
fn mix_track_range_result(
    project: &Project,
    track: &shrimply_project::project::AudioTrack,
    sessions: &mut HashMap<AudioSourceKey, AudioRenderSession>,
    mixed: &mut [f32],
    start_frame: u64,
    end_frame: u64,
    sample_rate: u32,
    sequence_stack: &mut Vec<uuid::Uuid>,
) -> Result<(), String> {
    if !track.enabled {
        return Ok(());
    }
    for (item_index, item) in track.items.iter().enumerate() {
        if !item.enabled {
            continue;
        }
        let (incoming_clip_transition, outgoing_clip_transition) =
            audio_clip_transitions_for_item(&track.items, item_index);
        let effective_start = incoming_clip_transition.map_or(item.start, |transition| {
            item.start
                .saturating_sub(crate::math::clip_transition_half_duration(
                    transition.duration,
                ))
        });
        let effective_end = outgoing_clip_transition.map_or(item.end, |transition| {
            item.end
                .saturating_add(crate::math::clip_transition_half_duration(
                    transition.duration,
                ))
        });
        let item_start = effective_start.as_sample_frame(sample_rate);
        let item_end = effective_end.as_sample_frame(sample_rate);
        let overlap_start = start_frame.max(item_start);
        let overlap_end = end_frame.min(item_end);
        if overlap_start >= overlap_end {
            continue;
        }
        let mut extended_item = None;
        if (incoming_clip_transition.is_some() || outgoing_clip_transition.is_some())
            && (overlap_start < item.start.as_sample_frame(sample_rate)
                || overlap_end > item.end.as_sample_frame(sample_rate))
        {
            let mut value = item.clone();
            value.repeat_strategy = RepeatStrategy::Empty;
            if overlap_end > item.end.as_sample_frame(sample_rate) {
                value.end = effective_end;
            }
            extended_item = Some(value);
        }
        let render_item = extended_item.as_ref().unwrap_or(item);
        let cached_item = super::modifier_cache::effective_item(render_item)?;
        let render_item = cached_item.as_ref().unwrap_or(render_item);
        if let AudioSource::FoldedSequence(reference) = &render_item.source {
            let samples = mix_folded_sequence_item(
                project,
                render_item,
                *reference,
                sessions,
                overlap_start,
                overlap_end,
                sample_rate,
                sequence_stack,
            )?;
            let destination_frame = overlap_start.saturating_sub(start_frame) as usize;
            let mut samples = samples;
            for transition in [incoming_clip_transition, outgoing_clip_transition]
                .into_iter()
                .flatten()
            {
                apply_audio_clip_transition_gain(
                    &mut samples,
                    transition,
                    overlap_start,
                    sample_rate,
                );
            }
            mix_samples(mixed, destination_frame, &samples);
            continue;
        }
        if !matches!(render_item.source, AudioSource::Generator(_))
            && render_item.file.as_os_str().is_empty()
        {
            continue;
        }
        let key = AudioSourceKey::new(render_item);
        let session = match sessions.entry(key) {
            Entry::Occupied(entry) => {
                shrimply_benchmarking::increment("Volume decoder cache / Hit");
                entry.into_mut()
            }
            Entry::Vacant(entry) => {
                shrimply_benchmarking::increment("Volume decoder cache / Miss");
                let _measurement = shrimply_benchmarking::measure("Volume sampling / Open decoder");
                let session = AudioRenderSession::new(render_item, sample_rate)
                    .map_err(|error| format!("could not open audio item {}: {error}", item.id))?;
                entry.insert(session)
            }
        };

        let destination_frame = overlap_start.saturating_sub(start_frame) as usize;
        let needed_frames = overlap_end.saturating_sub(overlap_start) as usize;
        let rendered = {
            let _measurement = shrimply_benchmarking::measure("Volume sampling / Decode item");
            session.render_processed_item_frames(render_item, overlap_start, needed_frames)
        };
        let samples = rendered
            .map_err(|error| format!("could not render audio item {}: {error}", item.id))?;
        let mut samples = samples;
        for transition in [incoming_clip_transition, outgoing_clip_transition]
            .into_iter()
            .flatten()
        {
            apply_audio_clip_transition_gain(&mut samples, transition, overlap_start, sample_rate);
        }
        mix_samples(mixed, destination_frame, &samples);
    }
    let gain = super::effects::db_gain(
        track
            .gain_db
            .clamp(AUDIO_TRACK_GAIN_MIN_DB, AUDIO_TRACK_GAIN_MAX_DB),
    );
    mixed.iter_mut().for_each(|sample| *sample *= gain);
    Ok(())
}

#[derive(Clone, Copy)]
struct ActiveAudioClipTransition {
    cut: Time,
    duration: Time,
    curve: AudioClipTransitionCurve,
    incoming: bool,
}

fn audio_clip_transitions_for_item(
    items: &[AudioItem],
    item_index: usize,
) -> (
    Option<ActiveAudioClipTransition>,
    Option<ActiveAudioClipTransition>,
) {
    let item = &items[item_index];
    let outgoing_transition = if let (Some(transition), Some(incoming)) =
        (item.transitions.to_next.as_ref(), items.get(item_index + 1))
        && transition.target_item_id == incoming.id
        && item.end == incoming.start
    {
        Some(ActiveAudioClipTransition {
            cut: item.end,
            duration: transition.duration,
            curve: transition.curve,
            incoming: false,
        })
    } else {
        None
    };
    let incoming_transition = item_index
        .checked_sub(1)
        .and_then(|index| items.get(index))
        .and_then(|outgoing| {
            let transition = outgoing.transitions.to_next.as_ref()?;
            (transition.target_item_id == item.id && outgoing.end == item.start).then_some(
                ActiveAudioClipTransition {
                    cut: item.start,
                    duration: transition.duration,
                    curve: transition.curve,
                    incoming: true,
                },
            )
        });
    (incoming_transition, outgoing_transition)
}

fn apply_audio_clip_transition_gain(
    samples: &mut [f32],
    transition: ActiveAudioClipTransition,
    timeline_start_frame: u64,
    sample_rate: u32,
) {
    let half = crate::math::clip_transition_half_duration(transition.duration);
    let transition_start = transition
        .cut
        .saturating_sub(half)
        .as_sample_frame(sample_rate);
    let transition_frames = transition.duration.as_sample_frame(sample_rate).max(1);
    for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
        let timeline_frame = timeline_start_frame.saturating_add(frame as u64);
        if timeline_frame < transition_start {
            continue;
        }
        let progress = timeline_frame
            .saturating_sub(transition_start)
            .min(transition_frames) as f32
            / transition_frames as f32;
        let (outgoing, incoming) =
            crate::math::audio_clip_transition_gains(transition.curve, progress);
        let gain = if transition.incoming {
            incoming
        } else {
            outgoing
        };
        for sample in channels {
            *sample *= gain;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn mix_folded_sequence_item(
    project: &Project,
    item: &AudioItem,
    reference: SequenceReference,
    sessions: &mut HashMap<AudioSourceKey, AudioRenderSession>,
    start_frame: u64,
    end_frame: u64,
    sample_rate: u32,
    sequence_stack: &mut Vec<uuid::Uuid>,
) -> Result<Vec<f32>, String> {
    if sequence_stack.contains(&reference.sequence_id) {
        return Err(format!(
            "cyclic folded sequence reference involving {}",
            reference.sequence_id
        ));
    }
    let sequence = project
        .folded_sequence(reference.sequence_id)
        .ok_or_else(|| format!("missing folded sequence {}", reference.sequence_id))?;
    let source_frames = (start_frame..end_frame)
        .map(|frame| {
            audio_source_time_at(item, time_from_sample_frame(frame, sample_rate))
                .map(|time| time.as_sample_frame(sample_rate))
        })
        .collect::<Vec<_>>();
    let Some(source_start) = source_frames.iter().flatten().copied().min() else {
        return Ok(vec![
            0.0;
            end_frame.saturating_sub(start_frame) as usize
                * CHANNELS
        ]);
    };
    let source_end = source_frames
        .iter()
        .flatten()
        .copied()
        .max()
        .unwrap_or(source_start)
        .saturating_add(1);
    let source_count = source_end.saturating_sub(source_start) as usize;
    let mut nested = vec![0.0; source_count * CHANNELS];
    sequence_stack.push(reference.sequence_id);
    for track in &sequence.audio_tracks {
        let mut track_samples = vec![0.0; nested.len()];
        mix_track_range_result(
            project,
            track,
            sessions,
            &mut track_samples,
            source_start,
            source_end,
            sample_rate,
            sequence_stack,
        )?;
        mix_samples(&mut nested, 0, &track_samples);
    }
    sequence_stack.pop();
    let mut output = vec![0.0; source_frames.len() * CHANNELS];
    for (frame, source) in source_frames.into_iter().enumerate() {
        let Some(source) = source else {
            continue;
        };
        let source = source.saturating_sub(source_start) as usize * CHANNELS;
        if source + 1 < nested.len() {
            output[frame * CHANNELS] = nested[source];
            output[frame * CHANNELS + 1] = nested[source + 1];
        }
    }
    let local_start = time_from_sample_frame(start_frame, sample_rate).saturating_sub(item.start);
    super::effects::Processor::new(item, sample_rate)?.process(&mut output, item, local_start)?;
    apply_transition_gain(&mut output, item, start_frame, sample_rate);
    Ok(output)
}

pub fn mix_project_offline(
    project: &Project,
    sample_rate: u32,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<Vec<f32>, String> {
    let total_frames = project.duration().as_sample_frame(sample_rate);
    let mut mixed = vec![0.0; total_frames as usize * CHANNELS];
    let enabled_track_count = project
        .audio_tracks
        .iter()
        .filter(|track| track.enabled)
        .count() as u64;
    let total_work = total_frames.saturating_mul(enabled_track_count);
    let mut completed = 0_u64;
    let mut sessions = HashMap::new();
    if !progress(0, total_work) {
        return Err("Export cancelled".to_string());
    }
    for track in &project.audio_tracks {
        if !track.enabled {
            continue;
        }
        let mut timeline_start = 0_u64;
        while timeline_start < total_frames {
            let frames = total_frames
                .saturating_sub(timeline_start)
                .min(OFFLINE_MIX_CHUNK_FRAMES) as usize;
            let mut samples = vec![0.0; frames * CHANNELS];
            mix_track_range_result(
                project,
                track,
                &mut sessions,
                &mut samples,
                timeline_start,
                timeline_start.saturating_add(frames as u64),
                sample_rate,
                &mut Vec::new(),
            )?;
            mix_samples(&mut mixed, timeline_start as usize, &samples);
            timeline_start = timeline_start.saturating_add(frames as u64);
            completed = completed.saturating_add(frames as u64);
            if !progress(completed, total_work) {
                return Err("Export cancelled".to_string());
            }
        }
    }
    Ok(mixed)
}

pub struct OfflineAudioRenderer {
    session: AudioRenderSession,
    sample_rate: u32,
    cached_item: Option<AudioItem>,
}

impl OfflineAudioRenderer {
    pub fn new(item: &AudioItem, sample_rate: u32) -> Result<Self, String> {
        let cached_item = super::modifier_cache::effective_item(item)?;
        let render_item = cached_item.as_ref().unwrap_or(item);
        Ok(Self {
            session: AudioRenderSession::new(render_item, sample_rate)?,
            sample_rate,
            cached_item,
        })
    }

    pub fn render(
        &mut self,
        item: &AudioItem,
        start: Time,
        duration: Time,
    ) -> Result<Vec<f32>, String> {
        let item = self.cached_item.as_ref().unwrap_or(item);
        let timeline_start = item
            .start
            .saturating_add(start)
            .as_sample_frame(self.sample_rate);
        self.session.render_processed_item_frames(
            item,
            timeline_start,
            duration.as_sample_frame(self.sample_rate) as usize,
        )
    }
}

pub fn retain_project_sessions(
    project: &Project,
    sessions: &mut HashMap<AudioSourceKey, AudioRenderSession>,
) {
    let active: HashSet<_> = project
        .audio_tracks
        .iter()
        .flat_map(|track| track.items.iter().map(AudioSourceKey::new))
        .collect();
    sessions.retain(|key, _| active.contains(key));
}

impl AudioSourceKey {
    fn new(item: &AudioItem) -> Self {
        Self {
            item_id: item.id,
            source: if item.uses_file_asset() {
                item.file.snapshot().ok()
            } else {
                None
            },
            track_id: item.track_id,
        }
    }
}

fn modifier_hash(item: &AudioItem) -> u64 {
    let mut hasher = DefaultHasher::new();
    serde_json::to_vec(&(&item.gain, &item.modifiers))
        .expect("audio gain and modifiers must serialize")
        .hash(&mut hasher);
    hasher.finish()
}

fn generator_hash(generator: &AudioGenerator) -> u64 {
    let mut hasher = DefaultHasher::new();
    serde_json::to_vec(generator)
        .expect("audio generator settings must serialize")
        .hash(&mut hasher);
    hasher.finish()
}

impl AudioRenderSession {
    fn new(item: &AudioItem, sample_rate: u32) -> Result<Self, String> {
        let kind = match &item.source {
            AudioSource::Generator(generator) => AudioRenderSessionKind::Generator(
                GeneratedAudioSession::new(item, generator, sample_rate)?,
            ),
            AudioSource::Media | AudioSource::Tts(_) => AudioRenderSessionKind::Decoder(Box::new(
                AudioDecoderSession::new(item, sample_rate)?,
            )),
            AudioSource::FoldedSequence(_) => {
                return Err("folded sequences are mixed recursively".to_string());
            }
        };
        Ok(Self { kind, sample_rate })
    }

    fn render_processed_item_frames(
        &mut self,
        item: &AudioItem,
        timeline_start_frame: u64,
        frame_count: usize,
    ) -> Result<Vec<f32>, String> {
        let compatible = matches!(
            (&self.kind, &item.source),
            (
                AudioRenderSessionKind::Generator(_),
                AudioSource::Generator(_)
            ) | (
                AudioRenderSessionKind::Decoder(_),
                AudioSource::Media | AudioSource::Tts(_)
            )
        );
        if !compatible {
            *self = Self::new(item, self.sample_rate)?;
        }
        match &mut self.kind {
            AudioRenderSessionKind::Decoder(session) => {
                session.decode_processed_item_frames(item, timeline_start_frame, frame_count)
            }
            AudioRenderSessionKind::Generator(session) => {
                session.render_processed_item_frames(item, timeline_start_frame, frame_count)
            }
        }
    }
}

impl GeneratedAudioSession {
    fn new(item: &AudioItem, generator: &AudioGenerator, sample_rate: u32) -> Result<Self, String> {
        Ok(Self {
            sample_rate,
            source_hash: generator_hash(generator),
            modifier_hash: modifier_hash(item),
            effects: super::effects::Processor::new(item, sample_rate)?,
            processed_until_frame: None,
            effect_input_until_frame: None,
            next_source_frame: None,
            phase: 0.0,
        })
    }

    fn render_processed_item_frames(
        &mut self,
        item: &AudioItem,
        timeline_start_frame: u64,
        frame_count: usize,
    ) -> Result<Vec<f32>, String> {
        let AudioSource::Generator(generator) = &item.source else {
            return Err("audio render session source changed unexpectedly".to_string());
        };
        let source_hash = generator_hash(generator);
        let modifier_hash = modifier_hash(item);
        let discontinuous = self.source_hash != source_hash
            || self.modifier_hash != modifier_hash
            || self.processed_until_frame != Some(timeline_start_frame);
        if discontinuous {
            self.effects = super::effects::Processor::new(item, self.sample_rate)?;
            self.source_hash = source_hash;
            self.modifier_hash = modifier_hash;
            self.next_source_frame = None;
        }
        let latency = self.effects.latency_frames();
        let item_start = item.start.as_sample_frame(self.sample_rate);
        let warmup = if discontinuous {
            self.effects
                .warmup_frames()
                .min(timeline_start_frame.saturating_sub(item_start) as usize)
        } else {
            0
        };
        let input_start = if discontinuous {
            timeline_start_frame.saturating_sub(warmup as u64)
        } else {
            self.effect_input_until_frame
                .unwrap_or(timeline_start_frame)
        };
        let input_frames = frame_count
            .saturating_add(warmup)
            .saturating_add(if discontinuous { latency } else { 0 });
        let mut samples = self.generate_frames(generator, item, input_start, input_frames);
        let local_start =
            time_from_sample_frame(input_start, self.sample_rate).saturating_sub(item.start);
        self.effects.process(&mut samples, item, local_start)?;
        apply_transition_gain(&mut samples, item, input_start, self.sample_rate);
        let discard = warmup.saturating_add(if discontinuous { latency } else { 0 });
        if discard > 0 {
            samples.drain(..discard.saturating_mul(CHANNELS).min(samples.len()));
        }
        samples.truncate(frame_count * CHANNELS);
        samples.resize(frame_count * CHANNELS, 0.0);
        self.processed_until_frame = Some(timeline_start_frame.saturating_add(frame_count as u64));
        self.effect_input_until_frame = Some(input_start.saturating_add(input_frames as u64));
        Ok(samples)
    }

    fn generate_frames(
        &mut self,
        generator: &AudioGenerator,
        item: &AudioItem,
        start_frame: u64,
        frame_count: usize,
    ) -> Vec<f32> {
        const MIN_FREQUENCY_HZ: f32 = 1.0;
        const NYQUIST_SAFETY_RATIO: f32 = 0.45;

        if self.next_source_frame != Some(start_frame) {
            self.phase = 0.0;
        }
        let item_start = item.start.as_sample_frame(self.sample_rate);
        let mut output = Vec::with_capacity(frame_count.saturating_mul(CHANNELS));
        for frame in 0..frame_count {
            let timeline_frame = start_frame.saturating_add(frame as u64);
            let local_frame = (i128::from(timeline_frame) - i128::from(item_start))
                .clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                as i64;
            let local_time = Time::from_fraction(local_frame, i64::from(self.sample_rate.max(1)));
            let frequency = generator.frequency_hz.value_at(local_time).clamp(
                MIN_FREQUENCY_HZ,
                self.sample_rate as f32 * NYQUIST_SAFETY_RATIO,
            );
            let phase_step = frequency / self.sample_rate.max(1) as f32;
            let sample = match generator.waveform {
                AudioWaveform::Sine => crate::math::audio_sine_sample(self.phase),
                AudioWaveform::SquarePulse => crate::math::audio_square_pulse_sample(
                    self.phase,
                    phase_step,
                    generator.pulse_width.value_at(local_time),
                ),
                AudioWaveform::Triangle => crate::math::audio_triangle_sample(self.phase),
                AudioWaveform::Sawtooth => {
                    crate::math::audio_sawtooth_sample(self.phase, phase_step)
                }
                AudioWaveform::WhiteNoise => crate::math::audio_white_noise_sample(
                    generator.seed.value_at(local_time).round().max(0.0) as u32,
                    local_frame,
                ),
                AudioWaveform::PinkNoise => crate::math::audio_pink_noise_sample(
                    generator.seed.value_at(local_time).round().max(0.0) as u32,
                    local_frame,
                ),
                AudioWaveform::BrownNoise => crate::math::audio_brown_noise_sample(
                    generator.seed.value_at(local_time).round().max(0.0) as u32,
                    local_frame,
                ),
            };
            self.phase = (self.phase + phase_step).fract();
            output.extend([sample, sample]);
        }
        self.next_source_frame = Some(start_frame.saturating_add(frame_count as u64));
        output
    }
}

impl AudioDecoderSession {
    fn new(item: &AudioItem, sample_rate: u32) -> Result<Self, String> {
        ffmpeg::init().map_err(|error| error.to_string())?;
        let source = item.file.snapshot()?;
        let pneuma_source = super::pneuma::source(item, &source)?;
        let input_path = pneuma_source.as_deref().unwrap_or_else(|| source.path());
        let input = ffmpeg::format::input(input_path).map_err(|error| error.to_string())?;
        let track_id = if pneuma_source.is_some() {
            0
        } else {
            item.track_id
        };
        let (stream_index, stream_time_base, stream_start_time, parameters) = {
            let stream = input
                .streams()
                .filter(|stream| stream.parameters().medium() == ffmpeg::media::Type::Audio)
                .nth(track_id as usize)
                .ok_or_else(|| format!("audio stream {track_id} not found"))?;
            (
                stream.index(),
                stream.time_base(),
                stream.start_time(),
                stream.parameters(),
            )
        };
        let context = ffmpeg::codec::context::Context::from_parameters(parameters)
            .map_err(|error| error.to_string())?;
        let mut decoder = context
            .decoder()
            .audio()
            .map_err(|error| error.to_string())?;
        if decoder.channel_layout().is_empty() {
            decoder.set_channel_layout(ffmpeg::channel_layout::ChannelLayout::default(
                decoder.channels() as i32,
            ));
        }
        let resampler = decoder
            .resampler(
                ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar),
                ffmpeg::channel_layout::ChannelLayout::STEREO,
                sample_rate,
            )
            .map_err(|error| error.to_string())?;

        Ok(Self {
            source,
            input,
            stream_index,
            stream_time_base,
            stream_start_time,
            decoder,
            resampler,
            sample_rate,
            buffer_start_frame: 0,
            next_resampled_frame: None,
            samples: VecDeque::new(),
            eof: false,
            modifier_hash: modifier_hash(item),
            effects: super::effects::Processor::new(item, sample_rate)?,
            processed_until_frame: None,
            effect_input_until_frame: None,
        })
    }

    fn decode_processed_item_frames(
        &mut self,
        item: &AudioItem,
        timeline_start_frame: u64,
        frame_count: usize,
    ) -> Result<Vec<f32>, String> {
        self.source.ensure_current()?;
        let modifier_hash = modifier_hash(item);
        let modifiers_changed = self.modifier_hash != modifier_hash;
        let discontinuous =
            modifiers_changed || self.processed_until_frame != Some(timeline_start_frame);
        if discontinuous {
            shrimply_benchmarking::increment("Volume sampling / Discontinuity");
            let next_effects = {
                let _measurement =
                    shrimply_benchmarking::measure("Volume sampling / Reinitialize effects");
                super::effects::Processor::new(item, self.sample_rate)?
            };
            self.effects = next_effects;
            self.modifier_hash = modifier_hash;
        } else {
            shrimply_benchmarking::increment("Volume sampling / Continuous");
        }
        let latency = self.effects.latency_frames();
        let item_start = item.start.as_sample_frame(self.sample_rate);
        let warmup = if discontinuous {
            self.effects
                .warmup_frames()
                .min(timeline_start_frame.saturating_sub(item_start) as usize)
        } else {
            0
        };
        let input_start = if discontinuous {
            timeline_start_frame.saturating_sub(warmup as u64)
        } else {
            self.effect_input_until_frame
                .unwrap_or(timeline_start_frame)
        };
        let input_frames = frame_count
            .saturating_add(warmup)
            .saturating_add(if discontinuous { latency } else { 0 });
        let mut samples = {
            let _measurement = shrimply_benchmarking::measure("Volume sampling / Decode samples");
            self.decode_item_frames_padded(item, input_start, input_frames)?
        };
        let local_start =
            time_from_sample_frame(input_start, self.sample_rate).saturating_sub(item.start);
        {
            let _measurement = shrimply_benchmarking::measure("Volume sampling / Apply effects");
            self.effects.process(&mut samples, item, local_start)?;
        }
        apply_transition_gain(&mut samples, item, input_start, self.sample_rate);
        let discard = warmup.saturating_add(if discontinuous { latency } else { 0 });
        if discard > 0 {
            samples.drain(..discard.saturating_mul(CHANNELS).min(samples.len()));
        }
        samples.truncate(frame_count * CHANNELS);
        samples.resize(frame_count * CHANNELS, 0.0);
        self.processed_until_frame = Some(timeline_start_frame.saturating_add(frame_count as u64));
        self.effect_input_until_frame = Some(input_start.saturating_add(input_frames as u64));
        Ok(samples)
    }

    fn decode_item_frames_padded(
        &mut self,
        item: &AudioItem,
        timeline_start_frame: u64,
        frame_count: usize,
    ) -> Result<Vec<f32>, String> {
        let item_end = item.end.as_sample_frame(self.sample_rate);
        let available = item_end.saturating_sub(timeline_start_frame) as usize;
        let decoded = frame_count.min(available);
        let mut samples = if decoded == 0 {
            Vec::new()
        } else {
            self.decode_item_frames(item, timeline_start_frame, decoded)?
        };
        samples.resize(frame_count * CHANNELS, 0.0);
        Ok(samples)
    }

    fn decode_item_frames(
        &mut self,
        item: &AudioItem,
        timeline_start_frame: u64,
        frame_count: usize,
    ) -> Result<Vec<f32>, String> {
        if playback_speed_is_zero(item.playback_speed) {
            return Ok(vec![0.0; frame_count * CHANNELS]);
        }
        let playback_speed = playback_speed_or_default(item.playback_speed);
        let timeline_start = time_from_sample_frame(timeline_start_frame, self.sample_rate);
        let timeline_end = time_from_sample_frame(
            timeline_start_frame.saturating_add(frame_count as u64),
            self.sample_rate,
        );
        if !range_stays_inside_source(item, timeline_start, timeline_end) {
            return self.decode_repeated_item_frames(item, timeline_start_frame, frame_count);
        }

        let Some(source_start_time) = audio_source_time_at(item, timeline_start) else {
            return Ok(vec![0.0; frame_count * CHANNELS]);
        };
        let Some(source_end_time) = audio_source_time_at(item, timeline_end) else {
            return Ok(vec![0.0; frame_count * CHANNELS]);
        };
        let source_start = source_start_time.as_sample_frame(self.sample_rate);
        let source_end = source_end_time.as_sample_frame(self.sample_rate);
        let backwards = playback_speed_is_negative(playback_speed);
        let (decode_start, source_frames) = if backwards {
            let decode_start = source_end.saturating_sub(2);
            (
                decode_start,
                source_start
                    .saturating_sub(decode_start)
                    .saturating_add(1)
                    .max(1) as usize,
            )
        } else {
            (
                source_start,
                source_end
                    .saturating_add(2)
                    .saturating_sub(source_start)
                    .max(1) as usize,
            )
        };
        let mut samples = self.decode_frames(decode_start, source_frames)?;
        if backwards {
            samples.reverse();
            for frame in samples.chunks_exact_mut(CHANNELS) {
                frame.swap(0, 1);
            }
        }
        let processing_speed = if backwards {
            -playback_speed
        } else {
            playback_speed
        };
        if is_default_speed(processing_speed) {
            return Ok(trim_or_pad_samples(samples, frame_count));
        }

        match item.speed_method {
            AudioSpeedMethod::Naive => Ok(naive_speed_samples(
                &samples,
                processing_speed,
                frame_count,
            )),
            AudioSpeedMethod::PreservePitch => tempo_adjust_samples(
                &samples,
                processing_speed,
                self.sample_rate,
                frame_count,
            )
            .or_else(|error| {
                tracing::warn!(
                    "Could not preserve audio pitch for {} audio {}: {error}; falling back to naive speed",
                    item.file.display(),
                    item.track_id
                );
                Ok(naive_speed_samples(
                    &samples,
                    processing_speed,
                    frame_count,
                ))
            }),
        }
    }

    fn decode_frames(&mut self, start_frame: u64, frame_count: usize) -> Result<Vec<f32>, String> {
        if frame_count == 0 {
            return Ok(Vec::new());
        }
        if !self.can_read_or_decode_forward(start_frame) {
            self.seek(start_frame)?;
        }

        let end_frame = start_frame.saturating_add(frame_count as u64);
        self.decode_until(end_frame)?;

        let mut output = vec![0.0; frame_count * CHANNELS];
        let buffer_end = self.buffer_end_frame();
        for frame in start_frame..end_frame.min(buffer_end) {
            if frame < self.buffer_start_frame {
                continue;
            }
            let source = frame.saturating_sub(self.buffer_start_frame) as usize * CHANNELS;
            let destination = frame.saturating_sub(start_frame) as usize * CHANNELS;
            output[destination] = self.samples.get(source).copied().unwrap_or(0.0);
            output[destination + 1] = self.samples.get(source + 1).copied().unwrap_or(0.0);
        }

        self.trim_before(start_frame.saturating_sub(BUFFER_LOOKBEHIND_FRAMES));
        Ok(output)
    }

    fn decode_repeated_item_frames(
        &mut self,
        item: &AudioItem,
        timeline_start_frame: u64,
        frame_count: usize,
    ) -> Result<Vec<f32>, String> {
        if item.speed_method == AudioSpeedMethod::PreservePitch {
            let playback_speed = playback_speed_or_default(item.playback_speed);
            let speed_numerator = fraction_numerator(playback_speed).unsigned_abs().max(1);
            let speed_denominator = fraction_denominator(playback_speed).unsigned_abs().max(1);
            let source_frames = ((frame_count as u128 * u128::from(speed_numerator))
                .div_ceil(u128::from(speed_denominator)))
            .min(usize::MAX as u128) as usize;
            let timeline_start = time_from_sample_frame(timeline_start_frame, self.sample_rate);
            let mut samples = vec![0.0; source_frames * CHANNELS];
            let source_end_frame = item.source_duration.as_sample_frame(self.sample_rate);
            let mut cached_source = None;
            let mut cached_samples = [0.0; CHANNELS];
            for frame in 0..source_frames {
                let delta_numerator = (frame as u64).saturating_mul(speed_denominator);
                let delta_denominator = u64::from(self.sample_rate).saturating_mul(speed_numerator);
                let timeline = timeline_start.saturating_add(Time::from_fraction(
                    delta_numerator.min(i64::MAX as u64) as i64,
                    delta_denominator.min(i64::MAX as u64) as i64,
                ));
                let Some(source_time) = audio_source_time_at(item, timeline) else {
                    continue;
                };
                if source_end_frame == 0 {
                    continue;
                }
                let source_frame = source_time
                    .as_sample_frame(self.sample_rate)
                    .min(source_end_frame.saturating_sub(1));
                if cached_source != Some(source_frame) {
                    let frame_samples = self.decode_frames(source_frame, 1)?;
                    cached_samples[0] = frame_samples.first().copied().unwrap_or(0.0);
                    cached_samples[1] = frame_samples.get(1).copied().unwrap_or(0.0);
                    cached_source = Some(source_frame);
                }
                let destination = frame * CHANNELS;
                samples[destination..destination + CHANNELS].copy_from_slice(&cached_samples);
            }
            let processing_speed = if playback_speed_is_negative(playback_speed) {
                -playback_speed
            } else {
                playback_speed
            };
            return tempo_adjust_samples(
                &samples,
                processing_speed,
                self.sample_rate,
                frame_count,
            )
            .or_else(|error| {
                tracing::warn!(
                    "Could not preserve repeated audio pitch for {} audio {}: {error}; falling back to naive speed",
                    item.file.display(),
                    item.track_id
                );
                self.decode_naive_repeated_item_frames(item, timeline_start_frame, frame_count)
            });
        }
        self.decode_naive_repeated_item_frames(item, timeline_start_frame, frame_count)
    }

    fn decode_naive_repeated_item_frames(
        &mut self,
        item: &AudioItem,
        timeline_start_frame: u64,
        frame_count: usize,
    ) -> Result<Vec<f32>, String> {
        let mut output = vec![0.0; frame_count * CHANNELS];
        if frame_count == 0 {
            return Ok(output);
        }
        let source_end_frame = item.source_duration.as_sample_frame(self.sample_rate);
        let mut cached_source = None;
        let mut cached_samples = [0.0; CHANNELS];
        for frame in 0..frame_count {
            let timeline = time_from_sample_frame(
                timeline_start_frame.saturating_add(frame as u64),
                self.sample_rate,
            );
            let Some(source_time) = audio_source_time_at(item, timeline) else {
                continue;
            };
            if source_end_frame == 0 {
                continue;
            }
            let source_frame = source_time
                .as_sample_frame(self.sample_rate)
                .min(source_end_frame.saturating_sub(1));
            if cached_source != Some(source_frame) {
                let samples = self.decode_frames(source_frame, 1)?;
                cached_samples[0] = samples.first().copied().unwrap_or(0.0);
                cached_samples[1] = samples.get(1).copied().unwrap_or(0.0);
                cached_source = Some(source_frame);
            }
            let destination = frame * CHANNELS;
            output[destination..destination + CHANNELS].copy_from_slice(&cached_samples);
        }
        Ok(output)
    }

    fn seek(&mut self, frame: u64) -> Result<(), String> {
        shrimply_benchmarking::increment("Volume decoder / Seek");
        let _measurement = shrimply_benchmarking::measure("Volume sampling / Seek");
        assert!(self.sample_rate > 0, "decoder sample rate must be positive");
        let target_timestamp = (u128::from(frame) * AV_TIME_BASE / u128::from(self.sample_rate))
            .min(i64::MAX as u128) as i64;
        self.input
            .seek(target_timestamp, ..target_timestamp)
            .map_err(|error| error.to_string())?;
        self.decoder.flush();
        self.resampler = self
            .decoder
            .resampler(
                ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar),
                ffmpeg::channel_layout::ChannelLayout::STEREO,
                self.sample_rate,
            )
            .map_err(|error| error.to_string())?;
        self.buffer_start_frame = frame;
        self.next_resampled_frame = None;
        self.samples.clear();
        self.eof = false;
        Ok(())
    }

    fn can_read_or_decode_forward(&self, frame: u64) -> bool {
        let buffer_end = self.buffer_end_frame();
        !self.samples.is_empty() && frame >= self.buffer_start_frame && frame <= buffer_end
    }

    fn decode_until(&mut self, end_frame: u64) -> Result<(), String> {
        let _measurement = shrimply_benchmarking::measure("Audio decode / Read packets");
        while self.buffer_end_frame() < end_frame && !self.eof {
            if self.receive_available()? > 0 {
                continue;
            }

            let packet = {
                let mut packets = self.input.packets();
                packets.next()
            };
            let Some((stream, packet)) = packet else {
                self.decoder.send_eof().map_err(|error| error.to_string())?;
                self.receive_available()?;
                self.flush_resampler()?;
                self.eof = true;
                break;
            };
            if stream.index() == self.stream_index {
                self.decoder
                    .send_packet(&packet)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    fn receive_available(&mut self) -> Result<usize, String> {
        let _measurement = shrimply_benchmarking::measure("Audio decode / Receive frames");
        let mut received_frames = 0;
        let mut decoded = ffmpeg::frame::Audio::empty();
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            if decoded.channel_layout().is_empty() {
                decoded.set_channel_layout(ffmpeg::channel_layout::ChannelLayout::default(
                    decoded.channels() as i32,
                ));
            }
            let timestamp_frame = self.decoded_start_frame(&decoded);
            let mut resampled = ffmpeg::frame::Audio::empty();
            self.resampler
                .run(&decoded, &mut resampled)
                .map_err(|error| error.to_string())?;
            // Timestamps anchor a seek; continuous resampler output advances by sample count.
            let start_frame = self
                .next_resampled_frame
                .or(timestamp_frame)
                .unwrap_or_else(|| self.buffer_end_frame());
            let frame_count = self.append_samples(start_frame, &resampled);
            if frame_count > 0 {
                self.next_resampled_frame = Some(start_frame.saturating_add(frame_count as u64));
                received_frames += frame_count;
            }
        }
        Ok(received_frames)
    }

    fn flush_resampler(&mut self) -> Result<(), String> {
        while let Some(delay_before) = self.resampler.delay() {
            if delay_before.output <= 0 {
                break;
            }

            let mut resampled = ffmpeg::frame::Audio::new(
                ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar),
                delay_before.output as usize,
                ffmpeg::channel_layout::ChannelLayout::STEREO,
            );
            let delay = self
                .resampler
                .flush(&mut resampled)
                .map_err(|error| error.to_string())?;
            let start_frame = self
                .next_resampled_frame
                .unwrap_or_else(|| self.buffer_end_frame());
            let frame_count = self.append_samples(start_frame, &resampled);
            if frame_count > 0 {
                self.next_resampled_frame = Some(start_frame.saturating_add(frame_count as u64));
            }
            match delay {
                Some(remaining) if remaining.output < delay_before.output => {}
                Some(remaining) => {
                    tracing::debug!(
                        before = delay_before.output,
                        after = remaining.output,
                        "Audio decoder resampler retained a non-draining filter delay"
                    );
                    break;
                }
                None => break,
            }
        }
        Ok(())
    }

    fn append_samples(&mut self, start_frame: u64, frame: &ffmpeg::frame::Audio) -> usize {
        let mut samples = Vec::new();
        append_frame_samples(frame, &mut samples);
        if samples.is_empty() {
            return 0;
        }
        let frame_count = samples.len() / CHANNELS;
        if self.samples.is_empty() {
            self.buffer_start_frame = start_frame;
            self.samples.extend(samples);
            return frame_count;
        }

        let buffer_end = self.buffer_end_frame();
        if start_frame > buffer_end {
            self.samples.extend(std::iter::repeat_n(
                0.0,
                start_frame.saturating_sub(buffer_end) as usize * CHANNELS,
            ));
            self.samples.extend(samples);
            return frame_count;
        }

        let skip_frames = buffer_end
            .saturating_sub(start_frame)
            .min((samples.len() / CHANNELS) as u64) as usize;
        self.samples
            .extend(samples.into_iter().skip(skip_frames * CHANNELS));
        frame_count
    }

    fn buffer_end_frame(&self) -> u64 {
        self.buffer_start_frame
            .saturating_add((self.samples.len() / CHANNELS) as u64)
    }

    fn trim_before(&mut self, frame: u64) {
        if frame <= self.buffer_start_frame {
            return;
        }
        let drain_frames = frame
            .saturating_sub(self.buffer_start_frame)
            .min((self.samples.len() / CHANNELS) as u64) as usize;
        self.samples.drain(..drain_frames * CHANNELS);
        self.buffer_start_frame = self.buffer_start_frame.saturating_add(drain_frames as u64);
    }

    fn decoded_start_frame(&self, frame: &ffmpeg::frame::Audio) -> Option<u64> {
        let timestamp = frame.timestamp()?;
        let start_time = self.stream_start_time.max(0);
        let timestamp = timestamp.saturating_sub(start_time).max(0) as u128;
        let time_numerator = u128::try_from(self.stream_time_base.numerator()).ok()?;
        let time_denominator = u128::try_from(self.stream_time_base.denominator()).ok()?;
        if time_denominator == 0 {
            return None;
        }
        let numerator = timestamp
            .checked_mul(time_numerator)?
            .checked_mul(u128::from(self.sample_rate))?;
        u64::try_from(
            numerator
                .saturating_add(time_denominator / 2)
                .checked_div(time_denominator)?,
        )
        .ok()
    }
}

fn mix_samples(mixed: &mut [f32], destination_frame: usize, samples: &[f32]) {
    let destination = destination_frame * CHANNELS;
    let frame_count =
        (mixed.len().saturating_sub(destination) / CHANNELS).min(samples.len() / CHANNELS);
    for frame in 0..frame_count {
        let destination = destination + frame * CHANNELS;
        let source = frame * CHANNELS;
        mixed[destination] += samples[source];
        mixed[destination + 1] += samples[source + 1];
    }
}

fn apply_transition_gain(
    samples: &mut [f32],
    item: &AudioItem,
    timeline_start_frame: u64,
    sample_rate: u32,
) {
    for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
        let position = time_from_sample_frame(
            timeline_start_frame.saturating_add(frame as u64),
            sample_rate,
        );
        let gain = item.transition_gain(position);
        for sample in channels {
            *sample *= gain;
        }
    }
}

pub fn pitch_preserving_speed(
    samples: &[f32],
    playback_speed: Fraction,
    sample_rate: u32,
    output_frames: usize,
) -> Result<Vec<f32>, String> {
    tempo_adjust_samples(samples, playback_speed, sample_rate, output_frames)
}

fn is_default_speed(playback_speed: Fraction) -> bool {
    let playback_speed = playback_speed_or_default(playback_speed);
    fraction_numerator(playback_speed) == 1 && fraction_denominator(playback_speed) == 1
}

fn range_stays_inside_source(item: &AudioItem, timeline_start: Time, timeline_end: Time) -> bool {
    let start = item.time_offset.saturating_add(scaled_time_delta(
        timeline_start.saturating_sub(item.start),
        item.playback_speed,
    ));
    let end = item.time_offset.saturating_add(scaled_time_delta(
        timeline_end.saturating_sub(item.start),
        item.playback_speed,
    ));
    start >= Time::ZERO
        && start <= item.source_duration
        && end >= Time::ZERO
        && end <= item.source_duration
        && if playback_speed_is_negative(item.playback_speed) {
            start >= end
        } else {
            start <= end
        }
}

fn trim_or_pad_samples(mut samples: Vec<f32>, output_frames: usize) -> Vec<f32> {
    let output_len = output_frames * CHANNELS;
    samples.truncate(output_len);
    samples.resize(output_len, 0.0);
    samples
}

fn naive_speed_samples(
    samples: &[f32],
    playback_speed: Fraction,
    output_frames: usize,
) -> Vec<f32> {
    let mut output = vec![0.0; output_frames * CHANNELS];
    let input_frames = samples.len() / CHANNELS;
    if input_frames == 0 || output_frames == 0 {
        return output;
    }

    let speed = playback_speed_or_default(playback_speed);
    for frame in 0..output_frames {
        let source = Fraction::from(frame as u64) * speed;
        let left = fraction_floor_i64(source)
            .expect("audio source sample exceeds the exact range")
            .max(0) as usize;
        let right = left.saturating_add(1).min(input_frames - 1);
        let mix = fraction_as_f64(source - Fraction::from(left as u64)).clamp(0.0, 1.0) as f32;
        let destination = frame * CHANNELS;
        let left_source = left.min(input_frames - 1) * CHANNELS;
        let right_source = right * CHANNELS;
        output[destination] =
            samples[left_source] + (samples[right_source] - samples[left_source]) * mix;
        output[destination + 1] =
            samples[left_source + 1] + (samples[right_source + 1] - samples[left_source + 1]) * mix;
    }
    output
}

fn tempo_adjust_samples(
    samples: &[f32],
    playback_speed: Fraction,
    sample_rate: u32,
    output_frames: usize,
) -> Result<Vec<f32>, String> {
    let _measurement = shrimply_benchmarking::measure("Audio decode / Preserve pitch");
    let input_frames = samples.len() / CHANNELS;
    if input_frames == 0 || output_frames == 0 {
        return Ok(vec![0.0; output_frames * CHANNELS]);
    }
    if is_default_speed(playback_speed) {
        return Ok(trim_or_pad_samples(samples.to_vec(), output_frames));
    }

    let mut graph = ffmpeg::filter::Graph::new();
    let format = ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar);
    let layout = ffmpeg::channel_layout::ChannelLayout::STEREO;
    let args = format!(
        "time_base=1/{sample_rate}:sample_rate={sample_rate}:sample_fmt={}:channel_layout=0x{:x}",
        format.name(),
        layout.bits()
    );
    graph
        .add(
            &ffmpeg::filter::find("abuffer").ok_or("FFmpeg abuffer filter not found")?,
            "in",
            &args,
        )
        .map_err(|error| error.to_string())?;
    graph
        .add(
            &ffmpeg::filter::find("abuffersink").ok_or("FFmpeg abuffersink filter not found")?,
            "out",
            "",
        )
        .map_err(|error| error.to_string())?;

    let filter_spec = format!(
        "{},aformat=sample_fmts={}:sample_rates={sample_rate}:channel_layouts=stereo",
        atempo_spec(playback_speed),
        format.name()
    );
    graph
        .output("in", 0)
        .map_err(|error| error.to_string())?
        .input("out", 0)
        .map_err(|error| error.to_string())?
        .parse(&filter_spec)
        .map_err(|error| error.to_string())?;
    graph.validate().map_err(|error| error.to_string())?;

    let mut input = ffmpeg::frame::Audio::new(format, input_frames, layout);
    input.set_rate(sample_rate);
    input.set_pts(Some(0));
    {
        let left = input.plane_mut::<f32>(0);
        for frame in 0..input_frames {
            left[frame] = samples[frame * CHANNELS];
        }
    }
    {
        let right = input.plane_mut::<f32>(1);
        for frame in 0..input_frames {
            right[frame] = samples[frame * CHANNELS + 1];
        }
    }

    graph
        .get("in")
        .ok_or("FFmpeg audio source not found")?
        .source()
        .add(&input)
        .map_err(|error| error.to_string())?;
    graph
        .get("in")
        .ok_or("FFmpeg audio source not found")?
        .source()
        .flush()
        .map_err(|error| error.to_string())?;

    let mut output = Vec::new();
    loop {
        let mut frame = ffmpeg::frame::Audio::empty();
        match graph
            .get("out")
            .ok_or("FFmpeg audio sink not found")?
            .sink()
            .frame(&mut frame)
        {
            Ok(()) => append_frame_samples(&frame, &mut output),
            Err(ffmpeg::Error::Eof) => break,
            Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => break,
            Err(error) => return Err(error.to_string()),
        }
    }

    Ok(trim_or_pad_samples(output, output_frames))
}

fn atempo_spec(playback_speed: Fraction) -> String {
    let mut speed = fraction_as_f64(playback_speed_or_default(playback_speed)).max(f64::EPSILON);
    let mut parts = Vec::new();
    while speed > 2.0 {
        parts.push("atempo=2".to_string());
        speed /= 2.0;
    }
    while speed < 0.5 {
        parts.push("atempo=0.5".to_string());
        speed /= 0.5;
    }
    parts.push(format!("atempo={speed:.6}"));
    parts.join(",")
}

fn append_frame_samples(frame: &ffmpeg::frame::Audio, samples: &mut Vec<f32>) {
    if frame.format() != ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar) {
        tracing::warn!(
            "Skipping resampled audio frame with unexpected format {:?}",
            frame.format()
        );
        return;
    }
    if frame.planes() < CHANNELS {
        tracing::warn!(
            "Skipping resampled audio frame with {} plane(s), expected {CHANNELS}",
            frame.planes()
        );
        return;
    }

    let left = frame.plane::<f32>(0);
    let right = frame.plane::<f32>(1);
    for index in 0..frame.samples() {
        let left = left[index];
        let right = right[index];
        samples.push(if left.is_finite() { left } else { 0.0 });
        samples.push(if right.is_finite() { right } else { 0.0 });
    }
}
