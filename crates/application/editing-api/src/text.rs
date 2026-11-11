use crate::{edit::MutationResult, preview, protocol::*, query};
use regex::{Captures, Regex, RegexBuilder};
use serde_json::json;
use shrimply_project_document::project::{ItemMut, Project, VideoItemContent};
use shrimply_property_model::timeline_value::TimelineBase;
use std::collections::HashSet;

pub fn apply(
    project: &mut Project,
    request: &TextReplaceRequest,
) -> Result<MutationResult, String> {
    if !request.all
        && request.addresses.as_ref().is_none_or(Vec::is_empty)
        && request.track_id.is_none()
        && request.range.is_none()
    {
        return Err("text replacement requires addresses, track_id, range, or all=true".into());
    }
    if request
        .range
        .as_ref()
        .is_some_and(|range| range.end_frame <= range.start_frame)
    {
        return Err("range must have a positive duration".into());
    }
    let (regex, replacement, global, sed) = replacement(request)?;
    let snapshot = preview::saved_snapshot(project.clone(), String::new());
    let scope = request.scope.clone().unwrap_or_default();
    let clips = query::all_clips(&snapshot)?.clips;
    let addresses = if let Some(addresses) = &request.addresses {
        for address in addresses {
            query::get_clip(&snapshot, Some(address), None)?;
        }
        addresses.clone()
    } else {
        clips
            .into_iter()
            .filter(|clip| {
                (if request.recursive {
                    clip.address.sequence_path.starts_with(&scope.sequence_path)
                } else {
                    clip.address.sequence_path == scope.sequence_path
                }) && request
                    .track_id
                    .as_ref()
                    .is_none_or(|id| &clip.address.track_id == id)
                    && request.range.as_ref().is_none_or(|range| {
                        clip.projected.start.frame < range.end_frame
                            && clip.projected.end.frame > range.start_frame
                    })
            })
            .map(|clip| clip.address)
            .collect()
    };
    let mut seen = HashSet::new();
    let mut changed_item_ids = Vec::new();
    let mut diffs = Vec::new();
    let mut replacements = 0;
    let mut matched_values = 0;
    for address in addresses {
        let model = query::model_item_address(&address)?;
        if !seen.insert((model.kind(), model.track_id(), model.item_id())) {
            continue;
        }
        let before = diffs.len();
        {
            let mut apply_text = |text: &mut String, address: &ClipAddress, property: String| {
                let count = regex
                    .find_iter(text)
                    .take(if global { usize::MAX } else { 1 })
                    .count();
                if count == 0 {
                    return;
                }
                matched_values += 1;
                replacements += count;
                let substitute = |captures: &Captures<'_>| {
                    if sed {
                        expand(&replacement, captures)
                    } else {
                        replacement.clone()
                    }
                };
                let after = if global {
                    regex.replace_all(text, substitute)
                } else {
                    regex.replace(text, substitute)
                }
                .into_owned();
                if after != *text {
                    diffs.push(json!({"address": address, "property": property, "before": text, "after": after, "replacements": count}));
                    *text = after;
                }
            };

            match project
                .item_mut(&model)
                .ok_or("text target was not found")?
            {
                ItemMut::Comment(item) => apply_text(&mut item.text, &address, "/text".into()),
                ItemMut::Caption(item) => apply_text(&mut item.text, &address, "/text".into()),
                ItemMut::Video(item) => {
                    if let VideoItemContent::Text(text) = &mut item.content {
                        match &mut text.text.base {
                            TimelineBase::Const(value) => {
                                apply_text(value, &address, "/content/text/base/const".into())
                            }
                            TimelineBase::Keyframes(keys) => {
                                for (index, key) in keys.iter_mut().enumerate() {
                                    apply_text(
                                        &mut key.value,
                                        &address,
                                        format!("/content/text/base/keyframes/{index}/value"),
                                    );
                                }
                            }
                        }
                    }
                }
                ItemMut::Audio(_) => {}
            }
        }
        if diffs.len() != before {
            changed_item_ids.push(model.item_id());
        }
    }
    Ok(MutationResult {
        changed_item_ids,
        details: Some(
            json!({"matched_values":matched_values,"replacements": replacements,"changes":diffs}),
        ),
        ..Default::default()
    })
}

