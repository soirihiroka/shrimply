use crate::ast::*;
use sanim_core::{Diagnostic, GenericParameter, Span};
mod markup;

macro_rules! keywords {
    ($($variant:ident => $text:literal),* $(,)?) => {
        #[derive(Clone,Copy,Debug,PartialEq,Eq)]
        pub(crate) enum Keyword { $($variant),* }
        impl Keyword {
            pub fn text(self)->&'static str {match self {$(Self::$variant=>$text),*}}
            pub fn from_name(name:&str)->Option<Self>{match name {$($text=>Some(Self::$variant)),*,_=>None}}
        }
    };
}
keywords! { Let=>"let",Mut=>"mut",Sig=>"sig",Reflect=>"reflect",Background=>"background",Animate=>"animate",Fn=>"fn",Type=>"type",Enum=>"enum",Import=>"import",As=>"as",If=>"if",Else=>"else",For=>"for",In=>"in",Return=>"return",Match=>"match",Case=>"case",And=>"and",Or=>"or",Not=>"not",Repeat=>"repeat",Every=>"every",Timer=>"timer",True=>"true",False=>"false",None=>"None" }

/// Parse source directly into the AST. No token list or intermediate syntax tree.
pub fn parse(source: &str, text: &str, max_steps: usize) -> Result<Module, Diagnostic> {
    let mut parser = Parser::new(source, text, max_steps, false);
    parser.trivia();
    let statements = parser.statements(false)?;
    if let Some(error) = parser.failure {
        return Err(error);
    }
    Ok(Module {
        statements,
        comments: parser.comments,
    })
}
/// The ordinary grammar emits layout directly only when source formatting is requested.
pub(crate) fn format(
    source: &str,
    text: &str,
    options: crate::formatter::FormatOptions,
    max_steps: usize,
) -> Result<String, Diagnostic> {
    let mut parser = Parser::new(source, text, max_steps, false);
    parser.formatting = Some(crate::formatter::Sink::new(options));
    parser.trivia();
    parser.statements(false)?;
    if let Some(error) = parser.failure {
        return Err(error);
    }
    parser
        .formatting
        .take()
        .expect("active source formatter")
        .finish()
        .map_err(|message| Diagnostic::new("F002", message, source, Span::default()))
}

/// A complete call prefix retained by the canonical parser during an edit.
#[derive(Clone, Debug)]
pub struct PendingCall {
    pub callee: Expr,
    pub span: Span,
    pub argument: usize,
    pub argument_name: Option<String>,
    pub argument_span: Option<Span>,
}

/// Opening-tag source metadata retained by the ordinary grammar for editor tools.
#[derive(Clone, Debug)]
pub struct MarkupOpening {
    pub name: String,
    pub span: Span,
    pub attributes: Vec<MarkupAttribute>,
}
#[derive(Clone, Debug)]
pub struct MarkupAttribute {
    pub name: String,
    pub span: Span,
    pub value: Option<Span>,
    pub utility: bool,
}

/// Completed declarations and the unfinished call context, without executing or
/// fabricating values. Editors can keep useful help while a call is incomplete.
#[derive(Clone, Debug)]
pub struct Recovery {
    pub module: Module,
    pub diagnostic: Option<Diagnostic>,
    pub pending_calls: Vec<PendingCall>,
    pub markup: Vec<MarkupOpening>,
    pub prefix_end: usize,
}

pub fn parse_recovering(source: &str, text: &str, max_steps: usize) -> Recovery {
    parse_recovering_highlighted(source, text, max_steps, false).0
}
/// Optional source classifications are emitted by the same parser operations.
/// They are editor metadata, never a second token stream used for parsing.
pub fn parse_highlighted(
    source: &str,
    text: &str,
    max_steps: usize,
) -> (Recovery, Vec<sanim_core::code::Highlight>) {
    parse_recovering_highlighted(source, text, max_steps, true)
}
fn parse_recovering_highlighted(
    source: &str,
    text: &str,
    max_steps: usize,
    highlighted: bool,
) -> (Recovery, Vec<sanim_core::code::Highlight>) {
    let mut parser = Parser::new(source, text, max_steps, true);
    parser.highlights = highlighted.then(Vec::new);
    parser.trivia();
    let result = parser.statements(false);
    let (mut module, error) = match result {
        Ok(statements) => (
            Module {
                statements,
                comments: vec![],
            },
            None,
        ),
        Err(error) => (
            parser.recovered.take().unwrap_or(Module {
                statements: vec![],
                comments: vec![],
            }),
            Some(error),
        ),
    };
    let mut pending_calls = parser.pending_calls.take().unwrap_or_default();
    for call in &mut pending_calls {
        call.span.end = parser.offset;
        if let Some(span) = &mut call.argument_span {
            span.end = parser.offset;
        }
    }
    let mut markup = parser.markup.take().unwrap_or_default();
    for opening in &mut markup {
        if opening.span.end == usize::MAX {
            opening.span.end = parser.offset;
        }
        for attribute in &mut opening.attributes {
            if let Some(value) = &mut attribute.value
                && value.end == usize::MAX
            {
                value.end = parser.offset;
            }
        }
    }
    let prefix_end = module
        .statements
        .last()
        .map_or(0, |statement| statement.span.end);
    module.comments = parser.comments;
    let highlights = parser.highlights.take().unwrap_or_default();
    (
        Recovery {
            module,
            diagnostic: parser.failure.or(error),
            pending_calls,
            markup,
            prefix_end,
        },
        highlights,
    )
}

