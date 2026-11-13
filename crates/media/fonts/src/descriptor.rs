//! Explicit font assets or ordered native family fallbacks.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Font {
    Asset(Arc<Vec<u8>>),
    Family(Arc<[String]>),
}
impl Default for Font {
    fn default() -> Self {
        Self::Family(vec![Self::DEFAULT_FAMILY.to_owned()].into())
    }
}
impl Font {
    pub const DEFAULT_FAMILY: &'static str = "sans-serif";
    pub fn family(preferred: &str, fallback: &str) -> Result<Self, String> {
        let mut families: Vec<String> = Vec::new();
        for list in [preferred, fallback] {
            for name in list.split(',') {
                let name = name.trim();
                let name = if name.starts_with(['\'', '"']) {
                    let quote = name.as_bytes()[0] as char;
                    name.strip_prefix(quote)
                        .and_then(|name| name.strip_suffix(quote))
                        .ok_or("font family has an unclosed quote")?
                } else {
                    name
                };
                validate_family(name)?;
                if !families
                    .iter()
                    .any(|family| family.eq_ignore_ascii_case(name))
                {
                    families.push(name.to_owned());
                }
            }
        }
        let font = Self::Family(families.into());
        font.validate()?;
        Ok(font)
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Self::Family(families) = self {
            if families.is_empty() {
                return Err(
                    "Font requires a family name, such as Font(\"Google Sans\", \"sans-serif\")"
                        .into(),
                );
            }
            for family in families.iter() {
                validate_family(family)?;
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_family(family: &str) -> Result<(), String> {
    if family.trim().is_empty() || family != family.trim() {
        return Err("font family names must be nonempty without surrounding whitespace".into());
    }
    if family.chars().any(|c| c.is_control() || c == ',') {
        return Err("font family names cannot contain control characters or commas; separate fallback names with commas".into());
    }
    Ok(())
}
