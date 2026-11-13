use crate::{Color, Value, Vec2, math};
use fraction::Fraction;
use imbl::{OrdMap, OrdSet, Vector};
use num_traits::Zero;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Geometry {
    Media {
        asset: Arc<crate::media::Asset>,
        width: Fraction,
        height: Fraction,
    },
    Circle {
        radius: Fraction,
    },
    Rectangle {
        width: Fraction,
        height: Fraction,
        #[serde(default)]
        radius: CornerRadius,
        #[serde(default)]
        children: Vector<Element>,
    },
    Line {
        end: Vec2,
        width: Fraction,
    },
    Polygon {
        points: Arc<Vec<Vec2>>,
    },
    /// A single open, stroked path; dense plots do not create one element per segment.
    Polyline {
        points: Arc<Vec<Vec2>>,
        width: Fraction,
    },
    Text {
        text: String,
        font: crate::Font,
        size: Fraction,
        outline: Arc<Vec<TextSegment>>,
        bounds: crate::verify::Bounds,
        /// A sampled text/font/size changed and requires native shaping before layout.
        pending: bool,
    },
    Box {
        #[serde(default)]
        children: Vector<Element>,
        #[serde(default)]
        radius: CornerRadius,
    },
    Code {
        document: Arc<crate::code::Document>,
        options: Arc<crate::code::view::Options>,
        children: Vector<Element>,
        #[serde(default)]
        pending: bool,
        #[serde(default)]
        source_changed: bool,
        #[serde(default)]
        presentation: Option<crate::code::view::Presentation>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CornerRadius {
    Fixed(Fraction),
    Pill,
}
impl Default for CornerRadius {
    fn default() -> Self {
        Self::Fixed(Fraction::from(0))
    }
}
impl From<Fraction> for CornerRadius {
    fn from(radius: Fraction) -> Self {
        Self::Fixed(radius)
    }
}
impl CornerRadius {
    pub fn resolve(&self, width: Fraction, height: Fraction) -> Fraction {
        match self {
            Self::Fixed(radius) => *radius,
            Self::Pill => width.min(height) / Fraction::from(2),
        }
    }
}
impl Geometry {
    pub fn children(&self) -> Option<&Vector<Element>> {
        match self {
            Self::Box { children, .. }
            | Self::Rectangle { children, .. }
            | Self::Code { children, .. } => Some(children),
            _ => None,
        }
    }
    pub fn children_mut(&mut self) -> Option<&mut Vector<Element>> {
        match self {
            Self::Box { children, .. }
            | Self::Rectangle { children, .. }
            | Self::Code { children, .. } => Some(children),
            _ => None,
        }
    }
}

pub use shrimply_fonts::TextSegment;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: String,
    pub geometry: Arc<Geometry>,
    pub position: Vec2,
    pub rotation: Fraction,
    pub scale: Fraction,
    pub opacity: Fraction,
    pub fill: crate::Paint,
    #[serde(default)]
    pub stroke: Option<crate::paint::Stroke>,
    #[serde(default)]
    pub reveal: Reveal,
    #[serde(default)]
    pub horizontal: HorizontalTransform,
    pub layout: Layout,
}

/// Horizontal drawing transform; layout dimensions remain stable while a range expands.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HorizontalTransform {
    pub scale: Fraction,
    pub origin: crate::Number,
    pub offset: Fraction,
}
impl Default for HorizontalTransform {
    fn default() -> Self {
        Self {
            scale: Fraction::from(1),
            origin: crate::Number {
                value: Fraction::from(0),
                unit: crate::Unit::Length,
            },
            offset: Fraction::from(0),
        }
    }
}

