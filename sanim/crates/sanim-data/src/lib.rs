//! Bounded, schema-driven decoding. No filesystem or network access.
pub mod code;
mod csv;
pub mod dataframe;
pub mod font;
pub mod media;
mod numpy;
pub mod sqlite;
mod yaml;

use sanim_core::{Color, Number, Type, Unit, Value};
use serde_json::Value as Json;

/// Native geometry resolution shared by rendering, layout, picking, and frame verification.
pub fn resolve_pending(
    element: &mut sanim_core::Element,
    time: sanim_core::Fraction,
) -> Result<(), String> {
    code::view::resolve_pending(element, time)?;
    font::resolve_pending(element)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_values: usize,
    pub max_depth: usize,
    pub max_sql_steps: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_values: 1_000_000,
            max_depth: 64,
            max_sql_steps: 1_000_000,
        }
    }
}

pub fn decode(path: &str, bytes: &[u8], ty: &Type, limits: Limits) -> Result<Value, String> {
    if bytes.len() > limits.max_bytes {
        return Err("asset exceeds byte limit".into());
    }
    if !ty.readable() {
        return Err(format!("type {ty:?} is not readable"));
    }
    if *ty == Type::Font {
        font::validate(bytes)?;
        return Ok(Value::Font(sanim_core::Font::Asset(std::sync::Arc::new(
            bytes.to_vec(),
        ))));
    }
    if let Type::Code(language) = ty {
        if limits.max_values == 0 {
            return Err("code asset exceeds value limit".into());
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|error| format!("code resource '{path}' is not valid UTF-8: {error}"))?;
        return sanim_core::code::Document::read(path, text, *language)
            .and_then(code::parse)
            .map(|document| Value::Code(std::sync::Arc::new(document)));
    }
    // An explicit String request reads the UTF-8 source verbatim, independently
    // of extension. Structured JSON strings remain available via read<Data>/decode.
    if *ty == Type::String {
        if limits.max_values == 0 {
            return Err("text asset exceeds value limit".into());
        }
        return std::str::from_utf8(bytes)
            .map(|text| Value::String(text.into()))
            .map_err(|error| format!("text resource '{path}' is not valid UTF-8: {error}"));
    }
    let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "csv" => csv::decode(bytes, ty, limits),
        "npy" => numpy::decode(bytes, ty, limits),
        "npz" => numpy::archive(bytes, ty, limits),
        "json" => {
            let value: Json = serde_json::from_slice(bytes).map_err(|e| format!("JSON: {e}"))?;
            validate(&value, ty, limits)
        }
        "toml" => {
            let text = std::str::from_utf8(bytes).map_err(|e| format!("TOML UTF-8: {e}"))?;
            let value: toml::Value = toml::from_str(text).map_err(|e| format!("TOML: {e}"))?;
            let value = toml_json(value)?;
            validate(&value, ty, limits)
        }
        "yaml" | "yml" => validate(&yaml::parse(bytes, limits)?, ty, limits),
        _ => Err(format!("unsupported data format '.{extension}'")),
    }
}

fn toml_json(value: toml::Value) -> Result<Json, String> {
    Ok(match value {
        toml::Value::Datetime(value) => Json::String(value.to_string()),
        toml::Value::Array(values) => Json::Array(
            values
                .into_iter()
                .map(toml_json)
                .collect::<Result<_, _>>()?,
        ),
        toml::Value::Table(values) => Json::Object(
            values
                .into_iter()
                .map(|(k, v)| Ok((k, toml_json(v)?)))
                .collect::<Result<_, String>>()?,
        ),
        value => serde_json::to_value(value).map_err(|e| e.to_string())?,
    })
}

pub fn validate(value: &Json, ty: &Type, limits: Limits) -> Result<Value, String> {
    let mut budget = limits.max_values;
    convert(value, ty, "$", 0, limits.max_depth, &mut budget)
}

