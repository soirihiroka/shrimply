//! Charge semantic type expansion before cloning resolved schemas.
use crate::{Type, TypeBound};
pub fn charge(ty: &Type, remaining: &mut usize) -> Result<(), String> {
    charge_substitution(ty, &std::collections::BTreeMap::new(), remaining)
}
pub fn charge_substitution(
    ty: &Type,
    bindings: &std::collections::BTreeMap<String, Type>,
    remaining: &mut usize,
) -> Result<(), String> {
    fn take(remaining: &mut usize, bytes: usize) -> Result<(), String> {
        *remaining = remaining
            .checked_sub(bytes)
            .ok_or("resolved type budget exceeded; simplify or split repeated nested schemas")?;
        Ok(())
    }
    fn bound(
        bound: &TypeBound,
        remaining: &mut usize,
        bindings: &std::collections::BTreeMap<String, Type>,
        depth: usize,
    ) -> Result<(), String> {
        if let TypeBound::Type(ty) = bound {
            walk(ty, remaining, bindings, depth)?;
        }
        Ok(())
    }
    fn walk(
        ty: &Type,
        remaining: &mut usize,
        bindings: &std::collections::BTreeMap<String, Type>,
        depth: usize,
    ) -> Result<(), String> {
        if depth == 0 {
            return Err("resolved type nesting limit exceeded".into());
        }
        if let Type::Variable(name) | Type::BoundedVariable(name, _) = ty
            && let Some(actual) = bindings.get(name)
        {
            return charge(actual, remaining);
        }
        take(remaining, std::mem::size_of::<Type>())?;
        match ty {
            Type::List(t)
            | Type::Option(t)
            | Type::Array(t)
            | Type::Series(t)
            | Type::Signal(t) => walk(t, remaining, bindings, depth - 1)?,
            Type::DataFrame { row, groups } => {
                walk(row, remaining, bindings, depth - 1)?;
                for name in groups {
                    take(remaining, name.len() + std::mem::size_of::<String>())?;
                }
            }
            Type::Union(alternatives) => {
                if alternatives.is_empty() {
                    return Err("a union type requires at least one alternative".into());
                }
                for ty in alternatives {
                    walk(ty, remaining, bindings, depth - 1)?;
                }
            }
            Type::Record(fields) => {
                for (name, ty) in fields {
                    take(remaining, name.len() + std::mem::size_of::<String>())?;
                    walk(ty, remaining, bindings, depth - 1)?;
                }
            }
            Type::Enum { name, variants } => {
                take(remaining, name.len())?;
                for (name, fields) in variants {
                    take(remaining, name.len())?;
                    for ty in fields {
                        walk(ty, remaining, bindings, depth - 1)?;
                    }
                }
            }
            Type::Variable(name) => take(remaining, name.len())?,
            Type::StringLiteral(value) => take(remaining, value.len())?,
            Type::BoundedVariable(name, b) => {
                take(remaining, name.len())?;
                bound(b, remaining, bindings, depth - 1)?;
            }
            Type::Function(sig) => {
                for parameter in &sig.generics {
                    take(remaining, parameter.name.len())?;
                    if let Some(ty) = &parameter.default {
                        walk(ty, remaining, bindings, depth - 1)?;
                    }
                    if let Some(b) = &parameter.constraint {
                        bound(b, remaining, bindings, depth - 1)?;
                    }
                }
                for parameter in &sig.parameters {
                    take(remaining, parameter.name.len())?;
                    walk(&parameter.ty, remaining, bindings, depth - 1)?;
                }
                walk(&sig.result, remaining, bindings, depth - 1)?;
            }
            _ => {}
        }
        Ok(())
    }
    walk(ty, remaining, bindings, 128)
}
