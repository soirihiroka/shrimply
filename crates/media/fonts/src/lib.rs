//! Shared font descriptors, native fallback, and application font registration.
mod descriptor;
pub use descriptor::Font;

#[cfg(feature = "native")]
pub mod native;

use serde::{Deserialize, Serialize};

/// Font source metadata used by the editor's existing local/Google font picker.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FontFamily {
    Local { name: String },
    GoogleFonts { name: String },
}

impl FontFamily {
    pub fn name(&self) -> &str {
        match self {
            Self::Local { name } | Self::GoogleFonts { name } => name,
        }
    }
}