#[derive(Clone)]
struct Mark {
    offset: usize,
    line: usize,
    column: usize,
    delimiters: Vec<u8>,
    formatting: Option<crate::formatter::Checkpoint>,
    comments: usize,
    highlights: usize,
    markup: usize,
}
struct Parser<'a> {
    source: &'a str,
    text: &'a str,
    offset: usize,
    line: usize,
    column: usize,
    depth: usize,
    delimiters: Vec<u8>,
    remaining: usize,
    failure: Option<Diagnostic>,
    recovered: Option<Module>,
    pending_calls: Option<Vec<PendingCall>>,
    markup: Option<Vec<MarkupOpening>>,
    formatting: Option<crate::formatter::Sink>,
    comments: Vec<Span>,
    highlights: Option<Vec<sanim_core::code::Highlight>>,
}
impl<'a> Parser<'a> {
    fn new(source: &'a str, text: &'a str, max_steps: usize, recovery: bool) -> Self {
        Self {
            source,
            text,
            offset: 0,
            line: 1,
            column: 1,
            depth: 0,
            delimiters: vec![],
            remaining: max_steps,
            failure: None,
            recovered: None,
            pending_calls: recovery.then(Vec::new),
            markup: recovery.then(Vec::new),
            formatting: None,
            comments: vec![],
            highlights: None,
        }
    }
}
impl Parser<'_> {
    fn span(&self) -> Span {
        Span {
            start: self.offset,
            end: self.offset + self.rest().chars().next().map_or(0, char::len_utf8),
            line: self.line,
            column: self.column,
        }
    }
    fn rest(&self) -> &str {
        &self.text[self.offset..]
    }
    fn mark(&self) -> Mark {
        Mark {
            offset: self.offset,
            line: self.line,
            column: self.column,
            delimiters: self.delimiters.clone(),
            comments: self.comments.len(),
            highlights: self.highlights.as_ref().map_or(0, Vec::len),
            markup: self.markup.as_ref().map_or(0, Vec::len),
            formatting: self
                .formatting
                .as_ref()
                .map(crate::formatter::Sink::checkpoint),
        }
    }
    fn restore(&mut self, mark: Mark) {
        self.offset = mark.offset;
        self.line = mark.line;
        self.column = mark.column;
        self.delimiters = mark.delimiters;
        self.comments.truncate(mark.comments);
        if let Some(markup) = &mut self.markup {
            markup.truncate(mark.markup);
        }
        if let Some(highlights) = &mut self.highlights {
            highlights.truncate(mark.highlights);
        }
        if let Some(checkpoint) = mark.formatting
            && let Some(sink) = &mut self.formatting
        {
            sink.restore(checkpoint);
        }
    }
    fn error(&self, message: impl Into<String>) -> Diagnostic {
        self.failure
            .clone()
            .unwrap_or_else(|| Diagnostic::new("S002", message, self.source, self.span()))
    }
    fn highlight(&mut self, start: usize, end: usize, kind: &str) {
        if let Some(highlights) = &mut self.highlights
            && start < end
        {
            highlights.push(sanim_core::code::Highlight {
                start,
                end,
                kind: kind.into(),
            });
        }
    }
    fn advance(&mut self, bytes: usize) {
        let mut chars = self.text[self.offset..self.offset + bytes]
            .chars()
            .peekable();
        while let Some(ch) = chars.next() {
            if ch == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                self.line += 1;
                self.column = 1;
            } else if ch == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
        self.offset += bytes;
    }
    fn charge(&mut self) {
        if self.remaining == 0 {
            if self.failure.is_none() {
                self.failure = Some(self.error("parser work limit exceeded"));
            }
        } else {
            self.remaining -= 1;
        }
    }
    fn trivia(&mut self) {
        loop {
            let horizontal = self
                .rest()
                .bytes()
                .take_while(|c| matches!(c, b' ' | b'\t'))
                .count();
            self.advance(horizontal);
            if self.rest().starts_with('#') {
                let count = self
                    .rest()
                    .bytes()
                    .take_while(|c| !matches!(c, b'\n' | b'\r'))
                    .count();
                self.comments.push(Span {
                    end: self.offset + count,
                    ..self.span()
                });
                self.highlight(self.offset, self.offset + count, "comment");
                if let Some(sink) = &mut self.formatting {
                    sink.comment(&self.text[self.offset..self.offset + count]);
                }
                self.advance(count);
                continue;
            }
            if self.line_break() && self.delimiters.iter().any(|c| matches!(c, b'(' | b'[')) {
                self.consume_line();
                continue;
            }
            break;
        }
    }
    fn line_break(&self) -> bool {
        self.rest().starts_with(['\r', '\n'])
    }
    fn eof(&self) -> bool {
        self.offset == self.text.len()
    }
    fn consume_line(&mut self) {
        if let Some(sink) = &mut self.formatting {
            sink.source_line();
        }
        self.advance(if self.rest().starts_with("\r\n") {
            2
        } else {
            1
        });
    }
    fn skip_lines(&mut self) {
        while self.line_break() {
            self.consume_line();
            self.trivia();
        }
    }
    fn is(&self, text: &str) -> bool {
        if !self.rest().starts_with(text) {
            return false;
        }
        if text.as_bytes()[0].is_ascii_alphabetic() || text.starts_with('_') {
            return !self
                .rest()
                .as_bytes()
                .get(text.len())
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
        }
        !matches!(
            (text, self.rest().as_bytes().get(text.len())),
            ("<" | ">" | "=" | "!", Some(b'='))
                | ("*", Some(b'*'))
                | (".", Some(b'.'))
                | ("-", Some(b'>'))
        )
    }
    fn eat(&mut self, text: &str) -> bool {
        if !self.is(text) {
            return false;
        }
        self.charge();
        if self.failure.is_some() {
            return false;
        }
        if text.len() == 1 {
            let c = text.as_bytes()[0];
            if matches!(c, b'(' | b'[' | b'{') {
                self.delimiters.push(c);
                if self.delimiters.len() > 128 && self.failure.is_none() {
                    self.failure = Some(self.error("delimiter nesting limit exceeded"));
                }
            } else if matches!(c, b')' | b']' | b'}') {
                self.delimiters.pop();
            }
        }
        if let Some(sink) = &mut self.formatting {
            sink.token(text);
        }
        let kind = if Keyword::from_name(text).is_some() {
            "keyword"
        } else if text
            .chars()
            .any(|character| "=+-*/%!&|".contains(character))
        {
            "operator"
        } else {
            "punctuation"
        };
        self.highlight(self.offset, self.offset + text.len(), kind);
        self.advance(text.len());
        self.trivia();
        true
    }
    fn expect(&mut self, text: &str) -> Result<(), Diagnostic> {
        if text == ">" && self.rest().starts_with('>') {
            self.charge();
            self.highlight(self.offset, self.offset + 1, "punctuation");
            if let Some(sink) = &mut self.formatting {
                sink.token(">");
            }
            self.advance(1);
            self.trivia();
            if self.failure.is_some() {
                return Err(self.error("parser work limit exceeded"));
            }
            return Ok(());
        }
        if self.eat(text) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{text}'")))
        }
    }
    fn name_start(&self) -> bool {
        self.rest()
            .as_bytes()
            .first()
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
    }
    fn quoted(&self) -> bool {
        self.rest().starts_with(['\'', '"'])
            || self
                .rest()
                .strip_prefix('r')
                .is_some_and(|rest| rest.starts_with(['\'', '"']))
    }
    fn name(&mut self) -> Result<String, Diagnostic> {
        if !self.name_start() {
            return Err(self.error("expected a name"));
        }
        let count = self
            .rest()
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
            .count();
        let name = self.rest()[..count].to_owned();
        if self.highlights.is_some() {
            let kind = if Keyword::from_name(&name).is_some() {
                "keyword"
            } else if crate::prelude::types().contains_key(name.as_str()) {
                "type"
            } else if crate::prelude::builtin(&name).is_some() {
                "function"
            } else {
                "variable"
            };
            self.highlight(self.offset, self.offset + count, kind);
        }
        if let Some(sink) = &mut self.formatting {
            sink.token(&name);
        }
        self.charge();
        self.advance(count);
        self.trivia();
        Ok(name)
    }
    fn string(&mut self) -> Result<String, Diagnostic> {
        let expression = self.string_expression()?;
        if let ExprKind::String(value) = expression.kind {
            Ok(value)
        } else {
            Err(Diagnostic::new(
                "S002",
                "an import path must be a literal string; interpolation is not allowed in module paths",
                self.source,
                expression.span,
            ))
        }
    }
    fn string_expression(&mut self) -> Result<Expr, Diagnostic> {
        if !self.quoted() {
            return Err(self.error("expected a quoted string"));
        }
        self.charge();
        let mut span = self.span();
        if let Some(sink) = &mut self.formatting {
            sink.pause();
        }
        let raw = self.rest().starts_with('r');
        if raw {
            self.advance(1);
        }
        let quote = if self.rest().starts_with("\"\"\"") {
            "\"\"\""
        } else if self.rest().starts_with("'''") {
            "'''"
        } else if self.rest().starts_with('"') {
            "\""
        } else {
            "'"
        };
        self.advance(quote.len());
        let mut expression = self.string_template(Some(quote), raw, span)?;
        if self.eof() {
            return Err(Diagnostic::new(
                "S002",
                format!("unterminated string; expected closing {quote}"),
                self.source,
                span,
            ));
        }
        self.advance(quote.len());
        span.end = self.offset;
        expression.span = span;
        self.highlight(span.start, span.end, "string");
        if let Some(sink) = &mut self.formatting {
            sink.literal(&self.text[span.start..span.end]);
        }
        self.trivia();
        Ok(expression)
    }
    /// Quoted strings and nested format specifications share one expression
    /// parser and escape implementation. None terminates at an interpolation's }.
    fn string_template(
        &mut self,
        quote: Option<&str>,
        raw: bool,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        self.depth += 1;
        if self.depth > 96 {
            return Err(self.error("string interpolation nesting limit exceeded"));
        }
        let mut value = String::new();
        let mut parts = Vec::new();
        let mut literal_span = self.span();
        while !self.eof()
            && !quote.map_or_else(
                || self.rest().starts_with('}'),
                |quote| self.rest().starts_with(quote),
            )
        {
            self.charge();
            if self.failure.is_some() {
                return Err(self.error("parser work limit exceeded"));
            }
            if self.line_break() {
                if quote.is_none_or(|quote| quote.len() == 1) {
                    return Err(Diagnostic::new(
                        "S002",
                        "newline in string literal; use triple quotes (\"\"\"…\"\"\") for multiline text or \\n for a line break",
                        self.source,
                        span,
                    ));
                }
                let count = if self.rest().starts_with("\r\n") {
                    2
                } else {
                    1
                };
                value.push_str(&self.rest()[..count]);
                self.advance(count);
                continue;
            }
            let character_span = self.span();
            let ch = self.rest().chars().next().expect("nonempty source");
            self.advance(ch.len_utf8());
            if !raw && matches!(ch, '{' | '}') {
                if self.rest().starts_with(ch) {
                    self.advance(1);
                    value.push(ch);
                    continue;
                }
                if ch == '}' {
                    return Err(self.error("unmatched '}' in formatted string; use '}}' for a literal brace, or prefix the string with r"));
                }
                if !value.is_empty() {
                    parts.push(sanim_core::TemplatePart::Text(
                        std::mem::take(&mut value).into(),
                    ));
                }
                parts.push(self.string_interpolation(character_span)?);
                literal_span = self.span();
            } else if ch == '\\' {
                let Some(escape) = self.rest().chars().next() else {
                    return Err(self.error("unterminated string escape"));
                };
                self.advance(escape.len_utf8());
                value.push(match escape {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '\\' => '\\',
                    '\'' => '\'',
                    '"' => '"',
                    _ => return Err(self.error("unsupported string escape")),
                });
            } else {
                value.push(ch);
            }
        }
        self.depth -= 1;
        literal_span.end = self.offset;
        if parts.is_empty() {
            return Ok(Expr {
                cache_id: crate::ast::next_expr_identity(),
                kind: ExprKind::String(value),
                span: literal_span,
            });
        }
        if !value.is_empty() {
            parts.push(sanim_core::TemplatePart::Text(value.into()));
        }
        Ok(Expr {
            cache_id: crate::ast::next_expr_identity(),
            kind: ExprKind::Template(parts),
            span: Span {
                end: self.offset,
                ..span
            },
        })
    }
    fn string_interpolation(
        &mut self,
        _span: Span,
    ) -> Result<sanim_core::TemplatePart<Expr>, Diagnostic> {
        // Synthetic parentheses permit multiline expression trivia. Closing an
        // interpolation must not consume trivia: following spaces are text.
        self.delimiters.push(b'(');
        self.trivia();
        if self.rest().starts_with('}') {
            return Err(self.error("empty string interpolation; put an expression between '{' and '}', or use '{{}}' for literal braces"));
        }
        let expression = self.expr(0)?;
        let mut conversion = None;
        if self.rest().starts_with('!') {
            self.advance(1);
            let Some(label @ ('s' | 'r' | 'a')) = self.rest().chars().next() else {
                return Err(self
                    .error("unknown string conversion; supported conversions are !s, !r and !a"));
            };
            self.advance(1);
            self.trivia();
            conversion = Some(label);
        }
        let format = if self.rest().starts_with(':') {
            self.advance(1);
            Some(self.string_template(None, false, self.span())?)
        } else {
            None
        };
        if !self.rest().starts_with('}') {
            return Err(self.error("expected '}' to close string interpolation; use '{{' for a literal opening brace, or prefix the string with r"));
        }
        self.advance(1);
        self.delimiters.pop();
        Ok(sanim_core::TemplatePart::Hole {
            value: expression,
            format,
            conversion,
        })
    }
    fn newline(&mut self) -> Result<(), Diagnostic> {
        if self.eat(";") || self.is("}") || self.eof() {
            return Ok(());
        }
        if self.line_break() {
            self.consume_line();
            self.trivia();
            Ok(())
        } else {
            Err(self.error("expected end of statement (assignment is not allowed)"))
        }
    }
    fn statements(&mut self, block: bool) -> Result<Vec<Statement>, Diagnostic> {
        let mut out = vec![];
        loop {
            while self.line_break() || self.is(";") {
                if self.eat(";") {
                    continue;
                }
                self.consume_line();
                self.trivia();
            }
            if self.eof() {
                if block {
                    return Err(self.error("unclosed block; expected closing brace"));
                }
                break;
            }
            if self.is("}") {
                if block {
                    self.expect("}")?;
                    break;
                }
                return Err(self.error("unexpected closing block"));
            }
            match self.statement() {
                Ok(statement) => out.push(statement),
                Err(error) => {
                    if !block {
                        self.recovered = Some(Module {
                            statements: out,
                            comments: vec![],
                        });
                    }
                    return Err(error);
                }
            }
        }
        if block && out.is_empty() {
            return Err(self.error("empty blocks are forbidden"));
        }
        Ok(out)
    }
    fn block(&mut self) -> Result<Vec<Statement>, Diagnostic> {
        self.expect("{")?;
        self.depth += 1;
        if self.depth > 64 {
            return Err(self.error("block nesting limit exceeded"));
        }
        let out = self.statements(true);
        self.depth -= 1;
        out
    }
    fn generics(&mut self) -> Result<Vec<GenericParameter<TypeExpr>>, Diagnostic> {
        let mut names = vec![];
        if self.eat("<") {
            loop {
                let name = self.name()?;
                let constraint = if self.eat(":") {
                    Some(self.ty()?)
                } else {
                    None
                };
                let default = if self.eat("=") {
                    Some(self.ty()?)
                } else {
                    None
                };
                names.push(GenericParameter {
                    name,
                    constraint,
                    default,
                });
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(">")?;
        }
        Ok(names)
    }
    fn arguments(
        &mut self,
        callee: Option<&Expr>,
    ) -> Result<Vec<(Option<String>, Expr)>, Diagnostic> {
        let frame = if let (Some(callee), Some(calls)) = (callee, &mut self.pending_calls) {
            calls.push(PendingCall {
                callee: callee.clone(),
                span: callee.span,
                argument: 0,
                argument_name: None,
                argument_span: None,
            });
            Some(calls.len() - 1)
        } else {
            None
        };
        let mut args = vec![];
        if !self.eat(")") {
            loop {
                let argument_span = self.span();
                if let (Some(frame), Some(calls)) = (frame, &mut self.pending_calls) {
                    calls[frame].argument = args.len();
                    calls[frame].argument_name = None;
                    calls[frame].argument_span = Some(Span {
                        end: argument_span.start,
                        ..argument_span
                    });
                }
                let candidate = self.expr(0)?;
                if self.is(":") {
                    return Err(self.error("named arguments use '='; write name=value. ':' is reserved for type annotations and dictionary entries"));
                }
                let (named, value) = if self.eat("=") {
                    let ExprKind::Name(name) = candidate.kind else {
                        return Err(self.error("named argument label must be a name"));
                    };
                    let argument_span = self.span();
                    if let (Some(frame), Some(calls)) = (frame, &mut self.pending_calls) {
                        calls[frame].argument_name = Some(name.clone());
                        calls[frame].argument_span = Some(Span {
                            end: argument_span.start,
                            ..argument_span
                        });
                    }
                    (Some(name), self.expr(0)?)
                } else {
                    (None, candidate)
                };
                args.push((named, value));
                if !self.eat(",") {
                    break;
                }
                if self.is(")") {
                    break;
                }
            }
            self.expect(")")?;
        }
        if let (Some(_), Some(calls)) = (frame, &mut self.pending_calls) {
            calls.pop();
        }
        Ok(args)
    }
    fn reflection_options(
        &self,
        args: Vec<(Option<String>, Expr)>,
    ) -> Result<sanim_core::reflection::Options, Diagnostic> {
        let mut options = sanim_core::reflection::Options::default();
        for (name, value) in args {
            let field = match name.as_deref() {
                None => {
                    return Err(Diagnostic::new(
                        "S002",
                        "reflect takes one positional default value; metadata must use label=\"Display Color\" and key=\"palette.accent\"",
                        self.source,
                        value.span,
                    ));
                }
                Some("key") => "key",
                Some("label") => "label",
                Some(other) => {
                    return Err(Diagnostic::new(
                        "S002",
                        format!(
                            "unknown reflection option '{other}'; supported options are key and label"
                        ),
                        self.source,
                        value.span,
                    ));
                }
            };
            let ExprKind::String(text) = value.kind else {
                return Err(Diagnostic::new(
                    "S002",
                    format!(
                        "reflection {field} must be a string literal, for example reflect(Color(), label=\"Display Color\", key=\"palette.accent\")"
                    ),
                    self.source,
                    value.span,
                ));
            };
            let slot = if field == "key" {
                &mut options.key
            } else {
                &mut options.label
            };
            if slot.replace(text).is_some() {
                return Err(Diagnostic::new(
                    "S002",
                    format!("reflection {field} was supplied more than once; provide it only once"),
                    self.source,
                    value.span,
                ));
            }
        }
        Ok(options)
    }
    fn reflected_value(
        &self,
        args: Vec<(Option<String>, Expr)>,
        span: Span,
    ) -> Result<Expr, Diagnostic> {
        let mut args = args.into_iter();
        let Some((None, value)) = args.next() else {
            return Err(Diagnostic::new(
                "S002",
                "reflect requires a positional default value, for example reflect(Color(), label=\"Display Color\"); its type is inferred from that value or the surrounding annotation",
                self.source,
                span,
            ));
        };
        let options = self.reflection_options(args.collect())?;
        Ok(Expr {
            cache_id: crate::ast::next_expr_identity(),
            kind: ExprKind::Reflect {
                value: Box::new(value),
                options,
            },
            span,
        })
    }
    fn declared_name(&mut self, location: &mut Option<Span>) -> Result<String, Diagnostic> {
        let start = self.span();
        let name = self.name()?;
        *location = Some(Span {
            end: start.start + name.len(),
            ..start
        });
        Ok(name)
    }
    fn statement(&mut self) -> Result<Statement, Diagnostic> {
        if self.failure.is_some() {
            return Err(self.error("parser work limit exceeded"));
        }
        let span = self.span();
        let mut name_span = None;
        let duration = if self.eat("@") {
            self.expect("duration")?;
            self.expect("(")?;
            let expression = self.expr(0)?;
            self.expect(")")?;
            self.skip_lines();
            if !self.is(Keyword::Fn.text()) {
                return Err(self.error("@duration(...) must immediately precede a function clause"));
            }
            Some(expression)
        } else {
            None
        };
        let binding = if self.eat(Keyword::Let.text()) {
            Some(BindingKind::Let)
        } else if self.eat(Keyword::Mut.text()) {
            Some(BindingKind::Mut)
        } else if self.eat(Keyword::Sig.text()) {
            Some(BindingKind::Sig)
        } else {
            None
        };
        let kind = if let Some(kind) = binding {
            let name = self.declared_name(&mut name_span)?;
            let annotation = if self.eat(":") {
                Some(self.ty()?)
            } else {
                None
            };
            self.expect("=")?;
            let value = self.expr(0)?;
            self.newline()?;
            StatementKind::Bind {
                kind,
                name,
                annotation,
                value,
            }
        } else if self.eat(Keyword::Background.text()) {
            if self.is("(") {
                return Err(self.error(
                    "background requires a job block; use bg(color) to set the background color",
                ));
            }
            StatementKind::Background {
                body: self.block()?,
            }
        } else if self.eat(Keyword::Animate.text()) {
            self.expect("(")?;
            let mut args = self.arguments(None)?.into_iter();
            let Some((
                None,
                Expr {
                    kind: ExprKind::Name(name),
                    ..
                },
            )) = args.next()
            else {
                return Err(Diagnostic::new(
                    "S002",
                    "animate requires a mutable binding name as its first argument, for example animate(phase, to=360deg, duration=2s)",
                    self.source,
                    span,
                ));
            };
            let mut value = None;
            let mut duration = None;
            let mut easing = None;
            for (label, arg) in args {
                let slot = match label.as_deref() {
                    Some("to") => &mut value,
                    Some("duration") => &mut duration,
                    Some("ease") => &mut easing,
                    _ => {
                        return Err(Diagnostic::new(
                            "S002",
                            "animate options are to=, duration=, and optional ease=; to and duration are required",
                            self.source,
                            arg.span,
                        ));
                    }
                };
                if slot.replace(arg).is_some() {
                    return Err(self.error("animate option was supplied more than once"));
                }
            }
            let (Some(value), Some(duration)) = (value, duration) else {
                return Err(Diagnostic::new(
                    "S002",
                    "animate requires both to= and duration=, for example animate(phase, to=360deg, duration=2s)",
                    self.source,
                    span,
                ));
            };
            self.newline()?;
            StatementKind::Animate {
                name,
                value,
                duration,
                easing,
            }
        } else if self.eat(Keyword::Type.text()) {
            let name = self.declared_name(&mut name_span)?;
            let generics = self.generics()?;
            self.expect("=")?;
            let ty = self.ty()?;
            self.newline()?;
            StatementKind::Alias { name, generics, ty }
        } else if self.eat(Keyword::Enum.text()) {
            let name = self.declared_name(&mut name_span)?;
            self.expect("{")?;
            let mut variants = vec![];
            while !self.eat("}") {
                self.skip_lines();
                if self.eat("}") {
                    break;
                }
                let variant = self.name()?;
                let mut fields = vec![];
                if self.eat("(") && !self.eat(")") {
                    loop {
                        fields.push(self.ty()?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                }
                variants.push((variant, fields));
                if !self.eat(",") {
                    self.skip_lines();
                    self.expect("}")?;
                    break;
                }
            }
            self.newline()?;
            StatementKind::Enum { name, variants }
        } else if self.eat(Keyword::Fn.text()) {
            let name = self.declared_name(&mut name_span)?;
            let generics = self.generics()?;
            self.expect("(")?;
            let mut parameters = vec![];
            if !self.eat(")") {
                loop {
                    let kind = if self.eat(Keyword::Sig.text()) {
                        BindingKind::Sig
                    } else if self.eat(Keyword::Mut.text()) {
                        BindingKind::Mut
                    } else {
                        BindingKind::Let
                    };
                    let mut location = None;
                    let name = self.declared_name(&mut location)?;
                    self.expect(":")?;
                    let ty = self.ty()?;
                    let default = if self.eat("=") {
                        Some(self.expr(0)?)
                    } else {
                        None
                    };
                    parameters.push(Parameter {
                        kind,
                        name,
                        name_span: location.expect("parsed parameter identifier"),
                        ty,
                        default,
                    });
                    if !self.eat(",") {
                        break;
                    }
                    if self.is(")") {
                        break;
                    }
                }
                self.expect(")")?;
            }
            self.expect(":")?;
            let result = self.ty()?;
            let body = self.block()?;
            StatementKind::Function {
                name,
                duration,
                generics,
                parameters,
                result,
                body,
            }
        } else if self.eat(Keyword::Import.text()) {
            let path = self.string()?;
            self.expect(Keyword::As.text())?;
            let alias = self.declared_name(&mut name_span)?;
            self.newline()?;
            StatementKind::Import { path, alias }
        } else if self.eat(Keyword::Return.text()) {
            let value = self.expr(0)?;
            self.newline()?;
            StatementKind::Return(value)
        } else if self.eat(Keyword::If.text()) {
            let condition = self.expr(0)?;
            let yes = self.block()?;
            self.skip_lines();
            let no = if self.eat(Keyword::Else.text()) {
                self.block()?
            } else {
                vec![]
            };
            StatementKind::If { condition, yes, no }
        } else if self.eat(Keyword::For.text()) {
            let name = self.declared_name(&mut name_span)?;
            self.expect(Keyword::In.text())?;
            let iterable = self.expr(0)?;
            let body = self.block()?;
            StatementKind::For {
                name,
                iterable,
                body,
            }
        } else if self.is(Keyword::Repeat.text()) || self.is(Keyword::Timer.text()) {
            let timer = self.eat(Keyword::Timer.text());
            if !timer {
                self.expect(Keyword::Repeat.text())?;
            }
            let count = self.expr(0)?;
            let period = if timer {
                self.expect(Keyword::Every.text())?;
                Some(self.expr(0)?)
            } else {
                None
            };
            let body = self.block()?;
            StatementKind::Repeat {
                count,
                period,
                body,
            }
        } else if self.eat(Keyword::Match.text()) {
            let value = self.expr(0)?;
            self.expect("{")?;
            let mut cases = vec![];
            loop {
                while self.line_break() {
                    self.consume_line();
                    self.trivia();
                }
                if self.eat("}") {
                    break;
                }
                let case_span = self.span();
                self.expect(Keyword::Case.text())?;
                let mut path = self.name()?;
                while self.eat(".") {
                    path.push('.');
                    path.push_str(&self.name()?);
                }
                let (qualifier, variant) = if let Some((prefix, tag)) = path.rsplit_once('.') {
                    (Some(prefix.to_string()), tag.to_string())
                } else {
                    (None, path)
                };
                let mut bindings = vec![];
                if self.eat("(") && !self.eat(")") {
                    loop {
                        bindings.push(self.name()?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                }
                cases.push(Case {
                    qualifier,
                    variant,
                    bindings,
                    body: self.block()?,
                    span: case_span,
                });
            }
            StatementKind::Match { value, cases }
        } else {
            let value = self.expr(0)?;
            if matches!(value.kind, ExprKind::Reflect { .. }) && self.name_start() {
                return Err(Diagnostic::new(
                    "S002",
                    "reflection declarations have been removed; write let name = reflect(default_value, label=\"Display Name\") instead",
                    self.source,
                    span,
                ));
            }
            let kind = if self.eat("=") {
                let ExprKind::Name(name) = value.kind else {
                    return Err(Diagnostic::new(
                        "S002",
                        "assignment requires a mutable binding name; replace the whole value with name = value instead of assigning a field or index",
                        self.source,
                        value.span,
                    ));
                };
                name_span = Some(Span {
                    end: value.span.start + name.len(),
                    ..value.span
                });
                StatementKind::Assign {
                    name,
                    value: self.expr(0)?,
                }
            } else {
                StatementKind::Expr(value)
            };
            self.newline()?;
            kind
        };
        let mut span = span;
        span.end = self.offset;
        Ok(Statement {
            kind,
            span,
            name_span,
        })
    }
    fn ty(&mut self) -> Result<TypeExpr, Diagnostic> {
        self.type_expression(0)
    }
    fn type_expression(&mut self, minimum: u8) -> Result<TypeExpr, Diagnostic> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(self.error("type nesting limit exceeded"));
        }
        let mut out = if self.eat(Keyword::Sig.text()) {
            TypeExpr::Signal(Box::new(self.type_expression(3)?))
        } else if self
            .rest()
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_digit)
        {
            let expression = self.atom()?;
            let ExprKind::Number(text) = expression.kind else {
                unreachable!("numeric type argument")
            };
            let length = sanim_core::Number::parse(&text)
                .and_then(sanim_core::Number::usize)
                .map_err(|reason| self.error(format!("List length {reason}")))?;
            TypeExpr::ConstLength(length)
        } else if self.quoted() {
            let expression = self.string_expression()?;
            match expression.kind {
                ExprKind::String(value) => TypeExpr::StringLiteral(value),
                _ => {
                    return Err(Diagnostic::new(
                        "S002",
                        "a string literal type must contain a fixed string; interpolation is not allowed in types",
                        self.source,
                        expression.span,
                    ));
                }
            }
        } else if self.eat("(") {
            let value = self.ty()?;
            self.expect(")")?;
            value
        } else if self.eat("{") {
            let mut fields = vec![];
            while !self.eat("}") {
                self.skip_lines();
                if self.eat("}") {
                    break;
                }
                let name = self.name()?;
                self.expect(":")?;
                fields.push((name, self.ty()?));
                if !self.eat(",") {
                    self.skip_lines();
                    self.expect("}")?;
                    break;
                }
            }
            TypeExpr::Record(fields)
        } else {
            let type_start = self.offset;
            let mut name = self.name()?;
            self.highlight(type_start, type_start + name.len(), "type");
            while self.eat(".") {
                name.push('.');
                name.push_str(&self.name()?);
            }
            let mut args = vec![];
            if self.is("<") && !self.numeric_refinement_ahead() && self.eat("<") {
                loop {
                    args.push(self.ty()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(">")?;
            }
            TypeExpr::Named(name, args)
        };
        while self.numeric_refinement_ahead() {
            let operator = ["<=", ">=", "==", "<", ">"]
                .into_iter()
                .find(|operator| self.is(operator))
                .expect("numeric comparison");
            self.expect(operator)?;
            let negative = self.eat("-");
            if !negative {
                self.eat("+");
            }
            let expression = self.atom()?;
            let ExprKind::Number(text) = expression.kind else {
                return Err(self.error("numeric refinement requires a fixed numeric literal"));
            };
            let mut number =
                sanim_core::Number::parse(&text).map_err(|reason| self.error(&reason))?;
            if negative {
                number.value = -number.value;
            }
            out = TypeExpr::Refined(Box::new(out), operator.into(), number);
        }
        loop {
            let operator = if self.is("|") {
                "|"
            } else if self.is("&") {
                "&"
            } else if self.is("-") {
                "-"
            } else {
                break;
            };
            let precedence = if operator == "|" { 1 } else { 2 };
            if precedence < minimum {
                break;
            }
            self.expect(operator)?;
            let right = self.type_expression(precedence + 1)?;
            out = match operator {
                "|" => match out {
                    TypeExpr::Union(mut values) => {
                        values.push(right);
                        TypeExpr::Union(values)
                    }
                    left => TypeExpr::Union(vec![left, right]),
                },
                "&" => match out {
                    TypeExpr::Intersection(mut values) => {
                        values.push(right);
                        TypeExpr::Intersection(values)
                    }
                    left => TypeExpr::Intersection(vec![left, right]),
                },
                _ => TypeExpr::Difference(Box::new(out), Box::new(right)),
            };
        }
        self.depth -= 1;
        Ok(out)
    }
    fn numeric_refinement_ahead(&self) -> bool {
        ["<=", ">=", "==", "<", ">"].into_iter().any(|operator| {
            self.rest().strip_prefix(operator).is_some_and(|rest| {
                let rest = rest.trim_start();
                let rest = rest.strip_prefix(['-', '+']).unwrap_or(rest).trim_start();
                rest.as_bytes().first().is_some_and(u8::is_ascii_digit)
            })
        })
    }
    fn expr(&mut self, min: u8) -> Result<Expr, Diagnostic> {
        if self.failure.is_some() {
            return Err(self.error("parser work limit exceeded"));
        }
        self.depth += 1;
        if self.depth > 96 {
            return Err(self.error("expression nesting limit exceeded"));
        }
        let span = self.span();
        let left = if self.is("-") || self.is("+") || self.is(Keyword::Not.text()) || self.is("!") {
            let op = ["-", "+", Keyword::Not.text(), "!"]
                .into_iter()
                .find(|op| self.is(op))
                .expect("unary operator")
                .to_owned();
            if let Some(sink) = &mut self.formatting {
                sink.role(crate::formatter::Role::Unary);
            }
            self.expect(&op)?;
            Expr {
                cache_id: crate::ast::next_expr_identity(),
                kind: ExprKind::Unary(op, Box::new(self.expr(7)?)),
                span,
            }
        } else {
            self.atom()?
        };
        let result = self.expr_tail(left, min, span);
        self.depth -= 1;
        result
    }
    fn expr_tail(&mut self, mut left: Expr, min: u8, span: Span) -> Result<Expr, Diagnostic> {
        let mut steps = 0usize;
        loop {
            steps += 1;
            if steps > 128 {
                return Err(self.error(
                    "expression has too many chained operations; split it into let bindings",
                ));
            }
            if self.eat(".") {
                let member = self.name()?;
                left = Expr {
                    cache_id: crate::ast::next_expr_identity(),
                    kind: ExprKind::Member(Box::new(left), member),
                    span: Span {
                        end: self.offset,
                        ..span
                    },
                };
                continue;
            }
            if self.eat("[") {
                let index = self.expr(0)?;
                self.expect("]")?;
                left = Expr {
                    cache_id: crate::ast::next_expr_identity(),
                    kind: ExprKind::Index(Box::new(left), Box::new(index)),
                    span: Span {
                        end: self.offset,
                        ..span
                    },
                };
                continue;
            }
            let types =
                if self.is("<") && matches!(left.kind, ExprKind::Name(_) | ExprKind::Member(..)) {
                    let saved = self.mark();
                    let saved_depth = self.depth;
                    self.expect("<")?;
                    let attempt = (|| {
                        let mut out = vec![];
                        loop {
                            out.push(self.ty()?);
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect(">")?;
                        if !self.is("(") {
                            return Err(self.error("expected call after type arguments"));
                        }
                        Ok(out)
                    })();
                    match attempt {
                        Ok(v) => v,
                        Err(_) => {
                            self.restore(saved);
                            self.depth = saved_depth;
                            vec![]
                        }
                    }
                } else {
                    vec![]
                };
            if self.eat("(") {
                let args = self.arguments(Some(&left))?;
                left = Expr {
                    cache_id: crate::ast::next_expr_identity(),
                    kind: ExprKind::Call {
                        callee: Box::new(left),
                        types,
                        args,
                    },
                    span: Span {
                        end: self.offset,
                        ..span
                    },
                };
                continue;
            }
            let op = [
                Keyword::Or.text(),
                Keyword::And.text(),
                "==",
                "!=",
                "<=",
                ">=",
                "<",
                ">",
                "+",
                "-",
                "**",
                "*",
                "/",
                "%",
            ]
            .into_iter()
            .find(|op| self.is(op))
            .unwrap_or("");
            let precedence = match op {
                "or" => 1,
                "and" => 2,
                "==" | "!=" => 3,
                "<" | ">" | "<=" | ">=" => 4,
                "+" | "-" => 5,
                "*" | "/" | "%" => 6,
                "**" => 7,
                _ => 0,
            };
            if precedence == 0 || precedence < min {
                break;
            }
            let op = op.to_string();
            if let Some(sink) = &mut self.formatting {
                sink.role(crate::formatter::Role::Binary);
            }
            self.expect(&op)?;
            let right = self.expr(if op == "**" {
                precedence
            } else {
                precedence + 1
            })?;
            left = Expr {
                cache_id: crate::ast::next_expr_identity(),
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
        // A plain string already records its closing quote. Consuming trivia
        // must not make semantic highlighting include a following comment.
        if !matches!(left.kind, ExprKind::String(_)) {
            left.span.end = self.offset;
        }
        Ok(left)
    }
    fn atom(&mut self) -> Result<Expr, Diagnostic> {
        let mut span = self.span();
        let kind = if self
            .rest()
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_digit)
        {
            let count = sanim_core::math::number_literal_len(self.rest());
            let value = self.rest()[..count].to_owned();
            self.highlight(self.offset, self.offset + count, "number");
            if let Some(sink) = &mut self.formatting {
                sink.token(&value);
            }
            self.charge();
            self.advance(count);
            self.trivia();
            ExprKind::Number(value)
        } else if self.is("<") {
            let start = self.offset;
            if let Some(sink) = &mut self.formatting {
                sink.pause();
            }
            let expression = self.markup_expression()?;
            if let Some(sink) = &mut self.formatting {
                sink.literal(&self.text[start..expression.span.end]);
            }
            self.trivia();
            return Ok(expression);
        } else if self.quoted() {
            return self.string_expression();
        } else if self.eat(Keyword::Reflect.text()) {
            if !self.eat("(") {
                return Err(Diagnostic::new(
                    "S002",
                    "reflection declarations have been removed; write let name = reflect(default_value, label=\"Display Name\") instead",
                    self.source,
                    span,
                ));
            }
            let args = self.arguments(None)?;
            return self.reflected_value(
                args,
                Span {
                    end: self.offset,
                    ..span
                },
            );
        } else if self.name_start() {
            let name = self.name()?;
            match Keyword::from_name(&name) {
                Some(Keyword::True) => ExprKind::Bool(true),
                Some(Keyword::False) => ExprKind::Bool(false),
                Some(Keyword::None) => ExprKind::None,
                _ => ExprKind::Name(name),
            }
        } else if self.eat("(") {
            let value = self.expr(0)?;
            self.expect(")")?;
            return Ok(value);
        } else if self.eat("[") {
            let mut values = vec![];
            while !self.eat("]") {
                values.push(self.expr(0)?);
                if !self.eat(",") {
                    self.expect("]")?;
                    break;
                }
            }
            ExprKind::List(values)
        } else if self.eat("{") {
            self.skip_lines();
            if self.quoted() || self.is("}") {
                let mut entries = vec![];
                while !self.eat("}") {
                    self.skip_lines();
                    if self.eat("}") {
                        break;
                    }
                    let key = self.expr(0)?;
                    self.expect(":")?;
                    entries.push((key, self.expr(0)?));
                    if !self.eat(",") {
                        self.skip_lines();
                        self.expect("}")?;
                        break;
                    }
                }
                span.end = self.offset;
                return Ok(Expr {
                    cache_id: crate::ast::next_expr_identity(),
                    kind: ExprKind::Dict(entries),
                    span,
                });
            }
            let mut fields = vec![];
            while !self.eat("}") {
                self.skip_lines();
                if self.eat("}") {
                    break;
                }
                let name = self.name()?;
                if !self.eat("=") && !self.eat(":") {
                    return Err(self.error("expected '=' between a record field name and its value; type annotations use ':'"));
                }
                fields.push((name, self.expr(0)?));
                if !self.eat(",") {
                    self.skip_lines();
                    self.expect("}")?;
                    break;
                }
            }
            ExprKind::Record(fields)
        } else {
            return Err(self.error("expected an expression"));
        };
        span.end = self.offset;
        Ok(Expr {
            cache_id: crate::ast::next_expr_identity(),
            kind,
            span,
        })
    }
}
