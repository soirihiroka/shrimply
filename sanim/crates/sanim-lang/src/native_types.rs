//! Builtin type rules registered alongside their signatures and execution handlers.
use crate::prelude::Builtin;
use sanim_core::{
    Type, Unit,
    math::{functions::Domain, operations::scalar_unit},
};
fn expect(expected: &Type, actual: &Type) -> Result<(), String> {
    if expected.accepts(actual) {
        Ok(())
    } else {
        Err(format!("expected {expected}, found {actual}"))
    }
}
pub(crate) fn fixed(builtin: &Builtin, _actuals: &[Type]) -> Result<Type, String> {
    Ok(builtin.result.clone())
}
pub(crate) fn vector(_builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    let mut unit = None;
    for ty in actuals.iter().filter(|t| **t != Type::Unit) {
        let next =
            scalar_unit(ty).ok_or("vector components must be numeric and have matching units")?;
        if unit.is_some_and(|u| u != next) {
            return Err("vector components must have matching units".into());
        }
        unit = Some(next);
    }
    Ok(Type::Vector(
        actuals.len(),
        Some(unit.unwrap_or(Unit::Number)),
    ))
}
pub(crate) fn dot(_builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    if !matches!((&actuals[0],&actuals[1]),(Type::Vector(n,Some(Unit::Number)),Type::Vector(m,Some(Unit::Number)))if n==m)
    {
        return Err("dot requires dimensionless vectors with matching dimensions".into());
    }
    Ok(Type::Numeric(Unit::Number))
}
pub(crate) fn len(_builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    if matches!(&actuals[0],Type::DataFrame{groups,..} if !groups.is_empty()) {
        return Err("aggregate a grouped DataFrame before requesting its length".into());
    }
    if !matches!(
        &actuals[0],
        Type::Dict
            | Type::Data
            | Type::List(_)
            | Type::Array(_)
            | Type::DataFrame { .. }
            | Type::Series(_)
    ) {
        return Err("len requires Dict, Data, List, Array, DataFrame, or Series".into());
    }
    Ok(Type::Numeric(Unit::Number))
}
pub(crate) fn get(_builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    let inner = match &actuals[0] {
        Type::Dict => {
            expect(&Type::String, &actuals[1])?;
            Type::Data
        }
        Type::Data => {
            if !matches!(actuals[1], Type::String | Type::Numeric(Unit::Number)) {
                return Err("get Data index requires String or Number".into());
            }
            Type::Data
        }
        Type::List(t) | Type::Array(t) => {
            expect(&Type::Numeric(Unit::Number), &actuals[1])?;
            *t.clone()
        }
        _ => return Err("get requires Dict, Data, List or Array".into()),
    };
    Ok(Type::Option(Box::new(inner)))
}
pub(crate) fn some(_builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    Ok(Type::Option(Box::new(actuals[0].clone())))
}
pub(crate) fn keys(builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    if !matches!(actuals[0], Type::Dict | Type::Data) {
        return Err("keys requires Dict or object Data".into());
    }
    Ok(builtin.result.clone())
}
pub(crate) fn unwrap(_builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    let Type::Option(inner) = &actuals[0] else {
        return Err("unwrap requires Option<T>".into());
    };
    expect(inner, &actuals[1])?;
    Ok(*inner.clone())
}
pub(crate) fn assertion(builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    if !matches!(actuals[0], Type::Bool | Type::Predicate) {
        return Err("assert/check requires Bool or a visual relationship".into());
    }
    Ok(builtin.result.clone())
}
pub(crate) fn math(builtin: &Builtin, actuals: &[Type]) -> Result<Type, String> {
    let function =
        sanim_core::math::functions::function(builtin.name).expect("registered math function");
    let unit = scalar_unit(&actuals[0]);
    let valid = match function.domain {
        Domain::Scalar => unit == Some(Unit::Number),
        Domain::Trig => matches!(unit, Some(Unit::Number | Unit::Angle)),
        Domain::Numeric => unit.is_some(),
    };
    if !valid {
        return Err(format!(
            "{} requires {}; found {}",
            builtin.name,
            if function.domain == Domain::Trig {
                "Number (radians) or Angle"
            } else if function.domain == Domain::Numeric {
                "numeric arguments"
            } else {
                "dimensionless Number"
            },
            actuals[0]
        ));
    }
    for other in actuals[1..].iter().filter(|t| **t != Type::Unit) {
        if scalar_unit(other) != unit {
            return Err("math arguments must have matching numeric units".into());
        }
    }
    Ok(
        if function.domain == Domain::Numeric && builtin.name != "sign" {
            actuals[0].clone()
        } else {
            Type::Numeric(Unit::Number)
        },
    )
}
