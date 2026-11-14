//! Compact labels and complete hover types share the compiler's structural types.
use sanim_lang::{AnalyzedModule, core::Type, prelude};

fn name<'a>(module: &'a AnalyzedModule, ty: &Type) -> Option<&'a str> {
    module
        .checked
        .schemas
        .iter()
        .find(|(name, schema)| !name.contains('.') && *schema == ty)
        .map(|(name, _)| name.as_str())
        .or_else(|| native_name(ty))
}

fn native_name(ty: &Type) -> Option<&'static str> {
    prelude::types()
        .iter()
        .filter_map(|(name, definition)| match definition {
            prelude::TypeDefinition::Plain(schema)
                if matches!(schema, Type::Record(_)) && schema == ty =>
            {
                Some(*name)
            }
            _ => None,
        })
        // Prefer the structural constructor over convenience factories
        // returning the same record (CodeRange rather than CodeLines).
        .max_by_key(|name| {
            let Type::Record(fields) = ty else {
                return 0;
            };
            prelude::builtin(name).map_or(0, |builtin| {
                builtin
                    .parameters
                    .iter()
                    .filter(|parameter| fields.contains_key(parameter.name))
                    .count()
            })
        })
}

pub(crate) fn compact(module: &AnalyzedModule, ty: &Type) -> String {
    if let Some(name) = name(module, ty) {
        return name.to_owned();
    }
    match ty {
        Type::StringLiteral(value) => {
            const MAX_LITERAL_LABEL: usize = 80;
            let mut characters = value.chars();
            let prefix: String = characters.by_ref().take(MAX_LITERAL_LABEL).collect();
            let truncated = characters.next().is_some();
            let quoted = format!("{prefix:?}");
            if truncated || quoted.chars().count() > MAX_LITERAL_LABEL {
                format!(
                    "{}…\"",
                    quoted
                        .chars()
                        .take(MAX_LITERAL_LABEL - 2)
                        .collect::<String>()
                )
            } else {
                quoted
            }
        }
        Type::Record(_) => "{…}".into(),
        Type::Overloaded(signatures) => format!("fn ({} clauses)", signatures.len()),
        Type::List(inner, length) => match length {
            sanim_lang::core::ListLength::Unknown => format!("List<{}>", compact(module, inner)),
            sanim_lang::core::ListLength::Exact(length) => {
                format!("List<{},{}>", compact(module, inner), length)
            }
        },
        Type::Option(inner) => format!("Option<{}>", compact(module, inner)),
        Type::Array(inner) => format!("Array<{}>", compact(module, inner)),
        Type::Series(inner) => format!("Series<{}>", compact(module, inner)),
        Type::Signal(inner) => format!("sig {}", compact(module, inner)),
        Type::DataFrame { row, .. } => format!("DataFrame<{}>", compact(module, row)),
        Type::Vector(n, Some(unit)) => {
            format!("Vec{n}<{}>", compact(module, &Type::Numeric(*unit)))
        }
        Type::Union(types) => types
            .iter()
            .map(|ty| compact(module, ty))
            .collect::<Vec<_>>()
            .join(" | "),
        _ => ty.to_string(),
    }
}

