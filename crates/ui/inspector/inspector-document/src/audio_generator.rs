use shrimply_project_document::project::{AudioGenerator, Time};

use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorRuntime, InspectorSection, LayeredState,
    NumberConstraint, NumberSpec, audio_generator::AudioGeneratorControl,
};

use super::{BasicInspectorAction, InspectorItem};

pub(super) fn item(generator: &AudioGenerator, runtime: InspectorRuntime) -> InspectorItem {
    let mut section = InspectorSection::default();
    for control in shrimply_inspector_core::audio_generator::controls(generator) {
        match control {
            AudioGeneratorControl::Waveform { value, choices } => {
                let selected = choices
                    .iter()
                    .find(|choice| choice.value == value)
                    .expect("generator waveform must have a choice");
                section.add(shrimply_inspector_core::selector::selector(
                    "/source/waveform",
                    "Waveform",
                    selected.key,
                    choices
                        .iter()
                        .map(|choice| (choice.key.to_string(), choice.label.to_string())),
                ));
            }
            AudioGeneratorControl::Number {
                value,
                presentation,
            } => {
                let path = format!("/source/{}", presentation.field.field());
                section.add(
                    InspectorControl::new(ControlKind::LayeredNumber, &path, presentation.label)
                        .value(
                            (presentation.display)(
                                value.value_at(runtime.local_time.unwrap_or(Time::ZERO)),
                            )
                            .to_string(),
                        )
                        .number(NumberSpec {
                            minimum: presentation.minimum,
                            maximum: presentation.maximum,
                            drag_step: presentation.drag_step,
                            digits: i32::try_from(presentation.digits)
                                .expect("generator digits must fit i32"),
                            unit: presentation.unit,
                        })
                        .store_multiplier(presentation.store_multiplier)
                        .number_constraint(NumberConstraint {
                            minimum: Some(presentation.minimum * presentation.store_multiplier),
                            maximum: Some(presentation.maximum * presentation.store_multiplier),
                            integer: presentation.integer,
                        })
                        .layered(&path, LayeredState::from(value))
                        .timeline(
                            value.id,
                            shrimply_inspector_core::keyframe_graph::scalar_graph(
                                value,
                                runtime,
                                presentation.display,
                            ),
                        )
                        .live_commit("audio-generator-value"),
                );
            }
        }
    }
    InspectorItem::new("audio-generator", "Generator", section)
        .reset(BasicInspectorAction::ResetAudioGenerator)
}
