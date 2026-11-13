//! Canonical pure configuration constructors, shared by preparation and retained DAGs.
use crate::{
    code::{
        Document,
        view::{Override, Range, Style, Theme},
    },
    *,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Native {
    Color,
    Font,
    CodeStyle,
    CodeRule,
    CodeSpan,
    CodeRange,
    CodeLines,
    CodeTheme,
}
impl Native {
    pub const CODE_VALUES: &'static [Self] = &[
        Self::CodeStyle,
        Self::CodeRule,
        Self::CodeSpan,
        Self::CodeRange,
        Self::CodeLines,
        Self::CodeTheme,
    ];
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Color" => Some(Self::Color),
            "Font" => Some(Self::Font),
            _ => Self::CODE_VALUES
                .iter()
                .copied()
                .find(|operation| operation.name() == name),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Color => "Color",
            Self::Font => "Font",
            Self::CodeStyle => "CodeStyle",
            Self::CodeRule => "CodeRule",
            Self::CodeSpan => "CodeSpan",
            Self::CodeRange => "CodeRange",
            Self::CodeLines => "CodeLines",
            Self::CodeTheme => "CodeTheme",
        }
    }
    pub fn signature(self) -> (Vec<(&'static str, Type, bool)>, Type) {
        let num = Type::Numeric(Unit::Number);
        match self {
            Self::Color => (vec![("value", Type::String, false)], Type::Color),
            Self::Font => (
                vec![
                    ("family", Type::String, false),
                    ("fallback", Type::String, false),
                ],
                Type::Font,
            ),
            Self::CodeStyle => (
                vec![
                    ("fill", Type::Color, false),
                    ("background", Type::Color, false),
                    ("bold", Type::Bool, false),
                    ("italic", Type::Bool, false),
                    ("underline", Type::Bool, false),
                    ("strike", Type::Bool, false),
                    ("opacity", num, false),
                ],
                style_type(),
            ),
            Self::CodeRule => (
                vec![
                    ("capture", Type::String, true),
                    ("style", style_type(), true),
                ],
                rule_type(),
            ),
            Self::CodeSpan => (
                vec![("range", range_type(), true), ("style", style_type(), true)],
                span_type(),
            ),
            Self::CodeRange => (
                ["start_line", "start_column", "end_line", "end_column"]
                    .map(|name| (name, num.clone(), true))
                    .into(),
                range_type(),
            ),
            Self::CodeLines => (
                vec![("start", num.clone(), true), ("end", num, false)],
                range_type(),
            ),
            Self::CodeTheme => (
                vec![
                    ("rules", Type::List(Box::new(rule_type())), false),
                    ("selection", Type::Color, false),
                    ("inactive_opacity", num, false),
                    ("line_number", Type::Color, false),
                ],
                theme_type(),
            ),
        }
    }
    pub fn check(self, actuals: &std::collections::BTreeMap<String, Type>) -> Result<Type, String> {
        let (parameters, result) = self.signature();
        for (name, ty, required) in &parameters {
            if let Some(actual) = actuals.get(*name) {
                if !ty.accepts(actual) {
                    return Err(format!(
                        "{} {name}: expected {ty}, found {actual}",
                        self.name()
                    ));
                }
            } else if *required {
                return Err(format!("{} missing '{name}'", self.name()));
            }
        }
        if let Some(name) = actuals.keys().find(|name| {
            !parameters
                .iter()
                .any(|(parameter, _, _)| *parameter == name.as_str())
        }) {
            return Err(format!("{} has no '{name}' argument", self.name()));
        }
        Ok(result)
    }
    pub fn execute(self, args: &imbl::OrdMap<String, Value>) -> Result<Value, String> {
        let required = |name: &str| {
            args.get(name)
                .ok_or_else(|| format!("{} missing '{name}'", self.name()))
        };
        match self.name() {
            "Color" => Ok(Value::Color(crate::Color::parse(
                args.get("value")
                    .map(Value::string)
                    .transpose()?
                    .unwrap_or("red3"),
            )?)),
            "Font" => Ok(Value::Font(crate::Font::family(
                args.get("family")
                    .map(Value::string)
                    .transpose()?
                    .unwrap_or(crate::Font::DEFAULT_FAMILY),
                args.get("fallback")
                    .map(Value::string)
                    .transpose()?
                    .unwrap_or(crate::Font::DEFAULT_FAMILY),
            )?)),
            "CodeStyle" => {
                let mut fields = imbl::OrdMap::new();
                for name in [
                    "fill",
                    "background",
                    "bold",
                    "italic",
                    "underline",
                    "strike",
                    "opacity",
                ] {
                    fields.insert(
                        name.into(),
                        args.get(name)
                            .cloned()
                            .map(|value| Value::Some(Arc::new(value)))
                            .unwrap_or(Value::None),
                    );
                }
                let value = Value::Record(fields);
                let _ = style(&value)?;
                Ok(value)
            }
            "CodeRule" | "CodeSpan" => Ok(Value::Record(
                args.iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            )),
            "CodeRange" => {
                let value = Value::Record(args.clone());
                range(&value)?;
                Ok(value)
            }
            "CodeLines" => {
                let start = integer(required("start")?, "CodeLines start")?;
                let end = args
                    .get("end")
                    .map(|value| integer(value, "CodeLines end"))
                    .transpose()?
                    .unwrap_or(start);
                let range = Range {
                    start_line: start,
                    start_column: 1,
                    end_line: end.checked_add(1).ok_or("CodeLines end overflow")?,
                    end_column: 1,
                };
                range.validate()?;
                Ok(range_value(range))
            }
            "CodeTheme" => {
                let defaults = Theme::default();
                let mut fields = imbl::OrdMap::new();
                fields.insert(
                    "rules".into(),
                    args.get("rules")
                        .cloned()
                        .unwrap_or_else(|| Value::List(List::typed(rule_type(), vec![]))),
                );
                for (name, color) in [
                    ("selection", defaults.selection),
                    ("line_number", defaults.line_number),
                ] {
                    fields.insert(
                        name.into(),
                        args.get(name).cloned().unwrap_or(Value::Color(color)),
                    );
                }
                fields.insert(
                    "inactive_opacity".into(),
                    args.get("inactive_opacity")
                        .cloned()
                        .unwrap_or(Value::Number(Number {
                            value: defaults.inactive_opacity,
                            unit: Unit::Number,
                        })),
                );
                let value = Value::Record(fields);
                theme(&value)?;
                Ok(value)
            }
            _ => unreachable!("registered native configuration constructor"),
        }
    }
}
pub fn record(fields: impl IntoIterator<Item = (&'static str, Type)>) -> Type {
    Type::Record(
        fields
            .into_iter()
            .map(|(name, ty)| (name.into(), ty))
            .collect(),
    )
}
pub fn option(ty: Type) -> Type {
    Type::Option(Box::new(ty))
}
pub fn style_type() -> Type {
    record([
        ("fill", option(Type::Color)),
        ("background", option(Type::Color)),
        ("bold", option(Type::Bool)),
        ("italic", option(Type::Bool)),
        ("underline", option(Type::Bool)),
        ("strike", option(Type::Bool)),
        ("opacity", option(Type::Numeric(Unit::Number))),
    ])
}
pub fn range_type() -> Type {
    record(
        ["start_line", "start_column", "end_line", "end_column"]
            .map(|name| (name, Type::Numeric(Unit::Number))),
    )
}
pub fn rule_type() -> Type {
    record([("capture", Type::String), ("style", style_type())])
}
pub fn span_type() -> Type {
    record([("range", range_type()), ("style", style_type())])
}
pub fn theme_type() -> Type {
    record([
        ("rules", Type::List(Box::new(rule_type()))),
        ("selection", Type::Color),
        ("inactive_opacity", Type::Numeric(Unit::Number)),
        ("line_number", Type::Color),
    ])
}
pub fn scalar(value: usize) -> Value {
    Value::Number(Number::scalar(value as i64))
}
pub fn document(value: &Value) -> Result<Arc<Document>, String> {
    if let Value::Code(document) = value {
        Ok(document.clone())
    } else {
        Err(format!(
            "CodeView source could not be resolved (found {}); use CodeView(src=\"file.py\")",
            value.ty()
        ))
    }
}
pub fn integer(value: &Value, label: &str) -> Result<usize, String> {
    value
        .number()?
        .usize()
        .map_err(|error| format!("{label}: {error}"))
}
pub fn fields(value: &Value) -> Result<&imbl::OrdMap<String, Value>, String> {
    if let Value::Record(fields) = value {
        Ok(fields)
    } else {
        Err("expected typed Code record".into())
    }
}
pub fn style(value: &Value) -> Result<Style, String> {
    let f = fields(value)?;
    let optional = |name: &str| -> Result<Option<&Value>, String> {
        match f.get(name) {
            Some(Value::Some(value)) => Ok(Some(value.as_ref())),
            Some(Value::None) | None => Ok(None),
            _ => Err(format!("CodeStyle {name} requires an optional value")),
        }
    };
    let opacity = optional("opacity")?
        .map(Value::number)
        .transpose()?
        .map(|number| number.value);
    if opacity.is_some_and(|value| value < 0.into() || value > 1.into()) {
        return Err("CodeStyle opacity must be in [0,1]".into());
    }
    let color = |name| -> Result<Option<Color>, String> {
        optional(name)?
            .map(|value| {
                if let Value::Color(color) = value {
                    Ok(color.clone())
                } else {
                    Err("CodeStyle color must be Color".into())
                }
            })
            .transpose()
    };
    Ok(Style {
        fill: color("fill")?,
        background: color("background")?,
        bold: optional("bold")?.map(Value::boolean).transpose()?,
        italic: optional("italic")?.map(Value::boolean).transpose()?,
        underline: optional("underline")?.map(Value::boolean).transpose()?,
        strike: optional("strike")?.map(Value::boolean).transpose()?,
        opacity,
    })
}
pub fn range(value: &Value) -> Result<Range, String> {
    let f = fields(value)?;
    let get = |name| integer(f.get(name).ok_or("CodeRange missing position")?, name);
    let range = Range {
        start_line: get("start_line")?,
        start_column: get("start_column")?,
        end_line: get("end_line")?,
        end_column: get("end_column")?,
    };
    range.validate()?;
    Ok(range)
}
pub fn range_value(range: Range) -> Value {
    Value::Record(
        [
            ("start_line".to_string(), scalar(range.start_line)),
            ("start_column".to_string(), scalar(range.start_column)),
            ("end_line".to_string(), scalar(range.end_line)),
            ("end_column".to_string(), scalar(range.end_column)),
        ]
        .into_iter()
        .collect(),
    )
}
pub fn spans(value: &Value) -> Result<Vec<Override>, String> {
    value
        .list()?
        .iter()
        .map(|value| {
            let f = fields(value)?;
            Ok(Override {
                range: range(f.get("range").ok_or("CodeSpan missing range")?)?,
                style: style(f.get("style").ok_or("CodeSpan missing style")?)?,
            })
        })
        .collect()
}
pub fn theme(value: &Value) -> Result<Theme, String> {
    let f = fields(value)?;
    let mut theme = Theme::default();
    if let Some(rules) = f.get("rules") {
        for rule in rules.list()?.iter() {
            let rule = fields(rule)?;
            let capture = rule
                .get("capture")
                .ok_or("CodeRule missing capture")?
                .string()?;
            if capture.is_empty() {
                return Err("CodeRule capture cannot be empty".into());
            }
            theme
                .styles
                .entry(capture.into())
                .or_default()
                .overlay(&style(rule.get("style").ok_or("CodeRule missing style")?)?);
        }
    }
    for (name, target) in [
        ("selection", &mut theme.selection),
        ("line_number", &mut theme.line_number),
    ] {
        if let Some(Value::Color(color)) = f.get(name) {
            *target = color.clone();
        }
    }
    if let Some(value) = f.get("inactive_opacity") {
        theme.inactive_opacity = value.number()?.value;
    }
    if theme.inactive_opacity < 0.into() || theme.inactive_opacity > 1.into() {
        return Err("CodeTheme inactive_opacity must be in [0,1]".into());
    }
    Ok(theme)
}

pub fn style_value(style: &Style) -> Value {
    let option = |value: Option<Value>| {
        value
            .map(|value| Value::Some(Arc::new(value)))
            .unwrap_or(Value::None)
    };
    Value::Record(
        [
            ("fill", option(style.fill.clone().map(Value::Color))),
            (
                "background",
                option(style.background.clone().map(Value::Color)),
            ),
            ("bold", option(style.bold.map(Value::Bool))),
            ("italic", option(style.italic.map(Value::Bool))),
            ("underline", option(style.underline.map(Value::Bool))),
            ("strike", option(style.strike.map(Value::Bool))),
            (
                "opacity",
                option(style.opacity.map(|value| {
                    Value::Number(Number {
                        value,
                        unit: Unit::Number,
                    })
                })),
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect(),
    )
}
pub fn theme_value(theme: &Theme) -> Value {
    let rules = Value::List(List::typed(
        rule_type(),
        theme
            .styles
            .iter()
            .map(|(capture, style)| {
                Value::Record(
                    [
                        (
                            "capture".to_string(),
                            Value::String(capture.as_str().into()),
                        ),
                        ("style".to_string(), style_value(style)),
                    ]
                    .into_iter()
                    .collect(),
                )
            })
            .collect(),
    ));
    Value::Record(
        [
            ("rules", rules),
            ("selection", Value::Color(theme.selection.clone())),
            ("line_number", Value::Color(theme.line_number.clone())),
            (
                "inactive_opacity",
                Value::Number(Number {
                    value: theme.inactive_opacity,
                    unit: Unit::Number,
                }),
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect(),
    )
}
pub fn spans_value(spans: &[Override]) -> Value {
    Value::List(List::typed(
        span_type(),
        spans
            .iter()
            .map(|span| {
                Value::Record(
                    [
                        ("range".to_string(), range_value(span.range)),
                        ("style".to_string(), style_value(&span.style)),
                    ]
                    .into_iter()
                    .collect(),
                )
            })
            .collect(),
    ))
}
