//! Presentation and commands for native hosts that cannot retain Rust controls.
//! Handles resolve against the current document, including its stable selection.
use crate::{InspectorDocument, InspectorItem, InspectorListItem};
use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_inspector_core::{ControlKind, InspectorControl, InspectorController};

mod resources;
mod tags;
pub use resources::resource_request;
mod fonts;
pub use fonts::Fonts;
pub mod graph;
mod numeric;
mod presentation;
pub use numeric::Interaction;
pub(crate) mod tts;
pub use presentation::performance_request;

fn item_key(document: &InspectorDocument, category: &str, item: &str) -> String {
    json!([format!("{:?}", document.target), category, item]).to_string()
}

fn control_key(control: &InspectorControl, index: usize) -> String {
    json!([
        control.path,
        format!("{:?}", control.kind),
        control.target_id,
        control.timeline_id,
        control.path.is_empty().then_some(index),
    ])
    .to_string()
}

pub fn snapshot(document: &InspectorDocument) -> Value {
    let categories: Vec<_> = document.categories.iter().filter_map(|category| {
        let items: Vec<_> = category.items.iter().enumerate().filter_map(|(index, item)| {
            let (key, title, section, header) = match item {
                InspectorListItem::Item(item) => (item.presentation.key.clone(), item.presentation.title.as_str(), &item.section, Some(item.as_ref())),
                InspectorListItem::Flat(section) => (format!("flat:{index}"), "", section, None),
            };
            let id = item_key(document, category.key, &key);
            let controls: Vec<_> = section.controls.iter().enumerate().filter(|(_, control)| control.visible)
                .map(|(index, control)| presentation::control(control, &id, &control_key(control, index))).collect();
            let mut actions: Vec<_> = header.into_iter().flat_map(|header| &header.actions).enumerate().map(|(index, action)| {
                json!({"label":text(action.tooltip),"icon":action.icon,"enabled":action.sensitive,"command":{"item":id,"action":"header_action","index":index}})
            }).collect();
            if header.is_some_and(|header| header.reset.is_some()) {
                actions.insert(0,json!({"label":text("Reset"),"icon":"view-refresh-symbolic","enabled":true,"command":{"item":id,"action":"reset"}}));
            }
            if controls.is_empty() && actions.is_empty() && header.is_none_or(|header| header.toggle.is_none() && header.button_toggle.is_none()) { return None; }
            Some(json!({"id":id,"title":text(title),"controls":controls,"actions":actions,"actions_label":text("Actions"),
                "toggle":header.and_then(|header| header.toggle.as_ref()).map(|toggle|json!({"selected":toggle.active,"label":text(toggle.tooltip),"command":{"item":id,"action":"toggle"}})),
                "button_toggle":header.and_then(|header| header.button_toggle.as_ref()).map(|toggle|json!({"selected":toggle.active,"label":text(toggle.tooltip),"icon":toggle.icon,"command":{"item":id,"action":"button_toggle"}}))}))
        }).collect();
        (!items.is_empty()).then(||json!({"key":category.key,"label":text(category.label),"icon":category.icon.symbolic_name(),"items":items}))
    }).collect();
    json!({"target":format!("{:?}",document.target),"title":text(&document.title),"categories":categories})
}

/// Bind advertised requests to the project that produced this inspector.
/// Native widgets forward these objects without replacing their target.
pub fn scope_commands(snapshot: &mut Value, project_path: &std::path::Path) {
    match snapshot {
        Value::Object(fields) => {
            if fields.contains_key("op")
                || (fields.contains_key("item") && fields.contains_key("action"))
            {
                fields.insert("project_path".into(), json!(project_path));
            }
            for value in fields.values_mut() {
                scope_commands(value, project_path);
            }
        }
        Value::Array(values) => {
            for value in values {
                scope_commands(value, project_path);
            }
        }
        _ => {}
    }
}

/// A native picker may only complete a file command currently advertised by
/// this selection. Writable paths and toolkit-defined inspector actions fail.
pub fn validate_file_command(
    controller: &InspectorController,
    document: &InspectorDocument,
    command: &Value,
) -> Result<(), String> {
    fn advertised(value: &Value, command: &Value, enabled: bool) -> bool {
        let enabled = enabled
            && value
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
        if value["widget"] == "file"
            && enabled
            && value["editable"] == true
            && value["command"] == *command
        {
            return true;
        }
        match value {
            Value::Object(fields) => fields
                .values()
                .any(|value| advertised(value, command, enabled)),
            Value::Array(values) => values
                .iter()
                .any(|value| advertised(value, command, enabled)),
            _ => false,
        }
    }
    let mut command = command.clone();
    if let Some(fields) = command.as_object_mut() {
        fields.remove("project_path");
    }
    if controller.target() != document.target || !advertised(&snapshot(document), &command, true) {
        return Err("File selection changed; open the inspector again".into());
    }
    Ok(())
}

