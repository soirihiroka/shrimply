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
        let mut families = vec![preferred.to_owned()];
        if !preferred.eq_ignore_ascii_case(fallback) {
            families.push(fallback.to_owned());
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
                if family.trim().is_empty() || family != family.trim() {
                    return Err(
                        "font family names must be nonempty without surrounding whitespace".into(),
                    );
                }
                if family.chars().any(|c| c.is_control() || c == ',') {
                    return Err("font family names cannot contain control characters or commas; pass a fallback as the second Font argument".into());
                }
            }
        }
        Ok(())
    }
}
