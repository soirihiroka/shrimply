use super::*;
use shrimply_components_core::presentation::normalize_control;
use shrimply_i18n::text;

pub(super) fn editor_value(request: &Value) -> Result<&str, String> {
    request["values"]
        .as_array()
        .filter(|values| values.len() == 1)
        .and_then(|values| values[0].as_str())
        .ok_or_else(|| "Enter one value".into())
}

impl Session {
    fn command(&self, action: &str) -> Value {
        json!({"session":self.id,"action":action})
    }
    fn field(&self, key: &str, value: Value) -> Value {
        let mut command = self.command("field");
        command["key"] = json!(key);
        command["value"] = value;
        command
    }
    fn editor(
        &self,
        key: &str,
        label: &str,
        value: impl ToString,
        min: &str,
        max: &str,
        step: &str,
        digits: usize,
    ) -> Value {
        let value = value.to_string();
        let mut command = self.command("field");
        command["key"] = json!(key);
        json!({"widget":"editor","label":text(label),"display":value,
            "editor":{"title":text(label),"fields":[{"value":value,"minimum":min,"maximum":max,"step":step,"digits":digits,"keyboard":"decimal","multiline":false}],"command":command,"confirm_label":text("Apply"),"cancel_label":text("Cancel")}})
    }
    fn choice(
        &self,
        key: &str,
        label: &str,
        value: &str,
        choices: impl IntoIterator<Item = (String, String)>,
    ) -> Value {
        let choices: Vec<_> = choices.into_iter().collect();
        json!({"widget":"choice","choice_style":"row","label":text(label),"display":choices.iter().find(|(id,_)| id==value).map(|(_,label)|text(label)).unwrap_or_default(),
            "choices":choices.iter().map(|(id,label)|json!({"label":text(label),"command":self.field(key,json!(id))})).collect::<Vec<_>>()})
    }
}

pub(super) fn snapshot(session: &Session) -> Value {
    let mut controls = Vec::new();
    let (title, submit) = match &session.operation {
        Operation::Speech { .. } => ("Generate Speech", "Generate Speech"),
        Operation::Transcribe { .. } => ("Transcribe", "Transcribe"),
        Operation::Silences(_) => ("Remove Silences", "Remove Silences"),
        Operation::Audio { .. } => ("Export Selected Audio", "Export"),
    };
    let configuring = session.stage == "configure";
    if configuring {
        match &session.operation {
            Operation::Speech {
                plan,
                models,
                settings,
            } => {
                controls.push(json!({"widget":"text","label":"","display":plan.description()}));
                controls.push(json!({"widget":"choice","choice_style":"row","label":text("Model"),"display":models.iter().find(|model|settings.model.as_deref()==Some(model.id.as_str())).map(|model|text(&model.label)).unwrap_or_default(),
                "enabled":!models.is_empty(),"choices":models.iter().map(|model|{let mut command=session.command("tts_model");command["value"]=json!(model.id);json!({"label":text(&model.label),"command":command})}).collect::<Vec<_>>()}));
                if let Some(model) = models
                    .iter()
                    .find(|model| settings.model.as_deref() == Some(model.id.as_str()))
                {
                    controls.extend(crate::remote::tts::input_controls(
                        &tts::caption_editor_controls(settings, model),
                        true,
                        |key, action| {
                            let mut command = session.command(action);
                            command["key"] = json!(key);
                            command["model"] = json!(model.id);
                            command
                        },
                    ));
                }
            }
            Operation::Transcribe {
                models,
                model,
                options,
            } => {
                controls.push(session.choice(
                    "model",
                    "Model",
                    model,
                    models.iter().map(|model| (model.clone(), model.clone())),
                ));
                controls.push(json!({"widget":"toggle","label":text(transcription::FOLLOW_CUTS_LABEL),"selected":options.chunked,"command":session.field("chunked",json!(!options.chunked))}));
                let value = match options.snap_source {
                    transcription::SnapSource::Audio => "audio",
                    transcription::SnapSource::Video => "video",
                    transcription::SnapSource::AudioAndVideo => "both",
                };
                controls.push(
                    session.choice(
                        "snap_source",
                        transcription::SNAP_SOURCE_LABEL,
                        value,
                        transcription::SNAP_SOURCES
                            .map(|(value, label, _)| (value.into(), label.into())),
                    ),
                );
                for field in transcription::NUMBER_FIELDS {
                    controls.push(session.editor(
                        field.key,
                        field.label,
                        options.field_value(field.key),
                        field.minimum,
                        field.maximum,
                        field.step,
                        field.digits,
                    ));
                }
            }
            Operation::Silences(config) => {
                for field in silence::FIELDS {
                    controls.push(session.editor(
                        field.key,
                        field.label,
                        config.field_value(field.key),
                        field.minimum,
                        field.maximum,
                        field.step,
                        field.digits,
                    ));
                }
            }
            Operation::Audio { format, .. } => controls.push(session.choice(
                "format",
                "Format",
                format.extension(),
                audio::Format::ALL.map(|format| (format.extension().into(), format.label().into())),
            )),
        }
    }
    if session.catalog_loading {
        controls.push(json!({"widget":"loading","label":text("Loading models…")}));
    }
    if !session.message.is_empty() {
        controls.push(json!({"widget":if session.running.is_some(){"loading"}else{"text"},"label":"","display":text(&session.message)}));
    }
    if let Some(error) = &session.error {
        controls.push(json!({"widget":"text","label":text("Error"),"display":text(error)}));
        if configuring
            && matches!(
                session.operation,
                Operation::Speech { .. } | Operation::Transcribe { .. }
            )
            && !session.catalog_loading
        {
            controls.push(
                json!({"widget":"button","label":text("Retry"),"command":session.command("retry")}),
            );
        }
    }
    for (index, control) in controls.iter_mut().enumerate() {
        normalize_control(control, &format!("{}:{index}", session.id));
    }
    let catalog_ready = match &session.operation {
        Operation::Speech { models, .. } => !models.is_empty(),
        Operation::Transcribe { models, .. } => !models.is_empty(),
        _ => true,
    };
    json!({"id":session.id,"title":text(title),"stage":session.stage,"controls":controls,"output":session.output,
        "submit":configuring.then(||json!({"label":text(submit),"enabled":catalog_ready&&!session.catalog_loading,"command":session.command("submit")})),
        "close":{"label":text(if session.running.is_some(){"Cancel"}else{"Close"}),"enabled":!session.cancelling||session.running.is_none(),"command":session.command(if session.running.is_some(){"cancel"}else{"close"})},
        "visible":!session.close_pending,"dismiss_command":session.command("dismiss"),
        "save_label":text("Save"),"saved_command":session.command("saved")})
}
