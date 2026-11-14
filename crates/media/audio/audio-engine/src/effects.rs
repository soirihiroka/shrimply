use std::collections::VecDeque;
use std::f32::consts::FRAC_PI_4;
use std::ffi::c_void;
use std::ptr::NonNull;

use fundsp::prelude32::*;

use shrimply_audio_modifiers::{
    AudioModifierEffect, DenoiseEngine, FilterMode, PitchQuality, ReverbMode,
};
use shrimply_project_document::project::{AudioItem, Time};
use shrimply_property_model::timeline_value::*;

use super::CHANNELS;

mod denoise;

use denoise::{DeepFilterNetState, RnnoiseState};

pub(super) struct Processor {
    states: Vec<EffectState>,
    sample_rate: u32,
}

enum EffectState {
    Stateless,
    Equalizer(EqualizerState),
    Filter(FilterState),
    NoiseGate(NoiseGateState),
    Bitcrusher(BitcrusherState),
    Chorus(Box<ChorusState>),
    Compressor(CompressorState),
    Limiter(LimiterState),
    Reverb(Box<ReverbState>),
    CloseUp(CloseUpState),
    VoiceColor(VoiceColorState),
    Echo(Box<EchoState>),
    Distortion(DistortionState),
    Rnnoise(Box<RnnoiseState>),
    DeepFilterNet(Box<DeepFilterNetState>),
    Pitch(PitchState),
}

struct EqualizerState {
    channels: [EqualizerChannel; CHANNELS],
    parameters: [f32; 3],
}

struct EqualizerChannel {
    low: An<FixedSvf<f32, LowshelfMode<f32>>>,
    mid: An<FixedSvf<f32, BellMode<f32>>>,
    high: An<FixedSvf<f32, HighshelfMode<f32>>>,
}

enum FilterState {
    LowPass([An<FixedSvf<f32, LowpassMode<f32>>>; CHANNELS]),
    HighPass([An<FixedSvf<f32, HighpassMode<f32>>>; CHANNELS]),
}

struct NoiseGateState {
    envelope: f32,
    gain: f32,
    open: bool,
}

struct BitcrusherState {
    held: [f32; CHANNELS],
    phase: f32,
}

struct ChorusState {
    samples: [Vec<f32>; CHANNELS],
    write: usize,
    phase: f32,
    sample_rate: u32,
    warmup_frames: usize,
}

#[derive(Default)]
struct CompressorState {
    envelope: f32,
}

struct LimiterState {
    unit: Box<dyn AudioUnit>,
    release_ms: f32,
    latency_frames: usize,
}

struct CloseUpState {
    channels: [CloseUpChannel; CHANNELS],
    distance_cm: f32,
    makeup_gain: f32,
    ceiling: f32,
}

struct CloseUpChannel {
    proximity: An<FixedSvf<f32, LowshelfMode<f32>>>,
    presence: An<FixedSvf<f32, BellMode<f32>>>,
    air: An<FixedSvf<f32, HighshelfMode<f32>>>,
}

struct VoiceColorState {
    channels: [VoiceColorChannel; CHANNELS],
    amount: f32,
    envelope_squared: f32,
    gain: f32,
    level_attack: f32,
    level_release: f32,
    gain_reduction: f32,
    gain_recovery: f32,
    target_level: f32,
    gain_min: f32,
    gain_max: f32,
    ceiling: f32,
    warmup_frames: usize,
}

struct VoiceColorChannel {
    rumble: An<FixedSvf<f32, HighpassMode<f32>>>,
    body: An<FixedSvf<f32, LowshelfMode<f32>>>,
    presence: An<FixedSvf<f32, BellMode<f32>>>,
    enclosure: An<FixedSvf<f32, BellMode<f32>>>,
}

const MAX_REVERB_PRE_DELAY_SECONDS: f32 = 0.5;
const MAX_ROOM_CAPTURE_DELAY_SECONDS: f32 = 0.1;
const MAX_ECHO_DELAY_SECONDS: f32 = 2.0;
const MAX_CHORUS_DELAY_SECONDS: f32 = 0.04;
const DELAY_SMOOTH_SECONDS: f32 = 0.01;
const LEVEL_WINDOW_SECONDS: f32 = 0.05;
const LEVEL_SMOOTH_SECONDS: f32 = 0.1;
const MIN_LEVEL_COMPENSATION: f32 = 0.003_981_071_7;
const MAX_LEVEL_COMPENSATION: f32 = 4.0;
const REVERB_ROOM_SCALE_MIN: f32 = 0.7;
const REVERB_ROOM_SCALE_MAX: f32 = 1.5;
const REVERB_DAMPING_CUTOFF_MAX: f32 = 20_000.0;
const REVERB_DAMPING_CUTOFF_MIN: f32 = 800.0;
const DISTORTION_TONE_CUTOFF_MIN: f32 = 800.0;
const DISTORTION_TONE_CUTOFF_MAX: f32 = 20_000.0;
const FILTER_CUTOFF_MIN: f32 = 20.0;
const FILTER_CUTOFF_MAX: f32 = 20_000.0;
const FILTER_RESONANCE_MIN: f32 = 0.5;
const FILTER_RESONANCE_MAX: f32 = 10.0;
const GATE_HYSTERESIS_DB: f32 = 3.0;
const CHORUS_PHASE_OFFSETS: [[f32; 2]; CHANNELS] = [
    [0.0, std::f32::consts::PI],
    [std::f32::consts::FRAC_PI_2, std::f32::consts::PI * 1.5],
];
const REVERB_COMB_DELAYS: [[f32; 4]; CHANNELS] = [
    [0.025_306, 0.026_939, 0.028_957, 0.030_748],
    [0.025_828, 0.027_460, 0.029_478, 0.031_270],
];
const REVERB_ALLPASS_DELAYS: [[f32; 2]; CHANNELS] = [[0.012_608, 0.01], [0.013_129, 0.010_522]];
const REVERB_ALLPASS_FEEDBACK: f32 = 0.5;
const ROOM_CAPTURE_EARLY_GAIN: f32 = 0.45;
const ROOM_CAPTURE_LATE_GAIN_MIN: f32 = 0.08;
const ROOM_CAPTURE_LATE_GAIN_MAX: f32 = 0.32;
const ROOM_CAPTURE_DISTANCE_MIN_M: f32 = 0.2;
const ROOM_CAPTURE_DISTANCE_MAX_M: f32 = 5.0;
const ROOM_CAPTURE_REFLECTION_CUTOFF_MAX: f32 = 18_000.0;
const ROOM_CAPTURE_REFLECTION_CUTOFF_MIN: f32 = 4_000.0;
const CLOSE_UP_DISTANCE_MIN_CM: f32 = 3.0;
const CLOSE_UP_DISTANCE_MAX_CM: f32 = 100.0;
const CLOSE_UP_PROXIMITY_GAIN_DB: f32 = 8.0;
const CLOSE_UP_PRESENCE_GAIN_DB: f32 = 3.0;
const CLOSE_UP_AIR_GAIN_DB: f32 = 2.0;
const CLOSE_UP_MAKEUP_GAIN_DB: f32 = 4.0;
const CLOSE_UP_CEILING_DBFS: f32 = -1.0;
const CLOSE_UP_PROXIMITY_FREQUENCY_HZ: f32 = 120.0;
const CLOSE_UP_PRESENCE_FREQUENCY_HZ: f32 = 4_000.0;
const CLOSE_UP_AIR_FREQUENCY_HZ: f32 = 9_000.0;
const VOICE_COLOR_RUMBLE_CUTOFF_MIN_HZ: f32 = 20.0;
const VOICE_COLOR_RUMBLE_CUTOFF_MAX_HZ: f32 = 60.0;
const VOICE_COLOR_BODY_GAIN_DB: f32 = -1.5;
const VOICE_COLOR_PRESENCE_GAIN_DB: f32 = 1.8;
const VOICE_COLOR_ENCLOSURE_GAIN_DB: f32 = -1.2;
const VOICE_COLOR_BODY_FREQUENCY_HZ: f32 = 180.0;
const VOICE_COLOR_PRESENCE_FREQUENCY_HZ: f32 = 3_200.0;
const VOICE_COLOR_ENCLOSURE_FREQUENCY_HZ: f32 = 7_500.0;
const VOICE_COLOR_ENCLOSURE_Q: f32 = 1.2;
const VOICE_COLOR_LEVEL_ATTACK_SECONDS: f32 = 0.02;
const VOICE_COLOR_LEVEL_RELEASE_SECONDS: f32 = 0.3;
const VOICE_COLOR_GAIN_REDUCTION_SECONDS: f32 = 0.05;
const VOICE_COLOR_GAIN_RECOVERY_SECONDS: f32 = 1.5;
const VOICE_COLOR_TARGET_DBFS: f32 = -18.0;
const VOICE_COLOR_GAIN_LIMIT_DB: f32 = 6.0;
const VOICE_COLOR_CEILING_DBFS: f32 = -1.0;
const VOICE_COLOR_LEVEL_FLOOR: f32 = 1.0e-9;
const RUBBERBAND_LIVE_WINDOW_MEDIUM: i32 = 0x0010_0000;
const RUBBERBAND_FORMANT_PRESERVED: i32 = 0x0100_0000;
const RUBBERBAND_CHANNELS_TOGETHER: i32 = 0x1000_0000;

