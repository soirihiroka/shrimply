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
    Position(PositionMode),
    Align(Align),
    Justify(Justify),
    Wrap,
    Grow(u32),
    Shrink(u32),
    Basis(u32),
    Border(u32),
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
    ("relative", &[Utility::Position(PositionMode::Relative)]),
    ("absolute", &[Utility::Position(PositionMode::Absolute)]),
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
    ("grow", &[Utility::Grow(1)]),
    ("grow-0", &[Utility::Grow(0)]),
    ("shrink", &[Utility::Shrink(1)]),
    ("shrink-0", &[Utility::Shrink(0)]),
    ("flex-none", &[Utility::Grow(0), Utility::Shrink(0)]),
    (
        "flex-1",
        &[Utility::Grow(1), Utility::Shrink(1), Utility::Basis(0)],
    ),
    ("border", &[Utility::Border(1)]),
];

#[derive(Clone, Copy)]
enum Values {
    Length { relative: bool },
    Color,
    Border,
    Overflow,
    WhiteSpace,
    OverflowWrap,
}
struct Family {
    prefix: &'static str,
    property: &'static str,
    values: Values,
}
// Prefix order matters: axis policies precede the shorthand policy.
const FAMILIES: &[Family] = &[
    Family {
        prefix: "whitespace-",
        property: "white_space",
        values: Values::WhiteSpace,
    },
    Family {
        prefix: "wrap-",
        property: "overflow_wrap",
        values: Values::OverflowWrap,
    },
    Family {
        prefix: "overflow-x-",
        property: "overflow_x",
        values: Values::Overflow,
    },
    Family {
        prefix: "overflow-y-",
        property: "overflow_y",
        values: Values::Overflow,
    },
    Family {
        prefix: "overflow-",
        property: "overflow",
        values: Values::Overflow,
    },
    Family {
        prefix: "w-",
        property: "width",
        values: Values::Length { relative: true },
    },
    Family {
        prefix: "h-",
        property: "height",
        values: Values::Length { relative: true },
    },
    Family {
        prefix: "basis-",
        property: "flex_basis",
        values: Values::Length { relative: true },
    },
    Family {
        prefix: "p-",
        property: "padding",
        values: Values::Length { relative: false },
    },
    Family {
        prefix: "gap-",
        property: "gap",
        values: Values::Length { relative: false },
    },
    Family {
        prefix: "rounded-",
        property: "radius",
        values: Values::Length { relative: false },
    },
    Family {
        prefix: "bg-",
        property: "fill",
        values: Values::Color,
    },
    Family {
        prefix: "text-",
        property: "fill",
        values: Values::Color,
    },
    Family {
        prefix: "border-",
        property: "border_color",
        values: Values::Border,
    },
];

