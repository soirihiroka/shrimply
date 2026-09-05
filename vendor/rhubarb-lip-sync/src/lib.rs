//! The Rhubarb Lip Sync phonetic animation pipeline used by Shrimply.
//!
//! This is a Rust port of the fixed phonetic-recognizer path from Rhubarb Lip
//! Sync. Input is signed 16-bit mono PCM sampled at 16 kHz.

mod animation;
mod timeline;

use std::path::Path;

use earshot::Detector;
use shrimply_pocketsphinx::{Model, PhoneSegment, SAMPLE_RATE};

use timeline::{Span, join};

pub const SAMPLE_RATE_HZ: u32 = SAMPLE_RATE;
pub const CENTISECONDS_PER_SECOND: i64 = 100;
pub const VAD_FRAME_SAMPLES: usize = 256;
pub const VAD_THRESHOLD: f32 = 0.5;
pub const MAX_VOICE_GAP_CENTISECONDS: i64 = 10;
pub const MIN_VOICE_SEGMENT_CENTISECONDS: i64 = 5;
pub const RECOGNITION_PADDING_CENTISECONDS: i64 = 3;
pub const MIN_NOISE_SOUND_CENTISECONDS: i64 = 12;
const MIN_NORMALIZED_DC_OFFSET: f64 = 1.0 / 15_000.0;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum MouthShape {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    #[default]
    X,
}

impl MouthShape {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
            Self::F => "F",
            Self::G => "G",
            Self::H => "H",
            Self::X => "X",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouthCue {
    pub start_centiseconds: i64,
    pub end_centiseconds: i64,
    pub shape: MouthShape,
}

pub struct Analyzer {
    model: Model,
}

impl Analyzer {
    pub fn new() -> Result<Self, String> {
        Self::load(&shrimply_pocketsphinx::model_path()?)
    }

    pub fn load(model_path: &Path) -> Result<Self, String> {
        Ok(Self {
            model: Model::load(model_path)?,
        })
    }

    pub fn analyze(&self, samples: &[i16]) -> Result<Vec<MouthCue>, String> {
        let duration = samples_to_centiseconds(samples.len());
        if duration == 0 {
            return Ok(Vec::new());
        }

        let samples = remove_dc_offset(samples);
        let utterances = detect_voice_activity(&samples, duration);
        let mut phones = Vec::new();
        for utterance in utterances {
            phones.extend(self.decode_utterance(&samples, utterance, duration)?);
        }
        phones.sort_by_key(|phone| (phone.start, phone.end));
        Ok(animation::animate(&phones, duration))
    }