struct DelayLine {
    samples: Vec<f32>,
    write: usize,
    filtered: f32,
}

struct ReverbState {
    pre_delay: [Vec<f32>; CHANNELS],
    pre_delay_write: usize,
    combs: [[DelayLine; 4]; CHANNELS],
    allpasses: [[DelayLine; 2]; CHANNELS],
    sample_rate: u32,
    pre_delay_frames: f32,
    room_size: f32,
    decay_seconds: f32,
    damping: f32,
    comb_delay_frames: [[f32; 4]; CHANNELS],
    comb_feedback: [[f32; 4]; CHANNELS],
    allpass_delay_frames: [[f32; 2]; CHANNELS],
    damping_coefficient: f32,
    capture_input: [Vec<f32>; CHANNELS],
    capture_write: usize,
    capture_filtered: [[f32; shrimply_math_media::AUDIO_ROOM_REFLECTIONS]; CHANNELS],
    capture_delay_frames: [f32; shrimply_math_media::AUDIO_ROOM_REFLECTIONS],
    capture_gains: [f32; shrimply_math_media::AUDIO_ROOM_REFLECTIONS],
    capture_filter_coefficient: f32,
    capture_late_gain: f32,
    capture_normalization: f32,
    capture_room_size: f32,
    capture_distance_m: f32,
    capture_absorption: f32,
    warmup_frames: usize,
}

struct EchoState {
    samples: [Vec<f32>; CHANNELS],
    write: usize,
    sample_rate: u32,
    delay_frames: f32,
    warmup_frames: usize,
}

struct DistortionState {
    tone: [f32; CHANNELS],
    dry_power: f32,
    wet_power: f32,
    compensation: f32,
    drive_db: f32,
    drive: f32,
    tone_amount: f32,
    tone_coefficient: f32,
}

struct PitchState {
    state: NonNull<c_void>,
    block_size: usize,
    start_delay: usize,
    input: [Vec<f32>; CHANNELS],
    output: [Vec<f32>; CHANNELS],
    ready: [VecDeque<f32>; CHANNELS],
    pending_pitch: f32,
    pending_formant: f32,
}

impl Processor {
    pub(super) fn new(item: &AudioItem, sample_rate: u32) -> Result<Self, String> {
        let mut states = Vec::with_capacity(item.modifiers.len());
        for modifier in &item.modifiers {
            if !modifier.enabled {
                states.push(EffectState::Stateless);
                continue;
            }
            states.push(match &modifier.effect {
                AudioModifierEffect::Equalizer(value) => {
                    EffectState::Equalizer(EqualizerState::new(
                        sample_rate,
                        [
                            value.low_db.fallback(),
                            value.mid_db.fallback(),
                            value.high_db.fallback(),
                        ],
                    ))
                }
                AudioModifierEffect::Filter(value) => {
                    EffectState::Filter(FilterState::new(sample_rate, value.mode))
                }
                AudioModifierEffect::NoiseGate(_) => EffectState::NoiseGate(NoiseGateState {
                    envelope: 0.0,
                    gain: 0.0,
                    open: false,
                }),
                AudioModifierEffect::Bitcrusher(_) => EffectState::Bitcrusher(BitcrusherState {
                    held: [0.0; CHANNELS],
                    phase: 1.0,
                }),
                AudioModifierEffect::Chorus(_) => {
                    EffectState::Chorus(Box::new(ChorusState::new(sample_rate)))
                }
                AudioModifierEffect::Compressor(_) => {
                    EffectState::Compressor(CompressorState::default())
                }
                AudioModifierEffect::Limiter(value) => EffectState::Limiter(LimiterState::new(
                    sample_rate,
                    value.release_ms.fallback(),
                )),
                AudioModifierEffect::Reverb(value) => {
                    EffectState::Reverb(Box::new(ReverbState::new(
                        sample_rate,
                        value.room_size.fallback(),
                        value.pre_delay_ms.fallback(),
                        value.mode,
                    )))
                }
                AudioModifierEffect::CloseUp(value) => EffectState::CloseUp(CloseUpState::new(
                    sample_rate,
                    value.distance_cm.fallback(),
                )),
                AudioModifierEffect::VoiceColor(value) => EffectState::VoiceColor(
                    VoiceColorState::new(sample_rate, value.amount.fallback(), value.auto_level),
                ),
                AudioModifierEffect::Echo(value) => EffectState::Echo(Box::new(EchoState::new(
                    sample_rate,
                    value.delay_ms.fallback(),
                ))),
                AudioModifierEffect::Distortion(value) => {
                    EffectState::Distortion(DistortionState::new(value.drive_db.fallback()))
                }
                AudioModifierEffect::Denoise(value) => match value.engine {
                    DenoiseEngine::Rnnoise => {
                        if sample_rate != 48_000 {
                            return Err(format!(
                                "RNNoise requires 48000 Hz audio, got {sample_rate} Hz"
                            ));
                        }
                        EffectState::Rnnoise(Box::new(RnnoiseState::new()))
                    }
                    DenoiseEngine::DeepFilterNet => EffectState::DeepFilterNet(Box::new(
                        DeepFilterNetState::new(sample_rate).map_err(|error| {
                            format!("could not initialize DeepFilterNet: {error}")
                        })?,
                    )),
                },
                AudioModifierEffect::Pitch(value) => {
                    EffectState::Pitch(PitchState::new(sample_rate, value)?)
                }
                AudioModifierEffect::Cache(_)
                | AudioModifierEffect::Gain(_)
                | AudioModifierEffect::Pan(_)
                | AudioModifierEffect::StereoWidth(_)
                | AudioModifierEffect::Tremolo(_)
                | AudioModifierEffect::VoiceChange(_) => EffectState::Stateless,
            });
        }
        Ok(Self {
            states,
            sample_rate,
        })
    }

