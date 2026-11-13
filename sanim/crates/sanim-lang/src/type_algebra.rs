//! Structural type operations resolve to canonical records/unions, never parent objects.
use sanim_core::Type;
use std::cell::Cell;

pub(crate) fn intersection(left: Type, right: Type, budget: &Cell<usize>) -> Result<Type, String> {
    charge(&left, budget)?;
    if left == right {
        return Ok(left);
    }
    match (left, right) {
        (Type::Union(values), right) => Type::union(
            values
                .into_iter()
                .map(|left| intersection(left, copy(&right, budget)?, budget))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        (left, Type::Union(values)) => Type::union(
            values
                .into_iter()
                .map(|right| intersection(copy(&left, budget)?, right, budget))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        (Type::Record(mut left), Type::Record(right)) => {
            for (name, ty) in right {
                let value = if let Some(previous) = left.remove(&name) {
                    intersection(previous, ty, budget).map_err(|message| {
                        format!("field '{name}' conflicts in record composition: {message}")
                    })?
                } else {
                    ty
                };
                left.insert(name, value);
            }
            Ok(Type::Record(left))
        }
        (left, right) => Err(format!(
            "cannot compose incompatible types {left} and {right}; '&' combines record fields, and repeated fields must have compatible types"
        )),
    }
}

pub(crate) fn difference(left: Type, right: Type, budget: &Cell<usize>) -> Result<Type, String> {
    match (left, right) {
        (Type::Union(values), right) => Type::union(
            values
                .into_iter()
                .map(|left| difference(left, copy(&right, budget)?, budget))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        (Type::Record(mut fields), Type::Record(removed)) => {
            for name in removed.keys() {
                if fields.remove(name).is_none() {
                    return Err(format!(
                        "cannot remove missing field '{name}'; remaining fields: {}",
                        fields.keys().cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
            }
            Ok(Type::Record(fields))
        }
        (left, right) => Err(format!(
            "record subtraction requires record operands, found {left} - {right}; write Base - {{field:Type}}"
        )),
    }
}

fn charge(ty: &Type, budget: &Cell<usize>) -> Result<(), String> {
    let mut remaining = budget.get();
    sanim_core::type_budget::charge(ty, &mut remaining)?;
    budget.set(remaining);
    Ok(())
}
fn copy(ty: &Type, budget: &Cell<usize>) -> Result<Type, String> {
    charge(ty, budget)?;
    Ok(ty.clone())
}

/// Member projection uses the same rule for a concrete type and every union arm.
pub(crate) fn member(ty: Type, name: &str) -> Result<Type, String> {
    use sanim_core::{TypeBound, Unit};
    match ty {
        Type::BoundedVariable(_, bound) => match *bound {
            TypeBound::Type(ty) => member(*ty, name),
            _ => Err("this generic constraint exposes no fields".into()),
        },
        Type::Union(alternatives) => {
            let mut fields = Vec::new();
            for alternative in alternatives {
                fields.push(member(alternative, name).map_err(|message| {
                    format!(
                        "field '{name}' must be available on every union alternative: {message}"
                    )
                })?);
            }
            Type::union(fields)
        }
        Type::DataFrame { .. } if name == "columns" => Ok(Type::List(Box::new(Type::String))),
        Type::Dict | Type::Data => Err(format!(
            "dictionary values use brackets: write value[\"{name}\"] instead of value.{name}"
        )),
        Type::Record(fields) => fields.get(name).cloned().ok_or_else(|| {
            format!(
                "unknown field '{name}'; available fields: {}",
                fields.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        }),
        Type::Vector(n, Some(unit))
            if ["x", "y", "z", "w"]
                .iter()
                .take(n)
                .any(|field| *field == name) =>
        {
            Ok(Type::Numeric(unit))
        }
        Type::Fraction if matches!(name, "numerator" | "denominator") => {
            Ok(Type::Numeric(Unit::Number))
        }
        Type::Matrix4 if name == "rows" => Ok(Type::List(Box::new(Type::List(Box::new(
            Type::Numeric(Unit::Number),
        ))))),
        Type::Element
            if sanim_core::code::view::INPUTS
                .iter()
                .any(|(input, _)| *input == name) =>
        {
            let property = sanim_core::code::view::INPUTS
                .iter()
                .find(|(input, _)| *input == name)
                .expect("checked native code input")
                .1;
            Ok(sanim_core::code::view::input_type(property).expect("typed native code input"))
        }
        Type::Element => match name {
            "position" | "scroll" => Type::union([
                Type::Vector(2, Some(Unit::Length)),
                Type::Vector(2, Some(Unit::Percent)),
            ]),
            "opacity" | "scale" => Ok(Type::Numeric(Unit::Number)),
            "rotation" => Ok(Type::Numeric(Unit::Angle)),
            "radius" => Ok(Type::Numeric(Unit::Length)),
            "width" | "height" => Ok(sanim_core::Layout::dimension_type()),
            "fill" => Ok(sanim_core::Paint::ty()),
            "id" => Ok(Type::String),
            _ => Err(format!("unknown element property '{name}'")),
        },
        Type::Array(_) if name == "shape" => Ok(Type::List(Box::new(Type::Numeric(Unit::Number)))),
        other => Err(format!("{other} has no field '{name}'")),
    }
}
