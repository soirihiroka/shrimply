//! Native element notation lowers directly to ordinary, typed constructor calls.
use super::*;

fn call(name: &str, args: Vec<(Option<String>, Expr)>, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Call {
            callee: Box::new(Expr {
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
        kind: ExprKind::String(value),
        span,
    }
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
        self.advance(1);
        if !self.name_start() {
            return Err(
                self.error("expected a native element name after '<', such as Rect or Label")
            );
        }
        let mut tag_span = self.span();
        let tag = self.name()?;
        tag_span.end = tag_span.start + tag.len();
        let constructor = tag.as_str();
        let mut args: Vec<(Option<String>, Expr)> = Vec::new();
        loop {
            self.trivia();
            self.skip_lines();
            if self.rest().starts_with("/>") {
                self.advance(2);
                span.end = self.offset;
                return Ok(tagged_call(constructor, args, span, tag_span));
            }
            if self.rest().starts_with('>') {
                self.advance(1);
                break;
            }
            if self.eof() {
                return Err(self.error(format!(
                    "unterminated <{tag}> opening tag; expected '>' or '/>'"
                )));
            }
            let attribute_span = self.span();
            let attribute = self.name()?;
            if args
                .iter()
                .any(|(name, _)| name.as_deref() == Some(attribute.as_str()))
            {
                return Err(self.error(format!("duplicate '{attribute}' attribute on <{tag}>")));
            }
            let mut value = if self.eat("=") {
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
                    kind: ExprKind::Bool(true),
                    span: attribute_span,
                }
            };
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
                self.advance(2);
                let closing = self.name()?;
                if closing != tag {
                    return Err(self.error(format!(
                        "closing </{closing}> does not match <{tag}>; expected </{tag}>"
                    )));
                }
                if !self.rest().starts_with('>') {
                    return Err(self.error(format!("expected '>' after </{tag}")));
                }
                self.advance(1);
                break;
            }
            if self.rest().starts_with('<') {
                if text_element {
                    return Err(self.error(format!("<{tag}> contains text, not nested elements; put labels inside a Row, Column or Rect")));
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
                    kind: ExprKind::List(children),
                    span,
                },
            ));
        }
        Ok(tagged_call(constructor, args, span, tag_span))
    }
}
