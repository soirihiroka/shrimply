//! Bounded native design utilities. Explicit constructor arguments always win.
use crate::{
    native::Context,
    native_types,
    prelude::{Builtin, Execution, Parameter, Timing},
};
use imbl::OrdMap;
use sanim_core::{paint::MAX_GRADIENT_STOPS, *};
fn parameter(name: &'static str, ty: Type, required: bool) -> Parameter {
    Parameter { name, ty, required }
}
pub(crate) fn builtins() -> Vec<Builtin> {
    let Type::Record(fields) = Paint::gradient_type() else {
        unreachable!()
    };
    vec![Builtin {
        name: "LinearGradient",
        parameters: vec![
            parameter("from", Type::Color, false),
            parameter("to", Type::Color, false),
            parameter("angle", Type::Numeric(Unit::Angle), false),
            parameter("stops", fields["stops"].clone(), false),
        ],
        result: Paint::gradient_type(),
        schedule: false,
        timing: Timing::default(),
        content: Default::default(),
        execute: Execution::Native(gradient),
        check: native_types::fixed,
    }]
}
fn gradient(
    _name: &str,
    args: &OrdMap<String, Value>,
    mut context: Context<'_>,
) -> Result<Value, String> {
    let count = args
        .get("stops")
        .map(|value| value.list().map(|values| values.len()))
        .transpose()?
        .unwrap_or(2);
    context.charge_work(count)?;
    if count > MAX_GRADIENT_STOPS {
        return Err(format!(
            "LinearGradient supports at most {MAX_GRADIENT_STOPS} stops"
        ));
    }
    if args.contains_key("stops") && (args.contains_key("from") || args.contains_key("to")) {
        return Err("LinearGradient accepts stops or from/to, not both".into());
    }
    let stops = if let Some(stops) = args.get("stops") {
        stops.clone()
    } else {
        Value::List(
            vec![
                Value::Record(
                    [
                        (
                            "offset".to_string(),
                            number(Fraction::from(0), Unit::Number),
                        ),
                        (
                            "color".to_string(),
                            args.get("from")
                                .cloned()
                                .unwrap_or(Value::Color(Color::parse("blue3")?)),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
                Value::Record(
                    [
                        (
                            "offset".to_string(),
                            number(Fraction::from(1), Unit::Number),
                        ),
                        (
                            "color".to_string(),
                            args.get("to")
                                .cloned()
                                .unwrap_or(Value::Color(Color::parse("purple3")?)),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ]
            .into(),
        )
    };
    let value = Value::Record(
        [
            (
                "angle".to_string(),
                args.get("angle")
                    .cloned()
                    .unwrap_or(number(Fraction::from(0), Unit::Angle)),
            ),
            ("stops".to_string(), stops),
        ]
        .into_iter()
        .collect(),
    );
    Paint::from_value(&value).map(|paint| paint.value())
}
fn number(value: Fraction, unit: Unit) -> Value {
    Value::Number(Number { value, unit })
}
#[derive(Clone, Copy)]
enum Utility {
    Radius(u32),
    Pill,
    Fill(&'static str),
    Padding(u32),
    Font(u32),
    Row,
    Column,
    Align(Align),
    Justify(Justify),
    Wrap,
}
const PRESETS: &[(&str, &[Utility])] = &[
    (
        "card",
        &[
            Utility::Radius(12),
            Utility::Fill("card_bg_dark"),
            Utility::Padding(12),
        ],
    ),
    (
        "raised",
        &[
            Utility::Radius(8),
            Utility::Fill("dark3"),
            Utility::Padding(8),
        ],
    ),
    ("pill", &[Utility::Pill, Utility::Padding(8)]),
    ("suggested", &[Utility::Fill("blue3")]),
    ("destructive", &[Utility::Fill("red3")]),
    ("rounded", &[Utility::Radius(8)]),
    ("rounded-sm", &[Utility::Radius(4)]),
    ("rounded-lg", &[Utility::Radius(12)]),
    ("rounded-xl", &[Utility::Radius(18)]),
    ("rounded-none", &[Utility::Radius(0)]),
    ("rounded-full", &[Utility::Pill]),
    ("flex", &[Utility::Row]),
    ("flex-row", &[Utility::Row]),
    ("flex-col", &[Utility::Column]),
    ("items-start", &[Utility::Align(Align::Start)]),
    ("items-center", &[Utility::Align(Align::Center)]),
    ("items-end", &[Utility::Align(Align::End)]),
    ("justify-start", &[Utility::Justify(Justify::Start)]),
    ("justify-center", &[Utility::Justify(Justify::Center)]),
    ("justify-end", &[Utility::Justify(Justify::End)]),
    (
        "justify-between",
        &[Utility::Justify(Justify::SpaceBetween)],
    ),
    ("justify-around", &[Utility::Justify(Justify::SpaceAround)]),
    ("flex-wrap", &[Utility::Wrap]),
    ("text-sm", &[Utility::Font(12)]),
    ("text-base", &[Utility::Font(16)]),
    ("text-lg", &[Utility::Font(20)]),
    ("text-xl", &[Utility::Font(24)]),
    ("title", &[Utility::Font(32)]),
];

/// Utilities lower to ordinary typed arguments before checking or evaluation.
/// This keeps markup, constructors, reflection, and signals on one property path.
pub(crate) fn lower(
    name: &str,
    token: &str,
    span: Span,
) -> Result<Vec<(String, crate::ast::Expr)>, String> {
    use crate::ast::{Expr, ExprKind};
    let expression = |kind| Expr { cache_id: crate::ast::next_expr_identity(),  kind, span };
    let text = |value: &str| expression(ExprKind::String(value.into()));
    let number = |value: String| expression(ExprKind::Number(value));
    let color = |value: &str| -> Result<Expr, String> {
        Color::parse(value)?;
        Ok(expression(ExprKind::Call {
            callee: Box::new(expression(ExprKind::Name("Color".into()))),
            types: vec![],
            args: vec![(None, text(value))],
        }))
    };
    let mut args = Vec::new();
    let mut set = |key: &str, value: Expr| {
        args.push((key.to_owned(), value));
    };
    if ["bg-gradient-", "from-", "via-", "to-"]
        .iter()
        .any(|prefix| token.starts_with(prefix))
    {
        return Err(format!(
            "gradient utility '{token}' is not supported; use fill={{LinearGradient(...)}} instead"
        ));
    }
    if let Some(mode) = token.strip_prefix("overflow-") {
        let (key, mode) = mode
            .strip_prefix("x-")
            .map(|v| ("overflow_x", v))
            .or_else(|| mode.strip_prefix("y-").map(|v| ("overflow_y", v)))
            .unwrap_or(("overflow", mode));
        Overflow::parse(mode)?;
        set(key, text(mode));
    } else if let Some((_, utilities)) = PRESETS.iter().find(|(name, _)| *name == token) {
        for utility in *utilities {
            match utility {
                Utility::Radius(value) => set("radius", number(format!("{value}pt"))),
                Utility::Pill => set("radius", text("full")),
                Utility::Fill(value) => set("fill", color(value)?),
                Utility::Padding(value) => set("padding", number(format!("{value}pt"))),
                Utility::Font(value) => set("font_size", number(format!("{value}pt"))),
                Utility::Row => set("flex_direction", text("row")),
                Utility::Column => set("flex_direction", text("column")),
                Utility::Align(value) => set("align", enum_expression(value, span)),
                Utility::Justify(value) => set("justify", enum_expression(value, span)),
                Utility::Wrap => set("wrap", expression(ExprKind::Bool(true))),
            }
        }
    } else if token == "grow" || token == "shrink" || token == "grow-0" || token == "shrink-0" {
        let key = if token.starts_with("grow") {
            "flex_grow"
        } else {
            "flex_shrink"
        };
        set(
            key,
            number(if token.ends_with("-0") { "0" } else { "1" }.into()),
        );
    } else if token == "flex-none" {
        set("flex_grow", number("0".into()));
        set("flex_shrink", number("0".into()));
    } else if token == "flex-1" {
        set("flex_grow", number("1".into()));
        set("flex_shrink", number("1".into()));
        set("flex_basis", number("0pt".into()));
    } else if token == "border" {
        set("border_width", number("1pt".into()));
    } else if let Some(value) = token.strip_prefix("border-") {
        if value.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            nonnegative(value, token)?;
            set("border_width", number(format!("{value}pt")));
        } else {
            set("border_color", color(value)?);
        }
    } else if let Some(value) = token.strip_prefix("bg-") {
        set("fill", color(value)?);
    } else if let Some(value) = token.strip_prefix("text-") {
        set("fill", color(value)?);
    } else if let Some((prefix, value)) = token.split_once('-') {
        let key = match prefix {
            "w" => "width",
            "h" => "height",
            "p" => "padding",
            "gap" => "gap",
            "rounded" => "radius",
            "basis" => "flex_basis",
            _ => return Err(unknown(token)),
        };
        let literal = if matches!(prefix, "w" | "h" | "basis") && value == "full" {
            "100%".into()
        } else if matches!(prefix, "w" | "h" | "basis") && value.ends_with('%') {
            nonnegative(&value[..value.len() - 1], token)?;
            value.into()
        } else {
            nonnegative(value, token)?;
            format!("{value}pt")
        };
        set(key, number(literal));
    } else {
        return Err(unknown(token));
    }
    let builtin = crate::prelude::builtin(name)
        .ok_or_else(|| format!("<{name}> has no registered native utilities"))?;
    for (key, _) in &args {
        if !builtin
            .parameters
            .iter()
            .any(|parameter| parameter.name == key)
        {
            return Err(format!(
                "utility '{token}' supplies '{key}', which <{name}> does not support"
            ));
        }
    }
    Ok(args)
}
fn enum_expression<E: NativeEnum + PartialEq>(value: &E, span: Span) -> crate::ast::Expr {
    use crate::ast::{Expr, ExprKind};
    let Type::Enum { name, .. } = E::ty() else {
        unreachable!("native enum name")
    };
    let case = E::VARIANTS
        .iter()
        .find(|(_, candidate)| candidate == value)
        .expect("native enum case")
        .0;
    Expr { cache_id: crate::ast::next_expr_identity(), 
        kind: ExprKind::Member(
            Box::new(Expr { cache_id: crate::ast::next_expr_identity(), 
                kind: ExprKind::Name(name),
                span,
            }),
            case.into(),
        ),
        span,
    }
}
fn nonnegative(value: &str, token: &str) -> Result<Fraction, String> {
    let value = math::decimal(value)
        .map_err(|_| format!("utility '{token}' requires a nonnegative logical point value"))?;
    if !math::finite(&value) || value < Fraction::from(0) {
        Err(format!("utility '{token}' must be finite and nonnegative"))
    } else {
        Ok(value)
    }
}
fn unknown(token: &str) -> String {
    format!(
        "unknown native utility '{token}'; use flex, flex-col, items-center, justify-between, gap-N, p-N, w-N, h-N, rounded-N, bg-COLOR or border. Numbers are logical pt."
    )
}
