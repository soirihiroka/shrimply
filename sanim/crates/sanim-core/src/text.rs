//! HTML white-space and wrapping controls shared by text and code elements.
use crate::{Fraction, Type};
use serde::{Deserialize, Serialize};
macro_rules! choices {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const VALUES: &'static [(&'static str, Self)] = &[$(($value, Self::$variant)),+];
            pub fn name(self) -> &'static str { Self::VALUES.iter().find(|(_, value)| *value == self).expect("registered text value").0 }
            pub fn parse(value: &str) -> Result<Self, String> { Self::VALUES.iter().find(|(name, _)| *name == value).map(|(_, value)| *value).ok_or_else(|| format!("unknown {} {value:?}; use {}", stringify!($name), Self::VALUES.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", "))) }
            pub fn ty() -> Type { Type::union(Self::VALUES.iter().map(|(name, _)| Type::StringLiteral((*name).into()))).expect("registered text choices") }
        }
    }
}
choices! { WhiteSpace { Normal => "normal", Nowrap => "nowrap", Pre => "pre", PreWrap => "pre-wrap", PreLine => "pre-line" } }
choices! { OverflowWrap { Normal => "normal", BreakWord => "break-word", Anywhere => "anywhere" } }
impl WhiteSpace {
    pub fn wraps(self) -> bool {
        !matches!(self, Self::Nowrap | Self::Pre)
    }
    pub fn preserves_spaces(self) -> bool {
        matches!(self, Self::Pre | Self::PreWrap)
    }
    pub fn preserves_newlines(self) -> bool {
        !matches!(self, Self::Normal | Self::Nowrap)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Recipe {
    pub white_space: WhiteSpace,
    pub overflow_wrap: OverflowWrap,
    pub width: Option<Fraction>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub white_space: Option<WhiteSpace>,
    pub overflow_wrap: Option<OverflowWrap>,
    /// Constraint supplied by the layout pass, never authored or retained across revisions.
    #[serde(skip)]
    pub width: Option<Fraction>,
    #[serde(skip)]
    pub resolved: Option<Recipe>,
}
impl Layout {
    pub fn recipe(&self, code: bool) -> Recipe {
        let white_space = self.white_space.unwrap_or(if code {
            WhiteSpace::Pre
        } else {
            WhiteSpace::Normal
        });
        Recipe {
            white_space,
            overflow_wrap: if white_space.wraps() {
                self.overflow_wrap.unwrap_or(OverflowWrap::Normal)
            } else {
                OverflowWrap::Normal
            },
            width: white_space.wraps().then_some(self.width).flatten(),
        }
    }
}
/// CSS collapsible space excludes nonbreaking and other Unicode spaces.
pub fn collapsible(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}')
}
/// Normalize a paragraph while retaining each displayed character's original UTF-8 offset.
/// Rendering and source mapping consume this same transformation.
pub fn normalize(text: &str, mode: WhiteSpace) -> (String, Vec<usize>) {
    let mut source = Vec::new();
    let output = normalize_with(text, mode, |offset| source.push(offset));
    (output, source)
}
pub fn normalize_plain(text: &str, mode: WhiteSpace) -> String {
    normalize_with(text, mode, |_| {})
}
fn normalize_with(text: &str, mode: WhiteSpace, mut source: impl FnMut(usize)) -> String {
    let mut output = String::with_capacity(text.len());
    let mut pending_space = None;
    let mut at_line_start = true;
    let mut chars = text.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        let ch = if ch == '\r' {
            if chars.peek().is_some_and(|(_, next)| *next == '\n') {
                chars.next();
            }
            '\n'
        } else {
            ch
        };
        if !mode.preserves_spaces() && collapsible(ch) {
            if ch == '\n' && mode.preserves_newlines() {
                pending_space = None;
                output.push('\n');
                source(offset);
                at_line_start = true;
            } else if !at_line_start && pending_space.is_none() {
                pending_space = Some(offset);
            }
            continue;
        }
        if let Some(space) = pending_space.take() {
            output.push(' ');
            source(space);
        }
        output.push(ch);
        source(offset);
        at_line_start = ch == '\n';
    }
    output
}
