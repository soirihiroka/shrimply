use serde_json::{Value, json};
use shrimply_project_document::project::{AudioItem, AudioSource, Time};

use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorDetail, InspectorRuntime, InspectorSection,
    LayeredState, NumberSpec,
};

use super::{
    BasicInspectorAction, CategoryIcon, InspectorCategory, InspectorItem, InspectorListItem,
    detail_item, text,
};

pub(super) fn categories(
    value: &Value,
    details: &[InspectorDetail],
    runtime: InspectorRuntime,
) -> Vec<InspectorCategory> {
    let audio: AudioItem =
        serde_json::from_value(value.clone()).expect("audio inspector value must be valid");
    let current_gain = audio
        .gain
        .decibels
        .value_at(runtime.local_time.unwrap_or(Time::ZERO));
    let mut output = InspectorSection::default();
    output.add(
        InspectorControl::new(ControlKind::Boolean, "/enabled", "Enabled")
            .value(audio.enabled.to_string()),
    );
    output.add(gain_control(&audio.gain.decibels, current_gain, runtime));
    let output = InspectorItem::new("output", "Output", output)
        .reset(BasicInspectorAction::ResetAudioOutput)
        .boxed();
    let mut categories = vec![InspectorCategory {
        key: "audio",
        label: "Audio",
        icon: CategoryIcon::Audio,
        items: vec![output],
    }];
    if let AudioSource::Generator(generator) = &audio.source {
        categories[0]
            .items
            .push(super::audio_generator::item(generator, runtime).boxed());
    }
    if matches!(audio.source, AudioSource::Tts(_)) {
        let mut section = InspectorSection::default();
        section.add(InspectorControl::new(
            ControlKind::TtsEditor,
            "/source",
            "Text to Speech",
        ));
        categories[0].items.push(
            InspectorItem::new("tts", "Text to Speech", section)
                .reset(BasicInspectorAction::Reset {
                    path: "/source".into(),
                    value: serde_json::to_value(AudioSource::Tts(Default::default()))
                        .expect("default TTS settings must serialize"),
                })
                .boxed(),
        );
    }
    categories[0]
        .items
        .extend(audio.modifiers.iter().enumerate().map(|(index, modifier)| {
            super::audio_modifiers::item(modifier, index, audio.modifiers.len(), runtime).boxed()
        }));
    categories[0].items.push(super::modifiers::menu(
        ControlKind::AudioModifierMenu,
        shrimply_inspector_core::audio_modifier_catalog()
            .into_iter()
            .map(|c| (c.key, c.label.to_string(), c.search_text)),
    ));
    if !matches!(audio.source, AudioSource::Generator(_)) {
        categories.push(InspectorCategory {
            key: "playback",
            label: "Playback",
            icon: CategoryIcon::Playback,
            items: playback_items(value),
        });
    }
    let mut info = Vec::new();
    if matches!(audio.source, AudioSource::Media | AudioSource::Tts(_))
        && !audio.file.as_os_str().is_empty()
    {
        let mut section = InspectorSection::default();
        section.add(
            InspectorControl::new(
                ControlKind::BeatDetection,
                "/beat_detection",
                "Beat Detection",
            )
            .target(audio.id)
            .subtitle("Analyze this clip for beat-grid snapping")
            .value(audio.beat_detection.to_string()),
        );
        info.push(InspectorListItem::Flat(section));
    }
    info.push(detail_item(details));
    categories.push(InspectorCategory {
        key: "info",
        label: "Info",
        icon: CategoryIcon::Info,
        items: info,
    });
    categories
}

fn gain_control(
    timeline: &shrimply_property_model::timeline_value::TimelineValue<f32>,
    current: f32,
    runtime: InspectorRuntime,
) -> InspectorControl {
    InspectorControl::new(ControlKind::LayeredNumber, "/gain/decibels", "Level")
        .value(current.to_string())
        .number(NumberSpec {
            minimum: -60.0,
            maximum: 36.0,
            drag_step: 0.01,
            digits: 2,
            unit: "dB",
        })
        .layered("/gain/decibels", LayeredState::from(timeline))
        .timeline(
            timeline.id,
            shrimply_inspector_core::keyframe_graph::scalar_graph(timeline, runtime, f64::from),
        )
        .live_commit("audio-gain")
}

fn playback_items(value: &Value) -> Vec<InspectorListItem> {
    let fraction = value
        .get("playback_speed")
        .and_then(Value::as_object)
        .expect("audio playback speed must be a fraction");
    let mut speed = InspectorSection::default();
    speed.add(
        InspectorControl::new(ControlKind::Fraction, "/playback_speed", "Value")
            .components(fraction_components(fraction))
            .width_characters(9)
            .number(NumberSpec {
                drag_step: 0.05,
                digits: 2,
                unit: "x",
                ..NumberSpec::default()
            }),
    );
    let mut method = InspectorSection::default();
    method.add(shrimply_inspector_core::selector::selector(
        "/speed_method",
        "Method",
        text(value, "/speed_method"),
        [
            ("naive".to_string(), "Naive".to_string()),
            ("preserve_pitch".to_string(), "Preserve pitch".to_string()),
        ],
    ));
    let mut repeat = InspectorSection::default();
    repeat.add(shrimply_inspector_core::selector::selector(
        "/repeat_strategy",
        "Strategy",
        text(value, "/repeat_strategy"),
        [
            ("repeat".to_string(), "Repeat".to_string()),
            ("ping_pong".to_string(), "Ping Pong".to_string()),
            ("hold".to_string(), "Hold".to_string()),
            ("empty".to_string(), "Empty".to_string()),
        ],
    ));
    vec![
        InspectorItem::new("speed", "Speed", speed)
            .reset(BasicInspectorAction::Reset {
                path: "/playback_speed".to_string(),
                value: json!({ "numerator": 1, "denominator": 1 }),
            })
            .boxed(),
        InspectorItem::new("speed-method", "Speed method", method)
            .reset(BasicInspectorAction::Reset {
                path: "/speed_method".to_string(),
                value: Value::String("preserve_pitch".to_string()),
            })
            .boxed(),
        InspectorItem::new("repeat", "Repeat", repeat)
            .reset(BasicInspectorAction::Reset {
                path: "/repeat_strategy".to_string(),
                value: Value::String("hold".to_string()),
            })
            .boxed(),
    ]
}

fn fraction_components(fraction: &serde_json::Map<String, Value>) -> Vec<String> {
    ["numerator", "denominator"]
        .map(|component| {
            fraction
                .get(component)
                .and_then(Value::as_i64)
                .expect("fraction component must be an integer")
                .to_string()
        })
        .to_vec()
}
