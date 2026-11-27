use std::collections::BTreeSet;

pub const EDITOR_HEIGHT: i32 = 86;
pub const INDENT_WIDTH: usize = 2;
pub const DIAGNOSTIC_DEBOUNCE_MILLISECONDS: i32 = 250;
pub const COMPLETION_DEBOUNCE_MILLISECONDS: i32 = 75;
pub const MINIMUM_COMPLETION_PREFIX_LENGTH: usize = 2;

const STATIC_WORDS: &str = "\
time t local_t value duration fps canvas_width canvas_height \
media_width media_height source_width source_height seed \
sin cos tan random shake vol Fraction abs int sqrt pow clamp lerp \
rgb rgba gray graya hsv hsva oklab oklaba";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpressionValue {
    Bool,
    Scalar,
    Step,
    Text,
    Vec2,
    Vec3,
    Color,
    Drawing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpressionDiagnostic {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Default)]
pub struct DiagnosticCache {
    source: String,
    diagnostic: Option<ExpressionDiagnostic>,
    initialized: bool,
}

impl DiagnosticCache {
    pub fn diagnostic(&mut self, source: &str) -> Option<&ExpressionDiagnostic> {
        if !self.initialized || self.source != source {
            self.source.clear();
            self.source.push_str(source);
            self.diagnostic = syntax_diagnostic(source);
            self.initialized = true;
        }
        self.diagnostic.as_ref()
    }
}

pub fn syntax_diagnostic(source: &str) -> Option<ExpressionDiagnostic> {
    shrimply_project_evaluation::TransformExpressionCache::syntax_diagnostic(source).map(
        |diagnostic| ExpressionDiagnostic {
            message: diagnostic.message,
            line: diagnostic.line,
            column: diagnostic.column,
        },
    )
}

pub fn completion_words(value: ExpressionValue) -> Vec<&'static str> {
    let mut words: Vec<_> = STATIC_WORDS.split_whitespace().collect();
    match value {
        ExpressionValue::Bool => words.extend(["true", "false"]),
        ExpressionValue::Vec2 => words.extend(["x", "y"]),
        ExpressionValue::Vec3 => words.extend(["x", "y", "z"]),
        ExpressionValue::Color => words.extend(["r", "g", "b", "a"]),
        ExpressionValue::Drawing => words.extend([
            "strokes",
            "fills",
            "points",
            "position",
            "pressure",
            "loops",
            "width_scale",
            "color_index",
        ]),
        ExpressionValue::Scalar | ExpressionValue::Step | ExpressionValue::Text => {}
    }
    words
}

pub fn completion_candidates(source: &str, value: ExpressionValue, prefix: &str) -> Vec<String> {
    let mut candidates = BTreeSet::new();
    candidates.extend(completion_words(value).into_iter().map(str::to_string));
    candidates.extend(identifiers(source).map(str::to_string));
    candidates
        .into_iter()
        .filter(|candidate| candidate != prefix && candidate.starts_with(prefix))
        .collect()
}

pub struct Completion {
    pub candidates: Vec<String>,
    pub start: usize,
    pub end: usize,
}

pub fn completion(source: &str, value: ExpressionValue, cursor: usize) -> Option<Completion> {
    completion_with_minimum_prefix(source, value, cursor, 1)
}

pub fn automatic_completion(
    source: &str,
    value: ExpressionValue,
    cursor: usize,
) -> Option<Completion> {
    completion_with_minimum_prefix(source, value, cursor, MINIMUM_COMPLETION_PREFIX_LENGTH)
}

fn completion_with_minimum_prefix(
    source: &str,
    value: ExpressionValue,
    cursor: usize,
    minimum_prefix_length: usize,
) -> Option<Completion> {
    if !code_at_cursor(source, cursor) {
        return None;
    }
    let range = current_identifier_range(source, cursor)?;
    if range.text.len() < minimum_prefix_length {
        return None;
    }
    let candidates = completion_candidates(source, value, range.text);
    (!candidates.is_empty()).then_some(Completion {
        candidates,
        start: range.start,
        end: range.end,
    })
}

pub struct IdentifierRange<'a> {
    pub text: &'a str,
    pub start: usize,
    pub end: usize,
}

pub fn current_identifier_range(source: &str, cursor: usize) -> Option<IdentifierRange<'_>> {
    let byte_offset = byte_index_for_char_offset(source, cursor)?;
    let before = source.get(..byte_offset)?;
    let start = before
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            (!(character == '_' || character.is_ascii_alphanumeric()))
                .then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    let prefix = before.get(start..)?;
    prefix
        .chars()
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        .then(|| IdentifierRange {
            text: prefix,
            start: before[..start].chars().count(),
            end: cursor,
        })
}

pub fn utf16_offset_to_char_offset(source: &str, offset: usize) -> usize {
    let mut remaining = offset;
    source
        .chars()
        .take_while(|character| {
            let width = character.len_utf16();
            if remaining < width {
                false
            } else {
                remaining -= width;
                true
            }
        })
        .count()
}

pub fn char_offset_to_utf16_offset(source: &str, offset: usize) -> usize {
    source.chars().take(offset).map(char::len_utf16).sum()
}

fn byte_index_for_char_offset(source: &str, offset: usize) -> Option<usize> {
    if offset == 0 {
        return Some(0);
    }
    source
        .char_indices()
        .nth(offset)
        .map(|(index, _)| index)
        .or_else(|| (source.chars().count() == offset).then_some(source.len()))
}

fn code_at_cursor(source: &str, cursor: usize) -> bool {
    #[derive(Clone, Copy)]
    enum Context {
        Code,
        LineComment,
        BlockComment,
        String(char),
    }

    let Some(end) = byte_index_for_char_offset(source, cursor) else {
        return false;
    };
    let mut context = Context::Code;
    let mut escaped = false;
    let mut characters = source[..end].chars().peekable();
    while let Some(character) = characters.next() {
        context = match context {
            Context::Code if character == '/' && characters.peek() == Some(&'/') => {
                characters.next();
                Context::LineComment
            }
            Context::Code if character == '/' && characters.peek() == Some(&'*') => {
                characters.next();
                Context::BlockComment
            }
            Context::Code if matches!(character, '\'' | '"' | '`') => Context::String(character),
            Context::LineComment if character == '\n' => Context::Code,
            Context::BlockComment if character == '*' && characters.peek() == Some(&'/') => {
                characters.next();
                Context::Code
            }
            Context::String(quote) if character == quote && !escaped => Context::Code,
            context => context,
        };
        escaped = matches!(context, Context::String(_)) && character == '\\' && !escaped;
        if character != '\\' {
            escaped = false;
        }
    }
    matches!(context, Context::Code)
}

fn identifiers(source: &str) -> impl Iterator<Item = &str> {
    let mut identifiers = Vec::new();
    let mut start = None;
    for (index, character) in source.char_indices() {
        match (start, character == '_' || character.is_ascii_alphanumeric()) {
            (None, true) if character == '_' || character.is_ascii_alphabetic() => {
                start = Some(index);
            }
            (Some(open), false) => {
                identifiers.push(&source[open..index]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(open) = start {
        identifiers.push(&source[open..]);
    }
    identifiers.into_iter()
}