/// Source annotations have stricter syntax than display labels (no ellipses or unit hints).
/// Do not offer an edit for internal/callable types the surface grammar cannot express.
pub(crate) fn annotation(module: &AnalyzedModule, ty: &Type) -> Option<String> {
    if let Some((name, _)) = module
        .checked
        .schemas
        .iter()
        .find(|(_, schema)| *schema == ty)
    {
        return Some(name.clone());
    }
    if let Some(name) = native_name(ty) {
        return Some(name.into());
    }
    let inner = |ty: &Type| annotation(module, ty);
    let container = |name: &str, ty: &Type| Some(format!("{name}<{}>", inner(ty)?));
    match ty {
        Type::Unknown | Type::Function(_) | Type::Overloaded(_) | Type::Code(_) => None,
        Type::Numeric(_) | Type::RefinedNumeric(_) => {
            prelude::types().iter().find_map(|(name, definition)| {
                matches!(definition, prelude::TypeDefinition::Plain(schema) if schema == ty)
                    .then(|| (*name).into())
            })
        }
        Type::Vector(n, Some(unit)) => Some(format!("Vec{n}<{}>", inner(&Type::Numeric(*unit))?)),
        // A unit-polymorphic vector cannot be annotated Vec2: that means Vec2<Number>.
        Type::Vector(_, None) => None,
        Type::List(ty, length) => Some(match length {
            sanim_lang::core::ListLength::Unknown => format!("List<{}>", inner(ty)?),
            sanim_lang::core::ListLength::Exact(length) => format!("List<{},{length}>", inner(ty)?),
        }),
        Type::Option(ty) => container("Option", ty),
        Type::Array(ty) => container("Array", ty),
        Type::Series(ty) => container("Series", ty),
        Type::DataFrame { row, groups } if groups.is_empty() => container("DataFrame", row),
        // Group keys carry capabilities that an ordinary DataFrame<T> annotation loses.
        Type::DataFrame { .. } => None,
        Type::Signal(ty) => Some(format!("sig ({})", inner(ty)?)),
        Type::Union(types) => Some(
            types
                .iter()
                .map(inner)
                .collect::<Option<Vec<_>>>()?
                .join(" | "),
        ),
        Type::Record(fields) => Some(format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(name, ty)| {
                    let mut chars = name.chars();
                    if !chars
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        return None;
                    }
                    Some(format!("{name}: {}", inner(ty)?))
                })
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        )),
        Type::Predicate => Some("Predicate".into()),
        _ => Some(ty.to_string()),
    }
}

fn expanded(module: &AnalyzedModule, ty: &Type, depth: usize) -> String {
    match ty {
        Type::StringLiteral(_) => ty.to_string(),
        Type::Overloaded(signatures) => signatures
            .iter()
            .map(|signature| {
                format!(
                    "{}  # duration: {}",
                    Type::Function(signature.clone()),
                    signature.duration
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Type::Record(fields) => {
            let indent = "    ".repeat(depth);
            let child_indent = "    ".repeat(depth + 1);
            let fields = fields
                .iter()
                .map(|(name, ty)| {
                    format!("{child_indent}{name}: {},", expanded(module, ty, depth + 1))
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("{{\n{fields}\n{indent}}}")
        }
        Type::List(inner, length) => match length {
            sanim_lang::core::ListLength::Unknown => {
                format!("List<{}>", expanded(module, inner, depth))
            }
            sanim_lang::core::ListLength::Exact(length) => {
                format!("List<{},{}>", expanded(module, inner, depth), length)
            }
        },
        Type::Option(inner) => format!("Option<{}>", expanded(module, inner, depth)),
        Type::Array(inner) => format!("Array<{}>", expanded(module, inner, depth)),
        Type::Series(inner) => format!("Series<{}>", expanded(module, inner, depth)),
        Type::Signal(inner) => format!("sig {}", expanded(module, inner, depth)),
        Type::DataFrame { row, groups } => {
            let grouped = if groups.is_empty() {
                String::new()
            } else {
                format!(" grouped by {}", groups.join(", "))
            };
            format!("DataFrame<{}>{grouped}", expanded(module, row, depth))
        }
        Type::Union(types) => types
            .iter()
            .map(|ty| expanded(module, ty, depth))
            .collect::<Vec<_>>()
            .join(" | "),
        _ => compact(module, ty),
    }
}

pub(crate) fn tooltip(module: &AnalyzedModule, ty: &Type) -> String {
    let label = compact(module, ty);
    let full = expanded(module, ty, 0);
    if label == full {
        format!("```sanim\n{full}\n```")
    } else {
        format!("**{label}**\n\n```sanim\n{full}\n```")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_literal_inlay_is_bounded_but_hover_keeps_the_complete_literal() {
        let module = AnalyzedModule {
            ast: std::sync::Arc::new(sanim_lang::ast::Module {
                statements: vec![],
                comments: vec![],
            }),
            checked: Default::default(),
        };
        let value = "SELECT name FROM users WHERE active = true; ".repeat(20);
        let ty = Type::StringLiteral(value.clone().into());
        let label = compact(&module, &ty);
        assert!(label.chars().count() <= 80);
        assert!(label.ends_with("…\""));
        assert!(tooltip(&module, &ty).contains(&value));
        assert_eq!(
            compact(&module, &Type::StringLiteral("left".into())),
            "\"left\""
        );
    }
}