    pub(super) fn warmup_frames(&self) -> usize {
        self.states
            .iter()
            .map(|state| match state {
                EffectState::Limiter(_) => self.sample_rate as usize / 200,
                EffectState::Rnnoise(_) => nnnoiseless::DenoiseState::FRAME_SIZE,
                EffectState::DeepFilterNet(state) => {
                    state.model.lookahead.saturating_add(1) * state.model.hop_size
                }
                EffectState::Pitch(state) => state.start_delay + state.block_size.saturating_sub(1),
                EffectState::Reverb(state) => state.warmup_frames,
                EffectState::VoiceColor(state) => state.warmup_frames,
                EffectState::Echo(state) => state.warmup_frames,
                EffectState::Chorus(state) => state.warmup_frames,
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }

    pub(super) fn latency_frames(&self) -> usize {
        self.states
            .iter()
            .map(|state| match state {
                EffectState::Limiter(state) => state.latency_frames,
                EffectState::Rnnoise(_) => nnnoiseless::DenoiseState::FRAME_SIZE - 1,
                EffectState::DeepFilterNet(state) => state.latency_frames(),
                EffectState::Pitch(state) => state.start_delay + state.block_size.saturating_sub(1),
                _ => 0,
            })
            .sum()
    }

    pub(super) fn process(
        &mut self,
        samples: &mut [f32],
        item: &AudioItem,
        local_start: Time,
    ) -> Result<(), String> {
        if self.states.len() != item.modifiers.len() {
            return Err("audio modifier processor does not match the item chain".to_string());
        }
        let mut upstream_latency: usize = 0;
        for (modifier, state) in item.modifiers.iter().zip(&mut self.states) {
            if !modifier.enabled {
                continue;
            }
            let state_latency = latency_frames(state);
            let effect_start = local_start.saturating_sub(Time::from_fraction(
                std::cmp::Ord::min(upstream_latency, i64::MAX as usize) as i64,
                i64::from(self.sample_rate),
            ));
            let effect_measurement = match &modifier.effect {
                AudioModifierEffect::Cache(_) => "Audio effect / Cache",
                AudioModifierEffect::Gain(_) => "Audio effect / Gain",
                AudioModifierEffect::Pan(_) => "Audio effect / Pan",
                AudioModifierEffect::Pitch(_) => "Audio effect / Pitch",
                AudioModifierEffect::Denoise(value) => match value.engine {
                    DenoiseEngine::Rnnoise => "Audio effect / RNNoise",
                    DenoiseEngine::DeepFilterNet => "Audio effect / DeepFilterNet",
                },
                AudioModifierEffect::Equalizer(_) => "Audio effect / Equalizer",
                AudioModifierEffect::Filter(_) => "Audio effect / Filter",
                AudioModifierEffect::NoiseGate(_) => "Audio effect / Noise gate",
                AudioModifierEffect::StereoWidth(_) => "Audio effect / Stereo width",
                AudioModifierEffect::Tremolo(_) => "Audio effect / Tremolo",
                AudioModifierEffect::Bitcrusher(_) => "Audio effect / Bitcrusher",
                AudioModifierEffect::Chorus(_) => "Audio effect / Chorus",
                AudioModifierEffect::Compressor(_) => "Audio effect / Compressor",
                AudioModifierEffect::Limiter(_) => "Audio effect / Limiter",
                AudioModifierEffect::Reverb(_) => "Audio effect / Reverb",
                AudioModifierEffect::CloseUp(_) => "Audio effect / Close up",
                AudioModifierEffect::VoiceColor(_) => "Audio effect / Phone mic",
                AudioModifierEffect::Echo(_) => "Audio effect / Echo",
                AudioModifierEffect::Distortion(_) => "Audio effect / Distortion",
                AudioModifierEffect::VoiceChange(_) => "Audio effect / Pneuma voice change",
            };
            let _measurement = shrimply_profiling::measure(effect_measurement);
            match (&modifier.effect, state) {
                (AudioModifierEffect::Cache(_), EffectState::Stateless) => {}
                (AudioModifierEffect::Gain(value), _) => {
                    if let TimelineBase::Const(decibels) = &value.decibels.base {
                        let gain = db_gain(decibels.clamp(-60.0, 36.0));
                        for sample in samples.iter_mut() {
                            *sample *= gain;
                        }
                    } else {
                        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
                            let time = local_time(effect_start, frame, self.sample_rate);
                            let gain = db_gain(value.decibels.value_at(time).clamp(-60.0, 36.0));
                            channels[0] *= gain;
                            channels[1] *= gain;
                        }
                    }
                }
                (AudioModifierEffect::Pan(value), _) => {
                    if let TimelineBase::Const(position) = &value.position.base {
                        let position = position.clamp(-1.0, 1.0);
                        let angle = (position + 1.0) * FRAC_PI_4;
                        let left = angle.cos() * 2.0_f32.sqrt();
                        let right = angle.sin() * 2.0_f32.sqrt();
                        for channels in samples.chunks_exact_mut(CHANNELS) {
                            channels[0] *= left;
                            channels[1] *= right;
                        }
                    } else {
                        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
                            let time = local_time(effect_start, frame, self.sample_rate);
                            let position = value.position.value_at(time).clamp(-1.0, 1.0);
                            let angle = (position + 1.0) * FRAC_PI_4;
                            channels[0] *= angle.cos() * 2.0_f32.sqrt();
                            channels[1] *= angle.sin() * 2.0_f32.sqrt();
                        }
                    }
                }
                (AudioModifierEffect::StereoWidth(value), _) => {
                    if let TimelineBase::Const(width) = &value.width.base {
                        let width = width.clamp(0.0, 2.0);
                        for channels in samples.chunks_exact_mut(CHANNELS) {
                            (channels[0], channels[1]) =
                                crate::math::audio_stereo_width(channels[0], channels[1], width);
                        }
                    } else {
                        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
                            let time = local_time(effect_start, frame, self.sample_rate);
                            let width = value.width.value_at(time).clamp(0.0, 2.0);
                            (channels[0], channels[1]) =
                                crate::math::audio_stereo_width(channels[0], channels[1], width);
                        }
                    }
                }
                (AudioModifierEffect::Tremolo(value), _) => {
                    let constant = match (&value.rate_hz.base, &value.depth.base) {
                        (TimelineBase::Const(rate), TimelineBase::Const(depth)) => {
                            Some((rate.clamp(0.1, 20.0), depth.clamp(0.0, 1.0)))
                        }
                        _ => None,
                    };
                    for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
                        let time = local_time(effect_start, frame, self.sample_rate);
                        let (rate, depth) = constant.unwrap_or_else(|| {
                            (
                                value.rate_hz.value_at(time).clamp(0.1, 20.0),
                                value.depth.value_at(time).clamp(0.0, 1.0),
                            )
                        });
                        let gain = crate::math::audio_tremolo_gain(time, rate, depth);
                        channels[0] *= gain;
                        channels[1] *= gain;
                    }
                }
                (AudioModifierEffect::Pitch(value), EffectState::Pitch(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::Denoise(value), EffectState::Rnnoise(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::Denoise(value), EffectState::DeepFilterNet(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate)?;
                }
                (AudioModifierEffect::Equalizer(value), EffectState::Equalizer(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::Filter(value), EffectState::Filter(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::NoiseGate(value), EffectState::NoiseGate(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::Bitcrusher(value), EffectState::Bitcrusher(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::Chorus(value), EffectState::Chorus(state)) => {
                    state.process(samples, value, effect_start);
                }
                (AudioModifierEffect::Compressor(value), EffectState::Compressor(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::Limiter(value), EffectState::Limiter(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::Reverb(value), EffectState::Reverb(state)) => {
                    state.process(samples, value, effect_start);
                }
                (AudioModifierEffect::CloseUp(value), EffectState::CloseUp(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::VoiceColor(value), EffectState::VoiceColor(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::Echo(value), EffectState::Echo(state)) => {
                    state.process(samples, value, effect_start);
                }
                (AudioModifierEffect::Distortion(value), EffectState::Distortion(state)) => {
                    state.process(samples, value, effect_start, self.sample_rate);
                }
                (AudioModifierEffect::VoiceChange(_), EffectState::Stateless) => {}
                _ => return Err("audio modifier processor state mismatch".to_string()),
            }
            upstream_latency = upstream_latency.saturating_add(state_latency);
        }
        if let TimelineBase::Const(decibels) = &item.gain.decibels.base {
            let gain = db_gain(decibels.clamp(-60.0, 36.0));
            for sample in samples.iter_mut() {
                *sample *= gain;
            }
        } else {
            for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
                let time = local_time(local_start, frame, self.sample_rate);
                let gain = db_gain(item.gain.decibels.value_at(time).clamp(-60.0, 36.0));
                channels[0] *= gain;
                channels[1] *= gain;
            }
        }
        Ok(())
    }
}

fn latency_frames(state: &EffectState) -> usize {
    match state {
        EffectState::Limiter(state) => state.latency_frames,
        EffectState::Rnnoise(_) => nnnoiseless::DenoiseState::FRAME_SIZE - 1,
        EffectState::DeepFilterNet(state) => state.latency_frames(),
        EffectState::Pitch(state) => state.start_delay + state.block_size.saturating_sub(1),
        _ => 0,
    }
}

impl EqualizerState {
    fn new(sample_rate: u32, parameters: [f32; 3]) -> Self {
        Self {
            channels: std::array::from_fn(|_| EqualizerChannel::new(sample_rate, parameters)),
            parameters,
        }
    }

    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::EqualizerModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        if let (TimelineBase::Const(low), TimelineBase::Const(mid), TimelineBase::Const(high)) =
            (&value.low_db.base, &value.mid_db.base, &value.high_db.base)
        {
            self.set_parameters([
                low.clamp(-24.0, 24.0),
                mid.clamp(-24.0, 24.0),
                high.clamp(-24.0, 24.0),
            ]);
            for samples in samples.chunks_exact_mut(CHANNELS) {
                for (sample, channel) in samples.iter_mut().zip(&mut self.channels) {
                    *sample = channel.process(*sample);
                }
            }
            return;
        }
        for (frame, samples) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let time = local_time(local_start, frame, sample_rate);
            let parameters = [
                value.low_db.value_at(time).clamp(-24.0, 24.0),
                value.mid_db.value_at(time).clamp(-24.0, 24.0),
                value.high_db.value_at(time).clamp(-24.0, 24.0),
            ];
            self.set_parameters(parameters);
            for (sample, channel) in samples.iter_mut().zip(&mut self.channels) {
                *sample = channel.process(*sample);
            }
        }
    }

    fn set_parameters(&mut self, parameters: [f32; 3]) {
        if parameters == self.parameters {
            return;
        }
        for channel in &mut self.channels {
            channel.set_gains(parameters);
        }
        self.parameters = parameters;
    }
}

impl EqualizerChannel {
    fn new(sample_rate: u32, parameters: [f32; 3]) -> Self {
        let mut channel = Self {
            low: lowshelf_hz(100.0, 1.0, db_amp(parameters[0])),
            mid: bell_hz(1_000.0, 1.0, db_amp(parameters[1])),
            high: highshelf_hz(10_000.0, 1.0, db_amp(parameters[2])),
        };
        channel.low.set_sample_rate(f64::from(sample_rate));
        channel.mid.set_sample_rate(f64::from(sample_rate));
        channel.high.set_sample_rate(f64::from(sample_rate));
        channel
    }

    fn set_gains(&mut self, parameters: [f32; 3]) {
        self.low.set_gain(db_amp(parameters[0]));
        self.mid.set_gain(db_amp(parameters[1]));
        self.high.set_gain(db_amp(parameters[2]));
    }

    fn process(&mut self, sample: f32) -> f32 {
        let sample = self.low.filter_mono(sample);
        let sample = self.mid.filter_mono(sample);
        self.high.filter_mono(sample)
    }
}

impl CloseUpState {
    fn new(sample_rate: u32, distance_cm: f32) -> Self {
        let distance_cm = distance_cm.clamp(CLOSE_UP_DISTANCE_MIN_CM, CLOSE_UP_DISTANCE_MAX_CM);
        let amount = crate::math::audio_proximity_amount(distance_cm);
        Self {
            channels: std::array::from_fn(|_| CloseUpChannel::new(sample_rate, distance_cm)),
            distance_cm,
            makeup_gain: db_gain(CLOSE_UP_MAKEUP_GAIN_DB * amount),
            ceiling: db_gain(CLOSE_UP_CEILING_DBFS),
        }
    }

    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::CloseUpModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        let constant = match &value.distance_cm.base {
            TimelineBase::Const(distance_cm) => {
                Some(distance_cm.clamp(CLOSE_UP_DISTANCE_MIN_CM, CLOSE_UP_DISTANCE_MAX_CM))
            }
            _ => None,
        };
        if let Some(distance_cm) = constant {
            self.set_distance(distance_cm);
        }
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let distance_cm = constant.unwrap_or_else(|| {
                value
                    .distance_cm
                    .value_at(local_time(local_start, frame, sample_rate))
                    .clamp(CLOSE_UP_DISTANCE_MIN_CM, CLOSE_UP_DISTANCE_MAX_CM)
            });
            if constant.is_none() {
                self.set_distance(distance_cm);
            }
            for (sample, channel) in channels.iter_mut().zip(&mut self.channels) {
                let colored = channel.process(*sample) * self.makeup_gain;
                *sample = if self.makeup_gain > 1.0 {
                    crate::math::audio_soft_ceiling(colored, self.ceiling)
                } else {
                    colored
                };
            }
        }
    }

    fn set_distance(&mut self, distance_cm: f32) {
        if distance_cm == self.distance_cm {
            return;
        }
        for channel in &mut self.channels {
            channel.set_distance(distance_cm);
        }
        self.distance_cm = distance_cm;
        self.makeup_gain =
            db_gain(CLOSE_UP_MAKEUP_GAIN_DB * crate::math::audio_proximity_amount(distance_cm));
    }
}

impl CloseUpChannel {
    fn new(sample_rate: u32, distance_cm: f32) -> Self {
        let amount = crate::math::audio_proximity_amount(distance_cm);
        let proximity_gain = CLOSE_UP_PROXIMITY_GAIN_DB * amount;
        let mut channel = Self {
            proximity: lowshelf_hz(CLOSE_UP_PROXIMITY_FREQUENCY_HZ, 1.0, db_amp(proximity_gain)),
            presence: bell_hz(
                CLOSE_UP_PRESENCE_FREQUENCY_HZ,
                std::f32::consts::FRAC_1_SQRT_2,
                db_amp(CLOSE_UP_PRESENCE_GAIN_DB * amount),
            ),
            air: highshelf_hz(
                CLOSE_UP_AIR_FREQUENCY_HZ,
                std::f32::consts::FRAC_1_SQRT_2,
                db_amp(CLOSE_UP_AIR_GAIN_DB * amount),
            ),
        };
        channel.proximity.set_sample_rate(f64::from(sample_rate));
        channel.presence.set_sample_rate(f64::from(sample_rate));
        channel.air.set_sample_rate(f64::from(sample_rate));
        channel
    }

    fn set_distance(&mut self, distance_cm: f32) {
        let amount = crate::math::audio_proximity_amount(distance_cm);
        self.proximity
            .set_gain(db_amp(CLOSE_UP_PROXIMITY_GAIN_DB * amount));
        self.presence
            .set_gain(db_amp(CLOSE_UP_PRESENCE_GAIN_DB * amount));
        self.air.set_gain(db_amp(CLOSE_UP_AIR_GAIN_DB * amount));
    }

    fn process(&mut self, sample: f32) -> f32 {
        let sample = self.proximity.filter_mono(sample);
        let sample = self.presence.filter_mono(sample);
        self.air.filter_mono(sample)
    }
}

impl VoiceColorState {
    fn new(sample_rate: u32, amount: f32, auto_level: bool) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        Self {
            channels: std::array::from_fn(|_| VoiceColorChannel::new(sample_rate, amount)),
            amount,
            envelope_squared: 0.0,
            gain: 1.0,
            level_attack: crate::math::audio_smoothing_coefficient(
                VOICE_COLOR_LEVEL_ATTACK_SECONDS,
                sample_rate,
            ),
            level_release: crate::math::audio_smoothing_coefficient(
                VOICE_COLOR_LEVEL_RELEASE_SECONDS,
                sample_rate,
            ),
            gain_reduction: crate::math::audio_smoothing_coefficient(
                VOICE_COLOR_GAIN_REDUCTION_SECONDS,
                sample_rate,
            ),
            gain_recovery: crate::math::audio_smoothing_coefficient(
                VOICE_COLOR_GAIN_RECOVERY_SECONDS,
                sample_rate,
            ),
            target_level: db_gain(VOICE_COLOR_TARGET_DBFS),
            gain_min: db_gain(-VOICE_COLOR_GAIN_LIMIT_DB),
            gain_max: db_gain(VOICE_COLOR_GAIN_LIMIT_DB),
            ceiling: db_gain(VOICE_COLOR_CEILING_DBFS),
            warmup_frames: if auto_level {
                (VOICE_COLOR_GAIN_RECOVERY_SECONDS * sample_rate as f32) as usize
            } else {
                0
            },
        }
    }

    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::VoiceColorModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        let constant = match &value.amount.base {
            TimelineBase::Const(amount) => Some(amount.clamp(0.0, 1.0)),
            _ => None,
        };
        if let Some(amount) = constant {
            self.set_amount(amount);
        }
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let amount = constant.unwrap_or_else(|| {
                value
                    .amount
                    .value_at(local_time(local_start, frame, sample_rate))
                    .clamp(0.0, 1.0)
            });
            if constant.is_none() {
                self.set_amount(amount);
            }
            for (sample, channel) in channels.iter_mut().zip(&mut self.channels) {
                *sample = crate::math::audio_soft_clip(channel.process(*sample), amount);
            }

            if value.auto_level {
                let level_squared =
                    (channels[0] * channels[0] + channels[1] * channels[1]) / CHANNELS as f32;
                let coefficient = if level_squared > self.envelope_squared {
                    self.level_attack
                } else {
                    self.level_release
                };
                self.envelope_squared += coefficient * (level_squared - self.envelope_squared);
                let target_gain = (self.target_level
                    / self.envelope_squared.max(VOICE_COLOR_LEVEL_FLOOR).sqrt())
                .clamp(self.gain_min, self.gain_max);
                let coefficient = if target_gain < self.gain {
                    self.gain_reduction
                } else {
                    self.gain_recovery
                };
                self.gain += coefficient * (target_gain - self.gain);
                for sample in channels.iter_mut() {
                    *sample = crate::math::audio_soft_ceiling(*sample * self.gain, self.ceiling);
                }
            }
        }
    }

