use serde_json::{Value, json};
use shrimply_editing_api::commands::COMMANDS;

const MAX_MATCHES: usize = 5;
const PATH_WEIGHT: usize = 8;
const SUMMARY_WEIGHT: usize = 3;
const ARGUMENT_WEIGHT: usize = 1;

pub fn search(query: &str) -> Value {
    let terms = terms(query);
    let mut matches: Vec<_> = COMMANDS
        .iter()
        .filter_map(|command| {
            let path = terms_for(command.path);
            let summary = terms_for(command.summary);
            let schema = (command.schema)();
            let arguments = terms_for(&schema.as_value().to_string());
            let score: usize = terms
                .iter()
                .map(|term| {
                    usize::from(path.contains(term)) * PATH_WEIGHT
                        + usize::from(summary.contains(term)) * SUMMARY_WEIGHT
                        + usize::from(arguments.contains(term)) * ARGUMENT_WEIGHT
                })
                .sum();
            (score > 0).then_some((score, command))
        })
        .collect();
    matches.sort_by(|(score_a, a), (score_b, b)| score_b.cmp(score_a).then(a.path.cmp(b.path)));
    Value::Array(matches.into_iter().take(MAX_MATCHES).map(|(_, command)| json!({"command":format!("shrimply {}",command.path),"summary":command.summary})).collect())
}

fn terms_for(text: &str) -> std::collections::BTreeSet<String> {
    terms(text).into_iter().collect()
}

fn terms(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|word| {
            let word = word.to_lowercase();
            let word = match word.as_str() {
                "the" | "a" | "an" | "to" | "of" | "in" | "and" | "my" | "i" | "want" | "how"
                | "can" | "do" | "with" | "for" | "on" | "all" => return None,
                "remove" | "erase" => "delete",
                "add" | "put" => "insert",
                "subtitle" | "subtitles" | "captions" => "caption",
                "words" | "word" | "lettering" => "text",
                "transcript" | "transcription" => "transcribe",
                "speech" | "voice" | "speak" => "tts",
                "cut" => "split",
                "shorten" | "shorter" | "lengthen" | "longer" => "trim",
                "export" | "encode" => "render",
                "rotate" | "rotation" | "scale" | "zoom" => "transform",
                "hide" | "mute" | "unmute" => "enabled",
                "copy" => "duplicate",
                "swap" | "substitute" | "substitution" => "replace",
                "picture" | "screenshot" | "image" => "frame",
                "inspect" | "show" | "read" => "get",
                "search" | "find" | "enumerate" => "list",
                _ => &word,
            };
            Some(word.strip_suffix('s').unwrap_or(word).to_owned())
        })
        .collect()
}
