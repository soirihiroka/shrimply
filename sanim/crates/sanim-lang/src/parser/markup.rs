//! Native element notation lowers directly to ordinary, typed constructor calls.
use super::*;

fn call(name: &str, args: Vec<(Option<String>, Expr)>, span: Span) -> Expr {
    Expr {
        cache_id: crate::ast::next_expr_identity(),
        kind: ExprKind::Call {
            callee: Box::new(Expr {
                cache_id: crate::ast::next_expr_identity(),
                kind: ExprKind::Name(name.into()),
                span: Span {
                    end: span.start,
                    ..span
                },
            }),
            types: vec![],
            args,
        },
        span,
    }
}
fn tagged_call(name: &str, args: Vec<(Option<String>, Expr)>, span: Span, tag_span: Span) -> Expr {
    let mut expression = call(name, args, span);
    if let ExprKind::Call { callee, .. } = &mut expression.kind {
        callee.span = tag_span;
    }
    expression
}
fn string(value: String, span: Span) -> Expr {
    Expr {
        cache_id: crate::ast::next_expr_identity(),
        kind: ExprKind::String(value),
        span,
    }
}

/// Remove the enclosing markup indentation, preserving the code's relative indentation and EOLs.
fn dedent_literal(text: &str) -> String {
    let mut lines = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        let end = remaining
            .find(['\r', '\n'])
            .map_or(remaining.len(), |offset| {
                offset
                    + if remaining[offset..].starts_with("\r\n") {
                        2
                    } else {
                        1
                    }
            });
        lines.push(&remaining[..end]);
        remaining = &remaining[end..];
    }
    let Some(first) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let last = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .expect("nonempty literal line");
    let lines = &lines[first..=last];
    let margin = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            &line[..line
                .bytes()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count()]
        })
        .reduce(|prefix, indent| {
            let length = prefix
                .bytes()
                .zip(indent.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            &prefix[..length]
        })
        .unwrap_or("");
    lines
        .iter()
        .map(|line| line.strip_prefix(margin).unwrap_or(line))
        .collect()
}

fn merge_utilities(
    mut args: Vec<(Option<String>, Expr)>,
    defaults: Vec<(String, Expr)>,
) -> Vec<(Option<String>, Expr)> {
    for (key, value) in defaults {
        if !args
            .iter()
            .any(|(name, _)| name.as_deref() == Some(key.as_str()))
        {
            args.push((Some(key), value));
        }
    }
    args
}