    fn set_amount(&mut self, amount: f32) {
        if amount == self.amount {
            return;
        }
        for channel in &mut self.channels {
            channel.set_amount(amount);
        }
        self.amount = amount;
    }
}

impl VoiceColorChannel {
    fn new(sample_rate: u32, amount: f32) -> Self {
        let cutoff = crate::math::audio_geometric_lerp(
            VOICE_COLOR_RUMBLE_CUTOFF_MIN_HZ,
            VOICE_COLOR_RUMBLE_CUTOFF_MAX_HZ,
            amount,
        );
        let mut channel = Self {
            rumble: highpass_hz(cutoff, std::f32::consts::FRAC_1_SQRT_2),
            body: lowshelf_hz(
                VOICE_COLOR_BODY_FREQUENCY_HZ,
                1.0,
                db_amp(VOICE_COLOR_BODY_GAIN_DB * amount),
            ),
            presence: bell_hz(
                VOICE_COLOR_PRESENCE_FREQUENCY_HZ,
                std::f32::consts::FRAC_1_SQRT_2,
                db_amp(VOICE_COLOR_PRESENCE_GAIN_DB * amount),
            ),
            enclosure: bell_hz(
                VOICE_COLOR_ENCLOSURE_FREQUENCY_HZ,
                VOICE_COLOR_ENCLOSURE_Q,
                db_amp(VOICE_COLOR_ENCLOSURE_GAIN_DB * amount),
            ),
        };
        channel.rumble.set_sample_rate(f64::from(sample_rate));
        channel.body.set_sample_rate(f64::from(sample_rate));
        channel.presence.set_sample_rate(f64::from(sample_rate));
        channel.enclosure.set_sample_rate(f64::from(sample_rate));
        channel
    }

    fn set_amount(&mut self, amount: f32) {
        let cutoff = crate::math::audio_geometric_lerp(
            VOICE_COLOR_RUMBLE_CUTOFF_MIN_HZ,
            VOICE_COLOR_RUMBLE_CUTOFF_MAX_HZ,
            amount,
        );
        self.rumble
            .set_cutoff_q(cutoff, std::f32::consts::FRAC_1_SQRT_2);
        self.body
            .set_gain(db_amp(VOICE_COLOR_BODY_GAIN_DB * amount));
        self.presence
            .set_gain(db_amp(VOICE_COLOR_PRESENCE_GAIN_DB * amount));
        self.enclosure
            .set_gain(db_amp(VOICE_COLOR_ENCLOSURE_GAIN_DB * amount));
    }

