use shrimply_audio_modifiers::{
    AudioModifierEffect, CacheModifier, DenoiseEngine, PNEUMA_MAX_PITCH_OFFSET, PNEUMA_MAX_SPEED,
    PNEUMA_MIN_PITCH_OFFSET, PNEUMA_MIN_SPEED, ReverbMode,
};
use shrimply_editor_state::player_state::{self, SharedPlayerState};
use shrimply_project_document::project::{AudioSource, ItemAddress, Project, Time};
use shrimply_property_model::{modifier_model::ModifierModel, timeline_value::TimelineValue};
use std::sync::{Mutex, OnceLock};

use crate::{AudioModifierChoice, InspectorTarget};

type VoiceModelCatalog = Vec<(String, Vec<String>)>;
static VOICE_MODELS: OnceLock<Mutex<VoiceModelCatalog>> = OnceLock::new();

#[derive(Clone)]
pub struct AudioModifierOption {
    pub value: &'static str,
    pub label: &'static str,
}

#[derive(Clone, Copy)]
pub struct AudioModifierScalarPresentation {
    pub minimum: f64,
    pub maximum: f64,
    pub drag_step: f64,
    pub digits: usize,
    pub unit: Option<&'static str>,
    pub store_multiplier: f64,
    pub display: fn(f32) -> f64,
    pub store: fn(f64) -> f32,
}

#[derive(Clone)]
pub enum AudioModifierControl {
    Cache(CacheModifier),
    Scalar {
        path: String,
        label: &'static str,
        value: TimelineValue<f32>,
        presentation: AudioModifierScalarPresentation,
    },
    Boolean {
        path: String,
        label: &'static str,
        value: bool,
    },
    Selector {
        path: String,
        label: &'static str,
        value: String,
        options: Vec<AudioModifierOption>,
    },
    Number {
        path: String,
        label: &'static str,
        value: f64,
        minimum: f64,
        maximum: f64,
        step: f64,
        digits: i32,
    },
    VoiceModel {
        path: String,
        label: &'static str,
        value: String,
    },
}

