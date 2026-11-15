//! Immutable animation values. Construction describes work; lowering alone creates tracks.
mod layout;
use crate::{Easing, Element, Fraction, Justify, Overflow, Value};
pub use layout::{Layout, MAX_PLAN_DEPTH, MAX_PLAN_NODES, Placement};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    MissingParent,
    CyclicParent,
    Invalid(String),
}
impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self{
        Self::MissingParent=>f.write_str("root Animation match_parent requires an external time viewport; provide the clip duration or an explicit Animation duration"),
        Self::CyclicParent=>f.write_str("Animation match_parent cannot size itself from an unconstrained auto parent; give the enclosing Animation an explicit duration or match_parent size"),
        Self::Invalid(message)=>f.write_str(message),
    }
    }
}
impl std::error::Error for LayoutError {}
impl From<String> for LayoutError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}
impl From<&str> for LayoutError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.into())
    }
}
impl From<LayoutError> for String {
    fn from(error: LayoutError) -> Self {
        error.to_string()
    }
}

/// Concrete temporal size of an immutable animation value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Extent {
    Finite(Fraction),
    Infinite,
}
impl std::fmt::Display for Extent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Finite(duration) => write!(f, "{duration}s"),
            Self::Infinite => f.write_str("forever"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeDimension {
    #[default]
    Auto,
    Fixed(Fraction),
    MatchParent,
    Infinite,
}
impl std::fmt::Display for TimeDimension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Fixed(duration) => write!(f, "{duration}s"),
            Self::MatchParent => f.write_str("match_parent"),
            Self::Infinite => f.write_str("forever"),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flow {
    #[default]
    Parallel,
    Sequence,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub duration: TimeDimension,
    pub flow: Flow,
    pub justify: Justify,
    pub gap: Fraction,
    pub padding_start: Fraction,
    pub padding_end: Fraction,
    pub margin_start: Fraction,
    pub margin_end: Fraction,
    pub offset: Fraction,
    pub position_mode: crate::PositionMode,
    pub overflow: Overflow,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            duration: Default::default(),
            flow: Default::default(),
            justify: Default::default(),
            gap: 0.into(),
            padding_start: 0.into(),
            padding_end: 0.into(),
            margin_start: 0.into(),
            margin_end: 0.into(),
            offset: 0.into(),
            position_mode: crate::PositionMode::Relative,
            overflow: Overflow::Hidden,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub node: Arc<Node>,
    pub style: Style,
    pub origin: Option<Origin>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub location: crate::SourceLocation,
    pub binding: Option<Arc<str>>,
    #[serde(default)]
    pub checkpoint: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    /// Canonical element-method registration; no separate motion implementation.
    pub operation: Arc<str>,
    pub arguments: imbl::OrdMap<String, Value>,
    pub duration: Fraction,
    pub easing: Easing,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Hold(Option<Fraction>),
    Element(Arc<Element>),
    Container(imbl::Vector<Arc<Animation>>),
    Effect {
        effect: Effect,
        children: imbl::Vector<Arc<Animation>>,
    },
    Scope {
        elements: imbl::Vector<Arc<Element>>,
        children: imbl::Vector<Arc<Animation>>,
    },
    Cell {
        cell: u64,
        to: Value,
        duration: Fraction,
        easing: Easing,
    },
    Repeat {
        animation: Arc<Animation>,
        count: Option<usize>,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub origin: Option<Origin>,
    pub intrinsic: Extent,
    pub requested: TimeDimension,
    pub resolved: Extent,
    pub start: Fraction,
    pub parent: Option<Fraction>,
    pub clip: Option<(Fraction, Fraction)>,
    pub clock: Option<Arc<crate::math::cycle::Cycle>>,
}

impl Receipt {
    pub fn natural_extent(&self) -> Extent {
        match self.requested {
            TimeDimension::Fixed(value) => Extent::Finite(value),
            TimeDimension::Infinite => Extent::Infinite,
            TimeDimension::Auto | TimeDimension::MatchParent => self.intrinsic,
        }
    }
    /// Actual visible temporal bounds; editor source maps share the runtime's window math.
    pub fn active_interval(
        &self,
        horizon: Fraction,
    ) -> Result<Option<(Fraction, Fraction)>, String> {
        let duration = match self.resolved {
            Extent::Finite(duration) => duration,
            Extent::Infinite => crate::math::arithmetic("-", horizon.max(self.start), self.start)?,
        };
        Ok(crate::math::temporal::Window {
            cycle: None,
            clip: self.clip,
        }
        .interval(self.start, duration)?
        .map(|interval| (interval.start, interval.end)))
    }
}