    fn process(&mut self, sample: f32) -> f32 {
        let sample = self.rumble.filter_mono(sample);
        let sample = self.body.filter_mono(sample);
        let sample = self.presence.filter_mono(sample);
        self.enclosure.filter_mono(sample)
    }
}

impl FilterState {
    fn new(sample_rate: u32, mode: FilterMode) -> Self {
        match mode {
            FilterMode::LowPass => {
                let mut channels =
                    std::array::from_fn(|_| lowpass_hz(5_000.0, std::f32::consts::FRAC_1_SQRT_2));
                for channel in &mut channels {
                    channel.set_sample_rate(f64::from(sample_rate));
                }
                Self::LowPass(channels)
            }
            FilterMode::HighPass => {
                let mut channels =
                    std::array::from_fn(|_| highpass_hz(5_000.0, std::f32::consts::FRAC_1_SQRT_2));
                for channel in &mut channels {
                    channel.set_sample_rate(f64::from(sample_rate));
                }
                Self::HighPass(channels)
            }
        }
    }

    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::FilterModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        let maximum_cutoff =
            (sample_rate as f32 * 0.45).clamp(FILTER_CUTOFF_MIN, FILTER_CUTOFF_MAX);
        let constant = match (&value.cutoff_hz.base, &value.resonance.base) {
            (TimelineBase::Const(cutoff), TimelineBase::Const(resonance)) => Some((
                cutoff.clamp(FILTER_CUTOFF_MIN, maximum_cutoff),
                resonance.clamp(FILTER_RESONANCE_MIN, FILTER_RESONANCE_MAX),
            )),
            _ => None,
        };
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let (cutoff, resonance) = constant.unwrap_or_else(|| {
                let time = local_time(local_start, frame, sample_rate);
                (
                    value
                        .cutoff_hz
                        .value_at(time)
                        .clamp(FILTER_CUTOFF_MIN, maximum_cutoff),
                    value
                        .resonance
                        .value_at(time)
                        .clamp(FILTER_RESONANCE_MIN, FILTER_RESONANCE_MAX),
                )
            });
            match self {
                Self::LowPass(filters) => {
                    for (sample, filter) in channels.iter_mut().zip(filters) {
                        if filter.cutoff() != cutoff || filter.q() != resonance {
                            filter.set_cutoff_q(cutoff, resonance);
                        }
                        *sample = filter.filter_mono(*sample);
                    }
                }
                Self::HighPass(filters) => {
                    for (sample, filter) in channels.iter_mut().zip(filters) {
                        if filter.cutoff() != cutoff || filter.q() != resonance {
                            filter.set_cutoff_q(cutoff, resonance);
                        }
                        *sample = filter.filter_mono(*sample);
                    }
                }
            }
        }
    }
}

impl NoiseGateState {
    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::NoiseGateModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        let constant = match (
            &value.threshold_db.base,
            &value.attack_ms.base,
            &value.release_ms.base,
        ) {
            (
                TimelineBase::Const(threshold),
                TimelineBase::Const(attack),
                TimelineBase::Const(release),
            ) => Some((
                threshold.clamp(-80.0, 0.0),
                attack.clamp(0.1, 500.0),
                release.clamp(1.0, 2_000.0),
            )),
            _ => None,
        };
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let (threshold_db, attack_ms, release_ms) = constant.unwrap_or_else(|| {
                let time = local_time(local_start, frame, sample_rate);
                (
                    value.threshold_db.value_at(time).clamp(-80.0, 0.0),
                    value.attack_ms.value_at(time).clamp(0.1, 500.0),
                    value.release_ms.value_at(time).clamp(1.0, 2_000.0),
                )
            });
            let release =
                crate::math::audio_smoothing_coefficient(release_ms / 1_000.0, sample_rate);
            let level = channels[0].abs().max(channels[1].abs());
            if level >= self.envelope {
                self.envelope = level;
            } else {
                self.envelope += release * (level - self.envelope);
            }
            let open_threshold = db_amp(threshold_db);
            let close_threshold = db_amp(threshold_db - GATE_HYSTERESIS_DB);
            self.open = if self.open {
                self.envelope >= close_threshold
            } else {
                self.envelope >= open_threshold
            };
            let coefficient = if self.open {
                crate::math::audio_smoothing_coefficient(attack_ms / 1_000.0, sample_rate)
            } else {
                release
            };
            let target = if self.open { 1.0 } else { 0.0 };
            self.gain += coefficient * (target - self.gain);
            channels[0] *= self.gain;
            channels[1] *= self.gain;
        }
    }
}

impl BitcrusherState {
    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::BitcrusherModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        let constant = match (
            &value.resolution_bits.base,
            &value.sample_rate_hz.base,
            &value.mix.base,
        ) {
            (
                TimelineBase::Const(resolution_bits),
                TimelineBase::Const(target_rate),
                TimelineBase::Const(mix),
            ) => Some((
                resolution_bits.clamp(2.0, 24.0),
                target_rate.clamp(1_000.0, sample_rate as f32),
                mix.clamp(0.0, 1.0),
            )),
            _ => None,
        };
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let (resolution_bits, target_rate, mix) = constant.unwrap_or_else(|| {
                let time = local_time(local_start, frame, sample_rate);
                (
                    value.resolution_bits.value_at(time).clamp(2.0, 24.0),
                    value
                        .sample_rate_hz
                        .value_at(time)
                        .clamp(1_000.0, sample_rate as f32),
                    value.mix.value_at(time).clamp(0.0, 1.0),
                )
            });
            let dry = [channels[0], channels[1]];
            if self.phase >= 1.0 {
                self.phase -= self.phase.floor();
                for (held, dry) in self.held.iter_mut().zip(dry) {
                    *held = crate::math::audio_quantize_sample(dry, resolution_bits);
                }
            }
            self.phase += target_rate / std::cmp::Ord::max(sample_rate, 1) as f32;
            for ((sample, dry), held) in channels.iter_mut().zip(dry).zip(self.held) {
                *sample = dry + (held - dry) * mix;
            }
        }
    }
}

impl ChorusState {
    fn new(sample_rate: u32) -> Self {
        let capacity = (MAX_CHORUS_DELAY_SECONDS * sample_rate as f32).ceil() as usize + 2;
        Self {
            samples: std::array::from_fn(|_| vec![0.0; capacity]),
            write: 0,
            phase: 0.0,
            sample_rate,
            warmup_frames: (MAX_CHORUS_DELAY_SECONDS * sample_rate as f32).ceil() as usize,
        }
    }

    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::ChorusModifier,
        local_start: Time,
    ) {
        let constant = match (
            &value.rate_hz.base,
            &value.depth_ms.base,
            &value.delay_ms.base,
            &value.mix.base,
        ) {
            (
                TimelineBase::Const(rate),
                TimelineBase::Const(depth),
                TimelineBase::Const(delay),
                TimelineBase::Const(mix),
            ) => Some((
                rate.clamp(0.05, 5.0),
                depth.clamp(0.0, 10.0),
                delay.clamp(5.0, 30.0),
                mix.clamp(0.0, 1.0),
            )),
            _ => None,
        };
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let (rate, depth, delay, mix) = constant.unwrap_or_else(|| {
                let time = local_time(local_start, frame, self.sample_rate);
                (
                    value.rate_hz.value_at(time).clamp(0.05, 5.0),
                    value.depth_ms.value_at(time).clamp(0.0, 10.0),
                    value.delay_ms.value_at(time).clamp(5.0, 30.0),
                    value.mix.value_at(time).clamp(0.0, 1.0),
                )
            });
            let dry = [channels[0], channels[1]];
            for channel in 0..CHANNELS {
                self.samples[channel][self.write] = dry[channel];
                let mut wet = 0.0;
                for phase_offset in CHORUS_PHASE_OFFSETS[channel] {
                    let delay_ms =
                        crate::math::audio_chorus_delay_ms(delay, depth, self.phase + phase_offset)
                            .clamp(
                                1_000.0 / std::cmp::Ord::max(self.sample_rate, 1) as f32,
                                40.0,
                            );
                    wet += delayed_sample(
                        &self.samples[channel],
                        self.write,
                        delay_ms * self.sample_rate as f32 / 1_000.0,
                    );
                }
                wet /= CHORUS_PHASE_OFFSETS[channel].len() as f32;
                channels[channel] = dry[channel] + (wet - dry[channel]) * mix;
            }
            self.write += 1;
            if self.write == self.samples[0].len() {
                self.write = 0;
            }
            self.phase +=
                std::f32::consts::TAU * rate / std::cmp::Ord::max(self.sample_rate, 1) as f32;
            if self.phase >= std::f32::consts::TAU {
                self.phase -= std::f32::consts::TAU;
            }
        }
    }
}

