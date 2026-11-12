use crate::{Color, Value, Vec2, math};
use fraction::Fraction;
use imbl::{OrdMap, OrdSet, Vector};
use num_traits::{CheckedMul, Zero};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Geometry {
    Circle {
        radius: Fraction,
    },
    Rectangle {
        width: Fraction,
        height: Fraction,
        #[serde(default)]
        radius: Fraction,
    },
    Line {
        end: Vec2,
        width: Fraction,
    },
    Polygon {
        points: Arc<Vec<Vec2>>,
    },
    Text {
        text: String,
        size: Fraction,
        outline: Arc<Vec<TextSegment>>,
        bounds: crate::verify::Bounds,
    },
    Group {
        children: Vector<Element>,
    },
}

/// Prepared Pango outlines, replayed by Skia without retaining platform font objects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TextSegment {
    Move([f64; 2]),
    Line([f64; 2]),
    Curve([f64; 2], [f64; 2], [f64; 2]),
    Close,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: String,
    pub geometry: Arc<Geometry>,
    pub position: Vec2,
    pub rotation: Fraction,
    pub scale: Fraction,
    pub opacity: Fraction,
    pub fill: Color,
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
    pub direction: Option<Direction>,
    pub gap: Fraction,
    pub width: Option<Fraction>,
    pub height: Option<Fraction>,
    pub padding: Fraction,
    pub justify: Justify,
    pub align: Align,
    pub wrap: bool,
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
    Rotation,
    Scale,
    StretchX,
    Opacity,
    Fill,
    Reveal,
    Radius,
    Width,
    Height,
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
    #[default]
    Linear,
    CubicIn,
    CubicOut,
    Smooth,
    Smootherstep,
    Overshoot,
}
impl Easing {
    pub fn parse(name: &str) -> Result<Self, String> {
        const OPTIONS: &[(&str, Easing)] = &[
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
    element_index: OrdMap<String, Vector<usize>>,
    #[serde(default)]
    flow_controlled: OrdSet<String>,
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
            background: Color([Fraction::zero(); 4]),
            layers: Vector::new(),
            tracks: OrdMap::new(),
            reactive: crate::reactive::ReactiveGraph::default(),
            element_index: OrdMap::new(),
            flow_controlled: OrdSet::new(),
        }
    }
}