fn replacement(request: &TextReplaceRequest) -> Result<(Regex, String, bool, bool), String> {
    if let Some(expression) = &request.expression {
        if request.find.is_some() || request.replace.is_some() {
            return Err("expression conflicts with find/replace".into());
        }
        let mut chars = expression.chars();
        if chars.next() != Some('s') {
            return Err("only sed-style substitution is supported".into());
        }
        let delimiter = chars
            .next()
            .filter(|ch| ch.is_ascii_punctuation() && *ch != '\\')
            .ok_or("substitution requires a punctuation delimiter")?;
        let mut parts = Vec::new();
        let mut part = String::new();
        while let Some(ch) = chars.next() {
            if ch == delimiter {
                parts.push(std::mem::take(&mut part));
                if parts.len() == 2 {
                    break;
                }
            } else if ch == '\\' {
                let next = chars.next().ok_or("unfinished substitution escape")?;
                if next == delimiter {
                    if parts.is_empty() {
                        part.push_str(&regex::escape(&next.to_string()));
                    } else {
                        if next == '&' {
                            part.push('\\');
                        }
                        part.push(next);
                    }
                } else {
                    part.push('\\');
                    part.push(next);
                }
            } else {
                part.push(ch);
            }
        }
        if parts.len() != 2 || parts[0].is_empty() {
            return Err("expected s/pattern/replacement/gi with a nonempty pattern".into());
        }
        let flags: String = chars.collect();
        if flags.chars().any(|flag| !matches!(flag, 'g' | 'i')) {
            return Err("only g and i substitution flags are supported".into());
        }
        let regex = RegexBuilder::new(&parts[0])
            .case_insensitive(request.ignore_case || flags.contains('i'))
            .build()
            .map_err(|error| error.to_string())?;
        validate_replacement(&parts[1], regex.captures_len())?;
        Ok((
            regex,
            parts[1].clone(),
            request.global || flags.contains('g'),
            true,
        ))
    } else {
        let find = request
            .find
            .as_ref()
            .filter(|find| !find.is_empty())
            .ok_or("a nonempty find string is required")?;
        let replace = request
            .replace
            .clone()
            .ok_or("replace is required (use an empty string to delete)")?;
        let regex = RegexBuilder::new(&regex::escape(find))
            .case_insensitive(request.ignore_case)
            .build()
            .map_err(|error| error.to_string())?;
        Ok((regex, replace, request.global, false))
    }
}

fn validate_replacement(replacement: &str, captures: usize) -> Result<(), String> {
    let mut chars = replacement.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            continue;
        }
        match chars.next() {
            Some(digit) if digit.is_ascii_digit() => {
                if digit.to_digit(10).expect("ASCII digit") as usize >= captures {
                    return Err(format!("replacement refers to missing capture \\{digit}"));
                }
            }
            Some('&' | '\\') => {}
            _ => return Err("replacement supports &, \\0 through \\9, \\&, and \\\\ only".into()),
        }
    }
    Ok(())
}

fn expand(replacement: &str, captures: &Captures<'_>) -> String {
    let mut result = String::new();
    let mut chars = replacement.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '&' => result.push_str(captures.get(0).expect("whole regex match exists").as_str()),
            '\\' => match chars.next().expect("replacement escapes were validated") {
                digit if digit.is_ascii_digit() => {
                    if let Some(value) =
                        captures.get(digit.to_digit(10).expect("ASCII digit") as usize)
                    {
                        result.push_str(value.as_str());
                    }
                }
                literal => result.push(literal),
            },
            literal => result.push(literal),
        }
    }
    result
}