impl CompressorState {
    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::CompressorModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let time = local_time(local_start, frame, sample_rate);
            let threshold_db = value.threshold_db.value_at(time).clamp(-60.0, 0.0);
            let ratio = value.ratio.value_at(time).clamp(1.0, 20.0);
            let attack = value.attack_ms.value_at(time).clamp(0.01, 2_000.0) / 1_000.0;
            let release = value.release_ms.value_at(time).clamp(0.01, 9_000.0) / 1_000.0;
            let level = channels[0].abs().max(channels[1].abs());
            let coefficient = if level > self.envelope {
                (-1.0 / (attack * sample_rate as f32)).exp()
            } else {
                (-1.0 / (release * sample_rate as f32)).exp()
            };
            self.envelope = level + coefficient * (self.envelope - level);
            let envelope_db = 20.0 * self.envelope.max(1.0e-9).log10();
            let reduction_db = if envelope_db > threshold_db {
                threshold_db + (envelope_db - threshold_db) / ratio - envelope_db
            } else {
                0.0
            };
            let gain = db_gain(reduction_db + value.makeup_db.value_at(time).clamp(0.0, 36.0));
            let mix = value.mix.value_at(time).clamp(0.0, 1.0);
            channels[0] *= 1.0 + (gain - 1.0) * mix;
            channels[1] *= 1.0 + (gain - 1.0) * mix;
        }
    }
}

impl LimiterState {
    fn new(sample_rate: u32, release_ms: f32) -> Self {
        let release_ms = release_ms.clamp(1.0, 8_000.0);
        let mut unit: Box<dyn AudioUnit> = Box::new(limiter_stereo(0.005, release_ms / 1_000.0));
        unit.set_sample_rate(f64::from(sample_rate));
        let latency_frames = unit.latency().unwrap_or(0.0).round() as usize;
        Self {
            unit,
            release_ms,
            latency_frames,
        }
    }

    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::LimiterModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        let release_ms = value.release_ms.value_at(local_start).clamp(1.0, 8_000.0);
        if (release_ms - self.release_ms).abs() > f32::EPSILON {
            *self = Self::new(sample_rate, release_ms);
        }
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let ceiling = db_gain(
                value
                    .ceiling_db
                    .value_at(local_time(local_start, frame, sample_rate))
                    .clamp(-24.0, 0.0),
            )
            .max(f32::EPSILON);
            let (left, right) = self
                .unit
                .filter_stereo(channels[0] / ceiling, channels[1] / ceiling);
            channels[0] = left * ceiling;
            channels[1] = right * ceiling;
        }
    }
}

impl DelayLine {
    fn new(frames: usize) -> Self {
        Self {
            samples: vec![0.0; std::cmp::Ord::max(frames, 2)],
            write: 0,
            filtered: 0.0,
        }
    }

    fn comb(&mut self, input: f32, delay_frames: f32, feedback: f32, damping: f32) -> f32 {
        let delayed = delayed_sample(&self.samples, self.write, delay_frames);
        self.filtered += damping * (delayed - self.filtered);
        self.samples[self.write] = input + self.filtered * feedback;
        self.write += 1;
        if self.write == self.samples.len() {
            self.write = 0;
        }
        delayed
    }

    fn allpass(&mut self, input: f32, delay_frames: f32) -> f32 {
        let delayed = delayed_sample(&self.samples, self.write, delay_frames);
        self.samples[self.write] = input + delayed * REVERB_ALLPASS_FEEDBACK;
        self.write += 1;
        if self.write == self.samples.len() {
            self.write = 0;
        }
        delayed - input
    }
}

impl ReverbState {
    fn new(sample_rate: u32, room_size: f32, pre_delay_ms: f32, mode: ReverbMode) -> Self {
        let pre_delay_capacity =
            (MAX_REVERB_PRE_DELAY_SECONDS * sample_rate as f32).ceil() as usize + 2;
        let capture_capacity =
            (MAX_ROOM_CAPTURE_DELAY_SECONDS * sample_rate as f32).ceil() as usize + 2;
        let combs = std::array::from_fn(|channel| {
            std::array::from_fn(|line| {
                let frames =
                    REVERB_COMB_DELAYS[channel][line] * REVERB_ROOM_SCALE_MAX * sample_rate as f32;
                DelayLine::new(frames.ceil() as usize + 2)
            })
        });
        let allpasses = std::array::from_fn(|channel| {
            std::array::from_fn(|line| {
                let frames = REVERB_ALLPASS_DELAYS[channel][line]
                    * REVERB_ROOM_SCALE_MAX
                    * sample_rate as f32;
                DelayLine::new(frames.ceil() as usize + 2)
            })
        });
        let room_scale = REVERB_ROOM_SCALE_MIN
            + room_size.clamp(0.0, 1.0) * (REVERB_ROOM_SCALE_MAX - REVERB_ROOM_SCALE_MIN);
        let reflection_seconds = REVERB_COMB_DELAYS
            .iter()
            .flatten()
            .copied()
            .fold(0.0_f32, f32::max)
            + REVERB_ALLPASS_DELAYS[0].iter().sum::<f32>();
        let warmup_seconds = match mode {
            ReverbMode::Classic => {
                pre_delay_ms.clamp(0.0, 500.0) / 1_000.0 + reflection_seconds * room_scale
            }
            ReverbMode::RoomCapture => MAX_ROOM_CAPTURE_DELAY_SECONDS,
        };
        Self {
            pre_delay: std::array::from_fn(|_| vec![0.0; pre_delay_capacity]),
            pre_delay_write: 0,
            combs,
            allpasses,
            sample_rate,
            pre_delay_frames: pre_delay_ms.clamp(0.0, 500.0) * sample_rate as f32 / 1_000.0,
            room_size: f32::NAN,
            decay_seconds: f32::NAN,
            damping: f32::NAN,
            comb_delay_frames: [[0.0; 4]; CHANNELS],
            comb_feedback: [[0.0; 4]; CHANNELS],
            allpass_delay_frames: [[0.0; 2]; CHANNELS],
            damping_coefficient: 0.0,
            capture_input: std::array::from_fn(|_| vec![0.0; capture_capacity]),
            capture_write: 0,
            capture_filtered: [[0.0; shrimply_math_media::AUDIO_ROOM_REFLECTIONS]; CHANNELS],
            capture_delay_frames: [0.0; shrimply_math_media::AUDIO_ROOM_REFLECTIONS],
            capture_gains: [0.0; shrimply_math_media::AUDIO_ROOM_REFLECTIONS],
            capture_filter_coefficient: 0.0,
            capture_late_gain: 0.0,
            capture_normalization: 1.0,
            capture_room_size: f32::NAN,
            capture_distance_m: f32::NAN,
            capture_absorption: f32::NAN,
            warmup_frames: (warmup_seconds * sample_rate as f32) as usize,
        }
    }

