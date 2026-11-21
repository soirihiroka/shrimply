//! Composing gradient utilities lower once to the existing LinearGradient constructor.
use crate::ast::{Expr, ExprKind, TypeExpr};
use sanim_core::{Color, Fraction, Span, math};

const DIRECTIONS: &[(&str, i32)] = &[
    ("r", 0),
    ("br", 45),
    ("b", 90),
    ("bl", 135),
    ("l", 180),
    ("tl", 225),
    ("t", 270),
    ("tr", 315),
];
const STOP_NAMES: [&str; 3] = ["from", "via", "to"];
#[derive(Clone)]
enum Token {
    Direction(i32),
    Color(usize, String),
    Position(usize, Fraction),
}
fn parse(token: &str) -> Result<Option<Token>, String> {
    if let Some(direction) = token
        .strip_prefix("bg-linear-to-")
        .or_else(|| token.strip_prefix("bg-gradient-to-"))
    {
        return DIRECTIONS
            .iter()
            .find(|(name, _)| *name == direction)
            .map(|(_, angle)| Some(Token::Direction(*angle)))
            .ok_or_else(|| {
                format!(
                    "gradient utility '{token}' requires a direction: t, tr, r, br, b, bl, l or tl"
                )
            });
    }
    for (index, name) in STOP_NAMES.iter().enumerate() {
        if let Some(value) = token
            .strip_prefix(name)
            .and_then(|value| value.strip_prefix('-'))
        {
            if let Some(value) = value.strip_suffix('%') {
                let value = super::nonnegative(value, token)?;
                if value > Fraction::from(100) {
                    return Err(format!(
                        "gradient position '{token}' must be between 0% and 100%"
                    ));
                }
                return Ok(Some(Token::Position(
                    index,
                    math::arithmetic("/", value, Fraction::from(100))?,
                )));
            }
            if !Color::names().any(|name| name.eq_ignore_ascii_case(value)) {
                Color::parse(value)?;
            }
            return Ok(Some(Token::Color(index, value.into())));
        }
    }
    Ok(None)
}
fn supports(element: &str, token: &str) -> Result<(), String> {
    if crate::prelude::builtin(element).is_some_and(|builtin| {
        builtin.parameters.iter().any(|parameter| {
            parameter.name == "fill" && parameter.ty.accepts(&sanim_core::Paint::gradient_type())
        })
    }) {
        Ok(())
    } else {
        Err(format!(
            "gradient utility '{token}' supplies a gradient fill, which <{element}> does not support"
        ))
    }
}
pub(super) fn description(element: &str, token: &str) -> Result<Option<String>, String> {
    let Some(part) = parse(token)? else {
        return Ok(None);
    };
    supports(element, token)?;
    Ok(Some(match part {
        Token::Direction(angle) => format!(
            "Linear gradient direction ({angle}deg); combine with from-COLOR and optional via-COLOR/to-COLOR. Lowers to LinearGradient fill."
        ),
        Token::Color(index, color) => format!(
            "{} gradient color: Color({color:?}); combine with bg-linear-to-DIRECTION.",
            STOP_NAMES[index]
        ),
        Token::Position(index, position) => format!(
            "{} gradient stop position: {position} of the gradient line; requires its color stop.",
            STOP_NAMES[index]
        ),
    }))
}
pub(super) fn completions(prefix: &str) -> Vec<String> {
    let mut tokens: Vec<_> = DIRECTIONS
        .iter()
        .map(|(direction, _)| format!("bg-linear-to-{direction}"))
        .collect();
    for name in STOP_NAMES {
        if prefix.starts_with(&format!("{name}-")) || format!("{name}-").starts_with(prefix) {
            tokens.extend(
                Color::names().map(|color| format!("{name}-{}", color.to_ascii_lowercase())),
            );
            tokens
                .extend([0, 10, 25, 50, 75, 90, 100].map(|position| format!("{name}-{position}%")));
        }
    }
    tokens
}
#[derive(Default)]
pub(crate) struct Gradient {
    direction: Option<(i32, Span)>,
    colors: [Option<(String, Span)>; 3],
    positions: [Option<(Fraction, Span)>; 3],
    first: Option<Span>,
}
impl Gradient {
    pub(crate) fn apply(&mut self, element: &str, token: &str, span: Span) -> Result<bool, String> {
        let Some(part) = parse(token)? else {
            return Ok(false);
        };
        supports(element, token)?;
        self.first.get_or_insert(span);
        match part {
            Token::Direction(angle) => {
                if self.direction.replace((angle, span)).is_some() {
                    return Err("multiple gradient directions on one element; keep one bg-linear-to-DIRECTION utility".into());
                }
            }
            Token::Color(index, color) => {
                if self.colors[index].replace((color, span)).is_some() {
                    return Err(format!(
                        "duplicate {} gradient color; keep one {}-COLOR utility",
                        STOP_NAMES[index], STOP_NAMES[index]
                    ));
                }
            }
            Token::Position(index, position) => {
                if self.positions[index].replace((position, span)).is_some() {
                    return Err(format!("duplicate {} gradient position", STOP_NAMES[index]));
                }
            }
        }
        Ok(true)
    }
    pub(crate) fn finish(&self, explicit_fill: bool) -> Result<Option<Expr>, (String, Span)> {
        let Some(span) = self.first else {
            return Ok(None);
        };
        let bad = |message: &str| (message.to_owned(), span);
        if explicit_fill {
            return Err(bad(
                "gradient utilities and explicit fill= cannot be combined; remove fill= or use fill={LinearGradient(...)}",
            ));
        }
        let (angle, _) = self
            .direction
            .ok_or_else(|| bad("gradient colors require a direction such as bg-linear-to-r"))?;
        let (from, _) = self.colors[0]
            .as_ref()
            .ok_or_else(|| bad("gradient requires a starting color such as from-blue3"))?;
        for (index, name) in STOP_NAMES.iter().enumerate() {
            if index != 2 && self.positions[index].is_some() && self.colors[index].is_none() {
                return Err(bad(&format!(
                    "{name} gradient position requires a {name}-COLOR utility"
                )));
            }
        }
        let start = self.positions[0].map_or(Fraction::from(0), |(value, _)| value);
        let end = self.positions[2].map_or(Fraction::from(1), |(value, _)| value);
        let middle = self.positions[1]
            .map(|(value, _)| value)
            .unwrap_or_else(|| {
                math::arithmetic(
                    "/",
                    math::arithmetic("+", start, end).expect("bounded offsets"),
                    Fraction::from(2),
                )
                .expect("bounded offsets")
            });
        if start > end || self.colors[1].is_some() && !(start..=end).contains(&middle) {
            return Err(bad(
                "gradient stop positions must be ordered: from <= via <= to",
            ));
        }
        let mut stops = vec![(start, color(from, span))];
        if let Some((via, _)) = &self.colors[1] {
            stops.push((middle, color(via, span)));
        }
        let last = if let Some((to, _)) = &self.colors[2] {
            color(to, span)
        } else {
            let source = self.colors[1].as_ref().map_or(from, |(color, _)| color);
            let mut channels = Color::parse(source).expect("validated utility color").0;
            channels[3] = Fraction::from(0);
            let channels = Expr::new(
                ExprKind::List(
                    channels
                        .into_iter()
                        .map(|value| Expr::new(ExprKind::String(value.to_string()), span))
                        .collect(),
                ),
                span,
            );
            call(
                "decode",
                vec![TypeExpr::Named("Color".into(), vec![])],
                vec![(None, call("Data", vec![], vec![(None, channels)], span))],
                span,
            )
        };
        let mut arguments = Vec::new();
        if angle != 0 {
            arguments.push((
                Some("angle".into()),
                Expr::new(ExprKind::Number(format!("{angle}deg")), span),
            ));
        }
        if self.colors[1].is_none() && self.positions.iter().all(Option::is_none) {
            arguments.extend([
                (Some("from".into()), stops.remove(0).1),
                (Some("to".into()), last),
            ]);
            return Ok(Some(call("LinearGradient", vec![], arguments, span)));
        }
        stops.push((end, last));
        if start != Fraction::from(0) {
            stops.insert(0, (Fraction::from(0), stops[0].1.clone()));
        }
        if end != Fraction::from(1) {
            stops.push((
                Fraction::from(1),
                stops.last().expect("two stops").1.clone(),
            ));
        }
        let stops = stops
            .into_iter()
            .map(|(offset, color)| {
                Expr::new(
                    ExprKind::Record(vec![
                        ("offset".into(), fraction(offset, span)),
                        ("color".into(), color),
                    ]),
                    span,
                )
            })
            .collect();
        arguments.push((Some("stops".into()), Expr::new(ExprKind::List(stops), span)));
        Ok(Some(call("LinearGradient", vec![], arguments, span)))
    }
}
fn call(name: &str, types: Vec<TypeExpr>, args: Vec<(Option<String>, Expr)>, span: Span) -> Expr {
    Expr::new(
        ExprKind::Call {
            callee: Box::new(Expr::new(ExprKind::Name(name.into()), span)),
            types,
            args,
        },
        span,
    )
}
fn color(value: &str, span: Span) -> Expr {
    call(
        "Color",
        vec![],
        vec![(None, Expr::new(ExprKind::String(value.into()), span))],
        span,
    )
}
fn fraction(value: Fraction, span: Span) -> Expr {
    Expr::new(
        ExprKind::Binary(
            "/".into(),
            Box::new(Expr::new(
                ExprKind::Number(value.numer().expect("finite offset").to_string()),
                span,
            )),
            Box::new(Expr::new(
                ExprKind::Number(value.denom().expect("finite offset").to_string()),
                span,
            )),
        ),
        span,
    )
}

pub(super) fn conflicts(existing: &str, candidate: &str) -> bool {
    // An explicit fill is mutually exclusive with every gradient component,
    // unlike ordinary utility defaults that an explicit property may override.
    if existing == "fill" || candidate == "fill" {
        let token = if existing == "fill" {
            candidate
        } else {
            existing
        };
        return parse(token).ok().flatten().is_some();
    }
    match (
        parse(existing).ok().flatten(),
        parse(candidate).ok().flatten(),
    ) {
        (Some(Token::Direction(_)), Some(Token::Direction(_))) => true,
        (Some(Token::Color(a, _)), Some(Token::Color(b, _)))
        | (Some(Token::Position(a, _)), Some(Token::Position(b, _))) => a == b,
        _ => false,
    }
}