pub fn audio_modifier_controls(effect: &AudioModifierEffect) -> Vec<AudioModifierControl> {
    match effect {
        AudioModifierEffect::Cache(value) => vec![AudioModifierControl::Cache(value.clone())],
        AudioModifierEffect::Gain(value) => vec![scalar(
            "decibels",
            "Level",
            &value.decibels,
            -60.0,
            36.0,
            Some("dB"),
        )],
        AudioModifierEffect::Pan(value) => vec![scalar(
            "position",
            "Position",
            &value.position,
            -1.0,
            1.0,
            None,
        )],
        AudioModifierEffect::Pitch(value) => {
            let mut controls = vec![
                selector(
                    "quality",
                    "Quality",
                    match value.quality {
                        shrimply_audio_modifiers::PitchQuality::Balanced => "balanced",
                        shrimply_audio_modifiers::PitchQuality::LowLatency => "low_latency",
                    },
                    &[("balanced", "Balanced"), ("low_latency", "Low latency")],
                ),
                scalar(
                    "semitones",
                    "Semitones",
                    &value.semitones,
                    -24.0,
                    24.0,
                    Some("st"),
                ),
                boolean(
                    "preserve_formants",
                    "Preserve formants",
                    value.preserve_formants,
                ),
            ];
            if !value.preserve_formants {
                controls.push(scalar(
                    "formant_semitones",
                    "Formant shift",
                    &value.formant_semitones,
                    -12.0,
                    12.0,
                    Some("st"),
                ));
            }
            controls.push(boolean(
                "link_channels",
                "Link stereo channels",
                value.link_channels,
            ));
            controls
        }
        AudioModifierEffect::Denoise(value) => {
            let mut controls = vec![
                selector(
                    "engine",
                    "Engine",
                    match value.engine {
                        DenoiseEngine::Rnnoise => "rnnoise",
                        DenoiseEngine::DeepFilterNet => "deep_filter_net",
                    },
                    &[("rnnoise", "RNNoise"), ("deep_filter_net", "DeepFilterNet")],
                ),
                scalar("amount", "Amount", &value.amount, 0.0, 1.0, Some("%")),
            ];
            if value.engine == DenoiseEngine::DeepFilterNet {
                controls.push(scalar(
                    "reduction_db",
                    "Reduction",
                    &value.reduction_db,
                    0.01,
                    97.0,
                    Some("dB"),
                ));
            }
            controls
        }
        AudioModifierEffect::Equalizer(value) => vec![
            scalar("low_db", "Low", &value.low_db, -24.0, 24.0, Some("dB")),
            scalar("mid_db", "Mid", &value.mid_db, -24.0, 24.0, Some("dB")),
            scalar("high_db", "High", &value.high_db, -24.0, 24.0, Some("dB")),
        ],
        AudioModifierEffect::Filter(value) => vec![
            selector(
                "mode",
                "Mode",
                match value.mode {
                    shrimply_audio_modifiers::FilterMode::LowPass => "low_pass",
                    shrimply_audio_modifiers::FilterMode::HighPass => "high_pass",
                },
                &[("low_pass", "Low-pass"), ("high_pass", "High-pass")],
            ),
            scalar(
                "cutoff_hz",
                "Cutoff",
                &value.cutoff_hz,
                20.0,
                20_000.0,
                Some("Hz"),
            ),
            scalar(
                "resonance",
                "Resonance",
                &value.resonance,
                0.5,
                10.0,
                Some("Q"),
            ),
        ],
        AudioModifierEffect::NoiseGate(value) => vec![
            scalar(
                "threshold_db",
                "Threshold",
                &value.threshold_db,
                -80.0,
                0.0,
                Some("dB"),
            ),
            scalar(
                "attack_ms",
                "Attack",
                &value.attack_ms,
                0.1,
                500.0,
                Some("ms"),
            ),
            scalar(
                "release_ms",
                "Release",
                &value.release_ms,
                1.0,
                2_000.0,
                Some("ms"),
            ),
        ],
        AudioModifierEffect::StereoWidth(value) => {
            vec![scalar("width", "Width", &value.width, 0.0, 2.0, Some("%"))]
        }
        AudioModifierEffect::Tremolo(value) => vec![
            scalar("rate_hz", "Rate", &value.rate_hz, 0.1, 20.0, Some("Hz")),
            scalar("depth", "Depth", &value.depth, 0.0, 1.0, Some("%")),
        ],
        AudioModifierEffect::Bitcrusher(value) => vec![
            scalar(
                "resolution_bits",
                "Resolution",
                &value.resolution_bits,
                2.0,
                24.0,
                Some("bit"),
            ),
            scalar(
                "sample_rate_hz",
                "Sample rate",
                &value.sample_rate_hz,
                1_000.0,
                48_000.0,
                Some("Hz"),
            ),
            scalar("mix", "Mix", &value.mix, 0.0, 1.0, Some("%")),
        ],
        AudioModifierEffect::Chorus(value) => vec![
            scalar("rate_hz", "Rate", &value.rate_hz, 0.05, 5.0, Some("Hz")),
            scalar("depth_ms", "Depth", &value.depth_ms, 0.0, 10.0, Some("ms")),
            scalar("delay_ms", "Delay", &value.delay_ms, 5.0, 30.0, Some("ms")),
            scalar("mix", "Mix", &value.mix, 0.0, 1.0, Some("%")),
        ],
        AudioModifierEffect::Compressor(value) => vec![
            scalar(
                "threshold_db",
                "Threshold",
                &value.threshold_db,
                -60.0,
                0.0,
                Some("dB"),
            ),
            scalar("ratio", "Ratio", &value.ratio, 1.0, 20.0, Some(":1")),
            scalar(
                "attack_ms",
                "Attack",
                &value.attack_ms,
                0.01,
                2_000.0,
                Some("ms"),
            ),
            scalar(
                "release_ms",
                "Release",
                &value.release_ms,
                0.01,
                9_000.0,
                Some("ms"),
            ),
            scalar(
                "makeup_db",
                "Makeup",
                &value.makeup_db,
                0.0,
                36.0,
                Some("dB"),
            ),
            scalar("mix", "Mix", &value.mix, 0.0, 1.0, None),
        ],
        AudioModifierEffect::Limiter(value) => vec![
            scalar(
                "ceiling_db",
                "Ceiling",
                &value.ceiling_db,
                -24.0,
                0.0,
                Some("dB"),
            ),
            scalar(
                "release_ms",
                "Release",
                &value.release_ms,
                1.0,
                8_000.0,
                Some("ms"),
            ),
        ],
        AudioModifierEffect::Reverb(value) => {
            let mut controls = vec![selector(
                "mode",
                "Mode",
                match value.mode {
                    ReverbMode::RoomCapture => "room_capture",
                    ReverbMode::Classic => "classic",
                },
                &[("room_capture", "Room capture"), ("classic", "Classic")],
            )];
            if value.mode == ReverbMode::RoomCapture {
                controls.push(scalar(
                    "distance_m",
                    "Distance",
                    &value.distance_m,
                    0.2,
                    5.0,
                    Some("m"),
                ));
            }
            controls.push(scalar(
                "room_size",
                "Room scale",
                &value.room_size,
                0.0,
                1.0,
                Some("%"),
            ));
            if value.mode == ReverbMode::RoomCapture {
                controls.push(scalar(
                    "damping",
                    "Absorption",
                    &value.damping,
                    0.0,
                    1.0,
                    Some("%"),
                ));
            } else {
                controls.extend([
                    scalar(
                        "decay_seconds",
                        "Decay",
                        &value.decay_seconds,
                        0.1,
                        20.0,
                        Some("s"),
                    ),
                    scalar("damping", "Damping", &value.damping, 0.0, 1.0, Some("%")),
                    scalar(
                        "pre_delay_ms",
                        "Pre-delay",
                        &value.pre_delay_ms,
                        0.0,
                        500.0,
                        Some("ms"),
                    ),
                    scalar("mix", "Mix", &value.mix, 0.0, 1.0, Some("%")),
                ]);
            }
            controls
        }
        AudioModifierEffect::CloseUp(value) => vec![scalar(
            "distance_cm",
            "Distance",
            &value.distance_cm,
            3.0,
            100.0,
            Some("cm"),
        )],
        AudioModifierEffect::VoiceColor(value) => vec![
            scalar(
                "amount",
                "Effect strength",
                &value.amount,
                0.0,
                1.0,
                Some("%"),
            ),
            boolean("auto_level", "Auto level", value.auto_level),
        ],
        AudioModifierEffect::Echo(value) => vec![
            scalar(
                "delay_ms",
                "Delay",
                &value.delay_ms,
                1.0,
                2_000.0,
                Some("ms"),
            ),
            scalar(
                "feedback",
                "Feedback",
                &value.feedback,
                0.0,
                0.95,
                Some("%"),
            ),
            boolean("ping_pong", "Ping-pong", value.ping_pong),
            scalar("mix", "Mix", &value.mix, 0.0, 1.0, Some("%")),
        ],
        AudioModifierEffect::Distortion(value) => vec![
            scalar("drive_db", "Drive", &value.drive_db, 0.0, 48.0, Some("dB")),
            scalar("tone", "Tone", &value.tone, 0.0, 1.0, Some("%")),
            scalar("mix", "Mix", &value.mix, 0.0, 1.0, Some("%")),
        ],
        AudioModifierEffect::VoiceChange(value) => vec![
            AudioModifierControl::VoiceModel {
                path: "/effect/config/model".to_string(),
                label: "Model",
                value: value.model.clone(),
            },
            AudioModifierControl::Number {
                path: "/effect/config/pitch_offset".to_string(),
                label: "Pitch offset",
                value: f64::from(value.pitch_offset),
                minimum: f64::from(PNEUMA_MIN_PITCH_OFFSET),
                maximum: f64::from(PNEUMA_MAX_PITCH_OFFSET),
                step: 1.0,
                digits: 0,
            },
            selector(
                "f0_method",
                "F0 method",
                match value.f0_method {
                    shrimply_audio_modifiers::F0Method::Crepe => "crepe",
                    shrimply_audio_modifiers::F0Method::Rmvpe => "rmvpe",
                    shrimply_audio_modifiers::F0Method::Fcpe => "fcpe",
                    shrimply_audio_modifiers::F0Method::SwiftF0 => "swift-f0",
                },
                &[
                    ("crepe", "CREPE"),
                    ("rmvpe", "RMVPE"),
                    ("fcpe", "FCPE"),
                    ("swift-f0", "Swift F0"),
                ],
            ),
            AudioModifierControl::Number {
                path: "/effect/config/speed".to_string(),
                label: "Speed",
                value: f64::from(value.speed),
                minimum: f64::from(PNEUMA_MIN_SPEED),
                maximum: f64::from(PNEUMA_MAX_SPEED),
                step: 0.1,
                digits: 1,
            },
            boolean(
                "maintain_pitch",
                "Maintain pitch while changing speed",
                value.maintain_pitch,
            ),
        ],
    }
}