    fn update_network(&mut self, room_size: f32, decay_seconds: f32, damping: f32) {
        if room_size != self.room_size || decay_seconds != self.decay_seconds {
            let room_scale =
                REVERB_ROOM_SCALE_MIN + room_size * (REVERB_ROOM_SCALE_MAX - REVERB_ROOM_SCALE_MIN);
            for channel in 0..CHANNELS {
                for (line, base_delay) in REVERB_COMB_DELAYS[channel].iter().enumerate() {
                    let delay_frames = base_delay * room_scale * self.sample_rate as f32;
                    self.comb_delay_frames[channel][line] = delay_frames;
                    self.comb_feedback[channel][line] = crate::math::audio_decay_feedback(
                        delay_frames / self.sample_rate as f32,
                        decay_seconds,
                    );
                }
                for (line, base_delay) in REVERB_ALLPASS_DELAYS[channel].iter().enumerate() {
                    self.allpass_delay_frames[channel][line] =
                        base_delay * room_scale * self.sample_rate as f32;
                }
            }
            self.room_size = room_size;
            self.decay_seconds = decay_seconds;
        }
        if damping != self.damping {
            let cutoff = crate::math::audio_geometric_lerp(
                REVERB_DAMPING_CUTOFF_MAX,
                REVERB_DAMPING_CUTOFF_MIN,
                damping,
            )
            .min(self.sample_rate as f32 * 0.45);
            self.damping_coefficient =
                crate::math::audio_lowpass_coefficient(cutoff, self.sample_rate);
            self.damping = damping;
        }
    }

    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::ReverbModifier,
        local_start: Time,
    ) {
        match value.mode {
            ReverbMode::Classic => self.process_classic(samples, value, local_start),
            ReverbMode::RoomCapture => self.process_capture(samples, value, local_start),
        }
    }

    fn process_classic(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::ReverbModifier,
        local_start: Time,
    ) {
        let delay_smoothing =
            crate::math::audio_smoothing_coefficient(DELAY_SMOOTH_SECONDS, self.sample_rate);
        let constant = match (
            &value.room_size.base,
            &value.decay_seconds.base,
            &value.damping.base,
            &value.pre_delay_ms.base,
            &value.mix.base,
        ) {
            (
                TimelineBase::Const(room_size),
                TimelineBase::Const(decay_seconds),
                TimelineBase::Const(damping),
                TimelineBase::Const(pre_delay_ms),
                TimelineBase::Const(mix),
            ) => Some((
                room_size.clamp(0.0, 1.0),
                decay_seconds.clamp(0.1, 20.0),
                damping.clamp(0.0, 1.0),
                pre_delay_ms.clamp(0.0, 500.0),
                mix.clamp(0.0, 1.0),
            )),
            _ => None,
        };
        if let Some((room_size, decay_seconds, damping, _, _)) = constant {
            self.update_network(room_size, decay_seconds, damping);
        }
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let (room_size, decay_seconds, damping, pre_delay_ms, mix) =
                constant.unwrap_or_else(|| {
                    let time = local_time(local_start, frame, self.sample_rate);
                    (
                        value.room_size.value_at(time).clamp(0.0, 1.0),
                        value.decay_seconds.value_at(time).clamp(0.1, 20.0),
                        value.damping.value_at(time).clamp(0.0, 1.0),
                        value.pre_delay_ms.value_at(time).clamp(0.0, 500.0),
                        value.mix.value_at(time).clamp(0.0, 1.0),
                    )
                });
            if constant.is_none() {
                self.update_network(room_size, decay_seconds, damping);
            }
            let target_pre_delay = pre_delay_ms * self.sample_rate as f32 / 1_000.0;
            self.pre_delay_frames += delay_smoothing * (target_pre_delay - self.pre_delay_frames);

            let dry = [channels[0], channels[1]];
            for (channel, sample) in dry.iter().copied().enumerate() {
                self.pre_delay[channel][self.pre_delay_write] = sample;
            }
            let delayed: [f32; CHANNELS] = std::array::from_fn(|channel| {
                delayed_sample(
                    &self.pre_delay[channel],
                    self.pre_delay_write,
                    self.pre_delay_frames,
                )
            });
            self.pre_delay_write += 1;
            if self.pre_delay_write == self.pre_delay[0].len() {
                self.pre_delay_write = 0;
            }

            let mut wet = [0.0; CHANNELS];
            for channel in 0..CHANNELS {
                for line in 0..self.combs[channel].len() {
                    wet[channel] += self.combs[channel][line].comb(
                        delayed[channel],
                        self.comb_delay_frames[channel][line],
                        self.comb_feedback[channel][line],
                        self.damping_coefficient,
                    );
                }
                wet[channel] /= self.combs[channel].len() as f32;
                for line in 0..self.allpasses[channel].len() {
                    wet[channel] = self.allpasses[channel][line]
                        .allpass(wet[channel], self.allpass_delay_frames[channel][line]);
                }
            }
            for channel in 0..CHANNELS {
                channels[channel] = dry[channel] + (wet[channel] - dry[channel]) * mix;
            }
        }
    }

    fn process_capture(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::ReverbModifier,
        local_start: Time,
    ) {
        let constant = match (
            &value.room_size.base,
            &value.damping.base,
            &value.distance_m.base,
        ) {
            (
                TimelineBase::Const(room_size),
                TimelineBase::Const(absorption),
                TimelineBase::Const(distance_m),
            ) => Some((
                room_size.clamp(0.0, 1.0),
                absorption.clamp(0.0, 1.0),
                distance_m.clamp(ROOM_CAPTURE_DISTANCE_MIN_M, ROOM_CAPTURE_DISTANCE_MAX_M),
            )),
            _ => None,
        };
        if let Some((room_size, absorption, distance_m)) = constant {
            self.update_capture_network(room_size, distance_m, absorption);
        }
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let (room_size, absorption, distance_m) = constant.unwrap_or_else(|| {
                let time = local_time(local_start, frame, self.sample_rate);
                (
                    value.room_size.value_at(time).clamp(0.0, 1.0),
                    value.damping.value_at(time).clamp(0.0, 1.0),
                    value
                        .distance_m
                        .value_at(time)
                        .clamp(ROOM_CAPTURE_DISTANCE_MIN_M, ROOM_CAPTURE_DISTANCE_MAX_M),
                )
            });
            if constant.is_none() {
                self.update_capture_network(room_size, distance_m, absorption);
            }

            let dry = [channels[0], channels[1]];
            let mut early = [0.0; CHANNELS];
            for channel in 0..CHANNELS {
                self.capture_input[channel][self.capture_write] = dry[channel];
                for reflection in 0..shrimply_math_media::AUDIO_ROOM_REFLECTIONS {
                    let delayed = delayed_sample(
                        &self.capture_input[channel],
                        self.capture_write,
                        self.capture_delay_frames[reflection],
                    );
                    let filtered = &mut self.capture_filtered[channel][reflection];
                    *filtered += self.capture_filter_coefficient * (delayed - *filtered);
                    early[channel] += *filtered * self.capture_gains[reflection];
                }
            }
            self.capture_write += 1;
            if self.capture_write == self.capture_input[0].len() {
                self.capture_write = 0;
            }

            let mut late = [0.0; CHANNELS];
            for channel in 0..CHANNELS {
                let input = dry[channel] + early[channel] * 0.5;
                for line in 0..self.combs[channel].len() {
                    late[channel] += self.combs[channel][line].comb(
                        input,
                        self.comb_delay_frames[channel][line],
                        self.comb_feedback[channel][line],
                        self.damping_coefficient,
                    );
                }
                late[channel] /= self.combs[channel].len() as f32;
                for line in 0..self.allpasses[channel].len() {
                    late[channel] = self.allpasses[channel][line]
                        .allpass(late[channel], self.allpass_delay_frames[channel][line]);
                }
                channels[channel] =
                    (dry[channel] + early[channel] + late[channel] * self.capture_late_gain)
                        * self.capture_normalization;
            }
        }
    }

    fn update_capture_network(&mut self, room_size: f32, distance_m: f32, absorption: f32) {
        if room_size == self.capture_room_size
            && distance_m == self.capture_distance_m
            && absorption == self.capture_absorption
        {
            return;
        }

        let reflection_coefficient = (1.0 - absorption).sqrt();
        let reflections = crate::math::audio_room_reflections(distance_m, room_size);
        for (index, reflection) in reflections.into_iter().enumerate() {
            self.capture_delay_frames[index] = reflection.delay_seconds * self.sample_rate as f32;
            self.capture_gains[index] =
                reflection.relative_gain * reflection_coefficient * ROOM_CAPTURE_EARLY_GAIN;
        }
        let cutoff = crate::math::audio_geometric_lerp(
            ROOM_CAPTURE_REFLECTION_CUTOFF_MAX,
            ROOM_CAPTURE_REFLECTION_CUTOFF_MIN,
            absorption,
        )
        .min(self.sample_rate as f32 * 0.45);
        self.capture_filter_coefficient =
            crate::math::audio_lowpass_coefficient(cutoff, self.sample_rate);
        let distance_amount = (distance_m - ROOM_CAPTURE_DISTANCE_MIN_M)
            / (ROOM_CAPTURE_DISTANCE_MAX_M - ROOM_CAPTURE_DISTANCE_MIN_M);
        self.capture_late_gain = (ROOM_CAPTURE_LATE_GAIN_MIN
            + (ROOM_CAPTURE_LATE_GAIN_MAX - ROOM_CAPTURE_LATE_GAIN_MIN) * distance_amount)
            * reflection_coefficient;
        let reflected_energy = self
            .capture_gains
            .iter()
            .map(|gain| gain * gain)
            .sum::<f32>()
            + self.capture_late_gain * self.capture_late_gain;
        self.capture_normalization = (1.0 + reflected_energy).sqrt().recip();
        self.update_network(
            room_size,
            crate::math::audio_room_decay_seconds(room_size, absorption),
            absorption,
        );
        self.capture_room_size = room_size;
        self.capture_distance_m = distance_m;
        self.capture_absorption = absorption;
    }
}

impl EchoState {
    fn new(sample_rate: u32, delay_ms: f32) -> Self {
        let capacity = (MAX_ECHO_DELAY_SECONDS * sample_rate as f32).ceil() as usize + 2;
        let delay_seconds = delay_ms.clamp(1.0, 2_000.0) / 1_000.0;
        Self {
            samples: std::array::from_fn(|_| vec![0.0; capacity]),
            write: 0,
            sample_rate,
            delay_frames: delay_ms.clamp(1.0, 2_000.0) * sample_rate as f32 / 1_000.0,
            warmup_frames: (delay_seconds * sample_rate as f32) as usize,
        }
    }

    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::EchoModifier,
        local_start: Time,
    ) {
        let smoothing =
            crate::math::audio_smoothing_coefficient(DELAY_SMOOTH_SECONDS, self.sample_rate);
        let constant = match (&value.delay_ms.base, &value.feedback.base, &value.mix.base) {
            (
                TimelineBase::Const(delay_ms),
                TimelineBase::Const(feedback),
                TimelineBase::Const(mix),
            ) => Some((
                delay_ms.clamp(1.0, 2_000.0),
                feedback.clamp(0.0, 0.95),
                mix.clamp(0.0, 1.0),
            )),
            _ => None,
        };
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let (delay_ms, feedback, mix) = constant.unwrap_or_else(|| {
                let time = local_time(local_start, frame, self.sample_rate);
                (
                    value.delay_ms.value_at(time).clamp(1.0, 2_000.0),
                    value.feedback.value_at(time).clamp(0.0, 0.95),
                    value.mix.value_at(time).clamp(0.0, 1.0),
                )
            });
            let target_delay = delay_ms * self.sample_rate as f32 / 1_000.0;
            self.delay_frames += smoothing * (target_delay - self.delay_frames);
            let dry = [channels[0], channels[1]];
            let wet: [f32; CHANNELS] = std::array::from_fn(|channel| {
                delayed_sample(&self.samples[channel], self.write, self.delay_frames)
            });
            if value.ping_pong {
                self.samples[0][self.write] = (dry[0] + dry[1]) * 0.5 + wet[1] * feedback;
                self.samples[1][self.write] = wet[0] * feedback;
            } else {
                for channel in 0..CHANNELS {
                    self.samples[channel][self.write] = dry[channel] + wet[channel] * feedback;
                }
            }
            self.write += 1;
            if self.write == self.samples[0].len() {
                self.write = 0;
            }
            for channel in 0..CHANNELS {
                channels[channel] = dry[channel] + wet[channel] * mix;
            }
        }
    }
}

