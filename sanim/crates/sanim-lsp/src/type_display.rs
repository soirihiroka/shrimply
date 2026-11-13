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
                .find_map(|(name, definition)| match definition {
                    prelude::TypeDefinition::Plain(schema)
                        if matches!(schema, Type::Record(_)) && schema == ty =>
                    {
                        Some(*name)
                    }
                    _ => None,
                })
        })
}

pub(crate) fn compact(module: &AnalyzedModule, ty: &Type) -> String {
    if let Some(name) = name(module, ty) {
        return name.to_owned();
    }
    match ty {
        Type::Record(_) => "{…}".into(),
        Type::List(inner) => format!("List<{}>", compact(module, inner)),
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
        Type::List(inner) => format!("List<{}>", expanded(module, inner, depth)),
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
