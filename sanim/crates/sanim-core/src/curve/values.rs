//! One schema for editable key values and relative incoming/outgoing handles.
use crate::{
    Fraction, ListLength, Number, Type, Unit, Value,
    curve::{Handle, KeyInterpolation, Keyframe, MAX_CHANNELS, Tangents},
};
use imbl::OrdMap;
use std::sync::{Arc, OnceLock};
pub fn value_type() -> Type {
    let number = Type::union(
        [
            Unit::Number,
            Unit::Length,
            Unit::Percent,
            Unit::Angle,
            Unit::Duration,
            Unit::Rate,
        ]
        .map(Type::Numeric),
    )
    .expect("numeric curve units");
    Type::union([
        number.clone(),
        Type::List(Box::new(number), ListLength::Unknown),
    ])
    .expect("curve values")
}
pub fn choices(values: &[&str]) -> Type {
    Type::union(values.iter().map(|v| Type::StringLiteral((*v).into()))).expect("curve choices")
}
pub fn handle_type(value: Type) -> Type {
    Type::Record(
        [
            ("time".to_owned(), Type::Numeric(Unit::Duration)),
            ("value".to_owned(), value),
        ]
        .into_iter()
        .collect(),
    )
}
/// Constructor arguments and materialized record fields use this same schema.
pub fn parameters(value: Type) -> Vec<(&'static str, Type, bool)> {
    vec![
        ("time", Type::Numeric(Unit::Duration), true),
        ("value", value.clone(), true),
        ("handle_in", handle_type(value.clone()), false),
        ("handle_out", handle_type(value), false),
        (
            "interpolation",
            choices(&["bezier", "linear", "hold"]),
            false,
        ),
        (
            "tangents",
            choices(&["auto", "auto_clamped", "manual"]),
            false,
        ),
    ]
}
pub fn keyframe_type(value: Type) -> Type {
    Type::Record(
        parameters(value)
            .into_iter()
            .map(|(name, ty, _)| {
                let ty = if matches!(name, "handle_in" | "handle_out") {
                    Type::Option(Box::new(ty))
                } else {
                    ty
                };
                (name.to_owned(), ty)
            })
            .collect(),
    )
}
pub fn checked_value(ty: &Type) -> Result<(), String> {
    if ty.numeric_unit().is_some()
        || matches!(ty,Type::List(inner,ListLength::Exact(n)) if *n>0&&*n<=MAX_CHANNELS&&numeric(inner))
    {
        Ok(())
    } else {
        Err(format!(
            "keyframe value requires a numeric scalar or a numeric List with a known length of 1..={MAX_CHANNELS}"
        ))
    }
}
fn numeric(ty: &Type) -> bool {
    ty.numeric_unit().is_some() || matches!(ty,Type::Union(values) if values.iter().all(numeric))
}
pub fn keyframe(args: &OrdMap<String, Value>) -> Result<Value, String> {
    let value = args["value"].clone();
    checked_value(&value.ty())?;
    let record = Value::Record(
        [
            ("time".to_owned(), args["time"].clone()),
            ("value".to_owned(), value),
            (
                "handle_in".to_owned(),
                args.get("handle_in")
                    .map_or(Value::None, |v| Value::Some(Arc::new(v.clone()))),
            ),
            (
                "handle_out".to_owned(),
                args.get("handle_out")
                    .map_or(Value::None, |v| Value::Some(Arc::new(v.clone()))),
            ),
            (
                "interpolation".to_owned(),
                args.get("interpolation")
                    .cloned()
                    .unwrap_or(Value::String("bezier".into())),
            ),
            (
                "tangents".to_owned(),
                args.get("tangents")
                    .cloned()
                    .unwrap_or(Value::String("auto_clamped".into())),
            ),
        ]
        .into_iter()
        .collect(),
    );
    parse(&record)?;
    Ok(record)
}
pub fn parse(value: &Value) -> Result<Keyframe, String> {
    static SCHEMA: OnceLock<Type> = OnceLock::new();
    crate::validate_value(SCHEMA.get_or_init(|| keyframe_type(value_type())), value)?;
    let Value::Record(fields) = value else {
        return Err("Curve keys must be Keyframe(...) values".into());
    };
    let handle = |name: &str| -> Result<Option<Handle>, String> {
        match &fields[name] {
            Value::None => Ok(None),
            Value::Some(value) => {
                let Value::Record(record) = value.as_ref() else {
                    return Err("keyframe handle must be {time=...,value=...} offsets".into());
                };
                Ok(Some(Handle {
                    time: record["time"].number()?.value,
                    value: record["value"].clone(),
                }))
            }
            _ => Err("keyframe handle must be optional".into()),
        }
    };
    let time = fields["time"].number()?.value;
    if !crate::math::finite(&time) || time < Fraction::from(0) {
        return Err("keyframe time must be a finite nonnegative Duration".into());
    }
    Ok(Keyframe {
        time,
        value: fields["value"].clone(),
        handle_in: handle("handle_in")?,
        handle_out: handle("handle_out")?,
        interpolation: match fields["interpolation"].string()? {
            "linear" => KeyInterpolation::Linear,
            "hold" => KeyInterpolation::Hold,
            "bezier" => KeyInterpolation::Bezier,
            _ => return Err("keyframe interpolation must be bezier, linear, or hold".into()),
        },
        tangents: match fields["tangents"].string()? {
            "auto" => Tangents::Auto,
            "auto_clamped" => Tangents::AutoClamped,
            "manual" => Tangents::Manual,
            _ => return Err("keyframe tangents must be auto, auto_clamped, or manual".into()),
        },
    })
}
pub fn duration_value(time: Fraction) -> Value {
    Value::Number(Number {
        value: time,
        unit: Unit::Duration,
    })
}
