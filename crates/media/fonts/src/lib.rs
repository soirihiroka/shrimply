//! Shared font descriptors, native fallback, and application font registration.
mod changes;
mod descriptor;
pub use changes::{FontChange, FontEpoch, subscribe};
pub use descriptor::Font;

/// Native outline commands shared by Sanim and the editor's 2D/3D text paths.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TextSegment {
    Move([f64; 2]),
    Line([f64; 2]),
    Curve([f64; 2], [f64; 2], [f64; 2]),
    Close,
}

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

/// Immutable native text geometry, independent of the shaping backend.
pub struct ShapedText {
    /// Bounds and outline use the first-baseline origin (before any alignment).
    pub bounds: [f64; 4],
    pub outline: std::sync::Arc<Vec<TextSegment>>,
    pub advance: f64,
    /// Translation from Pango's top-left coordinate system to these outlines.
    pub origin: [f64; 2],
    pub missing_glyphs: i32,
    pub color_glyphs: bool,
    #[cfg(feature = "native")]
    pub(crate) asset: Option<std::sync::Weak<Vec<u8>>>,
}