fn scalar(
    field: &'static str,
    label: &'static str,
    value: &TimelineValue<f32>,
    minimum: f64,
    maximum: f64,
    unit: Option<&'static str>,
) -> AudioModifierControl {
    let percentage = unit == Some("%") && minimum >= 0.0 && maximum <= 1.0;
    let display_multiplier = if percentage { 100.0 } else { 1.0 };
    AudioModifierControl::Scalar {
        path: format!("/effect/config/{field}"),
        label,
        value: value.clone(),
        presentation: AudioModifierScalarPresentation {
            minimum: minimum * display_multiplier,
            maximum: maximum * display_multiplier,
            drag_step: if percentage { 1.0 } else { 0.01 },
            digits: if percentage { 0 } else { 2 },
            unit,
            store_multiplier: if percentage { 0.01 } else { 1.0 },
            display: if percentage {
                |value| f64::from(value) * 100.0
            } else {
                f64::from
            },
            store: if percentage {
                |value| (value / 100.0) as f32
            } else {
                |value| value as f32
            },
        },
    }
}

fn boolean(field: &'static str, label: &'static str, value: bool) -> AudioModifierControl {
    AudioModifierControl::Boolean {
        path: format!("/effect/config/{field}"),
        label,
        value,
    }
}

fn selector(
    field: &'static str,
    label: &'static str,
    value: &str,
    options: &[(&'static str, &'static str)],
) -> AudioModifierControl {
    AudioModifierControl::Selector {
        path: format!("/effect/config/{field}"),
        label,
        value: value.to_string(),
        options: options
            .iter()
            .map(|(value, label)| AudioModifierOption { value, label })
            .collect(),
    }
}

