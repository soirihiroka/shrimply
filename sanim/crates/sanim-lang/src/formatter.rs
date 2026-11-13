//! Canonical source layout emitted by the existing parser, without a token stream.
use crate::parser::Keyword;
use sanim_core::Diagnostic;
const MAX_FORMATTED_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct FormatOptions {
    pub indent_width: usize,
    pub line_width: usize,
    pub use_tabs: bool,
}
impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            indent_width: 4,
            line_width: 100,
            use_tabs: false,
        }
    }
}
impl FormatOptions {
    pub fn validate(self) -> Result<(), String> {
        if !(1..=16).contains(&self.indent_width) || !(20..=1000).contains(&self.line_width) {
            return Err("formatter indent width must be 1..=16 and line width 20..=1000".into());
        }
        Ok(())
    }
}

/// Parse and format source without evaluating it or loading imported/data files.
/// Comments and complete raw/formatted string literals retain their source text.
pub fn format_source(
    source: &str,
    text: &str,
    options: FormatOptions,
    max_steps: usize,
) -> Result<String, Diagnostic> {
    options
        .validate()
        .map_err(|message| Diagnostic::new("F001", message, source, Default::default()))?;
    crate::parser::format(source, text, options, max_steps)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Word,
    Literal,
    Open(u8),
    Close(u8),
    Comma,
    Colon,
    Dot,
    Binary,
    Unary,
    UnaryWord,
    Comment,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Role {
    Unary,
    Binary,
}
#[derive(Clone)]
struct State {
    last: Option<Kind>,
    keyword: Option<Keyword>,
    delimiters: Vec<u8>,
    source_lines: usize,
    force_line: bool,
    suspended: usize,
    role: Option<Role>,
    column: usize,
    failed: bool,
}
#[derive(Clone)]
pub(crate) struct Checkpoint {
    length: usize,
    state: Box<State>,
}
pub(crate) struct Sink {
    output: String,
    state: State,
    options: FormatOptions,
}
impl Sink {
    pub fn new(options: FormatOptions) -> Self {
        Self {
            output: String::new(),
            state: State {
                last: None,
                keyword: None,
                delimiters: Vec::new(),
                source_lines: 0,
                force_line: false,
                suspended: 0,
                role: None,
                column: 0,
                failed: false,
            },
            options,
        }
    }
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            length: self.output.len(),
            state: Box::new(self.state.clone()),
        }
    }
    pub fn restore(&mut self, checkpoint: Checkpoint) {
        self.output.truncate(checkpoint.length);
        self.state = *checkpoint.state;
    }
    pub fn role(&mut self, role: Role) {
        if self.state.suspended == 0 {
            self.state.role = Some(role);
        }
    }
    pub fn pause(&mut self) {
        self.state.suspended += 1;
    }
    pub fn literal(&mut self, text: &str) {
        self.state.suspended = self
            .state
            .suspended
            .checked_sub(1)
            .expect("active formatter literal");
        if self.state.suspended == 0 {
            self.emit(text, Kind::Literal);
        }
    }
    pub fn source_line(&mut self) {
        if self.state.suspended == 0 {
            self.state.source_lines = (self.state.source_lines + 1).min(2);
        }
    }
    pub fn comment(&mut self, text: &str) {
        if self.state.suspended != 0 {
            return;
        }
        self.gap(true);
        self.append(text);
        self.state.column += text.chars().count();
        self.state.last = Some(Kind::Comment);
        self.state.force_line = true;
    }
    pub fn token(&mut self, text: &str) {
        if self.state.suspended != 0 {
            return;
        }
        let role = self.state.role.take();
        let kind = match (text, role) {
            (_, Some(Role::Binary)) => Kind::Binary,
            ("not", Some(Role::Unary)) => Kind::UnaryWord,
            (_, Some(Role::Unary)) => Kind::Unary,
            ("(", _) => Kind::Open(b'('),
            ("[", _) => Kind::Open(b'['),
            ("{", _) => Kind::Open(b'{'),
            ("<", _) => Kind::Open(b'<'),
            (")", _) => Kind::Close(b'('),
            ("]", _) => Kind::Close(b'['),
            ("}", _) => Kind::Close(b'{'),
            (">", _) => Kind::Close(b'<'),
            (",", _) => Kind::Comma,
            (":", _) => Kind::Colon,
            (".", _) => Kind::Dot,
            (
                "=" | "->" | "+" | "-" | "*" | "/" | "%" | "**" | "==" | "!=" | "<=" | ">=" | "|"
                | "&" | "and" | "or",
                _,
            ) => Kind::Binary,
            (";", _) => {
                self.state.force_line = true;
                return;
            }
            _ => Kind::Word,
        };
        self.emit(text, kind);
    }
    fn emit(&mut self, text: &str, kind: Kind) {
        let previous = self.state.last;
        if matches!(kind, Kind::Close(_)) {
            self.state.delimiters.pop();
        }
        if kind == Kind::Close(b'{') {
            if previous == Some(Kind::Open(b'{')) {
                self.state.force_line = false;
                self.state.source_lines = 0;
            } else {
                self.state.force_line = true;
            }
        }
        if crate::parser::Keyword::from_name(text) == Some(Keyword::Else)
            && previous == Some(Kind::Close(b'{'))
        {
            self.state.force_line = false;
            self.state.source_lines = 0;
        }
        let space = match kind {
            Kind::Close(_) | Kind::Comma | Kind::Colon | Kind::Dot | Kind::Open(b'<') => false,
            Kind::Open(b'(') => {
                matches!(
                    previous,
                    Some(Kind::Binary | Kind::Colon | Kind::Comma | Kind::UnaryWord)
                ) || self.keyword_space()
            }
            Kind::Open(b'[') => {
                matches!(
                    previous,
                    Some(Kind::Binary | Kind::Colon | Kind::Comma | Kind::UnaryWord)
                ) || self.keyword_space()
            }
            Kind::Open(b'{') => !matches!(previous, None | Some(Kind::Open(_) | Kind::Dot)),
            Kind::Binary => true,
            Kind::Unary => !matches!(
                previous,
                None | Some(
                    Kind::Open(_)
                        | Kind::Binary
                        | Kind::Unary
                        | Kind::Colon
                        | Kind::Comma
                        | Kind::Dot
                )
            ),
            _ => matches!(
                previous,
                Some(
                    Kind::Word
                        | Kind::Literal
                        | Kind::Close(_)
                        | Kind::Binary
                        | Kind::Colon
                        | Kind::Comma
                        | Kind::UnaryWord
                )
            ),
        };
        self.gap(space);
        self.append(text);
        self.state.column = if text.contains(['\n', '\r']) {
            text.rsplit(['\n', '\r'])
                .next()
                .unwrap_or("")
                .chars()
                .count()
        } else {
            self.state.column + text.chars().count()
        };
        self.state.last = Some(kind);
        self.state.keyword = if matches!(kind, Kind::Word | Kind::UnaryWord | Kind::Binary) {
            Keyword::from_name(text)
        } else {
            None
        };
        if let Kind::Open(delimiter) = kind {
            self.state.delimiters.push(delimiter);
        }
        if kind == Kind::Open(b'{') {
            self.state.force_line = true;
        }
        if kind == Kind::Comma
            && (self.state.delimiters.last() == Some(&b'{')
                || self.state.column >= self.options.line_width
                    && self
                        .state
                        .delimiters
                        .iter()
                        .any(|delimiter| matches!(delimiter, b'(' | b'[')))
        {
            self.state.force_line = true;
        }
    }
    fn keyword_space(&self) -> bool {
        self.state.keyword.is_some_and(|keyword| {
            !matches!(
                keyword,
                Keyword::Reflect
                    | Keyword::Animate
                    | Keyword::True
                    | Keyword::False
                    | Keyword::None
            )
        })
    }
    fn gap(&mut self, space: bool) {
        let lines = self
            .state
            .source_lines
            .max(usize::from(self.state.force_line));
        if !self.output.is_empty() && lines > 0 {
            self.append(&"\n".repeat(lines));
            self.state.column = 0;
        } else if !self.output.is_empty() && space && self.state.column > 0 {
            self.append(" ");
            self.state.column += 1;
        }
        if self.state.column == 0 {
            let depth = self
                .state
                .delimiters
                .iter()
                .filter(|delimiter| **delimiter != b'<')
                .count();
            if self.options.use_tabs {
                self.append(&"\t".repeat(depth));
            } else {
                self.append(&" ".repeat(depth * self.options.indent_width));
            }
            self.state.column = depth * self.options.indent_width;
        }
        self.state.source_lines = 0;
        self.state.force_line = false;
    }
    fn append(&mut self, text: &str) {
        if self.state.failed {
            return;
        }
        if self.output.len().saturating_add(text.len()) > MAX_FORMATTED_BYTES {
            self.state.failed = true;
            return;
        }
        self.output.push_str(text);
    }
    pub fn finish(self) -> Result<String, &'static str> {
        if self.state.failed {
            return Err(
                "formatted source exceeds the 16 MiB output budget; split this source into smaller modules",
            );
        }
        let mut output = self.output;
        if !output.is_empty() {
            output.push('\n');
        }
        Ok(output)
    }
}
