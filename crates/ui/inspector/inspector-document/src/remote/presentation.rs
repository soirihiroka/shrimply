//! Localized native-widget descriptions. Toolkit bridges do not classify inspector controls.
use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_inspector_core::{ControlKind, InspectorControl};

fn editor(control: &InspectorControl, command: Value, expression: bool) -> Value {
    let labels: &[&str] = if control.kind == ControlKind::ProjectSettings && !expression {
        &[
            "Width",
            "Height",
            "Frame rate numerator",
            "Frame rate denominator",
        ]
    } else {
        &[]
    };
    let values = if expression {
        vec![control.layered.expression_source.clone()]
    } else if control.components.is_empty() {
        vec![control.value.clone()]
    } else {
        control.components.clone()
    };
    let multiline = expression
        || matches!(
            control.kind,
            ControlKind::MultilineText | ControlKind::LayeredText
        );
    let keyboard = if expression
        || matches!(
            control.kind,
            ControlKind::Text
                | ControlKind::MultilineText
                | ControlKind::LayeredText
                | ControlKind::Fraction
        ) {
        "text"
    } else {
        "decimal"
    };
    let fields = values.into_iter().enumerate().map(|(index, value)| json!({
        "label":labels.get(index).map(|label|text(label)),"value":value,"multiline":multiline,"keyboard":keyboard,
    })).collect::<Vec<_>>();
    json!({"title":text(&control.label),"fields":fields,"command":command,"confirm_label":text("Apply"),"cancel_label":text("Cancel")})
}

pub(super) fn control(control: &InspectorControl, item: &str, id: &str) -> Option<Value> {
    use ControlKind as K;
    let widget = match control.kind {
        K::Boolean | K::LayeredBoolean => "toggle",
        K::Selector
        | K::OptionalSelector
        | K::OptionalNumberSelector
        | K::LayeredSelector
        | K::VoiceModel
        | K::AudioCachePreset
        | K::VisualCacheQuality
        | K::AudioModifierMenu
        | K::VisualModifierMenu => "choice",
        K::Action => "button",
        K::Performance => "live_rows",
        K::ReadOnly | K::FileLocation | K::InfoHeading => "text",
        K::InfoLoading => "loading",
        K::Number
        | K::Fraction
        | K::Text
        | K::MultilineText
        | K::LayeredNumber
        | K::LayeredText
        | K::Vector2
        | K::Vector3
        | K::LayeredVector2
        | K::LayeredVector3
        | K::Color
        | K::LayeredColor
        | K::ProjectSettings => {
            if control.editable {
                "editor"
            } else {
                "text"
            }
        }
        _ => return None,
    };
    let command = |action: &str| json!({"item":item,"control":id,"action":action});
    let editable = control.editable && control.sensitive;
    let display = if control.components.is_empty() {
        control.value.clone()
    } else {
        control.components.join(" × ")
    };
    let mut result = json!({"id":id,"widget":widget,"label":text(&control.label),"subtitle":text(&control.subtitle),
        "tooltip":text(&control.tooltip),"enabled":control.sensitive,"editable":editable,"display":display});
    match widget {
        "toggle" => {
            result["selected"] = json!(control.value == "true");
            let mut next = command("value");
            next["value"] = json!((control.value != "true").to_string());
            result["command"] = next;
        }
        "choice" => {
            let labels: Vec<_> = control
                .labels
                .iter()
                .map(|label| text(label).into_owned())
                .collect();
            let selected = control
                .values
                .iter()
                .position(|value| value == &control.value);
            result["label"] = json!(selected.and_then(|index| labels.get(index)).map_or_else(
                || if control.label.is_empty() {
                    text("Add modifier").into_owned()
                } else {
                    text(&control.label).into_owned()
                },
                |selected| shrimply_i18n::text_args(
                    "%{label}: %{value}",
                    &[
                        ("label", text(&control.label).into_owned()),
                        ("value", selected.clone())
                    ]
                ),
            ));
            result["choices"] = json!(
                control
                    .values
                    .iter()
                    .zip(labels)
                    .map(|(value, label)| {
                        let mut next = command("value");
                        next["value"] = json!(value);
                        json!({"label":label,"command":next})
                    })
                    .collect::<Vec<_>>()
            );
        }
        "button" => {
            result["enabled"] = json!(control.sensitive && control.action_sensitive);
            result["command"] = command("activate");
        }
        "editor" => {
            let mut next = command("edit");
            next["editor"] = json!("value");
            result["editor"] = editor(control, next, false);
        }
        "live_rows" => result["resource"] = live_rows(),
        _ => {}
    }
    if control.timeline_path.is_some() {
        result["toggles"] = json!(
            [
                ("Keyframes", "keyframes", control.layered.keyframes),
                ("Expression", "expression", control.layered.expression)
            ]
            .into_iter()
            .map(|(label, action, active)| {
                let mut next = command(action);
                next["enabled"] = json!(!active);
                json!({"label":text(label),"selected":active,"command":next})
            })
            .collect::<Vec<_>>()
        );
        if control.layered.expression {
            let mut next = command("edit");
            next["editor"] = json!("expression");
            result["extra_editor"] =
                json!({"label":text("Edit expression"),"editor":editor(control,next,true)});
        }
    }
    Some(result)
}

fn live_rows() -> Value {
    use shrimply_components_core::performance;
    json!({"label":text("Live Performance"),"request":{"op":"performance"},"interval_ms":performance::REFRESH_INTERVAL.as_millis(),
        "actions":[{"label":text("Clear"),"icon":"edit-clear-symbolic","request":{"op":"performance_clear"}},
            {"label":text("Copy JSON"),"icon":"edit-copy-symbolic","request":{"op":"performance_copy"}}]})
}

pub fn performance_request(operation: &str) -> Value {
    use shrimply_components_core::performance;
    if operation == "performance_clear" {
        performance::clear();
    }
    let mut result = json!({"rows":performance::rows().into_iter().map(|row| json!({"title":text(&row.title),"subtitle":row.subtitle})).collect::<Vec<_>>()});
    if operation == "performance_copy" {
        result["clipboard"] = json!(performance::report_json());
        result["clipboard_label"] = json!(text("Live Performance"));
    }
    result
}