/// Resolve controls anew; requests carry no writable property paths or indices
/// into the project. A selection change, removed effect or changed kind fails.
pub fn apply(
    controller: &InspectorController,
    document: &InspectorDocument,
    request: &Value,
) -> Result<Option<String>, String> {
    if controller.target() != document.target {
        return Err("inspector selection changed; refresh the controls".into());
    }
    let action = request["action"]
        .as_str()
        .ok_or("inspector action is required")?;
    if request["control"].is_string() {
        let control = resolve_control(document, request)?;
        return apply_control(controller, document, control, action, request).map(|()| None);
    }
    let (_, header) = resolve_item(document, request)?;
    apply_header(
        controller,
        document,
        header.ok_or("inspector section has no header")?,
        action,
        request,
    )
}

fn resolve_item<'a>(
    document: &'a InspectorDocument,
    request: &Value,
) -> Result<
    (
        &'a shrimply_inspector_core::InspectorSection,
        Option<&'a InspectorItem>,
    ),
    String,
> {
    let id = request["item"]
        .as_str()
        .ok_or("inspector item is required")?;
    for category in &document.categories {
        for (index, item) in category.items.iter().enumerate() {
            let (key, section, header) = match item {
                InspectorListItem::Item(item) => (
                    item.presentation.key.clone(),
                    &item.section,
                    Some(item.as_ref()),
                ),
                InspectorListItem::Flat(section) => (format!("flat:{index}"), section, None),
            };
            if item_key(document, category.key, &key) == id {
                return Ok((section, header));
            }
        }
    }
    Err("inspector item is no longer available".into())
}

pub(super) fn resolve_control<'a>(
    document: &'a InspectorDocument,
    request: &Value,
) -> Result<&'a InspectorControl, String> {
    let control = read_control(document, request)?;
    if !control.sensitive {
        return Err("inspector control is unavailable".into());
    }
    Ok(control)
}

fn read_control<'a>(
    document: &'a InspectorDocument,
    request: &Value,
) -> Result<&'a InspectorControl, String> {
    let (section, _) = resolve_item(document, request)?;
    let id = request["control"]
        .as_str()
        .ok_or("inspector control is required")?;
    let control = section
        .controls
        .iter()
        .enumerate()
        .find(|(index, control)| control_key(control, *index) == id)
        .map(|(_, control)| control)
        .ok_or("inspector control is no longer available")?;
    if !control.visible {
        return Err("inspector control is unavailable".into());
    }
    Ok(control)
}

/// Resolve canonical inspector preview focus without exposing facet semantics to a host.
pub fn preview_focus(
    controller: &InspectorController,
    document: &InspectorDocument,
    request: &Value,
) -> Option<(
    shrimply_project_document::ItemAddress,
    shrimply_preview_provider_skia::PreviewTarget,
)> {
    if controller.target() != document.target {
        return None;
    }
    let focus = resolve_control(document, request)
        .ok()?
        .preview_focus
        .as_ref()?;
    let focused = controller.resolve_preview_focus(&document.target, focus)?;
    Some((focused.item, focused.target))
}

fn apply_header(
    controller: &InspectorController,
    document: &InspectorDocument,
    header: &InspectorItem,
    action: &str,
    request: &Value,
) -> Result<Option<String>, String> {
    let action = match action {
        "reset" => header
            .reset
            .clone()
            .ok_or("inspector item cannot be reset")?,
        "toggle" => header
            .toggle
            .as_ref()
            .ok_or("inspector item has no toggle")?
            .activate
            .clone(),
        "button_toggle" => {
            let toggle = header
                .button_toggle
                .as_ref()
                .ok_or("inspector item has no button toggle")?;
            toggle.activate.action(!toggle.active)
        }
        "header_action" => {
            let index = request["index"]
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
                .ok_or("inspector action index is required")?;
            let action = header
                .actions
                .get(index)
                .filter(|action| action.sensitive)
                .ok_or("inspector action is unavailable")?;
            action.activate.clone()
        }
        _ => return Err("unknown inspector header action".into()),
    };
    controller.apply_basic_action(&document.target, &action)
}