pub fn default_audio_modifier_effect(effect: &AudioModifierEffect) -> AudioModifierEffect {
    match effect {
        AudioModifierEffect::Cache(_) => AudioModifierEffect::Cache(Default::default()),
        AudioModifierEffect::Gain(_) => AudioModifierEffect::Gain(Default::default()),
        AudioModifierEffect::Pan(_) => AudioModifierEffect::Pan(Default::default()),
        AudioModifierEffect::Pitch(_) => AudioModifierEffect::Pitch(Default::default()),
        AudioModifierEffect::Denoise(_) => AudioModifierEffect::Denoise(Default::default()),
        AudioModifierEffect::Equalizer(_) => AudioModifierEffect::Equalizer(Default::default()),
        AudioModifierEffect::Filter(_) => AudioModifierEffect::Filter(Default::default()),
        AudioModifierEffect::NoiseGate(_) => AudioModifierEffect::NoiseGate(Default::default()),
        AudioModifierEffect::StereoWidth(_) => AudioModifierEffect::StereoWidth(Default::default()),
        AudioModifierEffect::Tremolo(_) => AudioModifierEffect::Tremolo(Default::default()),
        AudioModifierEffect::Bitcrusher(_) => AudioModifierEffect::Bitcrusher(Default::default()),
        AudioModifierEffect::Chorus(_) => AudioModifierEffect::Chorus(Default::default()),
        AudioModifierEffect::Compressor(_) => AudioModifierEffect::Compressor(Default::default()),
        AudioModifierEffect::Limiter(_) => AudioModifierEffect::Limiter(Default::default()),
        AudioModifierEffect::Reverb(_) => AudioModifierEffect::Reverb(Default::default()),
        AudioModifierEffect::CloseUp(_) => AudioModifierEffect::CloseUp(Default::default()),
        AudioModifierEffect::VoiceColor(_) => AudioModifierEffect::VoiceColor(Default::default()),
        AudioModifierEffect::Echo(_) => AudioModifierEffect::Echo(Default::default()),
        AudioModifierEffect::Distortion(_) => AudioModifierEffect::Distortion(Default::default()),
        AudioModifierEffect::VoiceChange(_) => AudioModifierEffect::VoiceChange(Default::default()),
    }
}

