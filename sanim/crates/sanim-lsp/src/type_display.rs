//! Compact labels and complete hover types share the compiler's structural types.
use sanim_lang::{AnalyzedModule, core::Type, prelude};

fn name<'a>(module: &'a AnalyzedModule, ty: &Type) -> Option<&'a str> {
    module
        .checked
        .schemas
        .iter()
        .find(|(name, schema)| !name.contains('.') && *schema == ty)
        .map(|(name, _)| name.as_str())
        .or_else(|| {
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
