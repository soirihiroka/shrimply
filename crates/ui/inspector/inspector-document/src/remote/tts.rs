//! TTS uses the same primitive widgets and validated controller on every host.
use serde_json::{Value, json};
use shrimply_components_core::presentation::normalize_control;
use shrimply_i18n::text;
use shrimply_inspector_core::{
    InspectorControl, InspectorController, InspectorTarget,
    tts::{TtsEditorControl, TtsInputEdit},
};

fn value_editor(label: &str, field: Value, command: Value) -> Value {
    json!({"widget":"editor","label":text(label),"display":field["value"],"enabled":true,"editable":true,
        "editor":{"title":text(label),"fields":[field],"command":command,"confirm_label":text("Apply"),"cancel_label":text("Cancel")}})
}

pub(super) fn presentation(control: &InspectorControl, item: &str, id: &str) -> Value {
    let Some(editor) = &control.tts else {
        return json!({"id":id,"widget":"text","label":"","display":text(&control.subtitle),"subtitle":"","enabled":false,"editable":false});
    };
    let command = |action: &str| json!({"item":item,"control":id,"action":action,"model":editor.model.as_ref().map(|model| &model.id)});
    let input_command = |key: &str, action: &str| {
        let mut value = command(action);
        value["key"] = json!(key);
        value
    };
    let mut controls = Vec::new();
    if editor.loading {
        controls.push(json!({"widget":"loading","label":text(&editor.catalog_status),"enabled":false,"editable":false}));
    } else {
        if !editor.catalog_status.is_empty() {
            controls.push(json!({"widget":"text","label":text("Status"),"display":text(&editor.catalog_status),"enabled":true,"editable":false}));
        }
        controls.push(json!({"widget":"choice","choice_style":"row","label":text("Model"),"display":editor.model.as_ref().map(|model| text(&model.label)),"enabled":!editor.generation.running && !editor.models.is_empty(),"editable":true,
            "choices":editor.models.iter().map(|model| {
                let mut next = command("tts_model");
                next["value"] = json!(model.value);
                json!({"label":text(&model.label),"command":next})
            }).collect::<Vec<_>>()}));
    }
    if editor.can_retry {
        controls.push(json!({"widget":"button","label":text("Retry"),"enabled":true,"editable":false,"command":command("tts_retry")}));
    }
    controls.extend(input_controls(
        &editor.controls,
        !editor.generation.running,
        input_command,
    ));
    if !editor.generation.status.is_empty() {
        controls.push(json!({"widget":if editor.generation.running {"loading"} else {"text"},"label":text("Status"),"display":text(&editor.generation.status),"enabled":true,"editable":false}));
    }
    if editor.model.is_some() {
        controls.push(json!({"widget":"button","label":text(if editor.generation.running {"Cancel"} else if editor.generated {"Regenerate"} else {"Generate"}),"enabled":!editor.generation.cancelling,"editable":false,
            "command":command(if editor.generation.running {"tts_cancel"} else {"tts_generate"})}));
    }
    let mut result = json!({"id":id,"widget":"group","label":"","enabled":control.sensitive,"editable":control.editable,"controls":controls});
    normalize_control(&mut result, id);
    result
}

pub(crate) fn input_controls(
    fields: &[TtsEditorControl],
    enabled: bool,
    input_command: impl Fn(&str, &str) -> Value,
) -> Vec<Value> {
    let mut controls = Vec::new();
    for field in fields {
        let mut widget = match field {
            TtsEditorControl::Text {
                key,
                label,
                value,
                multiline,
                max_length,
            } => value_editor(
                label,
                json!({"value":value,"multiline":multiline,"max_length":max_length,"keyboard":"text"}),
                input_command(key, "tts_edit"),
            ),
            TtsEditorControl::Number {
                key,
                label,
                value,
                minimum,
                maximum,
                step,
                digits,
            } => value_editor(
                label,
                json!({"value":value.to_string(),"minimum":minimum.to_string(),"maximum":maximum.to_string(),"step":step.to_string(),"digits":digits,"keyboard":"decimal","multiline":false}),
                input_command(key, "tts_edit"),
            ),
            TtsEditorControl::Toggle { key, label, value } => {
                let mut next = input_command(key, "tts_toggle");
                next["value"] = json!(!value);
                json!({"widget":"toggle","label":text(label),"selected":value,"command":next})
            }
            TtsEditorControl::Select {
                key,
                label,
                value,
                choices,
            } => {
                json!({"widget":"choice","choice_style":"row","label":text(label),"display":choices.iter().find(|choice| &choice.value == value).map(|choice| text(&choice.label)),
                "choices":choices.iter().map(|choice| {
                    let mut next = input_command(key,"tts_select");
                    next["value"] = json!(choice.value);
                    json!({"label":text(&choice.label),"command":next})
                }).collect::<Vec<_>>()})
            }
            TtsEditorControl::Audio { key, label, path } => {
                let mut clear = input_command(key, "tts_audio");
                clear["path"] = Value::Null;
                json!({"widget":"file","label":text(label),"display":path.as_ref().map(|path| path.display().to_string()).unwrap_or_default(),"mime":"audio/*","choose_label":text("Choose audio…"),
                    "command":input_command(key,"tts_audio"),"clear":path.as_ref().map(|_|json!({"label":text("Clear audio"),"command":clear}))})
            }
            TtsEditorControl::Table {
                key,
                label,
                columns,
                rows,
            } => {
                let mut fields = Vec::new();
                for (row_index, row) in rows.iter().enumerate() {
                    for (column_index, column) in columns.iter().enumerate() {
                        let mut next = input_command(key, "tts_cell");
                        next["row"] = json!(row_index);
                        next["column"] = json!(column_index);
                        next["rows"] = json!(rows);
                        fields.push(value_editor(&column.label,json!({"value":row.get(&column.key).cloned().unwrap_or_default(),"keyboard":"text","multiline":false,"max_length":column.max_length}),next));
                    }
                    let mut next = input_command(key, "tts_remove_row");
                    next["row"] = json!(row_index);
                    next["rows"] = json!(rows);
                    fields.push(json!({"widget":"button","label":text("Remove row"),"enabled":true,"editable":false,"command":next}));
                }
                fields.push(json!({"widget":"button","label":text("Add row"),"enabled":true,"editable":false,"command":input_command(key,"tts_add_row")}));
                json!({"widget":"group","label":text(label),"controls":fields})
            }
        };
        widget["enabled"] = json!(enabled);
        if widget.get("editable").is_none() {
            widget["editable"] = json!(true);
        }
        controls.push(widget);
    }
    controls
}