pub fn same_audio_modifier_effect(left: &AudioModifierEffect, right: &AudioModifierEffect) -> bool {
    fn remove_ids(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Array(values) => values.iter_mut().for_each(remove_ids),
            serde_json::Value::Object(values) => {
                values.remove("id");
                values.values_mut().for_each(remove_ids);
            }
            _ => {}
        }
    }
    let mut left = serde_json::to_value(left).expect("audio modifier must serialize");
    let mut right = serde_json::to_value(right).expect("audio modifier must serialize");
    remove_ids(&mut left);
    remove_ids(&mut right);
    left == right
}

pub fn audio_title(item: &shrimply_project_document::project::AudioItem) -> &'static str {
    match &item.source {
        AudioSource::Media => "Audio",
        AudioSource::FoldedSequence(_) => "Folded Sequence",
        AudioSource::Tts(_) => "Text to Speech",
        AudioSource::Generator(_) => "Audio Generator",
    }
}

pub fn audio_modifier_catalog() -> Vec<AudioModifierChoice> {
    AudioModifierEffect::CATALOG
        .iter()
        .map(|new| new())
        .map(|effect| AudioModifierChoice {
            key: audio_modifier_key(&effect),
            label: effect.display_name(),
            search_text: effect.keywords().join(" "),
        })
        .collect()
}

pub fn voice_change_model_catalog(server_url: &str) -> Result<Vec<String>, String> {
    shrimply_audio_engine::pneuma::set_server_url(server_url);
    let mut cache = VOICE_MODELS
        .get_or_init(Mutex::default)
        .lock()
        .expect("voice model cache must not be poisoned");
    if let Some((_, models)) = cache.iter().find(|(url, _)| url == server_url) {
        return Ok(models.clone());
    }
    let models = shrimply_compute_client::pneuma::models(server_url)?
        .into_iter()
        .map(|model| model.name)
        .collect::<Vec<_>>();
    cache.push((server_url.to_string(), models.clone()));
    Ok(models)
}

pub(crate) fn cached_voice_change_model_catalog(server_url: &str) -> Option<Vec<String>> {
    let cache = VOICE_MODELS.get_or_init(Mutex::default).try_lock().ok()?;
    let (_, models) = cache.iter().find(|(url, _)| url == server_url)?;
    Some(models.clone())
}

