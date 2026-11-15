//! Runtime values and animation sizes describe saved project clips.
use crate::{RuntimeQuery, features, workspace::Workspace};
use sanim_lang::core::plan::Receipt;
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
    let input = features::reflected_at(workspace, uri, position);
    let animation = features::animation_at(workspace, uri, position);
    let Some(span) = input
        .as_ref()
        .map(|input| input.span)
        .or_else(|| animation.as_ref().map(|animation| animation.span))
    else {
        return;
    };
    let document = &workspace.documents[uri];
    let saved = || {
        std::fs::read_to_string(&document.path)
            .ok()
            .is_some_and(|source| source == document.document.text)
    };
    let mut detail = String::new();
    if !saved() {
        detail.push_str(
            "Save this document to inspect its values and animation sizes in the editor.",
        );
    } else {
        match query(&document.path).and_then(|runtime| {
            if saved() {
                Ok(runtime)
            } else {
                Err("Source changed during inspection; save the document and retry".into())
            }
        }) {
            Err(error) => detail = format!("Runtime values unavailable: {error}"),
            Ok(runtime) => {
                for context in runtime["contexts"].as_array().into_iter().flatten() {
                    let state = &context["state"];
                    let root = state["source"]
                        .as_str()
                        .and_then(|path| Path::new(path).parent());
                    let matches_source = |source: &str| {
                        root.is_some_and(|root| {
                            root.join(source).canonicalize().ok().as_ref() == Some(&document.path)
                        })
                    };
                    let mut sections = Vec::new();
                    if let Some(input) = &input {
                        let parameter = state["parameters"].as_array().into_iter().flatten().find(
                            |parameter| {
                                parameter["name"].as_str() == Some(&input.name)
                                    && parameter["span"]["start"].as_u64()
                                        == Some(input.span.start as u64)
                                    && parameter["span"]["end"].as_u64()
                                        == Some(input.span.end as u64)
                                    && parameter["source"].as_str().is_some_and(matches_source)
                            },
                        );
                        if let Some(parameter) = parameter {
                            let value = parameter["value"].to_string();
                            let mut display =
                                value.chars().take(MAX_VALUE_CHARACTERS).collect::<String>();
                            if display.len() < value.len() {
                                display.push('…');
                            }
                            sections.push(format!("Resolved value:\n```json\n{display}\n```"));
                        }
                    }
                    if let Some(animation) = &animation {
                        let receipts = state["temporal_layout"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|value| {
                                serde_json::from_value::<Receipt>(value.clone()).ok()
                            })
                            .filter(|receipt| {
                                receipt.origin.as_ref().is_some_and(|origin| {
                                    matches_source(&origin.location.source)
                                        && animation.matches(origin)
                                })
                            })
                            .collect::<Vec<_>>();
                        sections.extend(features::animation_sizes(receipts.iter()));
                    }
                    if sections.is_empty() {
                        continue;
                    }
                    if let Some(error) = state["error"].as_str() {
                        sections = vec![format!("Runtime preparation error: {error}")];
                    }
                    let label = context["label"].as_str().unwrap_or_default();
                    let project = context["project"].as_str().unwrap_or_default();
                    let item = state["address"]["item_id"].as_str().unwrap_or_default();
                    if !detail.is_empty() {
                        detail.push_str("\n\n");
                    }
                    detail.push_str(&format!(
                        "Runtime — {label}\n\nProject: `{project}`; clip: `{item}`\n\n{}",
                        sections.join("\n\n")
                    ));
                }
            }
        }
    }
    if detail.is_empty() {
        return;
    }
    if hover.is_null() {
        *hover = json!({"contents":{"kind":"markdown","value":detail},"range":document.document.range(span)});
    } else if let Some(value) = hover["contents"]["value"].as_str() {
        hover["contents"]["value"] = format!("{value}\n\n{detail}").into();
    }
}