fn apply_control(
    controller: &InspectorController,
    document: &InspectorDocument,
    control: &InspectorControl,
    action: &str,
    request: &Value,
) -> Result<(), String> {
    let target = &document.target;
    if control.kind == ControlKind::MultilineText && control.path == "/tags" {
        return tags::apply(controller, document, control, action, request);
    }
    if action == "file" {
        if !control.sensitive || !control.editable || !control.action_sensitive {
            return Err("File selection is unavailable".into());
        }
        let selected = control
            .action
            .filter(|action| action.file_selection().is_some())
            .ok_or("Control does not select a file")?;
        return controller.set_control_file(
            target,
            selected,
            std::path::Path::new(
                request["path"]
                    .as_str()
                    .ok_or("File selection requires a path")?,
            ),
        );
    }
    if control.kind == ControlKind::TtsEditor {
        return tts::apply(controller, target, control, action, request);
    }
    if action == "expression_finish" {
        shrimply_project_document::project::finish_coalesced_edit();
        return Ok(());
    }
    if action == "edit" {
        let values = request["values"]
            .as_array()
            .filter(|values| values.iter().all(Value::is_string))
            .ok_or("editor values must be text")?;
        let mut request = request.clone();
        let action = if request["editor"] == "expression" {
            if values.len() != 1 {
                return Err("expression editor requires one value".into());
            }
            request["value"] = values[0].clone();
            "expression_source"
        } else if !control.components.is_empty() {
            if values.len() != control.components.len() {
                return Err("component count changed while the editor was open".into());
            }
            request["components"] = json!(values);
            "components"
        } else {
            if values.len() != 1 {
                return Err("value editor requires one value".into());
            }
            request["value"] = values[0].clone();
            "value"
        };
        return apply_control(controller, document, control, action, &request);
    }
    if action == "activate" {
        if !control.action_sensitive {
            return Err("inspector action is unavailable".into());
        }
        return controller.activate_control(target, control, &document.server_url);
    }
    if !control.editable {
        return Err("inspector control is read-only".into());
    }
    match action {
        "value" => {
            let value = request["value"]
                .as_str()
                .ok_or("inspector value must be text")?;
            if !control.values.is_empty() && !control.values.iter().any(|choice| choice == value) {
                return Err("inspector choice is no longer available".into());
            }
            if matches!(
                control.kind,
                ControlKind::AudioModifierMenu | ControlKind::VisualModifierMenu
            ) {
                return controller.add_control_modifier(target, control.kind, value);
            }
            if control.kind == ControlKind::Fraction {
                let value = value.parse().map_err(|_| "invalid exact fraction")?;
                controller.set_basic_control_fraction(target, control, value)?;
            } else {
                controller.set_basic_control_value(target, control, value)?;
            }
        }
        "components" => {
            let components = request["components"]
                .as_array()
                .ok_or("inspector components are required")?;
            let components = components
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .ok_or("component must be text")?
                        .parse::<f64>()
                        .map_err(|_| "invalid numeric component")
                })
                .collect::<Result<Vec<_>, _>>()?;
            controller.set_basic_control_components(target, control, &components)?;
        }
        "settings_stage" => {
            if control.kind != ControlKind::ProjectSettings {
                return Err("control is not project settings".into());
            }
            match request["field"].as_str() {
                Some("frame_rate") => controller.stage_project_frame_rate(
                    request["value"]
                        .as_str()
                        .ok_or("frame rate is required")?
                        .parse()
                        .map_err(|_| "invalid project frame rate")?,
                )?,
                Some("resolution") => {
                    let values = request["values"]
                        .as_array()
                        .filter(|values| values.len() == 2)
                        .ok_or("project dimensions are required")?;
                    controller.stage_project_dimensions(
                        values[0]
                            .as_str()
                            .ok_or("width is required")?
                            .parse()
                            .map_err(|_| "invalid project width")?,
                        values[1]
                            .as_str()
                            .ok_or("height is required")?
                            .parse()
                            .map_err(|_| "invalid project height")?,
                    )?;
                }
                _ => return Err("unknown project settings field".into()),
            }
            return Ok(());
        }
        "settings_discard" | "settings_apply" => {
            if control.kind != ControlKind::ProjectSettings {
                return Err("control is not project settings".into());
            }
            if action == "settings_apply" {
                controller.apply_staged_project_settings()?;
            } else {
                controller.discard_project_settings();
            }
            return Ok(());
        }
        "keyframes" | "expression" => {
            if control.timeline_path.is_none() {
                return Err("control has no timeline".into());
            }
            let enabled = request["enabled"]
                .as_bool()
                .ok_or("timeline toggle requires a boolean")?;
            return if action == "keyframes" {
                controller.set_control_keyframes(target, control, enabled)
            } else {
                controller.set_control_expression(target, control, enabled)
            };
        }
        "expression_source" => {
            if control.timeline_path.is_none() {
                return Err("control has no expression".into());
            }
            return controller.set_control_expression_source(
                target,
                control,
                request["value"]
                    .as_str()
                    .ok_or("expression source is required")?,
            );
        }
        _ => return Err("unknown inspector control action".into()),
    }
    controller.commit_basic_control(target, control)
}