pub fn cached_voice_change_models(server_url: &str, current: &str) -> Option<Vec<String>> {
    cached_voice_change_model_catalog(server_url)
        .map(|models| voice_models_with_current(models, current))
}

pub fn voice_change_models(server_url: &str, current: &str) -> Result<Vec<String>, String> {
    Ok(voice_models_with_current(
        voice_change_model_catalog(server_url)?,
        current,
    ))
}

fn voice_models_with_current(mut models: Vec<String>, current: &str) -> Vec<String> {
    if !models.iter().any(|model| model == current) {
        models.insert(0, current.to_string());
    }
    models
}

pub(crate) fn audio_modifier_key(effect: &AudioModifierEffect) -> String {
    serde_json::to_value(effect)
        .expect("audio modifier effect must serialize")
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .expect("audio modifier effect kind must be text")
        .to_string()
}

pub(crate) fn audio_item_address(target: &InspectorTarget) -> Result<&ItemAddress, String> {
    match target {
        InspectorTarget::Item(address @ ItemAddress::Audio { .. }) => Ok(address),
        _ => Err("inspector target is not an audio item".to_string()),
    }
}

pub(crate) fn audio_modifier_number<'a>(
    project: &'a Project,
    target: &InspectorTarget,
    modifier_id: uuid::Uuid,
    value_id: uuid::Uuid,
) -> Result<&'a TimelineValue<f32>, String> {
    project
        .audio_item(audio_item_address(target)?)
        .and_then(|item| {
            item.modifiers
                .iter()
                .find(|modifier| modifier.id == modifier_id)
        })
        .and_then(|modifier| modifier.effect.number(value_id))
        .ok_or_else(|| "audio modifier number is no longer available".to_string())
}

pub(crate) fn audio_modifier_number_mut<'a>(
    project: &'a mut Project,
    target: &InspectorTarget,
    modifier_id: uuid::Uuid,
    value_id: uuid::Uuid,
) -> Result<&'a mut TimelineValue<f32>, String> {
    project
        .audio_item_mut(audio_item_address(target)?)
        .and_then(|item| {
            item.modifiers
                .iter_mut()
                .find(|modifier| modifier.id == modifier_id)
        })
        .and_then(|modifier| modifier.effect.number_mut(value_id))
        .ok_or_else(|| "audio modifier number is no longer available".to_string())
}

pub(crate) fn audio_modifier_time(
    project: &Project,
    player_state: &SharedPlayerState,
    target: &InspectorTarget,
) -> Result<Time, String> {
    let position = player_state::snapshot(player_state).position;
    project
        .keyframe_time(audio_item_address(target)?, position)
        .ok_or_else(|| "the current audio modifier time is no longer available".to_string())
}

pub(crate) fn audio_modifier_evaluation_time(
    project: &Project,
    player_state: &SharedPlayerState,
    target: &InspectorTarget,
) -> Result<Time, String> {
    let address = audio_item_address(target)?;
    let position = player_state::snapshot(player_state).position;
    let sequence_time = project
        .timeline_time_to_sequence(&address.track(), position)
        .ok_or_else(|| "the current audio sequence time is no longer available".to_string())?;
    let item = project
        .audio_item(address)
        .ok_or_else(|| "the current audio item is no longer available".to_string())?;
    Ok(sequence_time.saturating_sub(item.start))
}

pub(crate) fn audio_modifier_keyframe_time(
    project: &Project,
    target: &InspectorTarget,
    time: Time,
) -> Result<Time, String> {
    let address = audio_item_address(target)?;
    project
        .keyframe_timeline_time(address, time)
        .and_then(|timeline_time| project.keyframe_time(address, timeline_time))
        .ok_or_else(|| "the audio modifier keyframe time is no longer available".to_string())
}
