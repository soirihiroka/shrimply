use crate::ast::*;
use sanim_core::{Diagnostic, GenericParameter, Span};

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
keywords! { Let=>"let",Reflect=>"reflect",Fn=>"fn",Type=>"type",Enum=>"enum",Import=>"import",As=>"as",If=>"if",Else=>"else",For=>"for",In=>"in",Return=>"return",Match=>"match",Case=>"case",And=>"and",Or=>"or",Not=>"not",Repeat=>"repeat",Every=>"every",Timer=>"timer",True=>"true",False=>"false",None=>"None" }

/// Parse source directly into the AST. No token list or intermediate syntax tree.
pub fn parse(source: &str, text: &str, max_steps: usize) -> Result<Module, Diagnostic> {
    let mut parser = Parser {
        source,
        text,
        offset: 0,
        line: 1,
        column: 1,
        depth: 0,
        delimiters: vec![],
        remaining: max_steps,
        failure: None,
    };
    parser.trivia();
    let statements = parser.statements(false)?;
    if let Some(error) = parser.failure {
        return Err(error);
    }
    Ok(Module { statements })
}
#[derive(Clone)]
struct Mark {
    offset: usize,
    line: usize,
    column: usize,
    delimiters: Vec<u8>,
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
        }
    }
    fn restore(&mut self, mark: Mark) {
        self.offset = mark.offset;
        self.line = mark.line;
        self.column = mark.column;
        self.delimiters = mark.delimiters;
    }
    fn error(&self, message: impl Into<String>) -> Diagnostic {
        self.failure
            .clone()
            .unwrap_or_else(|| Diagnostic::new("S002", message, self.source, self.span()))
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
    fn color_len(&self) -> Option<usize> {
        let bytes = self.rest().as_bytes();
        if bytes.first() != Some(&b'#') {
            return None;
        }
        let count = bytes[1..]
            .iter()
            .take_while(|c| c.is_ascii_hexdigit())
            .count();
        (matches!(count, 6 | 8)
            && !bytes
                .get(count + 1)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_'))
        .then_some(count + 1)
    }
    fn trivia(&mut self) {
        loop {
            let horizontal = self
                .rest()
                .bytes()
                .take_while(|c| matches!(c, b' ' | b'\t'))
                .count();
            self.advance(horizontal);
            if self.rest().starts_with("//")
                || self.rest().starts_with('#') && self.color_len().is_none()
            {
                let count = self
                    .rest()
                    .bytes()
                    .take_while(|c| !matches!(c, b'\n' | b'\r'))
                    .count();
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
        self.advance(text.len());
        self.trivia();
        true
    }
    fn expect(&mut self, text: &str) -> Result<(), Diagnostic> {
        if text == ">" && self.rest().starts_with('>') {
            self.charge();
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
        self.charge();
        self.advance(count);
        self.trivia();
        Ok(name)
    }
    fn string(&mut self) -> Result<String, Diagnostic> {
        if !self.quoted() {
            return Err(self.error("expected a quoted string"));
        }
        self.charge();
        let start = self.span();
        let quote = self.rest().as_bytes()[0];
        self.advance(1);
        let mut value = String::new();
        while !self.eof() && self.rest().as_bytes()[0] != quote {
            if self.line_break() {
                return Err(Diagnostic::new(
                    "S002",
                    "newline in string literal",
                    self.source,
                    start,
                ));
            }
            let ch = self.rest().chars().next().expect("nonempty source");
            self.advance(ch.len_utf8());
            if ch == '\\' {
                let Some(escape) = self.rest().chars().next() else {
                    return Err(self.error("unterminated escape"));
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
        if self.eof() {
            return Err(Diagnostic::new(
                "S002",
                "unterminated string",
                self.source,
                start,
            ));
        }
        self.advance(1);
        self.trivia();
        Ok(value)
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
            out.push(self.statement()?);
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
    fn arguments(&mut self) -> Result<Vec<(Option<String>, Expr)>, Diagnostic> {
        let mut args = vec![];
        if !self.eat(")") {
            loop {
                let candidate = self.expr(0)?;
                let (named, value) = if self.eat("=") || self.eat(":") {
                    let ExprKind::Name(name) = candidate.kind else {
                        return Err(self.error("named argument label must be a name"));
                    };
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
        Ok(args)
    }
    fn reflection_options(
        &self,
        args: Vec<(Option<String>, Expr)>,
        positional_label: bool,
    ) -> Result<sanim_core::reflection::Options, Diagnostic> {
        let mut options = sanim_core::reflection::Options::default();
        let mut named = false;
        for (name, value) in args {
            let field = match name.as_deref() {
                None if positional_label && !named => "label",
                None => {
                    return Err(Diagnostic::new(
                        "S002",
                        if positional_label {
                            "reflection display label must precede named options; use label=\"Display Color\", key=\"palette.accent\""
                        } else {
                            "reflect takes one positional default value; metadata must use label=\"Display Color\" and key=\"palette.accent\""
                        },
                        self.source,
                        value.span,
                    ));
                }
                Some("key") => {
                    named = true;
                    "key"
                }
                Some("label") => {
                    named = true;
                    "label"
                }
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
        let options = self.reflection_options(args.collect(), false)?;
        Ok(Expr {
            kind: ExprKind::Reflect {
                value: Box::new(value),
                options,
            },
            span,
        })
    }
    fn statement(&mut self) -> Result<Statement, Diagnostic> {
        if self.failure.is_some() {
            return Err(self.error("parser work limit exceeded"));
        }
        let span = self.span();
        // The old declaration form lowers to the same reflection expression.
        // Only inspect the token after its arguments; expression statements keep normal parsing.
        let legacy = if self.eat(Keyword::Reflect.text()) {
            let parenthesized = self.eat("(");
            let args = if parenthesized {
                self.arguments()?
            } else {
                vec![]
            };
            let after_arguments = self.mark();
            let binding = if self.name_start() {
                self.name()?;
                self.is("=") || self.is(":")
            } else {
                false
            };
            self.restore(after_arguments);
            if binding {
                Some(self.reflection_options(args, true)?)
            } else {
                if !parenthesized {
                    return Err(self.error("reflect requires a default value in parentheses, for example let color = reflect(Color())"));
                }
                let value = self.reflected_value(
                    args,
                    Span {
                        end: self.offset,
                        ..span
                    },
                )?;
                let value = self.expr_tail(value, 0, span)?;
                self.newline()?;
                return Ok(Statement {
                    kind: StatementKind::Expr(value),
                    span: Span {
                        end: self.offset,
                        ..span
                    },
                });
            }
        } else {
            None
        };
        let kind = if legacy.is_some() || self.eat(Keyword::Let.text()) {
            let name = self.name()?;
            let annotation = if self.eat(":") {
                Some(self.ty()?)
            } else {
                None
            };
            self.expect("=")?;
            let value = self.expr(0)?;
            let value = if let Some(options) = legacy {
                Expr {
                    kind: ExprKind::Reflect {
                        value: Box::new(value),
                        options,
                    },
                    span: Span {
                        end: self.offset,
                        ..span
                    },
                }
            } else {
                value
            };
            self.newline()?;
            StatementKind::Bind {
                name,
                annotation,
                value,
            }
        } else if self.eat(Keyword::Type.text()) {
            let name = self.name()?;
            let generics = self.generics()?;
            self.expect("=")?;
            let ty = self.ty()?;
            self.newline()?;
            StatementKind::Alias { name, generics, ty }
        } else if self.eat(Keyword::Enum.text()) {
            let name = self.name()?;
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
            let name = self.name()?;
            let generics = self.generics()?;
            self.expect("(")?;
            let mut parameters = vec![];
            if !self.eat(")") {
                loop {
                    let name = self.name()?;
                    self.expect(":")?;
                    let ty = self.ty()?;
                    let default = if self.eat("=") {
                        Some(self.expr(0)?)
                    } else {
                        None
                    };
                    parameters.push((name, ty, default));
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
                generics,
                parameters,
                result,
                body,
            }
        } else if self.eat(Keyword::Import.text()) {
            let path = self.string()?;
            self.expect(Keyword::As.text())?;
            let alias = self.name()?;
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
            let name = self.name()?;
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
            self.newline()?;
            StatementKind::Expr(value)
        };
        let mut span = span;
        span.end = self.offset;
        Ok(Statement { kind, span })
    }
    fn ty(&mut self) -> Result<TypeExpr, Diagnostic> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(self.error("type nesting limit exceeded"));
        }
        let out = if self.eat("{") {
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
            let mut name = self.name()?;
            while self.eat(".") {
                name.push('.');
                name.push_str(&self.name()?);
            }
            let mut args = vec![];
            if self.eat("<") {
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
        self.depth -= 1;
        Ok(out)
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
            self.expect(&op)?;
            Expr {
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
                    kind: ExprKind::Member(Box::new(left), member),
                    span,
                };
                continue;
            }
            if self.eat("[") {
                let index = self.expr(0)?;
                self.expect("]")?;
                left = Expr {
                    kind: ExprKind::Index(Box::new(left), Box::new(index)),
                    span,
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
                let args = self.arguments()?;
                left = Expr {
                    kind: ExprKind::Call {
                        callee: Box::new(left),
                        types,
                        args,
                    },
                    span,
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
            self.expect(&op)?;
            let right = self.expr(if op == "**" {
                precedence
            } else {
                precedence + 1
            })?;
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
        left.span.end = self.offset;
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
            let bytes = self.rest().as_bytes();
            let mut count = bytes
                .iter()
                .take_while(|c| c.is_ascii_digit() || **c == b'_')
                .count();
            if bytes.get(count) == Some(&b'.')
                && bytes.get(count + 1).is_some_and(u8::is_ascii_digit)
            {
                count += 1;
                while bytes.get(count).is_some_and(u8::is_ascii_digit) {
                    count += 1;
                }
            }
            while bytes.get(count).is_some_and(u8::is_ascii_alphabetic) {
                count += 1;
            }
            if bytes.get(count) == Some(&b'%')
                && !bytes.get(count + 1).is_some_and(u8::is_ascii_digit)
            {
                count += 1;
            }
            let value = self.rest()[..count].to_owned();
            self.charge();
            self.advance(count);
            self.trivia();
            ExprKind::Number(value)
        } else if self.quoted() {
            ExprKind::String(self.string()?)
        } else if let Some(count) = self.color_len() {
            let value = self.rest()[..count].to_owned();
            self.charge();
            self.advance(count);
            self.trivia();
            ExprKind::Color(value)
        } else if self.eat(Keyword::Reflect.text()) {
            self.expect("(")?;
            let args = self.arguments()?;
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
        Ok(Expr { kind, span })
    }
}
