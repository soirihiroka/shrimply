//! Platform-independent values, exact arithmetic, scene tracks and verification.
pub mod cache;
pub mod code;
mod collection;
pub use collection::{List, ListEdit, ListEditResult};
pub mod dataframe;
pub mod duration;
pub mod font;
pub use font::Font;
pub mod layout;
pub mod list_views;
pub use duration::{DurationExpr, DurationSymbol, NodeDuration, TimelineEffect};
pub mod math;
pub mod media;
pub mod paint;
pub use paint::{GradientStop, Paint};
pub mod reactive;
pub mod reflection;
pub mod scene;
pub mod scrolling;
pub mod time_map;
pub mod timeline_diff;
pub use time_map::{TimeMap, TimeMapEntry, TimeMapKind};
pub use timeline_diff::{SampleTimes, TimeRange, TimelineDiff};
pub mod type_budget;
pub mod types;
pub mod validation;
pub mod value_budget;
pub use validation::validate_value;
pub mod verify;

pub use fraction::Fraction;
pub use math::{Number, Unit};
pub use scene::*;
pub use types::*;

use serde::{Deserialize, Serialize};
pub use shrimply_resource_pipeline::fingerprint::{digest, digest_bytes};

pub const SEMANTICS_VERSION: &str = "sanim-0.2";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub source: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(code: &str, message: impl Into<String>, source: &str, span: Span) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            source: source.into(),
            span,
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}: {}",
            self.source, self.span.line, self.span.column, self.code, self.message
        )
    }
}
impl std::error::Error for Diagnostic {}

pub mod template;
pub mod text;
pub use template::{Template, TemplatePart};
pub mod sql;
