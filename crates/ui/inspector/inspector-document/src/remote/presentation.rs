//! Localized native-widget descriptions. Toolkit bridges do not classify inspector controls.
use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_inspector_core::{ControlKind, InspectorControl};

fn editor(control: &InspectorControl, command: Value, expression: bool) -> Value {
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
            ControlKind::Text | ControlKind::MultilineText | ControlKind::LayeredText
        ) {
        "text"
    } else {
        "decimal"
    };
    let fields = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let label = if expression {
                None
            } else {
                control.prefixes.get(index).map(String::as_str)
            };
            json!({"label":label.map(text),"value":value,"multiline":multiline,"keyboard":keyboard})
        })
        .collect::<Vec<_>>();
    json!({"title":text(&control.label),"fields":fields,"command":command,"confirm_label":text("Apply"),"cancel_label":text("Cancel")})
}

pub(super) fn control(control: &InspectorControl, item: &str, id: &str) -> Value {
    use ControlKind as K;
    if control.kind == K::ProjectSettings {
        return project_settings(control, item, id);
    }
    if control.kind == K::MultilineText && control.path == "/tags" {
        return super::tags::presentation(control, item, id);
    }
    let widget =
        match control.kind {
            K::FontFamilies => return super::fonts::presentation(control, item, id),
            K::TtsEditor => return super::tts::presentation(control, item, id),
            K::Boolean | K::LayeredBoolean | K::BeatDetection => "toggle",
            K::Color | K::LayeredColor => "color",
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
            K::Analysis | K::AudioCache | K::VisualCache => "progress",
            K::Performance => "live_rows",
            K::ReadOnly | K::InfoHeading | K::LayeredDrawing => "text",
            K::FileLocation => "file_location",
            K::InfoArtwork => "image",
            K::InfoLoading => "loading",
            K::Number
            | K::Fraction
            | K::LayeredNumber
            | K::Vector2
            | K::Vector3
            | K::LayeredVector2
            | K::LayeredVector3 => "number",
            K::Text | K::MultilineText | K::LayeredText | K::ProjectSettings => {
                if control.editable { "editor" } else { "text" }
            }
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
        "color" => {
            let source = if control.components.is_empty() {
                json!(control.value)
            } else {
                json!(control.components)
            };
            let color =
                shrimply_components_core::color::picker::parse_color(&source, control.with_alpha)
                    .expect("color inspector requires valid RGBA components");
            result["display"] = json!(shrimply_components_core::color::color_hex(
                color,
                control.with_alpha
            ));
            result["argb"] = json!(u32::from_be_bytes([color.a, color.r, color.g, color.b]));
            result["checker"] = json!(shrimply_components_core::color::picker::CHECKER_SIZE);
            result["checker_dark"] = json!(shrimply_components_core::color::picker::CHECKER_DARK);
            result["checker_light"] = json!(shrimply_components_core::color::picker::CHECKER_LIGHT);
            result["picker"] = json!({"color":source,"with_alpha":control.with_alpha,"title":text(if control.tooltip.is_empty() { &control.label } else { &control.tooltip })});
            result["command"] = command("components");
        }

        "file_location" => {
            let mut next = command("show_in_files");
            next["op"] = json!("inspector_resource");
            result["reveal"] = json!({"label":text("Show in folder"),"icon":"folder-open-symbolic","command":next});
            result["enabled"] = json!(control.sensitive && !control.value.is_empty());
        }
        "image" => {
            result["thumbnail"] = json!(
                control
                    .artwork
                    .as_ref()
                    .expect("artwork control requires a thumbnail")
                    .path()
            );
            result["height"] = json!(shrimply_inspector_core::info::ARTWORK_PREVIEW_HEIGHT);
            result["error"] = json!(text("Artwork format is not supported."));
        }
        "number" => {
            let mut number = super::numeric::presentation(
                control,
                &super::numeric::values(control).expect("numeric control values must be valid"),
                control.lock,
            );
            number["command"] =
                json!({"op":"inspector_numeric","item":item,"control":id,"action":"number"});
            for (key, value) in number.as_object().expect("numeric presentation object") {
                result[key] = value.clone();
            }
        }
        "toggle" => {
            result["busy"] = json!(control.busy);
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
            let button = matches!(
                control.kind,
                ControlKind::AudioModifierMenu | ControlKind::VisualModifierMenu
            );
            result["choice_style"] = json!(if button { "button" } else { "row" });
            if button {
                result["choice_icon"] = json!("list-add-symbolic");
            }
            result["label"] = json!(text(if control.label.is_empty() {
                "Add modifier"
            } else {
                &control.label
            }));
            result["display"] = json!(if button {
                String::new()
            } else {
                selected
                    .and_then(|index| labels.get(index))
                    .cloned()
                    .unwrap_or_default()
            });
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
        "progress" => {
            let status = control
                .analysis
                .as_ref()
                .expect("background control requires shared progress");
            let button = status.button(false);
            result["label"] = json!(text(&button.label));
            result["tooltip"] = json!(text(button.tooltip.as_str()));
            result["enabled"] = json!(button.sensitive);
            result["progress"] = json!(button.progress);
            result["suggested"] = json!(button.suggested);
            result["command"] = command("activate");
            if status.running {
                result["cancel_label"] = json!(text(&status.button(true).label));
            }
        }
        "button" => {
            result["enabled"] = json!(control.sensitive && control.action_sensitive);
            if let Some(selection) = control.action.and_then(|action| action.file_selection()) {
                result["widget"] = json!("file");
                result["display"] = json!("");
                result["choose_label"] = json!(text(if control.value.is_empty() {
                    &control.label
                } else {
                    &control.value
                }));
                result["mime"] = json!("*/*");
                result["extensions"] = json!(selection.extensions);
                result["command"] = command("file");
            } else {
                result["command"] = command("activate");
            }
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
                (
                    "Keyframes",
                    "keyframes",
                    shrimply_components_core::layered::KEYFRAMES_ICON,
                    control.layered.keyframes
                ),
                (
                    "Expression",
                    "expression",
                    shrimply_components_core::layered::EXPRESSION_ICON,
                    control.layered.expression
                )
            ]
            .into_iter()
            .map(|(label, action, icon, active)| {
                let mut next = command(action);
                next["enabled"] = json!(!active);
                json!({"label":text(label),"icon":icon,"selected":active,"command":next})
            })
            .collect::<Vec<_>>()
        );
        result["wide"] = json!(matches!(control.kind, K::LayeredText | K::LayeredDrawing));
        result["reveal_duration_ms"] = json!(shrimply_components_core::layered::REVEAL_DURATION_MS);
        if control.layered.expression {
            let mut next = command("edit");
            next["editor"] = json!("expression");
            next["op"] = json!("inspector");
            let mut inline = editor(control, next, true);
            inline["height"] = json!(shrimply_inspector_core::rhai_editor::EDITOR_HEIGHT);
            let mut finish = command("expression_finish");
            finish["op"] = json!("inspector");
            inline["finish"] = finish;
            result["expression_editor"] = inline;
        }
    }
    if control.scalar_graph.is_some() {
        result["graph"] = json!({"item":item,"control":id});
    }
    result
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

fn project_settings(control: &InspectorControl, item: &str, id: &str) -> Value {
    use shrimply_inspector_core::project as shared;
    let model = control
        .project_settings
        .as_ref()
        .expect("shared project settings model");
    let command = |action: &str| json!({"item":item,"control":id,"action":action});
    let rate = &model.frame_rate;
    let choices: Vec<_> = rate
        .values
        .iter()
        .zip(&rate.labels)
        .map(|(value, label)| {
            let mut next = command("settings_stage");
            next["field"] = json!(rate.path);
            next["value"] = json!(value);
            json!({"value":value,"label":text(label),"command":next})
        })
        .collect();
    let selected = rate
        .values
        .iter()
        .position(|value| value == &rate.value)
        .expect("current frame rate is a choice");
    let resolution = &model.resolution;
    let mut stage = command("settings_stage");
    stage["field"] = json!(resolution.path);
    let mut resolution_editor = editor(resolution, stage, false);
    resolution_editor["confirm_label"] = json!(text("Done"));
    let mut controls = vec![
        json!({"id":"frame_rate","widget":"choice","label":text(&rate.label),"subtitle":"","enabled":true,"editable":true,"display":choices[selected]["label"],"choice_style":"row","choices":choices}),
        json!({"id":"resolution","widget":"editor","label":text(&resolution.label),"subtitle":"","enabled":true,"editable":true,"display":resolution.components.join(" × "),"editor":resolution_editor}),
    ];
    if model.dirty {
        controls.push(json!({"id":"actions","widget":"actions","label":"","subtitle":"","enabled":true,"editable":true,"actions":[
            {"label":text(shared::SETTINGS_DISCARD),"command":command("settings_discard")},
            {"label":text(shared::SETTINGS_APPLY),"command":command("settings_apply"),"confirmation":{
                "title":text(shared::SETTINGS_TITLE),"message":text(shared::SETTINGS_WARNING),"confirm_label":text(shared::SETTINGS_APPLY),"cancel_label":text(shared::SETTINGS_CANCEL)
            }}
        ]}));
    }
    json!({"id":id,"widget":"group","label":"","subtitle":"","enabled":control.sensitive,"editable":control.editable,"controls":controls})
}
