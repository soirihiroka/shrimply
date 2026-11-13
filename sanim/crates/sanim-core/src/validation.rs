//! Validate runtime values against their complete static schema.
use crate::{Fraction, Type, Value, math};

pub fn validate_value(ty: &Type, value: &Value) -> Result<(), String> {
    if !ty.accepts(&value.ty()) {
        return Err(format!("expected {ty}, found {}", value.ty()));
    }
    match (ty, value) {
        (Type::Union(alternatives), value) => {
            let mut reasons = Vec::new();
            for alternative in alternatives {
                match validate_value(alternative, value) {
                    Ok(()) => return Ok(()),
                    Err(reason) => reasons.push(reason),
                }
            }
            return Err(format!(
                "value matches no alternative of {ty}: {}",
                reasons.join("; ")
            ));
        }
        (Type::Fraction, Value::Fraction(value)) if !math::finite(value) => {
            return Err("fraction must be finite".into());
        }
        (Type::Matrix4, Value::Matrix4(matrix))
            if matrix.iter().flatten().any(|v| !v.is_finite()) =>
        {
            return Err("matrix entries must be finite".into());
        }
        (Type::Numeric(_), Value::Number(n)) if !math::finite(&n.value) => {
            return Err("non-finite numeric input".into());
        }
        (Type::Color, Value::Color(color))
            if color
                .0
                .iter()
                .any(|v| !math::finite(v) || *v < Fraction::from(0) || *v > Fraction::from(1)) =>
        {
            return Err("color channels must be in [0, 1]".into());
        }
        (Type::Vector(..), vector @ (Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_)))
            if vector.vector()?.0.iter().any(|v| !v.is_finite()) =>
        {
            return Err("vector components must be finite".into());
        }
        (Type::Array(_), Value::Array { shape, values, .. })
            if shape.iter().try_fold(1usize, |n, d| n.checked_mul(*d)) != Some(values.len()) =>
        {
            return Err("array shape does not match its value count".into());
        }
        (Type::List(inner) | Type::Array(inner), values) => {
            for value in values.list()? {
                validate_value(inner, value)?;
            }
        }
        (Type::Option(inner), Value::Some(value)) => validate_value(inner, value)?,
        (Type::Record(fields), Value::Record(values)) => {
            for (name, ty) in fields {
                validate_value(ty, &values[name]).map_err(|e| format!("field '{name}': {e}"))?;
            }
        }
        (Type::Enum { variants, .. }, Value::Variant { tag, fields, .. }) => {
            let types = variants.get(tag).ok_or("unknown enum variant")?;
            if types.len() != fields.len() {
                return Err("variant field count mismatch".into());
            }
            for (ty, value) in types.iter().zip(fields.iter()) {
                validate_value(ty, value)?;
            }
        }
        _ => {}
    }
    Ok(())
}