fn convert(
    value: &Json,
    ty: &Type,
    path: &str,
    depth: usize,
    max_depth: usize,
    budget: &mut usize,
) -> Result<Value, String> {
    if depth > max_depth {
        return Err(format!("{path}: nesting limit exceeded"));
    }
    *budget = budget
        .checked_sub(1)
        .ok_or("decoded value limit exceeded")?;
    let bad = || format!("{path}: expected {ty}, got {}", preview(value));
    match ty {
        Type::Union(alternatives) => {
            let mut reasons = vec![];
            for alternative in alternatives {
                match convert(value, alternative, path, depth, max_depth, budget) {
                    Ok(value) => return Ok(value),
                    Err(error) => {
                        if reasons.len() < 4 {
                            reasons.push(error);
                        }
                    }
                }
            }
            Err(format!(
                "{path}: value does not match union {ty}; alternative errors: {}{}",
                reasons.join("; "),
                if alternatives.len() > reasons.len() {
                    format!(
                        "; {} more alternatives failed",
                        alternatives.len() - reasons.len()
                    )
                } else {
                    String::new()
                }
            ))
        }
        Type::Data | Type::Dict => {
            validate_dynamic(value, depth, max_depth, budget)?;
            if *ty == Type::Dict {
                Ok(Value::Dict(
                    value
                        .as_object()
                        .ok_or_else(bad)?
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ))
            } else {
                Ok(Value::Data(value.clone().into()))
            }
        }
        Type::Bool => value.as_bool().map(Value::Bool).ok_or_else(bad),
        Type::String => value
            .as_str()
            .map(|v| Value::String(v.into()))
            .ok_or_else(bad),
        Type::Numeric(unit) => {
            let n = match value {
                Json::Number(v) => Number::parse(&expand_exponent(&v.to_string())?)
                    .map_err(|e| format!("{path}: expected {ty}: {e}"))?,
                Json::String(v) => {
                    Number::parse(v).map_err(|e| format!("{path}: expected {ty}: {e}"))?
                }
                _ => return Err(bad()),
            };
            if n.unit != *unit {
                return Err(bad());
            }
            Ok(Value::Number(n))
        }
        Type::Color => {
            if let Some(text) = value.as_str() {
                return Ok(Value::Color(Color::parse(text)?));
            }
            let values = value.as_array().filter(|v| v.len() == 4).ok_or_else(bad)?;
            let mut channels = [sanim_core::Fraction::from(0); 4];
            for (i, value) in values.iter().enumerate() {
                let Value::Fraction(channel) = convert(
                    value,
                    &Type::Fraction,
                    &format!("{path}[{i}]"),
                    depth + 1,
                    max_depth,
                    budget,
                )?
                else {
                    unreachable!()
                };
                if channel < sanim_core::Fraction::from(0)
                    || channel > sanim_core::Fraction::from(1)
                {
                    return Err(format!("{path}[{i}]: color channel must be in [0,1]"));
                }
                channels[i] = channel;
            }
            Ok(Value::Color(Color(channels)))
        }
        Type::Enum { variants, .. } => {
            let (tag, fields) = if let Some(tag) = value.as_str() {
                (tag, &[][..])
            } else {
                let object = value.as_object().ok_or_else(bad)?;
                if object.len() != 2 {
                    return Err(format!(
                        "{path}: enum data needs exactly 'variant' and 'fields'"
                    ));
                }
                (
                    object
                        .get("variant")
                        .and_then(Json::as_str)
                        .ok_or_else(bad)?,
                    object
                        .get("fields")
                        .and_then(Json::as_array)
                        .ok_or_else(bad)?
                        .as_slice(),
                )
            };
            let types = variants.get(tag).ok_or_else(|| {
                format!(
                    "{path}: unknown variant '{tag}'; expected {}",
                    variants.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            })?;
            if fields.len() != types.len() {
                return Err(format!(
                    "{path}: variant '{tag}' needs {} fields",
                    types.len()
                ));
            }
            let fields = fields
                .iter()
                .zip(types)
                .enumerate()
                .map(|(i, (value, ty))| {
                    convert(
                        value,
                        ty,
                        &format!("{path}.fields[{i}]"),
                        depth + 1,
                        max_depth,
                        budget,
                    )
                })
                .collect::<Result<_, _>>()?;
            Ok(Value::Variant {
                ty: ty.clone().into(),
                tag: tag.into(),
                fields,
            })
        }
        Type::Fraction => {
            let text = match value {
                Json::String(v) => v.clone(),
                Json::Number(v) => expand_exponent(&v.to_string())?,
                _ => return Err(bad()),
            };
            let fraction = if let Some((a, b)) = text.split_once('/') {
                sanim_core::math::arithmetic(
                    "/",
                    sanim_core::math::decimal(a)?,
                    sanim_core::math::decimal(b)?,
                )?
            } else {
                sanim_core::math::decimal(&text)?
            };
            Ok(Value::Fraction(fraction))
        }
        Type::Vector(n, unit) => {
            let values = value.as_array().filter(|v| v.len() == *n).ok_or_else(bad)?;
            let unit = unit.unwrap_or(Unit::Number);
            let values = values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    *budget = budget
                        .checked_sub(1)
                        .ok_or("decoded value limit exceeded")?;
                    floating(v, unit).map_err(|e| format!("{path}[{i}]: {e}"))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Value::from_vector(values, unit)
        }
        Type::Matrix4 => {
            let rows = value.as_array().filter(|v| v.len() == 4).ok_or_else(bad)?;
            let mut matrix = [[0.0; 4]; 4];
            for (i, row) in rows.iter().enumerate() {
                let row = row.as_array().filter(|v| v.len() == 4).ok_or_else(bad)?;
                for (j, v) in row.iter().enumerate() {
                    *budget = budget
                        .checked_sub(1)
                        .ok_or("decoded value limit exceeded")?;
                    matrix[i][j] =
                        floating(v, Unit::Number).map_err(|e| format!("{path}[{i}][{j}]: {e}"))?;
                }
            }
            Ok(Value::Matrix4(Box::new(matrix)))
        }
        Type::Option(inner) if value.is_null() => {
            let _ = inner;
            Ok(Value::None)
        }
        Type::Option(inner) => Ok(Value::Some(std::sync::Arc::new(convert(
            value,
            inner,
            path,
            depth + 1,
            max_depth,
            budget,
        )?))),
        Type::Record(fields) => {
            let object = value.as_object().ok_or_else(bad)?;
            for field in object.keys() {
                if !fields.contains_key(field) {
                    return Err(format!("{path}: unknown field '{field}'"));
                }
            }
            let mut out = std::collections::BTreeMap::new();
            for (name, ty) in fields {
                let value = object
                    .get(name)
                    .ok_or_else(|| format!("{path}: missing field '{name}'"))?;
                out.insert(
                    name.clone(),
                    convert(
                        value,
                        ty,
                        &format!("{path}.{name}"),
                        depth + 1,
                        max_depth,
                        budget,
                    )?,
                );
            }
            Ok(Value::Record(out.into_iter().collect()))
        }
        Type::List(inner) | Type::Array(inner) => {
            let rows = value.as_array().ok_or_else(bad)?;
            let values = rows
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    convert(
                        value,
                        inner,
                        &format!("{path}[{i}]"),
                        depth + 1,
                        max_depth,
                        budget,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(match ty {
                Type::Array(_) => Value::Array {
                    element_type: inner.clone().into(),
                    shape: vec![values.len()],
                    values: values.into(),
                },
                _ => Value::List(sanim_core::List::typed(inner.as_ref().clone(), values)),
            })
        }
        _ => Err(format!("{path}: {ty:?} cannot be decoded from this format")),
    }
}

fn preview(value: &Json) -> String {
    match value {
        Json::String(value) => {
            let short = value.chars().take(120).collect::<String>();
            format!(
                "string {short:?}{}",
                if value.chars().count() > 120 {
                    "…"
                } else {
                    ""
                }
            )
        }
        Json::Array(values) => format!("array with {} items", values.len()),
        Json::Object(values) => format!("object with {} fields", values.len()),
        Json::Null => "null".into(),
        Json::Bool(value) => value.to_string(),
        Json::Number(_) => "a number".into(),
    }
}

fn validate_dynamic(
    value: &Json,
    depth: usize,
    max_depth: usize,
    budget: &mut usize,
) -> Result<(), String> {
    if depth > max_depth {
        return Err("dynamic data nesting limit exceeded".into());
    }
    match value {
        Json::Array(items) => {
            for item in items {
                *budget = budget
                    .checked_sub(1)
                    .ok_or("decoded value limit exceeded")?;
                validate_dynamic(item, depth + 1, max_depth, budget)?;
            }
        }
        Json::Object(fields) => {
            for item in fields.values() {
                *budget = budget
                    .checked_sub(1)
                    .ok_or("decoded value limit exceeded")?;
                validate_dynamic(item, depth + 1, max_depth, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Convert language data to ordinary data, preserving the unstructured boundary.
pub fn to_json(value: &Value) -> Result<Json, String> {
    Ok(match value {
        Value::Data(value) => value.as_ref().clone(),
        Value::Dict(values) => {
            Json::Object(values.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        Value::Bool(value) => Json::Bool(*value),
        Value::String(value) => Json::String(value.to_string()),
        Value::Number(n) => {
            if n.unit == Unit::Number && n.value.denom() == Some(&1) {
                let text = n.value.to_string();
                Json::Number(
                    text.parse::<serde_json::Number>()
                        .map_err(|e| e.to_string())?,
                )
            } else {
                let (value, suffix) = n.display_parts()?;
                Json::String(format!("{value}{suffix}"))
            }
        }
        Value::Color(color) => Json::Array(
            color
                .0
                .iter()
                .map(|channel| Json::String(channel.to_string()))
                .collect(),
        ),
        Value::Variant { tag, fields, .. } => {
            serde_json::json!({"variant":tag,"fields":fields.iter().map(to_json).collect::<Result<Vec<_>,_>>()?})
        }
        Value::Fraction(value) => Json::String(value.to_string()),
        Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_) => {
            let (values, unit) = value.vector()?;
            Json::Array(
                values
                    .iter()
                    .map(|v| {
                        if unit == Unit::Number {
                            serde_json::to_value(v).map_err(|e| e.to_string())
                        } else {
                            to_json(&Value::Number(Number {
                                value: sanim_core::math::from_f64(*v)?,
                                unit,
                            }))
                        }
                    })
                    .collect::<Result<_, String>>()?,
            )
        }
        Value::Matrix4(matrix) => serde_json::to_value(matrix).map_err(|e| e.to_string())?,
        Value::List(_) | Value::Array { .. } => Json::Array(
            value
                .list()?
                .iter()
                .map(to_json)
                .collect::<Result<_, _>>()?,
        ),
        Value::Record(fields) => Json::Object(
            fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), to_json(v)?)))
                .collect::<Result<_, String>>()?,
        ),
        Value::Some(value) => to_json(value)?,
        Value::None => Json::Null,
        _ => {
            return Err(format!(
                "{} cannot be converted to unstructured data",
                value.ty()
            ));
        }
    })
}

pub(crate) fn expand_exponent(text: &str) -> Result<String, String> {
    let Some((mantissa, exp)) = text.split_once(['e', 'E']) else {
        return Ok(text.into());
    };
    let exponent: i32 = exp.parse().map_err(|_| "invalid number exponent")?;
    if exponent.unsigned_abs() > 308 {
        return Err("numeric exponent exceeds supported range".into());
    }
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches('-');
    let dot = mantissa.find('.').unwrap_or(mantissa.len()) as i32;
    let mut digits = mantissa.replace('.', "");
    let position = dot + exponent;
    if position <= 0 {
        digits = format!("0.{}{}", "0".repeat((-position) as usize), digits);
    } else if position as usize >= digits.len() {
        digits.push_str(&"0".repeat(position as usize - digits.len()));
    } else {
        digits.insert(position as usize, '.');
    }
    Ok(if negative {
        format!("-{digits}")
    } else {
        digits
    })
}

fn floating(value: &Json, unit: Unit) -> Result<f64, String> {
    let value = match value {
        Json::Number(value) => value.as_f64().ok_or("expected finite f64 component")?,
        Json::String(value) => {
            let number = Number::parse(value)?;
            if number.unit != unit {
                return Err(format!("expected {unit:?} vector component"));
            }
            sanim_core::math::to_f64(number.value)?
        }
        _ => return Err("expected numeric vector or matrix component".into()),
    };
    if !value.is_finite() {
        return Err("vector and matrix components must be finite".into());
    }
    Ok(value)
}