/// Vector reveal state; independent of opacity, geometry and layout.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reveal {
    pub progress: Fraction,
    pub kind: RevealKind,
}
impl Default for Reveal {
    fn default() -> Self {
        Self {
            progress: Fraction::from(1),
            kind: RevealKind::Create,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevealKind {
    Create,
    Write,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub x_align: Align,
    pub y_align: Align,
    /// Fixed native-composition translation added after the relative position.
    #[serde(default)]
    pub position_offset: [f64; 2],
    pub direction: Option<Direction>,
    pub gap: Fraction,
    pub width: Option<crate::Number>,
    pub height: Option<crate::Number>,
    pub padding: Fraction,
    pub justify: Justify,
    pub align: Align,
    pub wrap: bool,
    #[serde(default)]
    pub flex: bool,
    #[serde(default)]
    pub flex_grow: Fraction,
    #[serde(default)]
    pub flex_shrink: Fraction,
    #[serde(default)]
    pub flex_basis: Option<crate::Number>,
    #[serde(default)]
    pub overflow_x: Overflow,
    #[serde(default)]
    pub overflow_y: Overflow,
    /// Content offset inside the element's fixed viewport. Percentages use its overflow range.
    #[serde(default)]
    pub scroll: Option<Vec2>,
    #[serde(skip)]
    pub scroll_range: [f64; 2],
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    Auto,
    Scroll,
}
impl Overflow {
    pub const VALUES: &'static [(&'static str, Self)] = &[
        ("visible", Self::Visible),
        ("hidden", Self::Hidden),
        ("auto", Self::Auto),
        ("scroll", Self::Scroll),
    ];
    pub fn parse(value: &str) -> Result<Self, String> {
        Self::VALUES
            .iter()
            .find(|(name, _)| *name == value)
            .map(|(_, mode)| *mode)
            .ok_or_else(|| {
                format!(
                    "unknown overflow {value:?}; use {}",
                    Self::VALUES
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
    pub fn clips(self) -> bool {
        self != Self::Visible
    }
}
impl Layout {
    pub fn clips(&self) -> bool {
        self.overflow_x.clips() || self.overflow_y.clips()
    }
    pub fn configure_overflow(
        &mut self,
        overflow_x: Overflow,
        overflow_y: Overflow,
        scroll: Option<Vec2>,
    ) -> Result<(), String> {
        if !overflow_x.clips() && !overflow_y.clips() && scroll.is_some() {
            return Err(
                "scroll requires overflow=\"auto\", \"scroll\" or \"hidden\" on its container"
                    .into(),
            );
        }
        self.overflow_x = overflow_x;
        self.overflow_y = overflow_y;
        self.scroll = self
            .clips()
            .then(|| scroll.unwrap_or_else(|| Vec2::zero(crate::Unit::Length)));
        Ok(())
    }
    pub fn dimension_type() -> crate::Type {
        crate::Type::union([
            crate::Type::Numeric(crate::Unit::Length),
            crate::Type::Numeric(crate::Unit::Percent),
        ])
        .expect("layout dimension units")
    }
}
macro_rules! native_enum {
    ($name:ident {$($(#[$attribute:meta])* $variant:ident),* $(,)?})=>{
        #[derive(Clone,Copy,Debug,Default,PartialEq,Eq,PartialOrd,Ord,Serialize,Deserialize)]
        pub enum $name {$($(#[$attribute])* $variant),*}
        impl crate::NativeEnum for $name {
            const NAME:&'static str=stringify!($name);
            const VARIANTS:&'static [(&'static str,Self)]=&[$((stringify!($variant),Self::$variant)),*];
        }
    };
}
native_enum! {Align {Start,#[default] Center,End}}
native_enum! {Justify {#[default] Start,Center,End,SpaceBetween,SpaceAround,SpaceEvenly}}
impl Align {
    pub fn fraction(self) -> Fraction {
        match self {
            Self::Start => Fraction::from(0),
            Self::Center => Fraction::new(1u64, 2u64),
            Self::End => Fraction::from(1),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Direction {
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Property {
    Position,
    Scroll,
    Rotation,
    Scale,
    StretchX,
    Opacity,
    Fill,
    Reveal,
    Radius,
    Width,
    Height,
    Text,
    Font,
    FontSize,
    XAlign,
    YAlign,
    Code,
    CodeLanguage,
    CodeTheme,
    CodeSpans,
    CodeLineHeight,
    CodeTabSize,
    CodeLineNumbers,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    pub element: Element,
    pub property: Property,
    pub to: Value,
    pub from: Option<Value>,
    #[serde(default)]
    pub reveal: Option<RevealKind>,
    #[serde(default)]
    pub horizontal: Option<HorizontalTransform>,
    #[serde(default)]
    pub easing: Option<Easing>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Easing {
    Linear,
    #[default]
    Spring,
    SpringBouncy,
    CubicIn,
    CubicOut,
    Smooth,
    Smootherstep,
    Overshoot,
}
impl Easing {
    pub fn parse(name: &str) -> Result<Self, String> {
        const OPTIONS: &[(&str, Easing)] = &[
            ("spring", Easing::Spring),
            ("spring_bouncy", Easing::SpringBouncy),
            ("linear", Easing::Linear),
            ("cubic_in", Easing::CubicIn),
            ("cubic_out", Easing::CubicOut),
            ("smooth", Easing::Smooth),
            ("smootherstep", Easing::Smootherstep),
            ("overshoot", Easing::Overshoot),
        ];
        OPTIONS
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
            .ok_or_else(|| {
                format!(
                    "unknown easing '{name}'; use {}",
                    OPTIONS
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub start: Fraction,
    pub end: Fraction,
    pub from: Value,
    pub to: Value,
    pub easing: Easing,
    /// Constrained visual properties must not overshoot their valid endpoints.
    #[serde(default)]
    pub bounded: bool,
    #[serde(default)]
    pub reveal: Option<RevealKind>,
    #[serde(default)]
    pub horizontal: Option<HorizontalTransform>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub birth: Fraction,
    pub element: Element,
}

/// Host-supplied output settings. Scripts cannot modify these values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub rate: Fraction,
    pub dpi: u32,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            rate: Fraction::from(30),
            dpi: 96,
        }
    }
}
impl Viewport {
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.dpi == 0
            || !math::finite(&self.rate)
            || self.rate <= Fraction::zero()
        {
            Err(
                "external viewport requires positive width, height, DPI and finite frame rate"
                    .into(),
            )
        } else {
            Ok(())
        }
    }
    pub fn logical_size(&self) -> (Fraction, Fraction) {
        (
            Fraction::new(u64::from(self.width) * 72, u64::from(self.dpi)),
            Fraction::new(u64::from(self.height) * 72, u64::from(self.dpi)),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub dpi: u32,
    pub width: u32,
    pub height: u32,
    pub rate: Fraction,
    pub duration: Fraction,
    pub background: Color,
    pub layers: Vector<Layer>,
    /// Each property has its own sorted track index; sampling uses binary search.
    pub tracks: OrdMap<String, OrdMap<Property, Vector<Track>>>,
    #[serde(default)]
    pub reactive: crate::reactive::ReactiveGraph,
    #[serde(default)]
    pub list_views: crate::list_views::ListViews,
    /// COW preparation metadata. The actual visual state is in list_views/tracks.
    #[serde(default)]
    pub code_views: OrdMap<String, Arc<crate::code::view::State>>,
    pub(crate) element_index: OrdMap<String, Vector<usize>>,
    #[serde(default)]
    pub(crate) flow_controlled: OrdSet<String>,
}

impl Default for Animation {
    fn default() -> Self {
        Self::for_viewport(&Viewport::default())
    }
}
impl Animation {
    pub fn for_viewport(viewport: &Viewport) -> Self {
        Self {
            dpi: viewport.dpi,
            width: viewport.width,
            height: viewport.height,
            rate: viewport.rate,
            duration: Fraction::zero(),
            background: Color::parse("window_bg_dark").expect("shared Adwaita background color"),
            layers: Vector::new(),
            tracks: OrdMap::new(),
            reactive: crate::reactive::ReactiveGraph::default(),
            list_views: crate::list_views::ListViews::default(),
            code_views: OrdMap::new(),
            element_index: OrdMap::new(),
            flow_controlled: OrdSet::new(),
        }
    }
}

impl Element {
    pub fn property(&self, property: Property) -> Result<Value, String> {
        if matches!(self.geometry.as_ref(), Geometry::Code { .. })
            && crate::code::view::input_type(property).is_some()
        {
            return crate::code::view::property(self.geometry.as_ref(), property);
        }
        if property == Property::Radius
            && matches!(
                self.geometry.as_ref(),
                Geometry::Rectangle {
                    radius: CornerRadius::Pill,
                    ..
                } | Geometry::Box {
                    radius: CornerRadius::Pill,
                    ..
                }
            )
            && self
                .layout
                .width
                .into_iter()
                .chain(self.layout.height)
                .any(|number| number.unit == crate::Unit::Percent)
        {
            return Err("a relative pill's radius depends on its parent and is resolved during layout; provide an explicit fixed radius when reading .radius in source".into());
        }
        if let Some(number) = match property {
            Property::Width => self.layout.width,
            Property::Height => self.layout.height,
            _ => None,
        } {
            return Ok(Value::Number(number));
        }
        Ok(match property {
            Property::Position => Value::Vec2(self.position),
            Property::Scroll => Value::Vec2(
                self.layout
                    .scroll
                    .unwrap_or(Vec2::zero(crate::Unit::Length)),
            ),
            Property::Rotation => Value::Number(crate::Number {
                value: self.rotation,
                unit: crate::Unit::Angle,
            }),
            Property::Scale => Value::Number(crate::Number {
                value: self.scale,
                unit: crate::Unit::Number,
            }),
            Property::StretchX => Value::Number(crate::Number {
                value: self.horizontal.scale,
                unit: crate::Unit::Number,
            }),
            Property::Opacity => Value::Number(crate::Number {
                value: self.opacity,
                unit: crate::Unit::Number,
            }),
            Property::Reveal => Value::Number(crate::Number {
                value: self.reveal.progress,
                unit: crate::Unit::Number,
            }),
            Property::Fill => self.fill.value(),
            Property::XAlign | Property::YAlign => {
                use crate::NativeEnum;
                let alignment = if property == Property::XAlign {
                    self.layout.x_align
                } else {
                    self.layout.y_align
                };
                Value::Variant {
                    ty: Arc::new(Align::ty()),
                    tag: Align::VARIANTS
                        .iter()
                        .find(|(_, value)| *value == alignment)
                        .expect("registered alignment")
                        .0
                        .into(),
                    fields: vec![].into(),
                }
            }
            Property::Text => match self.geometry.as_ref() {
                Geometry::Text { text, .. } => Value::String(text.clone().into()),
                _ => return Err(format!("{} does not have text content", self.id)),
            },
            Property::Font => match self.geometry.as_ref() {
                Geometry::Text { font, .. } => Value::Font(font.clone()),
                _ => return Err(format!("{} does not have a font", self.id)),
            },
            Property::FontSize => match self.geometry.as_ref() {
                Geometry::Text { size, .. } => Value::Number(crate::Number {
                    value: *size,
                    unit: crate::Unit::Length,
                }),
                _ => return Err(format!("{} does not have a font size", self.id)),
            },
            Property::Radius | Property::Width | Property::Height => {
                let value = match (property, self.geometry.as_ref()) {
                    (Property::Radius, Geometry::Circle { radius }) => *radius,
                    (Property::Radius, Geometry::Rectangle {width,height,radius,..})=>radius.resolve(*width,*height),
                    (Property::Radius, Geometry::Box {radius,..})=>radius.resolve(self.layout.width.ok_or("Box radius needs resolved width")?.value,self.layout.height.ok_or("Box radius needs resolved height")?.value),
                    (Property::Width, Geometry::Rectangle { width, .. } | Geometry::Media { width, .. }) => *width,
                    (Property::Width, Geometry::Line { width, .. } | Geometry::Polyline { width, .. }) => *width,
                    (Property::Height, Geometry::Rectangle { height, .. } | Geometry::Media { height, .. }) => *height,
                    (Property::Width, Geometry::Box { .. } | Geometry::Code { .. }) => self.layout.width.map(|number|number.value).ok_or_else(|| format!("{}.width is automatic; provide an explicit width before reading it",self.id))?,
                    (Property::Height, Geometry::Box { .. } | Geometry::Code { .. }) => self.layout.height.map(|number|number.value).ok_or_else(|| format!("{}.height is automatic; provide an explicit height before reading it",self.id))?,
                    _ => {
                        return Err(format!(
                            "{} does not have a {property:?} geometry property",
                            self.id
                        ));
                    }
                };
                Value::Number(crate::Number {
                    value,
                    unit: crate::Unit::Length,
                })
            }
            Property::Code
            | Property::CodeLanguage
            | Property::CodeTheme
            | Property::CodeSpans
            | Property::CodeLineHeight
            | Property::CodeTabSize
            | Property::CodeLineNumbers => return Err(format!("{} is not a CodeView", self.id)),
        })
    }
    pub fn set(&mut self, property: Property, value: Value) -> Result<(), String> {
        if matches!(self.geometry.as_ref(), Geometry::Code { .. })
            && crate::code::view::input_type(property).is_some()
        {
            return crate::code::view::set(Arc::make_mut(&mut self.geometry), property, value);
        }
        match property {
            Property::XAlign | Property::YAlign => {
                use crate::NativeEnum;
                let Value::Variant { ty, tag, fields } = value else {
                    return Err("alignment requires Align.Start, Align.Center or Align.End".into());
                };
                if *ty != Align::ty() || !fields.is_empty() {
                    return Err("alignment requires an Align enum value".into());
                }
                let alignment = Align::from_name(&tag)?;
                if property == Property::XAlign {
                    self.layout.x_align = alignment;
                } else {
                    self.layout.y_align = alignment;
                }
            }
            Property::Code
            | Property::CodeLanguage
            | Property::CodeTheme
            | Property::CodeSpans
            | Property::CodeLineHeight
            | Property::CodeTabSize
            | Property::CodeLineNumbers => return Err(format!("{} is not a CodeView", self.id)),
            Property::Text => {
                let text = value.string()?;
                if text.len() > math::format::MAX_OUTPUT_BYTES {
                    return Err("text content exceeds its byte budget".into());
                }
                if matches!(self.geometry.as_ref(), Geometry::Text { text: current, .. } if current == text)
                {
                    return Ok(());
                }
                let Geometry::Text {
                    text: current,
                    pending,
                    ..
                } = Arc::make_mut(&mut self.geometry)
                else {
                    return Err(format!("{} does not have text content", self.id));
                };
                if current != text {
                    *current = text.to_owned();
                    *pending = true;
                }
            }
            Property::Font => {
                let Value::Font(font) = value else {
                    return Err("font requires a Font value".into());
                };
                font.validate()?;
                if matches!(self.geometry.as_ref(), Geometry::Text { font: current, .. } if current == &font)
                {
                    return Ok(());
                }
                let Geometry::Text {
                    font: current,
                    pending,
                    ..
                } = Arc::make_mut(&mut self.geometry)
                else {
                    return Err(format!("{} does not have a font", self.id));
                };
                if current != &font {
                    *current = font;
                    *pending = true;
                }
            }
            Property::FontSize => {
                let number = value.number()?;
                if number.unit != crate::Unit::Length
                    || !math::finite(&number.value)
                    || number.value <= Fraction::from(0)
                {
                    return Err("font size requires a finite positive Length".into());
                }
                if matches!(self.geometry.as_ref(), Geometry::Text { size, .. } if *size == number.value)
                {
                    return Ok(());
                }
                let Geometry::Text { size, pending, .. } = Arc::make_mut(&mut self.geometry) else {
                    return Err(format!("{} does not have a font size", self.id));
                };
                if *size != number.value {
                    *size = number.value;
                    *pending = true;
                }
            }
            Property::Scroll => {
                let offset = value.vec2()?;
                crate::scrolling::validate(self, offset)?;
                self.layout.scroll = Some(offset);
            }
            Property::Position => self.position = value.vec2()?,
            Property::Rotation => {
                let number = value.number()?;
                if number.unit != crate::Unit::Angle || !math::finite(&number.value) {
                    return Err("rotation requires a finite Angle".into());
                }
                self.rotation = number.value;
            }
            Property::Scale => {
                let number = value.number()?;
                if number.unit != crate::Unit::Number
                    || !math::finite(&number.value)
                    || number.value < Fraction::from(0)
                {
                    return Err("scale requires a finite nonnegative Number".into());
                }
                self.scale = number.value;
            }
            Property::StretchX => {
                let n = value.number()?;
                if n.unit != crate::Unit::Number
                    || !math::finite(&n.value)
                    || n.value < Fraction::from(0)
                {
                    return Err("horizontal scale must be a finite nonnegative Number".into());
                }
                self.horizontal.scale = n.value;
            }
            Property::Opacity => {
                let number = value.number()?;
                if number.unit != crate::Unit::Number
                    || !math::finite(&number.value)
                    || number.value < Fraction::from(0)
                    || number.value > Fraction::from(1)
                {
                    return Err("opacity requires a finite Number in [0,1]".into());
                }
                self.opacity = number.value;
            }
            Property::Reveal => {
                let number = value.number()?;
                if number.unit != crate::Unit::Number
                    || !math::finite(&number.value)
                    || number.value < Fraction::from(0)
                    || number.value > Fraction::from(1)
                {
                    return Err("reveal progress must be a finite Number in [0,1]".into());
                }
                self.reveal.progress = number.value;
            }
            Property::Fill => {
                self.fill = crate::Paint::from_value(&value)?;
            }
            Property::Radius | Property::Width | Property::Height => {
                let number = value.number()?;
                let positive = !matches!(self.geometry.as_ref(), Geometry::Box { .. })
                    && (property != Property::Radius
                        || matches!(self.geometry.as_ref(), Geometry::Circle { .. }));
                let stroke_width = property == Property::Width
                    && matches!(
                        self.geometry.as_ref(),
                        Geometry::Line { .. } | Geometry::Polyline { .. }
                    );
                let relative =
                    matches!(property, Property::Width | Property::Height) && !stroke_width;
                if !(number.unit == crate::Unit::Length
                    || relative && number.unit == crate::Unit::Percent)
                    || !math::finite(&number.value)
                    || number.value < Fraction::from(0)
                    || positive && number.value == Fraction::from(0)
                {
                    let requirement = if positive { "positive" } else { "nonnegative" };
                    return Err(format!(
                        "{property:?} requires a finite {requirement} Length"
                    ));
                }
                if property == Property::Radius
                    && !matches!(
                        self.geometry.as_ref(),
                        Geometry::Circle { .. } | Geometry::Rectangle { .. } | Geometry::Box { .. }
                    )
                {
                    return Err(format!(
                        "{} does not have a Radius geometry property",
                        self.id
                    ));
                }
                if relative {
                    match property {
                        Property::Width => self.layout.width = Some(number),
                        Property::Height => self.layout.height = Some(number),
                        _ => unreachable!("layout dimension"),
                    }
                    if !matches!(
                        self.geometry.as_ref(),
                        Geometry::Rectangle { .. } | Geometry::Media { .. }
                    ) {
                        return Ok(());
                    }
                }
                match (property, Arc::make_mut(&mut self.geometry)) {
                    (Property::Radius, Geometry::Circle { radius }) => *radius = number.value,
                    (Property::Width, Geometry::Rectangle { width, .. }) => *width = number.value,
                    (
                        Property::Width,
                        Geometry::Media {
                            asset,
                            width,
                            height,
                        },
                    ) => {
                        *width = number.value;
                        if self.layout.height.is_none() {
                            *height = math::arithmetic(
                                "*",
                                *width,
                                Fraction::new(u64::from(asset.height), u64::from(asset.width)),
                            )?;
                        }
                    }
                    (
                        Property::Radius,
                        Geometry::Rectangle { radius, .. } | Geometry::Box { radius, .. },
                    ) => *radius = CornerRadius::Fixed(number.value),
                    (
                        Property::Width,
                        Geometry::Line { width, .. } | Geometry::Polyline { width, .. },
                    ) => *width = number.value,
                    (Property::Height, Geometry::Rectangle { height, .. }) => {
                        *height = number.value
                    }
                    (
                        Property::Height,
                        Geometry::Media {
                            asset,
                            width,
                            height,
                        },
                    ) => {
                        *height = number.value;
                        if self.layout.width.is_none() {
                            *width = math::arithmetic(
                                "*",
                                *height,
                                Fraction::new(u64::from(asset.width), u64::from(asset.height)),
                            )?;
                        }
                    }
                    _ => unreachable!("validated geometry property"),
                }
            }
        }
        Ok(())
    }
    pub fn find(&self, id: &str) -> Option<&Element> {
        if self.id == id {
            return Some(self);
        }
        if let Some(children) = self.geometry.children() {
            children.iter().find_map(|v| v.find(id))
        } else {
            None
        }
    }
    pub fn ids(&self, ids: &mut Vec<String>) {
        ids.push(self.id.clone());
        if let Some(children) = self.geometry.children() {
            for child in children {
                child.ids(ids);
            }
        }
    }
}

impl Animation {
    pub fn logical_size(&self) -> (Fraction, Fraction) {
        Viewport {
            width: self.width,
            height: self.height,
            rate: self.rate,
            dpi: self.dpi,
        }
        .logical_size()
    }
    pub fn sample(&self, time: Fraction) -> Result<Vec<Element>, String> {
        self.sample_with(time, |_, _| Ok(()))
    }
    /// Sample the canonical state once, resolving native descriptors in the same traversal.
    pub fn sample_with(
        &self,
        time: Fraction,
        mut native: impl FnMut(&mut Element, Fraction) -> Result<(), String>,
    ) -> Result<Vec<Element>, String> {
        if !math::finite(&time) || time < Fraction::zero() {
            return Err("sample time must be finite and nonnegative".into());
        }
        let mut sampler = self.reactive.sampler(time)?;
        self.layers
            .iter()
            .filter(|v| v.birth <= time)
            .map(|v| {
                let mut out = v.element.clone();
                self.sample_in_place(&mut out, time, &mut sampler, &mut native)?;
                Ok(out)
            })
            .collect()
    }
    pub fn sample_element(&self, element: &Element, time: Fraction) -> Result<Element, String> {
        self.sample_element_with(element, time, |_, _| Ok(()))
    }
    pub fn sample_element_with(
        &self,
        element: &Element,
        time: Fraction,
        mut native: impl FnMut(&mut Element, Fraction) -> Result<(), String>,
    ) -> Result<Element, String> {
        let mut out = element.clone();
        let mut sampler = self.reactive.sampler(time)?;
        self.sample_in_place(&mut out, time, &mut sampler, &mut native)?;
        Ok(out)
    }
    pub fn sample_property(
        &self,
        element: &Element,
        property: Property,
        time: Fraction,
    ) -> Result<Value, String> {
        if let Some(node) = self.reactive.binding(&element.id, property) {
            if property == Property::Position && self.flow_controlled.contains(&element.id) {
                return Err(format!(
                    "flow layout controls '{}.position'; animate a containing Group or use a free Group instead",
                    element.id
                ));
            }
            if self
                .tracks
                .get(&element.id)
                .is_some_and(|tracks| tracks.contains_key(&property))
            {
                return Err(format!(
                    "retained property {}.{property:?} also has a motion writer",
                    element.id
                ));
            }
            return self.reactive.sampler(time)?.value(node);
        }
        self.tracks
            .get(&element.id)
            .and_then(|properties| properties.get(&property))
            .and_then(|tracks| Track::at(tracks, time))
            .map_or_else(|| element.property(property), |track| track.sample(time))
    }
    fn sample_in_place(
        &self,
        out: &mut Element,
        time: Fraction,
        sampler: &mut crate::reactive::Sampler<'_>,
        native: &mut impl FnMut(&mut Element, Fraction) -> Result<(), String>,
    ) -> Result<(), String> {
        if matches!(out.geometry.as_ref(), Geometry::Code { .. })
            && let Some(state) = self.code_views.get(&out.id)
        {
            let value = state.presentation(time)?;
            if let Geometry::Code {
                presentation,
                pending,
                ..
            } = Arc::make_mut(&mut out.geometry)
            {
                *pending |= value.is_some();
                *presentation = value;
            }
        }
        if let Some(properties) = self.reactive.properties(&out.id) {
            for (property, node) in properties {
                if *property == Property::Position && self.flow_controlled.contains(&out.id) {
                    return Err(format!(
                        "flow layout controls '{}.position'; animate a containing Group or use a free Group instead",
                        out.id
                    ));
                }
                if self
                    .tracks
                    .get(&out.id)
                    .is_some_and(|tracks| tracks.contains_key(property))
                {
                    return Err(format!(
                        "retained property {}.{property:?} also has a motion writer",
                        out.id
                    ));
                }
                out.set(*property, sampler.value(*node)?)?;
            }
        }
        if let Some(properties) = self.tracks.get(&out.id) {
            for (property, tracks) in properties {
                if let Some(track) = Track::at(tracks, time) {
                    out.set(*property, track.sample(time)?)?;
                    if let Some(horizontal) = track.horizontal {
                        out.horizontal.origin = horizontal.origin;
                        out.horizontal.offset = horizontal.offset;
                    }
                    if let Some(kind) = track.reveal {
                        out.reveal.kind = kind;
                    }
                }
            }
        }
        if let Some(children) = self.list_views.visual_children(&out.id, time)? {
            let Geometry::Box {
                children: current, ..
            } = Arc::make_mut(&mut out.geometry)
            else {
                return Err("list view requires a Box container".into());
            };
            *current = children;
        }
        native(out, time)?;
        if out
            .geometry
            .children()
            .is_some_and(|children| !children.is_empty())
            && let Some(children) = Arc::make_mut(&mut out.geometry).children_mut()
        {
            for child in children {
                self.sample_in_place(child, time, sampler, native)?;
            }
        }
        Ok(())
    }
    pub fn find(&self, id: &str) -> Option<&Element> {
        let Some(path) = self.element_index.get(id) else {
            return self.list_views.find(id);
        };
        if path.is_empty() {
            return self.list_views.find(id);
        }
        let mut element = &self.layers.get(*path.front()?)?.element;
        for index in path.iter().skip(1) {
            let children = element.geometry.children()?;
            element = children.get(*index)?;
        }
        Some(element)
    }
    pub fn element_count(&self) -> usize {
        self.element_index.len()
    }
    pub fn add(&mut self, element: Element, birth: Fraction) -> Result<(), String> {
        let mut flow = OrdSet::new();
        collect_flow_positions(&element, &mut flow);
        for id in &flow {
            if self.reactive.binding(id, Property::Position).is_some() {
                return Err(format!(
                    "flow layout controls '{id}.position'; animate a containing Group or use a free Group instead"
                ));
            }
        }
        let mut ids = vec![];
        element.ids(&mut ids);
        let mut unique = std::collections::BTreeSet::new();
        for id in ids {
            let owned_here = self.list_views.owner(&id).is_some_and(|cell| {
                self.list_views.view_cell(&element.id) == Some(cell)
                    || self.list_views.views_in(&element, cell)
            });
            if !unique.insert(id.clone())
                || self
                    .element_index
                    .get(&id)
                    .is_some_and(|path| !path.is_empty())
                || self.list_views.find(&id).is_some() && !owned_here
            {
                return Err(format!("duplicate element identity '{id}'"));
            }
        }
        fn index(element: &Element, path: Vec<usize>, map: &mut OrdMap<String, Vector<usize>>) {
            map.insert(element.id.clone(), path.iter().copied().collect());
            if let Some(children) = element.geometry.children() {
                for (i, child) in children.iter().enumerate() {
                    let mut path = path.clone();
                    path.push(i);
                    index(child, path, map);
                }
            }
        }
        index(&element, vec![self.layers.len()], &mut self.element_index);
        self.flow_controlled.extend(flow);
        self.layers.push_back(Layer { birth, element });
        Ok(())
    }
    pub fn schedule(
        &mut self,
        motions: &[Motion],
        start: Fraction,
        duration: Fraction,
        easing: Easing,
    ) -> Result<(), String> {
        if !math::finite(&duration) || duration < Fraction::zero() {
            return Err("animation duration must be nonnegative".into());
        }
        let end = math::timeline_end(start, duration)?;
        // Validate a small write set before committing; do not clone the whole scene.
        let mut additions: Vec<Element> = vec![];
        let mut writes = vec![];
        let mut targets = std::collections::BTreeSet::new();
        let mut new_ids = std::collections::BTreeSet::new();
        let mut pending_flow = OrdSet::new();
        let mut descendants = std::collections::BTreeSet::new();
        for motion in motions {
            collect_flow_positions(&motion.element, &mut pending_flow);
            if let Some(children) = motion.element.geometry.children() {
                let mut ids = vec![];
                for child in children {
                    child.ids(&mut ids);
                }
                descendants.extend(ids);
            }
        }
        // Insert only ancestor roots, regardless of argument order. Child writers
        // then resolve against the same pending tree as their parent writers.
        for motion in motions {
            if descendants.contains(&motion.element.id)
                || self.find(&motion.element.id).is_some()
                || additions
                    .iter()
                    .any(|element| element.id == motion.element.id)
            {
                continue;
            }
            let mut ids = vec![];
            motion.element.ids(&mut ids);
            for id in ids {
                if self.find(&id).is_some() || !new_ids.insert(id.clone()) {
                    return Err(format!("duplicate element identity '{id}'"));
                }
            }
            additions.push(motion.element.clone());
        }
        for id in &pending_flow {
            if self.reactive.binding(id, Property::Position).is_some() {
                return Err(format!(
                    "flow layout controls '{id}.position'; animate a containing Group or use a free Group instead"
                ));
            }
        }
        for motion in motions {
            if self
                .reactive
                .binding(&motion.element.id, motion.property)
                .is_some()
            {
                return Err(format!(
                    "{}.{:?} is controlled by a retained signal; animate its mutable input instead",
                    motion.element.id, motion.property
                ));
            }
            let flow_controlled = self.flow_controlled.contains(&motion.element.id)
                || pending_flow.contains(&motion.element.id);
            if motion.property == Property::Position && flow_controlled {
                return Err(format!(
                    "flow layout controls '{}.position'; animate a containing Group or use a free Group instead",
                    motion.element.id
                ));
            }
            let centered_growth = motion.property == Property::Scale
                && (motion.to.number()?.value == Fraction::from(0)
                    || motion
                        .from
                        .as_ref()
                        .is_some_and(|v| v.number().is_ok_and(|n| n.value == Fraction::from(0))));
            if centered_growth && flow_controlled {
                return Err(format!(
                    "flow layout controls the size of '{}'; grow or shrink the containing Row, Column or Group instead",
                    motion.element.id
                ));
            }
            let existing = self
                .find(&motion.element.id)
                .or_else(|| additions.iter().find_map(|v| v.find(&motion.element.id)));
            if let Some(existing) = existing {
                if existing != &motion.element {
                    return Err(format!(
                        "element id '{}' is used by different element definitions",
                        motion.element.id
                    ));
                }
            } else {
                return Err(format!(
                    "duplicate or cyclic element identity '{}' in pending groups",
                    motion.element.id
                ));
            }
            if !targets.insert((motion.element.id.clone(), motion.property)) {
                return Err(format!(
                    "overlapping writers for {}.{:?}",
                    motion.element.id, motion.property
                ));
            }
            if motion.property == Property::Reveal {
                let related = |other: &Element| {
                    other.id != motion.element.id
                        && (motion.element.find(&other.id).is_some()
                            || other.find(&motion.element.id).is_some())
                };
                let pending = motions
                    .iter()
                    .any(|other| other.property == Property::Reveal && related(&other.element));
                let existing = self.tracks.iter().any(|(id, properties)| {
                    self.find(id).is_some_and(related)
                        && properties.get(&Property::Reveal).is_some_and(|tracks| {
                            tracks
                                .iter()
                                .any(|track| track.start < end && track.end > start)
                        })
                });
                if pending || existing {
                    return Err(format!(
                        "overlapping ancestor/descendant reveals for '{}'; reveal the containing group, or animate parent and child in separate plays",
                        motion.element.id
                    ));
                }
                for value in std::iter::once(&motion.to).chain(motion.from.iter()) {
                    let n = value.number()?;
                    if n.unit != crate::Unit::Number
                        || !math::finite(&n.value)
                        || n.value < Fraction::from(0)
                        || n.value > Fraction::from(1)
                    {
                        return Err("reveal endpoints must be finite Numbers in [0,1]".into());
                    }
                }
            }
            if motion.property == Property::StretchX {
                for value in std::iter::once(&motion.to).chain(motion.from.iter()) {
                    let n = value.number()?;
                    if n.unit != crate::Unit::Number
                        || !math::finite(&n.value)
                        || n.value < Fraction::from(0)
                    {
                        return Err(
                            "horizontal scale endpoints must be finite nonnegative Numbers".into(),
                        );
                    }
                }
                if let Some(horizontal) = motion.horizontal
                    && (!matches!(
                        horizontal.origin.unit,
                        crate::Unit::Length | crate::Unit::Percent
                    ) || !math::finite(&horizontal.origin.value)
                        || !math::finite(&horizontal.offset))
                {
                    return Err("horizontal stretch origin requires a finite pt or % value and finite pt offset".into());
                }
            }
            let element = self.find(&motion.element.id).unwrap_or(&motion.element);
            let current = self.sample_property(element, motion.property, start)?;
            if motion.property == Property::Rotation
                && motion.to.number()?.value != Fraction::zero()
                && self
                    .tracks
                    .get(&motion.element.id)
                    .and_then(|properties| properties.get(&Property::Scale))
                    .is_some_and(|tracks| {
                        tracks.iter().any(|track| {
                            (track.start < end || duration.is_zero() && track.start == start)
                                && track.end > start
                                && [&track.from, &track.to].iter().any(|value| {
                                    value.number().is_ok_and(|number| number.value.is_zero())
                                })
                        })
                    })
            {
                return Err("GrowFromCenter/ShrinkToCenter require zero rotation throughout the motion; sequence rotation outside the centered growth interval".into());
            }
            if centered_growth
                && (self
                    .sample_property(element, Property::Rotation, start)?
                    .number()?
                    .value
                    != Fraction::from(0)
                    || motions.iter().any(|other| {
                        other.element.id == motion.element.id
                            && other.property == Property::Rotation
                    })
                    || self
                        .tracks
                        .get(&motion.element.id)
                        .and_then(|properties| properties.get(&Property::Rotation))
                        .is_some_and(|tracks| {
                            tracks
                                .iter()
                                .any(|track| track.start < end && track.end > start)
                        }))
            {
                return Err("GrowFromCenter/ShrinkToCenter require zero rotation throughout the motion; use ScaleTo for rotated geometry, or reset rotation in an earlier animation call".into());
            }
            let mut from = motion.from.clone().unwrap_or(current);
            let mut to = motion.to.clone();
            if let (Value::Vec2(a), Value::Vec2(b)) = (&mut from, &mut to) {
                if !matches!(b.unit, crate::Unit::Length | crate::Unit::Percent) {
                    return Err("position motion requires a Vec2 with pt or % components".into());
                }
                if a.unit != b.unit {
                    if a.values.iter().all(|v| *v == 0.0) {
                        a.unit = b.unit;
                    } else if b.values.iter().all(|v| *v == 0.0) {
                        b.unit = a.unit;
                    } else {
                        return Err(
                            "position track endpoints must use matching pt or % units".into()
                        );
                    }
                }
            }
            let tracks = self
                .tracks
                .get(&motion.element.id)
                .and_then(|v| v.get(&motion.property));
            if tracks.and_then(|v| v.back()).is_some_and(|v| v.end > start) {
                return Err(format!(
                    "overlapping writers for {}.{:?}",
                    motion.element.id, motion.property
                ));
            }
            let easing = motion.easing.unwrap_or(easing);
            if easing == Easing::Overshoot
                && !matches!(
                    motion.property,
                    Property::Position | Property::Scroll | Property::Opacity
                )
            {
                return Err("overshoot easing supports position and opacity motions; use smootherstep for bounded scale, reveal, rotation or color motions".into());
            }
            writes.push((
                motion.element.id.clone(),
                motion.property,
                Track {
                    start,
                    end,
                    from,
                    to,
                    easing,
                    bounded: matches!(
                        motion.property,
                        Property::Opacity
                            | Property::Scale
                            | Property::StretchX
                            | Property::Reveal
                            | Property::Radius
                            | Property::Width
                            | Property::Height
                            | Property::FontSize
                            | Property::Fill
                    ),
                    reveal: motion.reveal,
                    horizontal: motion.horizontal,
                },
            ));
        }
        for element in additions {
            self.add(element, start)?;
        }
        for (id, property, track) in writes {
            self.tracks
                .entry(id)
                .or_default()
                .entry(property)
                .or_default()
                .push_back(track);
        }
        self.duration = self.duration.max(end);
        Ok(())
    }
    pub fn frame_time(&self, index: u64) -> Result<Fraction, String> {
        use num_traits::CheckedDiv;
        Fraction::from(index)
            .checked_div(&self.rate)
            .ok_or("frame time overflow".into())
    }
}

pub(crate) fn collect_flow_positions(element: &Element, ids: &mut OrdSet<String>) {
    if let Some(children) = element.geometry.children() {
        if element.layout.direction.is_some() {
            ids.extend(children.iter().map(|child| child.id.clone()));
        }
        for child in children {
            collect_flow_positions(child, ids);
        }
    }
}

impl Track {
    pub fn at(tracks: &Vector<Self>, time: Fraction) -> Option<&Self> {
        let count = tracks
            .binary_search_by(|track| {
                if track.start <= time {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            })
            .unwrap_err();
        count.checked_sub(1).map(|index| &tracks[index])
    }
    pub fn sample(&self, time: Fraction) -> Result<Value, String> {
        if time >= self.end {
            return Ok(self.to.clone());
        }
        let p = math::progress(time, self.start, self.end)?;
        let one = Fraction::from(1);
        let p = self.easing.apply(p)?;
        let bounded_progress = p.max(Fraction::zero()).min(one);
        let scalar_progress = if self.bounded || self.easing == Easing::Overshoot {
            bounded_progress
        } else {
            p
        };
        let overshooting = matches!(
            self.easing,
            Easing::Overshoot | Easing::Spring | Easing::SpringBouncy
        );
        let scalar_lerp = |from, to| {
            if matches!(self.easing, Easing::Spring | Easing::SpringBouncy) {
                math::visual_lerp(from, to, scalar_progress)
            } else {
                math::lerp(from, to, scalar_progress)
            }
        };
        match (&self.from, &self.to) {
            (Value::Number(a), Value::Number(b)) if a.unit == b.unit => {
                Ok(Value::Number(crate::Number {
                    value: scalar_lerp(a.value, b.value)?,
                    unit: a.unit,
                }))
            }
            (Value::Fraction(a), Value::Fraction(b)) => Ok(Value::Fraction(scalar_lerp(*a, *b)?)),
            (Value::Vec2(a), Value::Vec2(b)) if a.unit == b.unit => Ok(Value::Vec2(Vec2 {
                values: math::motion_vector_lerp(a.values, b.values, p, overshooting)?,
                unit: a.unit,
            })),
            (Value::Vec3(a), Value::Vec3(b)) if a.unit == b.unit => Ok(Value::Vec3(crate::Vec3 {
                values: math::motion_vector_lerp(a.values, b.values, p, overshooting)?,
                unit: a.unit,
            })),
            (Value::Vec4(a), Value::Vec4(b)) if a.unit == b.unit => Ok(Value::Vec4(crate::Vec4 {
                values: math::motion_vector_lerp(a.values, b.values, p, overshooting)?,
                unit: a.unit,
            })),
            (Value::Color(a), Value::Color(b)) => {
                let mut out = a.clone();
                for i in 0..4 {
                    out.0[i] = math::visual_lerp(a.0[i], b.0[i], bounded_progress)?;
                }
                Ok(Value::Color(out))
            }
            (from, to) if from.ty().accepts(&to.ty()) || to.ty().accepts(&from.ty()) => {
                Ok(from.clone())
            }
            _ => Err("incompatible track endpoints".into()),
        }
    }
}

impl Easing {
    pub fn apply(self, p: Fraction) -> Result<Fraction, String> {
        Ok(match self {
            Easing::Linear => p,
            Easing::Spring => math::spring(p)?,
            Easing::SpringBouncy => math::spring_bouncy(p)?,
            Easing::CubicIn => math::visual_polynomial(p, &[0, 0, 0, 1])?,
            Easing::CubicOut => math::visual_polynomial(p, &[0, 3, -3, 1])?,
            Easing::Smooth => math::visual_polynomial(p, &[0, 0, 3, -2])?,
            Easing::Smootherstep => math::smootherstep(p)?,
            Easing::Overshoot => math::overshoot(p)?,
        })
    }
}
