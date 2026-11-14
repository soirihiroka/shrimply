//! Read-only native declarations, generated once from the language's registries.
use crate::{features, text::Document};
use sanim_lang::{
    core::{Span, Type},
    prelude,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};

pub(crate) const URI: &str = "sanim-stdlib:/standard-library.md";

struct Reference {
    document: Document,
    definitions: BTreeMap<String, Span>,
}
fn reference() -> &'static Reference {
    static REFERENCE: OnceLock<Reference> = OnceLock::new();
    REFERENCE.get_or_init(|| {
        let mut sections = BTreeMap::<String, Vec<String>>::new();
        for builtin in prelude::all_builtins() {
            sections.entry(builtin.name.into()).or_default().push(format!(
                "```sanim\n{}\n```\n\nDefault timeline duration: {}s. Argument and result types may be specialized by the compiler at each call.",
                features::native_signature(builtin, false).0,
                builtin.timing.default,
            ));
        }
        for (name, definition) in prelude::types() {
            use prelude::TypeDefinition;
            let description = match definition {
                TypeDefinition::Plain(Type::Record(fields)) => format!("type {name} = {}", Type::Record(fields.clone())),
                TypeDefinition::Plain(Type::Enum { variants, .. }) => {
                    for (variant, fields) in variants {
                        let qualified = format!("{name}.{variant}");
                        let arguments = fields.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
                        sections.entry(qualified.clone()).or_default().push(format!("```sanim\n{qualified}({arguments}) -> {name}\n```"));
                    }
                    format!("enum {name} {{ {} }}", variants.keys().cloned().collect::<Vec<_>>().join(", "))
                }
                TypeDefinition::Plain(_) => format!("Native type: `{name}`."),
                TypeDefinition::List => format!("`{name}<T>` or `{name}<T, N>` — persistent list, with an optional exact length."),
                TypeDefinition::Container(_) => format!("`{name}<T>` — one element or row type argument."),
                TypeDefinition::Vector(_) => format!("`{name}` or `{name}<Unit>` — components default to Number."),
                TypeDefinition::Bound(_) => format!("Generic constraint: `{name}`."),
            };
            sections.entry((*name).into()).or_default().push(description);
        }
        for (name, value) in prelude::values() {
            if !matches!(value.ty(), Type::Record(_)) {
                sections.entry((*name).into()).or_default().push(format!("```sanim\nlet {name}: {}\n```", value.ty()));
            }
        }
        let mut text = String::from("# Sanim standard library\n\nRead-only native declarations generated from the running compiler. Native implementations live in Rust; this reference is not an executable Sanim module.\n\n");
        let mut definitions = BTreeMap::new();
        for (name, descriptions) in sections {
            text.push_str("## ");
            let start = text.len();
            text.push_str(&name);
            definitions.insert(name, Span { start, end: text.len(), ..Default::default() });
            text.push_str("\n\n");
            text.push_str(&descriptions.join("\n\n"));
            text.push_str("\n\n");
        }
        Reference { document: Document::new(text, 0), definitions }
    })
}

pub(crate) fn content() -> &'static str {
    &reference().document.text
}
pub(crate) fn location(name: &str) -> Option<Value> {
    let reference = reference();
    Some(json!({"uri":URI,"range":reference.document.range(*reference.definitions.get(name)?)}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_registered_native_declaration_has_a_precise_destination() {
        let mut names = std::collections::BTreeSet::new();
        for builtin in prelude::all_builtins() {
            assert!(
                names.insert(builtin.name),
                "duplicate native declaration {}",
                builtin.name
            );
            assert!(location(builtin.name).is_some(), "{}", builtin.name);
            assert!(std::ptr::eq(
                prelude::builtin(builtin.name).unwrap(),
                builtin
            ));
        }
        for name in prelude::types().keys().chain(prelude::values().keys()) {
            assert!(location(name).is_some(), "{name}");
        }
        for (name, span) in &reference().definitions {
            assert_eq!(&content()[span.start..span.end], name);
        }
    }
}
