//! Canonical literal fragments and typed interpolation holes, shared by syntax
//! and evaluated templates. A template becomes text only when String is wanted.
use crate::Value;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Emit a plain value as Sanim source, escaping interpolation braces as well
/// as the escapes understood by the language's direct string parser.
pub fn quote_string(value: &str) -> String {
    let mut source = String::with_capacity(value.len().saturating_add(2));
    source.push('"');
    for character in value.chars() {
        match character {
            '\n' => source.push_str("\\n"),
            '\r' => source.push_str("\\r"),
            '\t' => source.push_str("\\t"),
            '"' => source.push_str("\\\""),
            '\\' => source.push_str("\\\\"),
            '{' => source.push_str("{{"),
            '}' => source.push_str("}}"),
            character => source.push(character),
        }
    }
    source.push('"');
    source
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TemplatePart<T> {
    Text(Arc<str>),
    Hole {
        value: T,
        format: Option<T>,
        conversion: Option<char>,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub parts: Arc<[TemplatePart<Value>]>,
}
impl Template {
    pub fn render(&self, limit: usize) -> Result<String, String> {
        render_with(&self.parts, |value| Ok(value.clone()), limit)
    }
}

/// Render syntax, stored values, or retained nodes through the same formatter.
pub fn render_with<T>(
    parts: &[TemplatePart<T>],
    mut resolve: impl FnMut(&T) -> Result<Value, String>,
    limit: usize,
) -> Result<String, String> {
    let mut output = String::new();
    for part in parts {
        let rendered;
        let text = match part {
            TemplatePart::Text(text) => text.as_ref(),
            TemplatePart::Hole {
                value,
                format,
                conversion,
            } => {
                let value = resolve(value)?;
                let format = format.as_ref().map(&mut resolve).transpose()?;
                let conversion = conversion.map(|value| value.to_string());
                rendered = crate::math::format::value(
                    &value,
                    format
                        .as_ref()
                        .map(Value::string)
                        .transpose()?
                        .unwrap_or(""),
                    conversion.as_deref(),
                )?;
                &rendered
            }
        };
        output
            .len()
            .checked_add(text.len())
            .filter(|size| *size <= limit)
            .ok_or("rendered template exceeds the configured string byte budget")?;
        output.push_str(text);
    }
    Ok(output)
}