/// Resolved constructor arguments from the same utility lowering used by parsing.
#[derive(Clone, Debug)]
pub struct Description {
    pub token: String,
    pub properties: Vec<(String, String)>,
}
impl Description {
    pub fn detail(&self) -> String {
        self.properties
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
    pub fn markdown(&self) -> String {
        format!(
            "```sanim\n{}\n```\n\nNative utility `{}`. Numeric sizes use logical **pt**, independent of output resolution; `%` sizes use the parent layout box. Explicit properties override utilities.",
            self.detail(),
            self.token
        )
    }
}
/// Describe a valid utility, including target-element compatibility checks.
pub fn describe(element: &str, token: &str) -> Result<Description, String> {
    fn literal(expression: &crate::ast::Expr) -> String {
        use crate::ast::ExprKind;
        match &expression.kind {
            ExprKind::Number(value) | ExprKind::Name(value) => value.clone(),
            ExprKind::String(value) => format!("{value:?}"),
            ExprKind::Bool(value) => value.to_string(),
            ExprKind::Member(base, member) => format!("{}.{member}", literal(base)),
            ExprKind::Call { callee, args, .. } => format!(
                "{}({})",
                literal(callee),
                args.iter()
                    .map(|(_, value)| literal(value))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => unreachable!("native utilities only lower to literals and Color calls"),
        }
    }
    Ok(Description {
        token: token.into(),
        properties: lower(element, token, Span::default())?
            .into_iter()
            .map(|(name, value)| (name, literal(&value)))
            .collect(),
    })
}
/// Concrete completions, filtered by the registered constructor's supported properties.
/// Families accept arbitrary nonnegative sizes; these are useful starting values.
pub fn completions(element: &str, prefix: &str) -> Vec<Description> {
    let mut tokens: std::collections::BTreeSet<String> =
        PRESETS.iter().map(|(name, _)| (*name).into()).collect();
    for family in FAMILIES {
        if !prefix.starts_with(family.prefix) && !family.prefix.starts_with(prefix) {
            continue;
        }
        let mut values: Vec<String> = match family.values {
            Values::Length { .. } | Values::Border => [0, 4, 8, 12, 16, 24, 32, 64, 100, 180, 320]
                .iter()
                .map(ToString::to_string)
                .collect(),
            Values::Color => vec![],
            Values::WhiteSpace => sanim_core::text::WhiteSpace::VALUES
                .iter()
                .map(|(name, _)| (*name).to_owned())
                .collect(),
            Values::OverflowWrap => sanim_core::text::OverflowWrap::VALUES
                .iter()
                .map(|(name, _)| (*name).to_owned())
                .collect(),
            Values::Overflow => Overflow::VALUES
                .iter()
                .map(|(name, _)| (*name).to_owned())
                .collect(),
        };
        if matches!(family.values, Values::Length { relative: true }) {
            values.extend(["full", "25%", "50%", "75%", "100%"].map(str::to_owned));
        }
        if matches!(family.values, Values::Color | Values::Border) {
            values.extend(Color::names().map(str::to_owned));
        }
        tokens.extend(
            values
                .into_iter()
                .map(|value| format!("{}{value}", family.prefix)),
        );
    }
    tokens
        .into_iter()
        .filter(|token| token.starts_with(prefix))
        .filter_map(|token| describe(element, &token).ok())
        .collect()
}

/// Utilities lower to ordinary typed arguments before checking or evaluation.
/// This keeps markup, constructors, reflection, and signals on one property path.
pub(crate) fn lower(
    name: &str,
    token: &str,
    span: Span,
) -> Result<Vec<(String, crate::ast::Expr)>, String> {
    use crate::ast::{Expr, ExprKind};
    let expression = |kind| Expr {
        cache_id: crate::ast::next_expr_identity(),
        kind,
        span,
    };
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
    if let Some((_, utilities)) = PRESETS.iter().find(|(name, _)| *name == token) {
        for utility in *utilities {
            match utility {
                Utility::Radius(value) => set("radius", number(format!("{value}pt"))),
                Utility::Pill => set("radius", text("full")),
                Utility::Fill(value) => set("fill", color(value)?),
                Utility::Padding(value) => set("padding", number(format!("{value}pt"))),
                Utility::Font(value) => set("font_size", number(format!("{value}pt"))),
                Utility::Row => set("flex_direction", text("row")),
                Utility::Column => set("flex_direction", text("column")),
                Utility::Position(mode) => set("position_mode", text(mode.name())),
                Utility::Align(value) => set("align", enum_expression(value, span)),
                Utility::Justify(value) => set("justify", enum_expression(value, span)),
                Utility::Wrap => set("wrap", expression(ExprKind::Bool(true))),
                Utility::Grow(value) => set("flex_grow", number(value.to_string())),
                Utility::Shrink(value) => set("flex_shrink", number(value.to_string())),
                Utility::Basis(value) => set("flex_basis", number(format!("{value}pt"))),
                Utility::Border(value) => set("border_width", number(format!("{value}pt"))),
            }
        }
    } else if let Some((family, value)) = FAMILIES.iter().find_map(|family| {
        token
            .strip_prefix(family.prefix)
            .map(|value| (family, value))
    }) {
        match family.values {
            Values::Length { relative } => {
                let literal = if relative && value == "full" {
                    "100%".into()
                } else if relative && value.ends_with('%') {
                    nonnegative(&value[..value.len() - 1], token)?;
                    value.into()
                } else {
                    nonnegative(value, token)?;
                    format!("{value}pt")
                };
                set(family.property, number(literal));
            }
            Values::Color => set(family.property, color(value)?),
            Values::Border => {
                if value
                    .as_bytes()
                    .first()
                    .is_some_and(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b'.'))
                {
                    nonnegative(value, token)?;
                    set("border_width", number(format!("{value}pt")));
                } else {
                    set("border_color", color(value)?);
                }
            }
            Values::WhiteSpace => {
                sanim_core::text::WhiteSpace::parse(value)?;
                set(family.property, text(value));
            }
            Values::OverflowWrap => {
                sanim_core::text::OverflowWrap::parse(value)?;
                set(family.property, text(value));
            }
            Values::Overflow => {
                Overflow::parse(value)?;
                set(family.property, text(value));
            }
        }
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
    Expr {
        cache_id: crate::ast::next_expr_identity(),
        kind: ExprKind::Member(
            Box::new(Expr {
                cache_id: crate::ast::next_expr_identity(),
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
    let families = FAMILIES
        .iter()
        .map(|family| format!("{}VALUE", family.prefix))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "unknown native utility '{token}'; use a native preset such as flex, or {families}. Numeric sizes are nonnegative logical pt; w-, h- and basis- also accept percentages/full."
    )
}