impl Element {
    pub fn property(&self, property: Property) -> Result<Value, String> {
        Ok(match property {
            Property::Position => Value::Vec2(self.position),
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
            Property::Fill => Value::Color(self.fill.clone()),
            Property::Radius | Property::Width | Property::Height => {
                let value = match (property, self.geometry.as_ref()) {
                    (Property::Radius, Geometry::Circle { radius } | Geometry::Rectangle { radius, .. }) => *radius,
                    (Property::Width, Geometry::Rectangle { width, .. }) => *width,
                    (Property::Width, Geometry::Line { width, .. }) => *width,
                    (Property::Height, Geometry::Rectangle { height, .. }) => *height,
                    (Property::Width, Geometry::Group { .. }) => self.layout.width.ok_or_else(|| format!("{}.width is automatic; provide an explicit width before reading it",self.id))?,
                    (Property::Height, Geometry::Group { .. }) => self.layout.height.ok_or_else(|| format!("{}.height is automatic; provide an explicit height before reading it",self.id))?,
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
        })
    }
    pub fn set(&mut self, property: Property, value: Value) -> Result<(), String> {
        match property {
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
                if let Value::Color(color) = value {
                    self.fill = color
                } else {
                    return Err("expected Color".into());
                }
            }
            Property::Radius | Property::Width | Property::Height => {
                let number = value.number()?;
                let positive = property != Property::Radius
                    || matches!(self.geometry.as_ref(), Geometry::Circle { .. });
                if number.unit != crate::Unit::Length
                    || !math::finite(&number.value)
                    || number.value < Fraction::from(0)
                    || positive && number.value == Fraction::from(0)
                {
                    let requirement = if positive { "positive" } else { "nonnegative" };
                    return Err(format!(
                        "{property:?} requires a finite {requirement} Length"
                    ));
                }
                match (property, self.geometry.as_ref()) {
                    (Property::Radius, Geometry::Circle { .. } | Geometry::Rectangle { .. })
                    | (Property::Width | Property::Height, Geometry::Rectangle { .. }) => {}
                    (Property::Width | Property::Height, Geometry::Group { .. }) => {}
                    (Property::Width, Geometry::Line { .. }) => {}
                    _ => {
                        return Err(format!(
                            "{} does not have a {property:?} geometry property",
                            self.id
                        ));
                    }
                }
                if matches!(self.geometry.as_ref(), Geometry::Group { .. }) {
                    match property {
                        Property::Width => self.layout.width = Some(number.value),
                        Property::Height => self.layout.height = Some(number.value),
                        _ => unreachable!("validated group layout property"),
                    }
                    return Ok(());
                }
                match (property, Arc::make_mut(&mut self.geometry)) {
                    (
                        Property::Radius,
                        Geometry::Circle { radius } | Geometry::Rectangle { radius, .. },
                    ) => *radius = number.value,
                    (Property::Width, Geometry::Rectangle { width, .. }) => *width = number.value,
                    (Property::Width, Geometry::Line { width, .. }) => *width = number.value,
                    (Property::Height, Geometry::Rectangle { height, .. }) => {
                        *height = number.value
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
        if let Geometry::Group { children } = self.geometry.as_ref() {
            children.iter().find_map(|v| v.find(id))
        } else {
            None
        }
    }
    pub fn ids(&self, ids: &mut Vec<String>) {
        ids.push(self.id.clone());
        if let Geometry::Group { children } = self.geometry.as_ref() {
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
        if !math::finite(&time) || time < Fraction::zero() {
            return Err("sample time must be finite and nonnegative".into());
        }
        let mut sampler = self.reactive.sampler(time)?;
        self.layers
            .iter()
            .filter(|v| v.birth <= time)
            .map(|v| {
                let mut out = v.element.clone();
                self.sample_in_place(&mut out, time, &mut sampler)?;
                Ok(out)
            })
            .collect()
    }
    pub fn sample_element(&self, element: &Element, time: Fraction) -> Result<Element, String> {
        let mut out = element.clone();
        let mut sampler = self.reactive.sampler(time)?;
        self.sample_in_place(&mut out, time, &mut sampler)?;
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
    ) -> Result<(), String> {
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
        if matches!(out.geometry.as_ref(), Geometry::Group { .. })
            && let Geometry::Group { children } = Arc::make_mut(&mut out.geometry)
        {
            for child in children {
                self.sample_in_place(child, time, sampler)?;
            }
        }
        Ok(())
    }
    pub fn find(&self, id: &str) -> Option<&Element> {
        let path = self.element_index.get(id)?;
        let mut element = &self.layers.get(*path.front()?)?.element;
        for index in path.iter().skip(1) {
            let Geometry::Group { children } = element.geometry.as_ref() else {
                return None;
            };
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
            if !unique.insert(id.clone()) || self.find(&id).is_some() {
                return Err(format!("duplicate element identity '{id}'"));
            }
        }
        fn index(element: &Element, path: Vec<usize>, map: &mut OrdMap<String, Vector<usize>>) {
            map.insert(element.id.clone(), path.iter().copied().collect());
            if let Geometry::Group { children } = element.geometry.as_ref() {
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
        if !math::finite(&duration) || duration <= Fraction::zero() {
            return Err("play duration must be positive".into());
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
            if let Geometry::Group { children } = motion.element.geometry.as_ref() {
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
                return Err("GrowFromCenter/ShrinkToCenter require zero rotation throughout the motion; use ScaleTo for rotated geometry, or reset rotation in an earlier play".into());
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
                && !matches!(motion.property, Property::Position | Property::Opacity)
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

fn collect_flow_positions(element: &Element, ids: &mut OrdSet<String>) {
    if let Geometry::Group { children } = element.geometry.as_ref() {
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
        let p = match self.easing {
            Easing::Linear => p,
            Easing::CubicIn => p
                .checked_mul(&p)
                .and_then(|v| v.checked_mul(&p))
                .ok_or("easing overflow")?,
            Easing::CubicOut => {
                let q = math::arithmetic("-", one, p)?;
                math::arithmetic(
                    "-",
                    one,
                    math::arithmetic("*", math::arithmetic("*", q, q)?, q)?,
                )?
            }
            Easing::Smooth => math::arithmetic(
                "*",
                math::arithmetic("*", p, p)?,
                math::arithmetic(
                    "-",
                    Fraction::from(3),
                    math::arithmetic("*", Fraction::from(2), p)?,
                )?,
            )?,
            Easing::Smootherstep => math::smootherstep(p)?,
            Easing::Overshoot => math::overshoot(p)?,
        };
        match (&self.from, &self.to) {
            (Value::Number(a), Value::Number(b)) if a.unit == b.unit => {
                Ok(Value::Number(crate::Number {
                    value: math::lerp(
                        a.value,
                        b.value,
                        if self.easing == Easing::Overshoot {
                            p.min(one)
                        } else {
                            p
                        },
                    )?,
                    unit: a.unit,
                }))
            }
            (Value::Vec2(a), Value::Vec2(b)) if a.unit == b.unit => Ok(Value::Vec2(Vec2 {
                values: [
                    math::motion_lerp(
                        a.values[0],
                        b.values[0],
                        p,
                        self.easing == Easing::Overshoot,
                    )?,
                    math::motion_lerp(
                        a.values[1],
                        b.values[1],
                        p,
                        self.easing == Easing::Overshoot,
                    )?,
                ],
                unit: a.unit,
            })),
            (Value::Color(a), Value::Color(b)) => {
                let mut out = a.clone();
                for i in 0..4 {
                    out.0[i] = math::lerp(a.0[i], b.0[i], p)?;
                }
                Ok(Value::Color(out))
            }
            _ => Err("incompatible track endpoints".into()),
        }
    }
}
