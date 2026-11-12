//! Timeline effects are separate from value types. Every syntax node has one.
use crate::{Fraction, Span, math};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimelineEffect {
    Fixed(Fraction),
    Dependent,
}
impl Default for TimelineEffect {
    fn default() -> Self {
        Self::Fixed(Fraction::from(0))
    }
}
impl TimelineEffect {
    pub fn then(self, next: Self) -> Result<Self, String> {
        match (self, next) {
            (Self::Fixed(a), Self::Fixed(b)) => math::timeline_end(a, b).map(Self::Fixed),
            _ => Ok(Self::Dependent),
        }
    }
    pub fn is_zero(self) -> bool {
        self == Self::default()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDuration {
    pub span: Span,
    pub effect: TimelineEffect,
}
