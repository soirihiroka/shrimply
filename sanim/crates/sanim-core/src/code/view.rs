//! Immutable CodeView snapshots, reactive inputs and bounded presentation events.
use super::Document;
use crate::{
    Color, Element, Font, Fraction, Geometry, Number, Property, Type, Unit, Value, Vec2, math,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub fill: Option<Color>,
    pub background: Option<Color>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub opacity: Option<Fraction>,
}
impl Style {
    pub fn overlay(&mut self, other: &Self) {
        if other.fill.is_some() {
            self.fill = other.fill.clone();
        }
        if other.background.is_some() {
            self.background = other.background.clone();
        }
        if other.bold.is_some() {
            self.bold = other.bold;
        }
        if other.italic.is_some() {
            self.italic = other.italic;
        }
        if other.underline.is_some() {
            self.underline = other.underline;
        }
        if other.strike.is_some() {
            self.strike = other.strike;
        }
        if other.opacity.is_some() {
            self.opacity = other.opacity;
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}
impl Range {
    pub fn contains(&self, line: usize, column: usize) -> bool {
        (line, column) >= (self.start_line, self.start_column)
            && (line, column) < (self.end_line, self.end_column)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.start_line == 0
            || self.start_column == 0
            || self.end_line == 0
            || self.end_column == 0
            || (self.start_line, self.start_column) > (self.end_line, self.end_column)
        {
            return Err("CodeRange uses 1-based line/Unicode-column positions and an exclusive end; end must follow start".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Override {
    pub range: Range,
    pub style: Style,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub styles: BTreeMap<String, Style>,
    pub selection: Color,
    pub inactive_opacity: Fraction,
    pub line_number: Color,
}
impl Default for Theme {
    fn default() -> Self {
        // GtkSourceView's Adwaita-dark semantic colors. See docs/code-view.md.
        let style = |hex: &str, bold| Style {
            fill: Some(Color::parse(hex).expect("Adwaita source color")),
            bold: Some(bold),
            ..Default::default()
        };
        Self {
            styles: [
                ("text", style("#c0bfbc", false)),
                ("comment", style("#777777", false)),
                ("string", style("#5bc8af", false)),
                ("number", style("#7d8ac7", false)),
                ("boolean", style("#7d8ac7", false)),
                ("constant", style("#7d8ac7", false)),
                ("keyword", style("#ffa348", true)),
                ("type", style("#5bc8af", true)),
                ("function", style("#62a0ea", false)),
                ("operator", style("#c0bfbc", false)),
                ("punctuation", style("#c0bfbc", false)),
                ("property", style("#ff7800", false)),
                ("tag", style("#33b2a4", false)),
            ]
            .into_iter()
            .map(|(name, value)| (name.into(), value))
            .collect(),
            selection: Color::parse("#3584e4").expect("Adwaita selection"),
            inactive_opacity: Fraction::new(3u64, 10u64),
            line_number: Color::parse("#5e5e5e").expect("Adwaita line number"),
        }
    }
}
impl Theme {
    pub fn style(&self, kind: &str) -> Style {
        let mut output = self.styles.get("text").cloned().unwrap_or_default();
        let mut prefix = String::new();
        for part in kind.split('.') {
            if !prefix.is_empty() {
                prefix.push('.');
            }
            prefix.push_str(part);
            if let Some(style) = self.styles.get(&prefix) {
                output.overlay(style);
            }
        }
        output
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Options {
    pub font: Font,
    pub font_size: Fraction,
    pub line_height: Fraction,
    #[serde(default = "automatic_line_height")]
    pub auto_line_height: bool,
    pub tab_size: usize,
    pub line_numbers: bool,
    pub theme: Arc<Theme>,
    pub spans: Vec<Override>,
}
impl Options {
    /// Both stable visual keys and native layout caches use this recipe. The
    /// caller supplies its asset-font identity without copying font bytes.
    pub fn visual_recipe<'a>(&'a self, font: impl Serialize + 'a) -> impl Serialize + 'a {
        let Self {
            font: _,
            font_size,
            line_height,
            auto_line_height,
            tab_size,
            line_numbers,
            theme,
            spans,
        } = self;
        (
            font,
            font_size,
            line_height,
            auto_line_height,
            tab_size,
            line_numbers,
            theme,
            spans,
        )
    }
}
fn automatic_line_height() -> bool {
    true
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Token {
    /// Source revision distinguishes folding/layout changes from actual edits.
    pub revision: [u8; 32],
    pub key: Arc<str>,
    pub style: Arc<Style>,
    pub element: Arc<Element>,
    pub position: Vec2,
    pub line: usize,
    pub column: usize,
}
/// Immutable source and presentation settings at one scheduled operation boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub document: Arc<Document>,
    pub options: Arc<Options>,
    pub folded: imbl::OrdSet<Arc<str>>,
    pub selection: Arc<[Range]>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatchAnimation {
    #[default]
    Morph,
    Fade,
    Typewriter,
}
impl PatchAnimation {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "morph" => Ok(Self::Morph),
            "fade" => Ok(Self::Fade),
            "typewriter" => Ok(Self::Typewriter),
            _ => Err(format!(
                "unknown code patch animation '{name}'; use morph, fade, or typewriter"
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresentationEvent {
    pub start: Fraction,
    pub end: Fraction,
    pub easing: crate::Easing,
    pub animation: PatchAnimation,
    pub from: Arc<Snapshot>,
    pub to: Arc<Snapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Presentation {
    pub from: Arc<Snapshot>,
    pub to: Arc<Snapshot>,
    pub progress: Fraction,
    pub appearance: Fraction,
    pub raw_progress: Fraction,
    pub animation: PatchAnimation,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub document: Arc<Document>,
    pub options: Arc<Options>,
    pub folded: imbl::OrdSet<Arc<str>>,
    pub selection: Vec<Range>,
    pub end: Fraction,
    /// Persistent time index; duplicate instant operations at one time keep their final state.
    pub events: imbl::OrdMap<Fraction, PresentationEvent>,
}

impl State {
    /// Conservatively count retained event/snapshot roots, excluding shared
    /// document/options payloads charged by the host's ordinary value budget.
    pub fn metadata_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(Self::event_metadata_bytes().saturating_mul(self.events.len()))
    }
    pub fn event_metadata_bytes() -> usize {
        // imbl 7's largest ordered-map node has sixteen slots (small-chunks
        // uses six). Charge a complete node per event, including child links;
        // this also overcounts slack and persistent checkpoint path copies.
        const MAP_NODE_SLOTS: usize = 16;
        std::mem::size_of::<PresentationEvent>()
            .saturating_add(2 * std::mem::size_of::<Snapshot>())
            .saturating_add(
                MAP_NODE_SLOTS
                    * (std::mem::size_of::<(Fraction, PresentationEvent)>()
                        + std::mem::size_of::<usize>() * 2),
            )
    }
    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::new(Snapshot {
            document: self.document.clone(),
            options: self.options.clone(),
            folded: self.folded.clone(),
            selection: self.selection.as_slice().into(),
        })
    }

    /// O(log n) predecessor lookup, independent of seek direction or previous samples.
    pub fn presentation(&self, time: Fraction) -> Result<Option<Presentation>, String> {
        let Some((_, event)) = self.events.range(..=time).next_back() else {
            return Ok(None);
        };
        if event.end < event.start {
            return Err("CodeView presentation ends before it starts".into());
        }
        let raw = if time >= event.end {
            1.into()
        } else {
            crate::math::progress(time, event.start, event.end)?
        };
        let (progress, appearance) = crate::math::eased_progress(event.easing, raw)?;
        Ok(Some(Presentation {
            from: event.from.clone(),
            to: event.to.clone(),
            progress,
            appearance,
            raw_progress: raw,
            animation: event.animation,
        }))
    }
}

pub const INPUTS: &[(&str, Property)] = &[
    ("code", Property::Code),
    ("lang", Property::CodeLanguage),
    ("theme", Property::CodeTheme),
    ("spans", Property::CodeSpans),
    ("font", Property::Font),
    ("font_size", Property::FontSize),
    ("line_height", Property::CodeLineHeight),
    ("tab_size", Property::CodeTabSize),
    ("line_numbers", Property::CodeLineNumbers),
];
pub fn input_type(property: Property) -> Option<Type> {
    Some(match property {
        Property::Code | Property::CodeLanguage => Type::String,
        Property::CodeTheme => super::values::theme_type(),
        Property::CodeSpans => Type::List(Box::new(super::values::span_type())),
        Property::Font => Type::Font,
        Property::FontSize | Property::CodeLineHeight => Type::Numeric(Unit::Length),
        Property::CodeTabSize => Type::Numeric(Unit::Number),
        Property::CodeLineNumbers => Type::Bool,
        _ => return None,
    })
}
pub fn property(geometry: &Geometry, property: Property) -> Result<Value, String> {
    let Geometry::Code {
        document, options, ..
    } = geometry
    else {
        return Err("code property requires CodeView".into());
    };
    Ok(match property {
        Property::Code => Value::String(document.text.clone()),
        Property::CodeLanguage => {
            Value::String(document.language.name().to_ascii_lowercase().into())
        }
        Property::CodeTheme => super::values::theme_value(&options.theme),
        Property::CodeSpans => super::values::spans_value(&options.spans),
        Property::Font => Value::Font(options.font.clone()),
        Property::FontSize => Value::Number(Number {
            value: options.font_size,
            unit: Unit::Length,
        }),
        Property::CodeLineHeight => Value::Number(Number {
            value: options.line_height,
            unit: Unit::Length,
        }),
        Property::CodeTabSize => Value::Number(Number::scalar(options.tab_size as i64)),
        Property::CodeLineNumbers => Value::Bool(options.line_numbers),
        _ => return Err("unknown CodeView input property".into()),
    })
}
enum Input {
    Text(Arc<str>),
    Language(super::Language),
    Font(Font),
    Size(Fraction),
    LineHeight(Fraction),
    TabSize(usize),
    LineNumbers(bool),
    Theme(Arc<Theme>),
    Spans(Vec<Override>),
}
impl Input {
    fn read(property: Property, value: Value) -> Result<Self, String> {
        Ok(match property {
            Property::Code => {
                let Value::String(text) = value else {
                    return Err("CodeView code requires String".into());
                };
                Self::Text(text)
            }
            Property::CodeLanguage => Self::Language(super::Language::parse(value.string()?)?),
            Property::Font => {
                let Value::Font(font) = value else {
                    return Err("CodeView font requires Font".into());
                };
                font.validate()?;
                Self::Font(font)
            }
            Property::FontSize | Property::CodeLineHeight => {
                let value = value.number()?;
                if value.unit != Unit::Length
                    || !math::finite(&value.value)
                    || value.value <= 0.into()
                {
                    return Err(
                        "CodeView font_size/line_height requires finite positive Length".into(),
                    );
                }
                if property == Property::FontSize {
                    Self::Size(value.value)
                } else {
                    Self::LineHeight(value.value)
                }
            }
            Property::CodeTabSize => {
                let value = value.number()?.usize()?;
                if !(1..=32).contains(&value) {
                    return Err("CodeView tab_size must be an integer from 1 to 32".into());
                }
                Self::TabSize(value)
            }
            Property::CodeLineNumbers => Self::LineNumbers(value.boolean()?),
            Property::CodeTheme => Self::Theme(Arc::new(super::values::theme(&value)?)),
            Property::CodeSpans => Self::Spans(super::values::spans(&value)?),
            _ => return Err("unknown CodeView input property".into()),
        })
    }
    fn apply(
        &self,
        document: &mut Arc<Document>,
        options: &mut Arc<Options>,
    ) -> Result<bool, String> {
        let changed = match self {
            Self::Text(value) => document.text != *value,
            Self::Language(value) => {
                document.language
                    != if *value == super::Language::Auto {
                        super::Language::detect(&document.path)
                    } else {
                        *value
                    }
            }
            Self::Font(value) => &options.font != value,
            Self::Size(value) => options.font_size != *value,
            Self::LineHeight(value) => options.line_height != *value,
            Self::TabSize(value) => options.tab_size != *value,
            Self::LineNumbers(value) => options.line_numbers != *value,
            Self::Theme(value) => options.theme != *value,
            Self::Spans(value) => options.spans != *value,
        };
        if !changed {
            return Ok(false);
        }
        match self {
            Self::Text(value) => {
                *document = Arc::new(Document::source_shared(
                    &document.path,
                    value.clone(),
                    document.language,
                )?)
            }
            Self::Language(value) => {
                let doc = Arc::make_mut(document);
                doc.language = if *value == super::Language::Auto {
                    super::Language::detect(&doc.path)
                } else {
                    *value
                };
                doc.syntax_parsed = false;
                doc.syntax = super::SyntaxArtifact::default();
                doc.scopes = Arc::from([]);
                doc.highlights = Arc::from([]);
                doc.diagnostics = Arc::from([]);
            }
            Self::Font(value) => Arc::make_mut(options).font = value.clone(),
            Self::Size(value) => {
                let options = Arc::make_mut(options);
                options.font_size = *value;
                if options.auto_line_height {
                    options.line_height = *value * Fraction::new(3u64, 2u64);
                }
            }
            Self::LineHeight(value) => {
                let options = Arc::make_mut(options);
                options.line_height = *value;
                options.auto_line_height = false;
            }
            Self::TabSize(value) => Arc::make_mut(options).tab_size = *value,
            Self::LineNumbers(value) => Arc::make_mut(options).line_numbers = *value,
            Self::Theme(value) => Arc::make_mut(options).theme = value.clone(),
            Self::Spans(value) => Arc::make_mut(options).spans = value.clone(),
        };
        Ok(true)
    }
}
pub fn set(geometry: &mut Geometry, property: Property, value: Value) -> Result<(), String> {
    let input = Input::read(property, value)?;
    let Geometry::Code {
        document,
        options,
        pending,
        source_changed,
        presentation,
        ..
    } = geometry
    else {
        return Err("code input requires CodeView".into());
    };
    let changed = input.apply(document, options)?;
    *pending |= changed || presentation.is_some();
    if matches!(input, Input::Text(_) | Input::Language(_)) {
        *source_changed |= changed;
    }
    if let Some(presentation) = presentation {
        for endpoint in [&mut presentation.from, &mut presentation.to] {
            let endpoint = Arc::make_mut(endpoint);
            if input.apply(&mut endpoint.document, &mut endpoint.options)?
                && matches!(input, Input::Text(_) | Input::Language(_))
            {
                endpoint.selection = Arc::from([]);
            }
        }
    }
    Ok(())
}