impl DistortionState {
    fn new(drive_db: f32) -> Self {
        let drive_db = drive_db.clamp(0.0, 48.0);
        Self {
            tone: [0.0; CHANNELS],
            dry_power: 0.0,
            wet_power: 0.0,
            compensation: db_gain(-drive_db),
            drive_db,
            drive: db_gain(drive_db),
            tone_amount: f32::NAN,
            tone_coefficient: 0.0,
        }
    }

    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::DistortionModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        let level_coefficient =
            crate::math::audio_smoothing_coefficient(LEVEL_WINDOW_SECONDS, sample_rate);
        let compensation_coefficient =
            crate::math::audio_smoothing_coefficient(LEVEL_SMOOTH_SECONDS, sample_rate);
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            let time = local_time(local_start, frame, sample_rate);
            let drive_db = value.drive_db.value_at(time).clamp(0.0, 48.0);
            if drive_db != self.drive_db {
                self.drive_db = drive_db;
                self.drive = db_gain(drive_db);
            }
            let tone_amount = value.tone.value_at(time).clamp(0.0, 1.0);
            if tone_amount != self.tone_amount {
                let cutoff = crate::math::audio_geometric_lerp(
                    DISTORTION_TONE_CUTOFF_MIN,
                    DISTORTION_TONE_CUTOFF_MAX,
                    tone_amount,
                )
                .min(sample_rate as f32 * 0.45);
                self.tone_coefficient = crate::math::audio_lowpass_coefficient(cutoff, sample_rate);
                self.tone_amount = tone_amount;
            }
            let dry = [channels[0], channels[1]];
            for (tone, dry) in self.tone.iter_mut().zip(dry) {
                let clipped = (dry * self.drive).clamp(-1.0, 1.0);
                *tone += self.tone_coefficient * (clipped - *tone);
            }
            let wet = self.tone;
            let dry_power = (dry[0] * dry[0] + dry[1] * dry[1]) * 0.5;
            let wet_power = (wet[0] * wet[0] + wet[1] * wet[1]) * 0.5;
            self.dry_power += level_coefficient * (dry_power - self.dry_power);
            self.wet_power += level_coefficient * (wet_power - self.wet_power);
            if self.dry_power > f32::EPSILON && self.wet_power > f32::EPSILON {
                let target = (self.dry_power / self.wet_power)
                    .sqrt()
                    .clamp(MIN_LEVEL_COMPENSATION, MAX_LEVEL_COMPENSATION);
                self.compensation += compensation_coefficient * (target - self.compensation);
            }
            let mix = value.mix.value_at(time).clamp(0.0, 1.0);
            for channel in 0..CHANNELS {
                let wet = wet[channel] * self.compensation;
                channels[channel] = (dry[channel] + (wet - dry[channel]) * mix).clamp(-1.0, 1.0);
            }
        }
    }
}

fn delayed_sample(samples: &[f32], write: usize, delay_frames: f32) -> f32 {
    let mut position = write as f32 - delay_frames;
    if position < 0.0 {
        position += samples.len() as f32;
    }
    debug_assert!(position >= 0.0 && position < samples.len() as f32);
    let before = position as usize;
    let after = if before + 1 == samples.len() {
        0
    } else {
        before + 1
    };
    let amount = position - before as f32;
    samples[before] + (samples[after] - samples[before]) * amount
}

impl PitchState {
    fn new(
        sample_rate: u32,
        value: &shrimply_audio_modifiers::PitchModifier,
    ) -> Result<Self, String> {
        let mut options = match value.quality {
            PitchQuality::LowLatency => 0,
            PitchQuality::Balanced => RUBBERBAND_LIVE_WINDOW_MEDIUM,
        };
        if value.preserve_formants {
            options |= RUBBERBAND_FORMANT_PRESERVED;
        }
        if value.link_channels {
            options |= RUBBERBAND_CHANNELS_TOGETHER;
        }
        let state =
            NonNull::new(unsafe { rubberband_live_new(sample_rate, CHANNELS as u32, options) })
                .ok_or("could not initialize Rubber Band live pitch shifter")?;
        let block_size = unsafe { rubberband_live_get_block_size(state.as_ptr()) } as usize;
        let start_delay = unsafe { rubberband_live_get_start_delay(state.as_ptr()) } as usize;
        if block_size == 0 {
            unsafe { rubberband_live_delete(state.as_ptr()) };
            return Err("Rubber Band returned a zero live block size".to_string());
        }
        Ok(Self {
            state,
            block_size,
            start_delay,
            input: std::array::from_fn(|_| Vec::with_capacity(block_size)),
            output: std::array::from_fn(|_| vec![0.0; block_size]),
            ready: std::array::from_fn(|_| VecDeque::with_capacity(block_size * 2)),
            pending_pitch: 1.0,
            pending_formant: 0.0,
        })
    }

    fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::PitchModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        unsafe {
            rubberband_live_set_formant_option(
                self.state.as_ptr(),
                if value.preserve_formants {
                    RUBBERBAND_FORMANT_PRESERVED
                } else {
                    0
                },
            );
        }
        for (frame, channels) in samples.chunks_exact_mut(CHANNELS).enumerate() {
            if self.input[0].is_empty() {
                let semitones = value
                    .semitones
                    .value_at(local_time(local_start, frame, sample_rate))
                    .clamp(-24.0, 24.0);
                self.pending_pitch = 2.0_f32.powf(semitones / 12.0);
                self.pending_formant = if value.preserve_formants {
                    0.0
                } else {
                    let formant_semitones = value
                        .formant_semitones
                        .value_at(local_time(local_start, frame, sample_rate))
                        .clamp(-12.0, 12.0);
                    2.0_f32.powf(formant_semitones / 12.0)
                };
            }
            for (channel, sample) in channels.iter().copied().enumerate() {
                self.input[channel].push(sample);
            }
            if self.input[0].len() == self.block_size {
                let input = self.input.each_ref().map(|channel| channel.as_ptr());
                let mut output = self.output.each_mut().map(|channel| channel.as_mut_ptr());
                unsafe {
                    rubberband_live_set_pitch_scale(
                        self.state.as_ptr(),
                        f64::from(self.pending_pitch),
                    );
                    rubberband_live_set_formant_scale(
                        self.state.as_ptr(),
                        f64::from(self.pending_formant),
                    );
                    rubberband_live_shift(self.state.as_ptr(), input.as_ptr(), output.as_mut_ptr());
                }
                for channel in 0..CHANNELS {
                    self.ready[channel].extend(&self.output[channel]);
                    self.input[channel].clear();
                }
            }
            for (channel, sample) in channels.iter_mut().enumerate() {
                *sample = self.ready[channel].pop_front().unwrap_or(0.0);
            }
        }
    }
}

impl Drop for PitchState {
    fn drop(&mut self) {
        unsafe { rubberband_live_delete(self.state.as_ptr()) };
    }
}

fn local_time(start: Time, frame: usize, sample_rate: u32) -> Time {
    start.saturating_add(Time::from_fraction(
        std::cmp::Ord::min(frame, i64::MAX as usize) as i64,
        i64::from(std::cmp::Ord::max(sample_rate, 1)),
    ))
}

pub(crate) fn db_gain(decibels: f32) -> f32 {
    if decibels <= -60.0 {
        0.0
    } else {
        10.0_f32.powf(decibels / 20.0)
    }
}

#[cfg_attr(target_os = "android", link(name = "rubberband", kind = "static"))]
#[cfg_attr(not(target_os = "android"), link(name = "rubberband"))]
unsafe extern "C" {
    fn rubberband_live_new(sample_rate: u32, channels: u32, options: i32) -> *mut c_void;
    fn rubberband_live_delete(state: *mut c_void);
    fn rubberband_live_set_pitch_scale(state: *mut c_void, scale: f64);
    fn rubberband_live_set_formant_scale(state: *mut c_void, scale: f64);
    fn rubberband_live_set_formant_option(state: *mut c_void, options: i32);
    fn rubberband_live_get_start_delay(state: *const c_void) -> u32;
    fn rubberband_live_get_block_size(state: *mut c_void) -> u32;
    fn rubberband_live_shift(state: *mut c_void, input: *const *const f32, output: *mut *mut f32);
}