impl Parser<'_> {
    pub(super) fn markup_expression(&mut self) -> Result<Expr, Diagnostic> {
        self.depth += 1;
        if self.depth > 96 {
            return Err(self.error(
                "native markup nesting exceeds 96 elements; split the component into functions",
            ));
        }
        let result = self.markup_element();
        self.depth -= 1;
        result
    }
    fn markup_element(&mut self) -> Result<Expr, Diagnostic> {
        let mut span = self.span();
        self.charge();
        self.highlight(self.offset, self.offset + 1, "punctuation");
        self.advance(1);
        if !self.name_start() {
            return Err(
                self.error("expected a native element name after '<', such as Rect or Label")
            );
        }
        let mut tag_span = self.span();
        let tag = self.name()?;
        tag_span.end = tag_span.start + tag.len();
        self.highlight(tag_span.start, tag_span.end, "tag");
        let constructor = tag.as_str();
        let opening = self.markup.as_mut().map(|openings| {
            let index = openings.len();
            openings.push(super::MarkupOpening {
                name: tag.clone(),
                span: Span {
                    start: tag_span.end,
                    end: usize::MAX,
                    ..tag_span
                },
                attributes: vec![],
            });
            index
        });
        let mut args: Vec<(Option<String>, Expr)> = Vec::new();
        let mut utilities: Vec<(String, Expr)> = Vec::new();
        loop {
            self.trivia();
            self.skip_lines();
            if self.rest().starts_with("/>") {
                self.highlight(self.offset, self.offset + 2, "punctuation");
                self.advance(2);
                span.end = self.offset;
                if let (Some(index), Some(openings)) = (opening, &mut self.markup) {
                    openings[index].span.end = self.offset - 2;
                }
                return Ok(tagged_call(
                    constructor,
                    merge_utilities(args, utilities),
                    span,
                    tag_span,
                ));
            }
            if self.rest().starts_with('>') {
                self.highlight(self.offset, self.offset + 1, "punctuation");
                self.advance(1);
                if let (Some(index), Some(openings)) = (opening, &mut self.markup) {
                    openings[index].span.end = self.offset - 1;
                }
                break;
            }
            if self.eof() {
                return Err(self.error(format!(
                    "unterminated <{tag}> opening tag; expected '>' or '/>'"
                )));
            }
            let mut attribute_span = self.span();
            if !self.name_start() {
                return Err(self.error("expected a property or utility after the element name"));
            }
            let count = self
                .rest()
                .bytes()
                .take_while(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'%')
                })
                .count();
            let attribute = self.rest()[..count].to_owned();
            self.highlight(self.offset, self.offset + count, "property");
            if let Some(sink) = &mut self.formatting {
                sink.token(&attribute);
            }
            self.charge();
            self.advance(count);
            attribute_span.end = self.offset;
            self.trivia();
            let assigned = self.rest().starts_with('=');
            let boolean = crate::prelude::builtin(constructor).is_some_and(|builtin| {
                builtin.parameters.iter().any(|parameter| {
                    parameter.name == attribute && parameter.ty == sanim_core::Type::Bool
                })
            });
            let attribute_index = if let (Some(index), Some(openings)) = (opening, &mut self.markup)
            {
                let attributes = &mut openings[index].attributes;
                let attribute_index = attributes.len();
                attributes.push(super::MarkupAttribute {
                    name: attribute.clone(),
                    span: attribute_span,
                    value: None,
                    utility: !assigned && !boolean,
                });
                Some(attribute_index)
            } else {
                None
            };
            if !assigned && !boolean {
                let lowered = crate::utilities::lower(constructor, &attribute, attribute_span)
                    .map_err(|message| {
                        Diagnostic::new("S002", message, self.source, attribute_span)
                    })?;
                for (key, value) in lowered {
                    utilities.retain(|(previous, _)| *previous != key);
                    utilities.push((key, value));
                }
                continue;
            }
            if attribute == "design" {
                return Err(self.error("design= has been removed; put native utilities directly on the opening tag, such as <Box w-180 flex flex-col>"));
            }
            if args
                .iter()
                .any(|(name, _)| name.as_deref() == Some(attribute.as_str()))
            {
                return Err(self.error(format!("duplicate '{attribute}' attribute on <{tag}>")));
            }
            let mut value = if self.eat("=") {
                if let (Some(index), Some(attribute), Some(openings)) =
                    (opening, attribute_index, &mut self.markup)
                {
                    openings[index].attributes[attribute].value = Some(Span {
                        start: self.offset,
                        end: usize::MAX,
                        ..attribute_span
                    });
                }
                if self.quoted() {
                    self.string_expression()?
                } else if self.eat("{") {
                    let expression = self.expr(0)?;
                    self.expect("}")?;
                    expression
                } else {
                    return Err(self.error(format!("attribute '{attribute}' requires a quoted string or {{expression}}; write {attribute}={{value}}")));
                }
            } else {
                Expr {
                    cache_id: crate::ast::next_expr_identity(),
                    kind: ExprKind::Bool(true),
                    span: attribute_span,
                }
            };
            if let (Some(index), Some(attribute), Some(openings)) =
                (opening, attribute_index, &mut self.markup)
            {
                openings[index].attributes[attribute].value = Some(value.span);
            }
            // Constructor metadata owns the type rule. Color-valued string
            // attributes are just the existing Color(...) call, never CSS parsing.
            if crate::prelude::builtin(constructor)
                .and_then(|builtin| {
                    builtin
                        .parameters
                        .iter()
                        .find(|parameter| parameter.name == attribute)
                })
                .is_some_and(|parameter| {
                    parameter.ty.accepts(&sanim_core::Type::Color)
                        && !parameter.ty.accepts(&sanim_core::Type::String)
                })
                && matches!(value.kind, ExprKind::String(_))
            {
                let conversion_span = Span {
                    end: value.span.end,
                    ..attribute_span
                };
                value = call("Color", vec![(None, value)], conversion_span);
            }
            args.push((Some(attribute), value));
        }
        if let Some(crate::prelude::MarkupContent::Literal { parameter, dedent }) =
            crate::prelude::builtin(constructor).map(|builtin| builtin.content)
        {
            let mut body_span = self.span();
            let closing = format!("</{tag}>");
            let count = self.rest().find(&closing).ok_or_else(|| {
                self.error(format!("unclosed <{tag}> literal body; expected {closing}"))
            })?;
            let raw = &self.text[self.offset..self.offset + count];
            let body = if dedent {
                dedent_literal(raw)
            } else {
                raw.to_owned()
            };
            self.charge();
            self.highlight(self.offset, self.offset + count, "string");
            self.advance(count);
            body_span.end = self.offset;
            if !body.trim().is_empty()
                && args
                    .iter()
                    .any(|(name, _)| name.as_deref() == Some(parameter))
            {
                return Err(self.error(format!(
                    "<{tag}> cannot specify both {parameter}= and a literal body"
                )));
            }
            if !body.trim().is_empty() {
                args.push((Some(parameter.into()), string(body, body_span)));
            }
            self.highlight(self.offset, self.offset + 2, "punctuation");
            self.highlight(self.offset + 2, self.offset + 2 + tag.len(), "tag");
            self.highlight(
                self.offset + closing.len() - 1,
                self.offset + closing.len(),
                "punctuation",
            );
            self.advance(closing.len());
            span.end = self.offset;
            return Ok(tagged_call(
                constructor,
                merge_utilities(args, utilities),
                span,
                tag_span,
            ));
        }
        let text_element = crate::prelude::builtin(constructor).is_some_and(|builtin| {
            builtin.result == sanim_core::Type::Element
                && builtin.parameters.iter().any(|parameter| {
                    parameter.name == "text" && parameter.ty == sanim_core::Type::String
                })
        });
        let mut body_span = self.span();
        let mut children = Vec::new();
        let mut text = Vec::new();
        loop {
            if self.eof() {
                return Err(self.error(format!("unclosed <{tag}> element; expected </{tag}>")));
            }
            if self.rest().starts_with("</") {
                body_span.end = self.offset;
                self.highlight(self.offset, self.offset + 2, "punctuation");
                self.advance(2);
                let start = self.offset;
                let closing = self.name()?;
                self.highlight(start, start + closing.len(), "tag");
                if closing != tag {
                    return Err(self.error(format!(
                        "closing </{closing}> does not match <{tag}>; expected </{tag}>"
                    )));
                }
                if !self.rest().starts_with('>') {
                    return Err(self.error(format!("expected '>' after </{tag}")));
                }
                self.highlight(self.offset, self.offset + 1, "punctuation");
                self.advance(1);
                break;
            }
            if self.rest().starts_with('<') {
                if text_element {
                    return Err(self.error(format!(
                        "<{tag}> contains text, not nested elements; put labels inside a Box"
                    )));
                }
                children.push(self.markup_expression()?);
                continue;
            }
            if self.rest().starts_with('{') && !self.rest().starts_with("{{") {
                let expression_span = self.span();
                self.advance(1);
                if text_element {
                    text.push(self.string_interpolation(expression_span)?);
                } else {
                    self.delimiters.push(b'(');
                    self.trivia();
                    let expression = self.expr(0)?;
                    if !self.rest().starts_with('}') {
                        return Err(self.error("expected '}' after the child expression"));
                    }
                    self.advance(1);
                    self.delimiters.pop();
                    children.push(expression);
                }
                continue;
            }
            let mut literal_span = self.span();
            let mut literal = String::new();
            while !self.eof() && !self.rest().starts_with('<') {
                self.charge();
                if self.failure.is_some() {
                    return Err(self.error("parser work limit exceeded"));
                }
                if self.rest().starts_with("{{") {
                    literal.push('{');
                    self.advance(2);
                } else if self.rest().starts_with("}}") {
                    literal.push('}');
                    self.advance(2);
                } else if self.rest().starts_with('{') {
                    break;
                } else if self.rest().starts_with('}') {
                    return Err(
                        self.error("unmatched '}' in markup text; use '}}' for a literal brace")
                    );
                } else {
                    let ch = self.rest().chars().next().expect("nonempty source");
                    literal.push(ch);
                    self.advance(ch.len_utf8());
                }
            }
            literal_span.end = self.offset;
            self.highlight(literal_span.start, literal_span.end, "string");
            let literal = literal
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&amp;", "&");
            if text_element {
                text.push(sanim_core::TemplatePart::Text(literal.into()));
            } else if !literal.trim().is_empty() {
                children.push(call(
                    "Text",
                    vec![(
                        Some("text".into()),
                        string(
                            literal,
                            Span {
                                end: literal_span.start,
                                ..literal_span
                            },
                        ),
                    )],
                    literal_span,
                ));
            }
        }
        span.end = self.offset;
        if text_element {
            if args.iter().any(|(name, _)| name.as_deref() == Some("text")) && !text.is_empty() {
                return Err(self.error(format!(
                    "<{tag}> cannot specify both text= and text children"
                )));
            }
            if !text.is_empty() {
                args.push((
                    Some("text".into()),
                    Expr {
                        cache_id: crate::ast::next_expr_identity(),
                        kind: ExprKind::Template(text),
                        span: body_span,
                    },
                ));
            }
        } else if !children.is_empty() {
            if args
                .iter()
                .any(|(name, _)| name.as_deref() == Some("children"))
            {
                return Err(self.error(format!(
                    "<{tag}> cannot specify both children= and nested children"
                )));
            }
            args.push((
                Some("children".into()),
                Expr {
                    cache_id: crate::ast::next_expr_identity(),
                    kind: ExprKind::List(children),
                    span,
                },
            ));
        }
        Ok(tagged_call(
            constructor,
            merge_utilities(args, utilities),
            span,
            tag_span,
        ))
    }
}
