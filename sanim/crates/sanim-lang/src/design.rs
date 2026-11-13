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
    ("row", &[Utility::Row]),
    ("col", &[Utility::Column]),
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
    ("wrap", &[Utility::Wrap]),
    ("text-sm", &[Utility::Font(12)]),
    ("text-base", &[Utility::Font(16)]),
    ("text-lg", &[Utility::Font(20)]),
    ("text-xl", &[Utility::Font(24)]),
    ("title", &[Utility::Font(32)]),
];
pub(crate) struct Design {
    pub args: OrdMap<String, Value>,
    pub direction: Option<Direction>,
    pub pill: bool,
    pub stroke: Option<crate::design::Border>,
}
pub(crate) struct Border {
    pub color: Color,
    pub width: Fraction,
}
pub(crate) fn resolve(name: &str, args: &OrdMap<String, Value>) -> Result<Design, String> {
    let mut result = Design {
        args: OrdMap::new(),
        direction: None,
        pill: false,
        stroke: None,
    };
    let mut gradient_from = None;
    let mut gradient_to = None;
    let mut gradient_angle = None;
    let mut border_width = None;
    let mut border_color = None;
    if let Some(design) = args.get("design") {
        let design = design.string()?;
        if design.len() > 4096 {
            return Err(
                "design utilities exceed 4096 bytes; split reusable components into functions"
                    .into(),
            );
        }
        for token in design.split_whitespace() {
            if let Some((_, utilities)) = PRESETS.iter().find(|(name, _)| *name == token) {
                for utility in *utilities {
                    match utility {
                        Utility::Radius(value) => {
                            result.args.insert(
                                "radius".into(),
                                number(Fraction::from(*value), Unit::Length),
                            );
                            result.pill = false;
                        }
                        Utility::Pill => result.pill = true,
                        Utility::Fill(color) => {
                            result
                                .args
                                .insert("fill".into(), Value::Color(Color::parse(color)?));
                        }
                        Utility::Padding(value) => {
                            result.args.insert(
                                "padding".into(),
                                number(Fraction::from(*value), Unit::Length),
                            );
                        }
                        Utility::Font(value) => {
                            result.args.insert(
                                "font_size".into(),
                                number(Fraction::from(*value), Unit::Length),
                            );
                        }
                        Utility::Row => result.direction = Some(Direction::Row),
                        Utility::Column => result.direction = Some(Direction::Column),
                        Utility::Align(value) => {
                            result.args.insert("align".into(), variant(value));
                        }
                        Utility::Justify(value) => {
                            result.args.insert("justify".into(), variant(value));
                        }
                        Utility::Wrap => {
                            result.args.insert("wrap".into(), Value::Bool(true));
                        }
                    }
                }
                continue;
            }
            if token == "border" {
                border_width = Some(Fraction::from(1));
                continue;
            }
            if let Some(value) = token.strip_prefix("border-") {
                if value.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                    border_width = Some(nonnegative(value, token)?)
                } else {
                    border_color = Some(Color::parse(value)?)
                }
                continue;
            }
            if let Some(value) = token.strip_prefix("bg-gradient-to-") {
                gradient_angle = Some(match value {
                    "r" => 0,
                    "br" => 45,
                    "b" => 90,
                    "bl" => 135,
                    "l" => 180,
                    "tl" => 225,
                    "t" => 270,
                    "tr" => 315,
                    _ => {
                        return Err(format!(
                            "unknown gradient direction '{value}'; use r, br, b, bl, l, tl, t, or tr"
                        ));
                    }
                });
                continue;
            }
            if let Some(value) = token.strip_prefix("from-") {
                gradient_from = Some(Color::parse(value)?);
                continue;
            }
            if let Some(value) = token.strip_prefix("to-") {
                gradient_to = Some(Color::parse(value)?);
                continue;
            }
            if let Some(value) = token.strip_prefix("bg-") {
                result
                    .args
                    .insert("fill".into(), Value::Color(Color::parse(value)?));
                continue;
            }
            if let Some(value) = token.strip_prefix("text-") {
                if !matches!(name, "Text" | "Label") {
                    return Err("text-COLOR utilities belong on Label/Text; use bg-COLOR for a container's fill".into());
                }
                result
                    .args
                    .insert("fill".into(), Value::Color(Color::parse(value)?));
                continue;
            }
            if let Some((prefix, value)) = token.split_once('-') {
                let key = match prefix {
                    "w" => "width",
                    "h" => "height",
                    "p" => "padding",
                    "gap" => "gap",
                    "rounded" => "radius",
                    _ => return Err(unknown(token)),
                };
                if prefix == "rounded" {
                    result.pill = false;
                }
                let size = if matches!(prefix, "w" | "h") && value == "full" {
                    number(Fraction::from(1), Unit::Percent)
                } else if matches!(prefix, "w" | "h") && value.ends_with('%') {
                    number(
                        nonnegative(&value[..value.len() - 1], token)? / Fraction::from(100),
                        Unit::Percent,
                    )
                } else {
                    number(nonnegative(value, token)?, Unit::Length)
                };
                result.args.insert(key.into(), size);
                continue;
            }
            return Err(unknown(token));
        }
    }
    if gradient_angle.is_some() || gradient_from.is_some() || gradient_to.is_some() {
        let from = gradient_from.ok_or("gradient utilities require from-COLOR")?;
        let to = gradient_to.ok_or("gradient utilities require to-COLOR")?;
        result.args.insert(
            "fill".into(),
            Paint::LinearGradient {
                angle: Fraction::from(gradient_angle.unwrap_or(0u32)),
                stops: vec![
                    GradientStop {
                        offset: Fraction::from(0),
                        color: from,
                    },
                    GradientStop {
                        offset: Fraction::from(1),
                        color: to,
                    },
                ]
                .into(),
            }
            .value(),
        );
    }
    if (border_width.is_some() || border_color.is_some()) && border_width != Some(Fraction::from(0))
    {
        result.stroke = Some(Border {
            color: border_color.unwrap_or(Color::parse("light5")?),
            width: border_width.unwrap_or(Fraction::from(1)),
        });
    }
    // Utilities only provide defaults. Native explicit arguments remain authoritative.
    for (key, value) in args {
        if key != "design" {
            result.args.insert(key.clone(), value.clone());
        }
    }
    if args.contains_key("radius") {
        result.pill = false;
    }
    if result.args.contains_key("font_size") && !matches!(name, "Text" | "Label") {
        return Err("text size utilities belong on Label/Text, not its container".into());
    }
    if args.contains_key("design") && matches!(name, "Rect" | "Rectangle") {
        result
            .args
            .entry("fill".into())
            .or_insert(Value::Color(Color::parse("dark3")?));
        result
            .args
            .entry("width".into())
            .or_insert(number(Fraction::from(96), Unit::Length));
        result
            .args
            .entry("height".into())
            .or_insert(number(Fraction::from(40), Unit::Length));
    }
    if let Some(builtin) = crate::prelude::builtin(name) {
        for key in result.args.keys() {
            if !builtin
                .parameters
                .iter()
                .any(|parameter| parameter.name == key)
            {
                return Err(format!(
                    "design utility supplies '{key}', which {name} does not support; move that utility to a matching shape or label"
                ));
            }
        }
    }
    if result.stroke.is_some() && matches!(name, "Group" | "Row" | "Column") {
        return Err(
            "border requires a painted shape such as Rect; Group, Row and Column are layout-only"
                .into(),
        );
    }
    Ok(result)
}
fn nonnegative(value: &str, token: &str) -> Result<Fraction, String> {
    let value = math::decimal(value).map_err(|_| {
        format!("utility '{token}' requires a nonnegative point value, such as gap-8 or w-120")
    })?;
    if !math::finite(&value) || value < Fraction::from(0) {
        Err(format!(
            "utility '{token}' requires a finite nonnegative value"
        ))
    } else {
        Ok(value)
    }
}
fn unknown(token: &str) -> String {
    format!(
        "unknown native design utility '{token}'; use card, raised, pill, suggested, destructive, row/col, items-center, justify-center, gap-N, p-N, w-N, h-N, rounded-N, bg-COLOR, text-COLOR, border[-N], or bg-gradient-to-r from-COLOR to-COLOR. Numbers are pt; these are native utilities, not browser CSS."
    )
}

fn variant<E: NativeEnum + PartialEq>(value: &E) -> Value {
    Value::Variant {
        ty: std::sync::Arc::new(E::ty()),
        tag: E::VARIANTS
            .iter()
            .find(|(_, candidate)| candidate == value)
            .expect("registered enum")
            .0
            .into(),
        fields: std::sync::Arc::new([]),
    }
}
