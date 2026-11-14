//! Structural type operations resolve to canonical records/unions, never parent objects.
use sanim_core::Type;
use std::cell::Cell;

pub(crate) fn intersection(left: Type, right: Type, budget: &Cell<usize>) -> Result<Type, String> {
    intersection_inner(left, right, budget)?
        .ok_or_else(|| "type intersection is empty; no value satisfies all alternatives".into())
}
fn intersection_inner(
    left: Type,
    right: Type,
    budget: &Cell<usize>,
) -> Result<Option<Type>, String> {
    charge(&left, budget)?;
    if left == right {
        return Ok(Some(left));
    }
    if let (Some(left), Some(right)) = (left.numeric_domain(), right.numeric_domain()) {
        return Ok(left.intersection(&right).map(Type::RefinedNumeric));
    }
    match (left, right) {
        (Type::Union(values), right) => union_present(
            values
                .into_iter()
                .map(|left| intersection_inner(left, copy(&right, budget)?, budget)),
        ),
        (left, Type::Union(values)) => union_present(
            values
                .into_iter()
                .map(|right| intersection_inner(copy(&left, budget)?, right, budget)),
        ),
        (Type::Record(mut left), Type::Record(right)) => {
            for (name, ty) in right {
                let value = if let Some(previous) = left.remove(&name) {
                    let Some(value) =
                        intersection_inner(previous, ty, budget).map_err(|message| {
                            format!("field '{name}' conflicts in record composition: {message}")
                        })?
                    else {
                        return Ok(None);
                    };
                    value
                } else {
                    ty
                };
                left.insert(name, value);
            }
            Ok(Some(Type::Record(left)))
        }
        (left, right) => Err(format!(
            "cannot compose incompatible types {left} and {right}; '&' combines record fields, and repeated fields must have compatible types"
        )),
    }
}
pub(crate) fn difference(left: Type, right: Type, budget: &Cell<usize>) -> Result<Type, String> {
    difference_inner(left, right, budget)?.ok_or_else(|| {
        "type subtraction is empty; it removes every alternative in the base type".into()
    })
}
fn difference_inner(left: Type, right: Type, budget: &Cell<usize>) -> Result<Option<Type>, String> {
    charge(&left, budget)?;
    if let (Some(left), Some(right)) = (left.numeric_domain(), right.numeric_domain()) {
        return union_present(
            left.difference(&right)?
                .into_iter()
                .map(|domain| Ok(Some(Type::RefinedNumeric(domain)))),
        );
    }
    match (left, right) {
        (Type::Union(values), right) => union_present(
            values
                .into_iter()
                .map(|left| difference_inner(left, copy(&right, budget)?, budget)),
        ),
        (left, Type::Union(values)) => {
            let mut output = Some(left);
            for right in values {
                let Some(left) = output else {
                    break;
                };
                output = difference_inner(left, right, budget)?;
            }
            Ok(output)
        }
        (Type::Record(mut fields), Type::Record(removed)) => {
            for name in removed.keys() {
                if fields.remove(name).is_none() {
                    return Err(format!(
                        "cannot remove missing field '{name}'; remaining fields: {}",
                        fields.keys().cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
            }
            Ok(Some(Type::Record(fields)))
        }
        (left, right) => Err(format!(
            "record subtraction requires record operands, found {left} - {right}; write Base - {{field:Type}}"
        )),
    }
}
/// Bottom remains internal while distributing; only a wholly empty public type fails.
fn union_present(
    values: impl IntoIterator<Item = Result<Option<Type>, String>>,
) -> Result<Option<Type>, String> {
    let values = values
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if values.is_empty() {
        Ok(None)
    } else {
        Type::union(values).map(Some)
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
        Type::DataFrame { .. } if name == "columns" => Ok(Type::List(
            Box::new(Type::String),
            sanim_core::ListLength::Unknown,
        )),
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
        Type::Matrix4 if name == "rows" => Ok(Type::List(
            Box::new(Type::List(
                Box::new(Type::Numeric(Unit::Number)),
                sanim_core::ListLength::Unknown,
            )),
            sanim_core::ListLength::Unknown,
        )),
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
            "overflow" | "overflow_x" | "overflow_y" => Ok(sanim_core::Overflow::ty()),
            _ => Err(format!("unknown element property '{name}'")),
        },
        Type::Array(_) if name == "shape" => Ok(Type::List(
            Box::new(Type::Numeric(Unit::Number)),
            sanim_core::ListLength::Unknown,
        )),
        other => Err(format!("{other} has no field '{name}'")),
    }
}

/// A branch can refine an immutable snapshot through a string equality proof.
/// Unknown String exclusions remain String; finite unions can exclude a choice.
pub(crate) fn narrow_string(ty: &Type, path: &[&str], literal: &str, equal: bool) -> Option<Type> {
    if let Type::Union(types) = ty {
        return Type::union(
            types
                .iter()
                .filter_map(|ty| narrow_string(ty, path, literal, equal)),
        )
        .ok();
    }
    if let Some((name, path)) = path.split_first() {
        let Type::Record(fields) = ty else {
            return None;
        };
        let narrowed = narrow_string(fields.get(*name)?, path, literal, equal)?;
        let mut fields = fields.clone();
        fields.insert((*name).into(), narrowed);
        return Some(Type::Record(fields));
    }
    match ty {
        Type::StringLiteral(value) if (value.as_ref() == literal) == equal => Some(ty.clone()),
        Type::String if equal => Some(Type::StringLiteral(literal.into())),
        Type::String => Some(Type::String),
        _ => None,
    }
}
