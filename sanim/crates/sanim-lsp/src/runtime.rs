//! Runtime values are observations of saved project clips, never evaluations of editor buffers.
use crate::{RuntimeQuery, features, workspace::Workspace};
use serde_json::{Value, json};
use std::path::Path;

const MAX_VALUE_CHARACTERS: usize = 1024;

pub(crate) fn hover(
    workspace: &Workspace,
    uri: &str,
    position: &Value,
    hover: &mut Value,
    query: RuntimeQuery<'_>,
) {
    let Some(input) = features::reflected_at(workspace, uri, position) else {
        return;
    };
    let document = &workspace.documents[uri];
    let mut detail = String::new();
    if std::fs::read_to_string(&document.path).ok().as_deref()
        != Some(document.document.text.as_str())
    {
        detail.push_str("Runtime values describe saved source; save this document to inspect its current reflected values.");
    } else {
        match query(&document.path).and_then(|runtime| {
            if std::fs::read_to_string(&document.path).ok().as_deref()
                != Some(document.document.text.as_str())
            {
                Err("Source changed during runtime inspection; save the document and retry".into())
            } else {
                Ok(runtime)
            }
        }) {
            Err(error) => detail = format!("Runtime values unavailable: {error}"),
            Ok(runtime) => {
                for context in runtime["contexts"].as_array().into_iter().flatten() {
                    let state = &context["state"];
                    let source = state["source"]
                        .as_str()
                        .and_then(|path| Path::new(path).parent());
                    let parameter =
                        state["parameters"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|parameter| {
                                parameter["name"].as_str() == Some(&input.name)
                                    && parameter["span"]["start"].as_u64()
                                        == Some(input.span.start as u64)
                                    && parameter["span"]["end"].as_u64()
                                        == Some(input.span.end as u64)
                                    && source.zip(parameter["source"].as_str()).is_some_and(
                                        |(root, source)| {
                                            root.join(source).canonicalize().ok().as_ref()
                                                == Some(&document.path)
                                        },
                                    )
                            });
                    let project = context["project"].as_str().unwrap_or_default();
                    let label = context["label"].as_str().unwrap_or_default();
                    let item = state["address"]["item_id"].as_str().unwrap_or_default();
                    let Some(parameter) = parameter else { continue };
                    if !detail.is_empty() {
                        detail.push_str("\n\n");
                    }
                    detail.push_str(&format!(
                        "Runtime reflection — {label}\n\nProject: `{project}`; clip: `{item}`\n\n"
                    ));
                    if let Some(error) = state["error"].as_str() {
                        detail.push_str(&format!("Runtime preparation error: {error}"));
                    } else {
                        let value = parameter["value"].to_string();
                        let mut display =
                            value.chars().take(MAX_VALUE_CHARACTERS).collect::<String>();
                        if display.len() < value.len() {
                            display.push('…');
                        }
                        detail.push_str(&format!("Resolved value:\n```json\n{display}\n```"));
                    }
                }
            }
        }
    }
    if detail.is_empty() {
        return;
    }
    if hover.is_null() {
        *hover = json!({"contents":{"kind":"markdown","value":detail},"range":document.document.range(input.span)});
    } else if let Some(value) = hover["contents"]["value"].as_str() {
        hover["contents"]["value"] = format!("{value}\n\n{detail}").into();
    }
}