pub(super) fn apply(
    controller: &InspectorController,
    target: &InspectorTarget,
    control: &InspectorControl,
    action: &str,
    request: &Value,
) -> Result<(), String> {
    let editor = control
        .tts
        .as_ref()
        .ok_or("Text-to-speech editor is unavailable")?;
    if editor.generation.running && action != "tts_cancel" {
        return Err("Text-to-speech generation is running".into());
    }
    controller.finish_live_edit();
    match action {
        "tts_model" => {
            return controller.select_tts_model(
                target,
                &editor.server_url,
                request["value"]
                    .as_str()
                    .ok_or("Text-to-speech model is required")?,
            );
        }
        "tts_retry" if editor.can_retry => {
            controller.retry_tts_models(&editor.server_url);
            return Ok(());
        }
        "tts_cancel" => return controller.cancel_tts_generation(target),
        _ => {}
    }
    let model = editor
        .model
        .as_ref()
        .filter(|model| request["model"].as_str() == Some(model.id.as_str()))
        .ok_or("Text-to-speech model changed; refresh the inspector")?;
    if action == "tts_generate" {
        return controller.toggle_tts_generation(target, &editor.server_url, model);
    }
    let (key, edit) = input_edit(&editor.controls, action, request)?;
    controller.edit_tts_input(target, model, &key, edit)?;
    controller.finish_live_edit();
    Ok(())
}

pub(crate) fn input_edit(
    fields: &[TtsEditorControl],
    action: &str,
    request: &Value,
) -> Result<(String, TtsInputEdit), String> {
    let key = request["key"]
        .as_str()
        .ok_or("Text-to-speech input is required")?;
    let field = fields
        .iter()
        .find(|field| match field {
            TtsEditorControl::Text { key: current, .. }
            | TtsEditorControl::Number { key: current, .. }
            | TtsEditorControl::Toggle { key: current, .. }
            | TtsEditorControl::Select { key: current, .. }
            | TtsEditorControl::Audio { key: current, .. }
            | TtsEditorControl::Table { key: current, .. } => current == key,
        })
        .ok_or("Text-to-speech input is no longer available")?;
    let edit = match (action, field) {
        ("tts_edit", TtsEditorControl::Text { .. }) => {
            TtsInputEdit::Text(editor_value(request)?.to_owned())
        }
        ("tts_edit", TtsEditorControl::Number { .. }) => {
            let value = editor_value(request)?
                .parse()
                .map_err(|_| "Invalid text-to-speech number")?;
            if !shrimply_math_core::fraction_is_finite(value) {
                return Err("Text-to-speech number must be finite".into());
            }
            TtsInputEdit::Number(value)
        }
        ("tts_toggle", TtsEditorControl::Toggle { .. }) => TtsInputEdit::Toggle(
            request["value"]
                .as_bool()
                .ok_or("Text-to-speech toggle requires a boolean")?,
        ),
        ("tts_select", TtsEditorControl::Select { .. }) => TtsInputEdit::Select(
            request["value"]
                .as_str()
                .ok_or("Text-to-speech choice is required")?
                .to_owned(),
        ),
        ("tts_audio", TtsEditorControl::Audio { .. }) => {
            TtsInputEdit::Audio(if request["path"].is_null() {
                None
            } else {
                Some(
                    request["path"]
                        .as_str()
                        .ok_or("Reference audio requires a path")?
                        .into(),
                )
            })
        }
        ("tts_add_row", TtsEditorControl::Table { .. }) => TtsInputEdit::AddTableRow,
        ("tts_cell" | "tts_remove_row", TtsEditorControl::Table { rows, .. }) => {
            if request["rows"] != json!(rows) {
                return Err("Text-to-speech table changed; refresh the inspector".into());
            }
            let row = usize::try_from(
                request["row"]
                    .as_u64()
                    .ok_or("Text-to-speech row is required")?,
            )
            .map_err(|_| "Text-to-speech row is out of range")?;
            if action == "tts_remove_row" {
                TtsInputEdit::RemoveTableRow(row)
            } else {
                TtsInputEdit::TableCell {
                    row,
                    column: usize::try_from(
                        request["column"]
                            .as_u64()
                            .ok_or("Text-to-speech column is required")?,
                    )
                    .map_err(|_| "Text-to-speech column is out of range")?,
                    value: editor_value(request)?.to_owned(),
                }
            }
        }
        _ => return Err("Text-to-speech action does not match its input".into()),
    };
    Ok((key.to_owned(), edit))
}

fn editor_value(request: &Value) -> Result<&str, String> {
    request["values"]
        .as_array()
        .filter(|values| values.len() == 1)
        .and_then(|values| values[0].as_str())
        .ok_or_else(|| "Text-to-speech editor requires one text value".into())
}
