//! Reflection identity and presentation metadata, shared by compiler and hosts.
use crate::{Span, Type};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Options {
    pub key: Option<String>,
    pub label: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Declaration {
    pub key: String,
    pub name: String,
    pub label: String,
    /// Normalized path relative to the source root, never an absolute machine path.
    pub source: String,
    pub span: Span,
    pub ty: Arc<Type>,
}
impl Options {
    pub fn resolve(
        &self,
        source: &str,
        name: &str,
        ty: Type,
        span: Span,
    ) -> Result<Arc<Declaration>, String> {
        let key = self
            .key
            .clone()
            .unwrap_or_else(|| format!("{source}::{name}"));
        if key.trim().is_empty() || key.trim() != key || key.chars().any(char::is_control) {
            return Err("reflection key must be nonempty, without surrounding whitespace or control characters; use key=\"palette.accent\" or omit it for a file-qualified key".into());
        }
        let label = self.label.clone().unwrap_or_else(|| display_name(name));
        if label.trim().is_empty() {
            return Err("reflection label must contain visible text; omit it to format the variable name automatically".into());
        }
        Ok(Arc::new(Declaration {
            key,
            name: name.into(),
            label,
            source: source.into(),
            span,
            ty: Arc::new(ty),
        }))
    }
}

/// Split snake_case, camelCase, acronym boundaries and numeric suffixes without a dependency.
pub fn display_name(name: &str) -> String {
    let chars: Vec<_> = name.chars().collect();
    let mut words = Vec::new();
    let mut word = String::new();
    for (index, &ch) in chars.iter().enumerate() {
        if !ch.is_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        let previous = index.checked_sub(1).map(|i| chars[i]);
        let next = chars.get(index + 1);
        let boundary = previous.is_some_and(|prev| {
            (prev.is_lowercase() && ch.is_uppercase())
                || (prev.is_uppercase()
                    && ch.is_uppercase()
                    && next.is_some_and(|c| c.is_lowercase()))
                || (prev.is_alphabetic() && ch.is_numeric())
                || (prev.is_numeric()
                    && ch.is_uppercase()
                    && next.is_some_and(|c| c.is_lowercase()))
        });
        if boundary && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        if word.is_empty() {
            word.extend(ch.to_uppercase());
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    if words.is_empty() {
        name.to_owned()
    } else {
        words.join(" ")
    }
}

/// Canonical identity and legacy-key compatibility for compilers, editors and headless hosts.
#[derive(Clone, Debug)]
pub struct Schema {
    entry: String,
    declarations: std::collections::BTreeMap<String, Arc<Declaration>>,
    legacy_root_names: std::collections::BTreeMap<String, Option<String>>,
}
impl Schema {
    pub fn new(
        entry: &str,
        declarations: impl IntoIterator<Item = Arc<Declaration>>,
    ) -> Result<Self, crate::Diagnostic> {
        let mut schema = Self {
            entry: entry.into(),
            declarations: Default::default(),
            legacy_root_names: Default::default(),
        };
        for declaration in declarations {
            if let Some(previous) = schema
                .declarations
                .insert(declaration.key.clone(), declaration.clone())
            {
                return Err(crate::Diagnostic::new(
                    "S113",
                    format!(
                        "duplicate reflection key '{}': '{}::{}' at {}:{} and '{}::{}' use the same config key. Choose distinct key= strings, or omit key to use each file's variable name.",
                        declaration.key,
                        previous.source,
                        previous.name,
                        previous.span.line,
                        previous.span.column,
                        declaration.source,
                        declaration.name
                    ),
                    &declaration.source,
                    declaration.span,
                ));
            }
            if declaration.source == entry {
                schema
                    .legacy_root_names
                    .entry(declaration.name.clone())
                    .and_modify(|key| *key = None)
                    .or_insert_with(|| Some(declaration.key.clone()));
            }
        }
        for (name, legacy_key) in &schema.legacy_root_names {
            let Some(canonical) = schema.declarations.get(name) else {
                continue;
            };
            let legacy = legacy_key
                .as_ref()
                .map(|key| &schema.declarations[key])
                .or_else(|| {
                    schema
                        .declarations
                        .values()
                        .find(|v| v.source == entry && v.name == *name && v.key != canonical.key)
                });
            if let Some(legacy) = legacy
                && canonical.key != legacy.key
            {
                return Err(crate::Diagnostic::new(
                    "S113",
                    format!(
                        "reflection key '{name}' for '{}::{}' conflicts with the legacy root name of '{}::{}'. Existing overrides would target the wrong input. Choose a distinct key such as key=\"palette.{name}\", or omit the explicit key.",
                        canonical.source, canonical.name, legacy.source, legacy.name
                    ),
                    &canonical.source,
                    canonical.span,
                ));
            }
        }
        Ok(schema)
    }
    pub fn declaration(&self, provided: &str) -> Result<&Arc<Declaration>, crate::Diagnostic> {
        if let Some(declaration) = self.declarations.get(provided) {
            return Ok(declaration);
        }
        if let Some(alias) = self.legacy_root_names.get(provided) {
            if let Some(key) = alias {
                return Ok(&self.declarations[key]);
            }
            return Err(crate::Diagnostic::new(
                "S302",
                format!(
                    "ambiguous reflected input name '{provided}': several inline inputs share this inferred name. Supply one of their canonical config keys: {}",
                    self.declarations
                        .values()
                        .filter(|v| v.source == self.entry && v.name == provided)
                        .map(|v| v.key.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                &self.entry,
                Span::default(),
            ));
        }
        Err(crate::Diagnostic::new(
            "S302",
            format!(
                "unknown reflected input '{provided}'; available config keys: {}. Use a key reported by the reflection metadata.",
                self.declarations
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            &self.entry,
            Span::default(),
        ))
    }
    pub fn normalize<T>(
        &self,
        supplied: &std::collections::BTreeMap<String, T>,
        convert: impl Fn(&Type, &T) -> Result<crate::Value, String>,
    ) -> Result<std::collections::BTreeMap<String, crate::Value>, crate::Diagnostic> {
        let mut normalized = std::collections::BTreeMap::new();
        let mut originals = std::collections::BTreeMap::new();
        for (provided, value) in supplied {
            let declaration = self.declaration(provided)?;
            let key = &declaration.key;
            if let Some(previous) = originals.insert(key, provided) {
                return Err(crate::Diagnostic::new(
                    "S302",
                    format!(
                        "reflected input '{key}' was provided twice, as '{previous}' and '{provided}'; supply only the canonical config key '{key}'"
                    ),
                    &declaration.source,
                    declaration.span,
                ));
            }
            let value = convert(&declaration.ty, value).map_err(|message| {
                crate::Diagnostic::new(
                    "S302",
                    format!(
                        "reflected input '{}' ({}): expected {}; {message}",
                        declaration.key, declaration.label, declaration.ty
                    ),
                    &declaration.source,
                    declaration.span,
                )
            })?;
            normalized.insert(key.clone(), value);
        }
        Ok(normalized)
    }
}