    fn decode_utterance(
        &self,
        samples: &[i16],
        utterance: Span<()>,
        duration: i64,
    ) -> Result<Vec<animation::TimedPhone>, String> {
        let padded_start = (utterance.start - RECOGNITION_PADDING_CENTISECONDS).max(0);
        let padded_end = (utterance.end + RECOGNITION_PADDING_CENTISECONDS).min(duration);
        let sample_start = centiseconds_to_samples(padded_start);
        let sample_end = centiseconds_to_samples(padded_end).min(samples.len());
        let decoded = self.model.decode(&samples[sample_start..sample_end])?;
        let mut phones: Vec<_> = decoded
            .into_iter()
            .filter_map(|phone| decoded_phone(phone, padded_start, padded_end))
            .collect();

        add_noise_sounds(&mut phones, utterance);
        Ok(phones)
    }
}

fn decoded_phone(
    segment: PhoneSegment,
    offset: i64,
    padded_end: i64,
) -> Option<animation::TimedPhone> {
    let start = (segment.start_centiseconds + offset).clamp(offset, padded_end);
    let end = (segment.end_centiseconds + offset).clamp(offset, padded_end);
    (start < end).then(|| animation::TimedPhone {
        start,
        end,
        phone: animation::Phone::from_sphinx(segment.phone, end - start),
    })
}

fn detect_voice_activity(samples: &[i16], duration: i64) -> Vec<Span<()>> {
    // Rhubarb removes the offset once for recognition and once again on the
    // VAD stream. Earshot consumes the 16 kHz stream without resampling.
    let samples = remove_dc_offset(samples);
    let mut detector = Detector::default();
    let active = samples
        .chunks_exact(VAD_FRAME_SAMPLES)
        .enumerate()
        .filter_map(|(index, frame)| {
            if detector.predict_i16(frame) < VAD_THRESHOLD {
                return None;
            }
            let sample_start = index * VAD_FRAME_SAMPLES;
            Some(Span {
                start: sample_position_to_centiseconds(sample_start),
                end: sample_position_to_centiseconds_ceil(sample_start + VAD_FRAME_SAMPLES)
                    .min(duration),
                value: (),
            })
        })
        .collect();
    let mut active = join_nearby(active, MAX_VOICE_GAP_CENTISECONDS);
    active.retain(|span| span.duration() >= MIN_VOICE_SEGMENT_CENTISECONDS);
    active
}

fn join_nearby<T: Clone + Eq>(mut spans: Vec<Span<T>>, max_gap: i64) -> Vec<Span<T>> {
    spans.sort_by_key(|span| span.start);
    let mut result: Vec<Span<T>> = Vec::with_capacity(spans.len());
    for span in join(spans) {
        if let Some(previous) = result.last_mut()
            && span.start - previous.end <= max_gap
            && span.value == previous.value
        {
            previous.end = previous.end.max(span.end);
        } else {
            result.push(span);
        }
    }
    result
}

fn add_noise_sounds(phones: &mut Vec<animation::TimedPhone>, utterance: Span<()>) {
    let mut covered: Vec<_> = phones
        .iter()
        .filter_map(|phone| {
            let start = phone.start.max(utterance.start);
            let end = phone.end.min(utterance.end);
            (start < end).then_some(Span {
                start,
                end,
                value: (),
            })
        })
        .collect();
    covered.sort_by_key(|span| span.start);
    covered = join_nearby(covered, 0);

    let mut position = utterance.start;
    for occupied in covered {
        if occupied.start - position >= MIN_NOISE_SOUND_CENTISECONDS && position != 0 {
            phones.push(animation::TimedPhone::noise(position, occupied.start));
        }
        position = position.max(occupied.end);
    }
    if utterance.end - position >= MIN_NOISE_SOUND_CENTISECONDS && position != 0 {
        phones.push(animation::TimedPhone::noise(position, utterance.end));
    }
}

fn remove_dc_offset(samples: &[i16]) -> Vec<i16> {
    if samples.is_empty() {
        return Vec::new();
    }
    let sample_count = if samples.len() > 4 * SAMPLE_RATE as usize {
        3 * SAMPLE_RATE as usize
    } else {
        samples.len()
    };
    let fade_count = if samples.len() > 4 * SAMPLE_RATE as usize {
        SAMPLE_RATE as usize
    } else {
        0
    };
    let flat_sum: f64 = samples[..sample_count]
        .iter()
        .map(|sample| f64::from(*sample))
        .sum();
    let fade_sum: f64 = samples[sample_count..sample_count + fade_count]
        .iter()
        .enumerate()
        .map(|(index, sample)| f64::from(*sample) * (fade_count - index) as f64 / fade_count as f64)
        .sum();
    let total_weight = sample_count as f64 + (fade_count + 1) as f64 / 2.0;
    let offset = (flat_sum + fade_sum) / total_weight;
    if offset.abs() < f64::from(i16::MAX) * MIN_NORMALIZED_DC_OFFSET {
        return samples.to_vec();
    }
    let normalized_offset = offset / f64::from(i16::MAX);
    let factor = 1.0 / (1.0 + normalized_offset.abs());
    samples
        .iter()
        .map(|sample| {
            ((f64::from(*sample) * factor) - offset).clamp(f64::from(i16::MIN), f64::from(i16::MAX))
                as i16
        })
        .collect()
}

const fn samples_to_centiseconds(samples: usize) -> i64 {
    (samples as i64 * CENTISECONDS_PER_SECOND) / SAMPLE_RATE as i64
}

const fn sample_position_to_centiseconds(sample: usize) -> i64 {
    samples_to_centiseconds(sample)
}

const fn sample_position_to_centiseconds_ceil(sample: usize) -> i64 {
    (sample as i64 * CENTISECONDS_PER_SECOND + SAMPLE_RATE as i64 - 1) / SAMPLE_RATE as i64
}

const fn centiseconds_to_samples(centiseconds: i64) -> usize {
    (centiseconds * SAMPLE_RATE as i64 / CENTISECONDS_PER_SECOND) as usize
}
