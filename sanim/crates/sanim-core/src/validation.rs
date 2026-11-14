//! Validate runtime values against their complete static schema.
use crate::{Fraction, Type, Value, math};

pub fn validate_value(ty: &Type, value: &Value) -> Result<(), String> {
    // Values retain broad primitive representations; literal constraints are
    // schema proofs, not one runtime datatype for every distinct string.
    match (ty, value) {
        (Type::RefinedNumeric(domain), Value::Number(number)) => {
            return if domain.contains(*number) {
                Ok(())
            } else {
                Err(format!(
                    "expected {ty}, found {} ({})",
                    number.value,
                    Type::Numeric(number.unit)
                ))
            };
        }
        (Type::StringLiteral(expected), Value::String(actual)) => {
            return if expected == actual {
                Ok(())
            } else {
                Err(format!("expected {ty}, found {actual:?}"))
            };
        }
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
        (Type::Record(fields), Value::Record(values)) => {
            if !fields.keys().eq(values.keys()) {
                return Err(format!("expected fields of {ty}, found {}", value.ty()));
            }
            for (name, ty) in fields {
                validate_value(ty, &values[name])
                    .map_err(|error| format!("field '{name}': {error}"))?;
            }
            return Ok(());
        }
        (Type::List(inner, length), Value::List(values)) => {
            if !length.allows(values.len()) {
                return Err(format!(
                    "expected {ty}, found List with {} items",
                    values.len()
                ));
            }
            for value in &values.values {
                validate_value(inner, value)?;
            }
            return Ok(());
        }
        (Type::Array(inner), Value::Array { shape, values, .. }) => {
            if shape.iter().try_fold(1usize, |n, d| n.checked_mul(*d)) != Some(values.len()) {
                return Err("array shape does not match its value count".into());
            }
            for value in values {
                validate_value(inner, value)?;
            }
            return Ok(());
        }
        (Type::Option(inner), Value::Some(value)) => return validate_value(inner, value),
        _ => {}
    }
    if !ty.accepts(&value.ty()) {
        return Err(format!("expected {ty}, found {}", value.ty()));
    }
    match (ty, value) {
        (Type::Font, Value::Font(font)) => font.validate()?,
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
