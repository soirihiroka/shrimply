//! Sanim keeps its own direct parser; foreign Code languages use the native grammar registry.
use crate::ast::{Statement, StatementKind};
use sanim_core::code::{Content, Document, Language, Scope, SyntaxIssue};
use std::{collections::BTreeMap, sync::Arc};

// Displayed source is data, not an executable Engine module. A fixed policy lets
// all views reuse one syntax artifact independent of the host program's limits.
const MAX_DISPLAY_PARSE_STEPS: usize = 200_000;

pub(super) fn install() {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        sanim_data::code::register_parser(Language::Sanim, parse_sanim)
            .expect("canonical Sanim source parser")
    });
}
pub(super) fn parse(document: Document) -> Result<Document, String> {
    install();
    sanim_data::code::parse(document)
}
fn parse_sanim(mut document: Document) -> Result<Document, String> {
    if document.syntax_parsed && document.syntax.is_present()
        || !matches!(document.content, Content::Source)
    {
        return Ok(document);
    }
    if document.language != Language::Sanim {
        return sanim_data::code::parse(document);
    }
    let (recovery, highlights) =
        crate::parser::parse_highlighted(&document.path, &document.text, MAX_DISPLAY_PARSE_STEPS);
    document.highlights = highlights.into();
    if let Some(error) = recovery.diagnostic {
        document.diagnostics = vec![SyntaxIssue {
            line: error.span.line,
            column: error.span.column,
            message: error.message.into(),
        }]
        .into();
    }
    let mut scopes = Vec::new();
    let line_ends: Vec<_> = document
        .text
        .char_indices()
        .filter(|(index, ch)| {
            *ch == '\n' || *ch == '\r' && !document.text[*index..].starts_with("\r\n")
        })
        .map(|(at, ch)| at + ch.len_utf8())
        .collect();
    fn walk(
        statements: &[Statement],
        line_ends: &[usize],
        parent: &str,
        depth: usize,
        scopes: &mut Vec<Scope>,
    ) {
        let mut names = BTreeMap::<String, usize>::new();
        for statement in statements {
            let (label, bodies): (String, Vec<&[Statement]>) = match &statement.kind {
                StatementKind::Function { name, body, .. } => {
                    (format!("function:{name}"), vec![body])
                }
                StatementKind::Background { body } => ("background".into(), vec![body]),
                StatementKind::If { yes, no, .. } => ("if".into(), vec![yes, no]),
                StatementKind::For { body, .. } => ("for".into(), vec![body]),
                StatementKind::Repeat { body, period, .. } => (
                    if period.is_some() { "timer" } else { "repeat" }.into(),
                    vec![body],
                ),
                StatementKind::Match { cases, .. } => (
                    "match".into(),
                    cases.iter().map(|case| case.body.as_slice()).collect(),
                ),
                _ => continue,
            };
            let index = names.entry(label.clone()).or_default();
            *index += 1;
            let id = format!(
                "{}{label}#{}",
                if parent.is_empty() {
                    String::new()
                } else {
                    format!("{parent}/")
                },
                index
            );
            let end_line = line_ends.partition_point(|end| *end < statement.span.end) + 1;
            scopes.push(Scope {
                id: Arc::from(id.as_str()),
                label: label.into(),
                start_line: statement.span.line,
                end_line,
                start_byte: statement.span.start,
                end_byte: statement.span.end,
                depth,
            });
            for (branch, body) in bodies.into_iter().enumerate() {
                walk(
                    body,
                    line_ends,
                    &format!("{id}/branch:{branch}"),
                    depth + 1,
                    scopes,
                );
            }
        }
    }
    let module = Arc::new(recovery.module);
    walk(&module.statements, &line_ends, "", 1, &mut scopes);
    let bytes = serde_json::to_vec(module.as_ref())
        .map_err(|error| error.to_string())?
        .len();
    document.syntax = sanim_core::code::SyntaxArtifact::shared(module, bytes);
    document.scopes = scopes.into();
    document.syntax_parsed = true;
    Ok(document)
}
