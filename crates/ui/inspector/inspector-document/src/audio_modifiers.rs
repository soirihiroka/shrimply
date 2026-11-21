use super::{BasicInspectorAction as Action, InspectorItem};
use shrimply_audio_modifiers::AudioModifier;
use shrimply_inspector_core::item::HeaderToggle;
use shrimply_inspector_core::{
    AudioModifierControl, ControlKind, InspectorControl, InspectorRuntime, InspectorSection,
    NumberSpec,
};
use shrimply_project_document::project::Time;
use shrimply_property_model::{modifier_model::ModifierModel, timeline_value::TimelineValue};

pub(super) fn item(
    modifier: &AudioModifier,
    index: usize,
    len: usize,
    runtime: InspectorRuntime,
) -> InspectorItem {
    let mut section = InspectorSection::default();
    for control in shrimply_inspector_core::audio_modifier_controls(&modifier.effect) {
        match control {
            AudioModifierControl::Cache(cache) => {
                section.controls.extend(
                    shrimply_inspector_core::audio_cache_presentation(&cache, modifier.id)
                        .section
                        .controls,
                );
            }
            AudioModifierControl::Scalar {
                path,
                label,
                value: timeline,
                presentation,
            } => scalar(&mut section, &path, label, &timeline, runtime, presentation),
            AudioModifierControl::Boolean { path, label, value } => section.add(
                InspectorControl::new(ControlKind::Boolean, path, label).value(value.to_string()),
            ),
            AudioModifierControl::Selector {
                path,
                label,
                value,
                options,
            } => section.add(shrimply_inspector_core::selector::selector(
                path,
                label,
                &value,
                options
                    .into_iter()
                    .map(|option| (option.value.to_string(), option.label.to_string())),
            )),
            AudioModifierControl::Number {
                path,
                label,
                value,
                minimum,
                maximum,
                step,
                digits,
            } => section.add(
                InspectorControl::new(ControlKind::Number, path, label)
                    .value(value.to_string())
                    .number(NumberSpec {
                        minimum,
                        maximum,
                        drag_step: step,
                        digits,
                        ..NumberSpec::default()
                    }),
            ),
            AudioModifierControl::VoiceModel { path, label, value } => {
                section
                    .add(InspectorControl::new(ControlKind::VoiceModel, path, label).value(value));
            }
        }
    }
    for control in &mut section.controls {
        control.target_id = Some(modifier.id);
        control.audio_modifier = true;
    }

    let mut item = InspectorItem::new(
        format!("audio-modifier:{}", modifier.id),
        modifier.effect.display_name(),
        section,
    )
    .reset(Action::ResetModifier {
        audio: true,
        id: modifier.id,
    })
    .toggle(HeaderToggle {
        active: modifier.enabled,
        tooltip: if modifier.enabled {
            "Disable modifier"
        } else {
            "Enable modifier"
        },
        activate: Action::SetModifierEnabled {
            audio: true,
            id: modifier.id,
            enabled: !modifier.enabled,
        },
    });
    item.actions =
        super::modifiers::chain_actions(modifier.id, true, index > 0, index + 1 < len, true);
    item
}

#[allow(clippy::too_many_arguments)]
fn scalar(
    section: &mut InspectorSection,
    path: &str,
    label: &str,
    timeline: &TimelineValue<f32>,
    runtime: InspectorRuntime,
    presentation: shrimply_inspector_core::AudioModifierScalarPresentation,
) {
    section.add(
        InspectorControl::new(ControlKind::LayeredNumber, path, label)
            .value(
                (presentation.display)(timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO)))
                    .to_string(),
            )
            .number(NumberSpec {
                minimum: presentation.minimum,
                maximum: presentation.maximum,
                drag_step: presentation.drag_step,
                digits: i32::try_from(presentation.digits)
                    .expect("audio modifier scalar digits must fit i32"),
                unit: presentation.unit.unwrap_or_default(),
            })
            .store_multiplier(presentation.store_multiplier)
            .layered(path, shrimply_inspector_core::LayeredState::from(timeline))
            .timeline(
                timeline.id,
                shrimply_inspector_core::keyframe_graph::scalar_graph(
                    timeline,
                    runtime,
                    presentation.display,
                ),
            ),
    );
}
