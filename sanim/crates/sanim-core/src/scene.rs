use crate::{Color, Value, Vec2, math};
use fraction::Fraction;
use num_traits::{CheckedMul, Zero};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Geometry {
    Circle {
        radius: Fraction,
    },
    Rectangle {
        width: Fraction,
        height: Fraction,
    },
    Line {
        end: Vec2,
        width: Fraction,
    },
    Polygon {
        points: Vec<Vec2>,
    },
    Text {
        text: String,
        size: Fraction,
        outline: Vec<TextSegment>,
        bounds: crate::verify::Bounds,
    },
    Group {
        children: Vec<Element>,
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
    pub geometry: Geometry,
    pub position: Vec2,
    pub rotation: Fraction,
    pub scale: Fraction,
    pub opacity: Fraction,
    pub fill: Color,
    pub layout: Layout,
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
    Opacity,
    Fill,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    pub element: Element,
    pub property: Property,
    pub to: Value,
    pub from: Option<Value>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Easing {
    #[default]
    Linear,
    CubicIn,
    CubicOut,
    Smooth,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub start: Fraction,
    pub end: Fraction,
    pub from: Value,
    pub to: Value,
    pub easing: Easing,
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
    pub layers: Vec<Layer>,
    /// Each property has its own sorted track index; sampling uses binary search.
    pub tracks: BTreeMap<String, BTreeMap<Property, Vec<Track>>>,
    element_index: BTreeMap<String, Vec<usize>>,
    #[serde(default)]
    flow_controlled: std::collections::BTreeSet<String>,
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
            layers: vec![],
            tracks: BTreeMap::new(),
            element_index: BTreeMap::new(),
            flow_controlled: std::collections::BTreeSet::new(),
        }
    }
}

impl Element {
    pub fn property(&self, property: Property) -> Value {
        match property {
            Property::Position => Value::Vec2(self.position),
            Property::Rotation => Value::Number(crate::Number {
                value: self.rotation,
                unit: crate::Unit::Angle,
            }),
            Property::Scale => Value::Number(crate::Number {
                value: self.scale,
                unit: crate::Unit::Number,
            }),
            Property::Opacity => Value::Number(crate::Number {
                value: self.opacity,
                unit: crate::Unit::Number,
            }),
            Property::Fill => Value::Color(self.fill.clone()),
        }
    }
    pub fn set(&mut self, property: Property, value: Value) -> Result<(), String> {
        match property {
            Property::Position => self.position = value.vec2()?,
            Property::Rotation => self.rotation = value.number()?.value,
            Property::Scale => self.scale = value.number()?.value,
            Property::Opacity => self.opacity = value.number()?.value,
            Property::Fill => {
                if let Value::Color(color) = value {
                    self.fill = color
                } else {
                    return Err("expected Color".into());
                }
            }
        }
        Ok(())
    }
    pub fn find(&self, id: &str) -> Option<&Element> {
        if self.id == id {
            return Some(self);
        }
        if let Geometry::Group { children } = &self.geometry {
            children.iter().find_map(|v| v.find(id))
        } else {
            None
        }
    }
    pub fn ids(&self, ids: &mut Vec<String>) {
        ids.push(self.id.clone());
        if let Geometry::Group { children } = &self.geometry {
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
        self.layers
            .iter()
            .filter(|v| v.birth <= time)
            .map(|v| self.sample_element(&v.element, time))
            .collect()
    }
    pub fn sample_element(&self, element: &Element, time: Fraction) -> Result<Element, String> {
        let mut out = element.clone();
        if let Some(properties) = self.tracks.get(&element.id) {
            for (property, tracks) in properties {
                let count = tracks.partition_point(|v| v.start <= time);
                if count > 0 {
                    out.set(*property, tracks[count - 1].sample(time)?)?;
                }
            }
        }
        if let Geometry::Group { children } = &element.geometry {
            out.geometry = Geometry::Group {
                children: children
                    .iter()
                    .map(|v| self.sample_element(v, time))
                    .collect::<Result<_, _>>()?,
            };
        }
        Ok(out)
    }
    pub fn find(&self, id: &str) -> Option<&Element> {
        let path = self.element_index.get(id)?;
        let mut element = &self.layers.get(*path.first()?)?.element;
        for index in &path[1..] {
            let Geometry::Group { children } = &element.geometry else {
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
        let mut ids = vec![];
        element.ids(&mut ids);
        let mut unique = std::collections::BTreeSet::new();
        for id in ids {
            if !unique.insert(id.clone()) || self.find(&id).is_some() {
                return Err(format!("duplicate element identity '{id}'"));
            }
        }
        fn index(element: &Element, path: Vec<usize>, map: &mut BTreeMap<String, Vec<usize>>) {
            map.insert(element.id.clone(), path.clone());
            if let Geometry::Group { children } = &element.geometry {
                for (i, child) in children.iter().enumerate() {
                    let mut path = path.clone();
                    path.push(i);
                    index(child, path, map);
                }
            }
        }
        index(&element, vec![self.layers.len()], &mut self.element_index);
        collect_flow_positions(&element, &mut self.flow_controlled);
        self.layers.push(Layer { birth, element });
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
        let mut pending_flow = std::collections::BTreeSet::new();
        for motion in motions {
            collect_flow_positions(&motion.element, &mut pending_flow);
        }
        for motion in motions {
            if motion.property == Property::Position
                && (self.flow_controlled.contains(&motion.element.id)
                    || pending_flow.contains(&motion.element.id))
            {
                return Err(format!(
                    "flow layout controls '{}.position'; animate a containing Group or use a free Group instead",
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
                let mut ids = vec![];
                motion.element.ids(&mut ids);
                for id in ids {
                    if self.find(&id).is_some() || !new_ids.insert(id.clone()) {
                        return Err(format!("duplicate element identity '{id}'"));
                    }
                }
                additions.push(motion.element.clone());
            }
            if !targets.insert((motion.element.id.clone(), motion.property)) {
                return Err(format!(
                    "overlapping writers for {}.{:?}",
                    motion.element.id, motion.property
                ));
            }
            let current = self.sample_element(
                self.find(&motion.element.id).unwrap_or(&motion.element),
                start,
            )?;
            let mut from = motion
                .from
                .clone()
                .unwrap_or_else(|| current.property(motion.property));
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
            if tracks.and_then(|v| v.last()).is_some_and(|v| v.end > start) {
                return Err(format!(
                    "overlapping writers for {}.{:?}",
                    motion.element.id, motion.property
                ));
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
                .push(track);
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

fn collect_flow_positions(element: &Element, ids: &mut std::collections::BTreeSet<String>) {
    if let Geometry::Group { children } = &element.geometry {
        if element.layout.direction.is_some() {
            ids.extend(children.iter().map(|child| child.id.clone()));
        }
        for child in children {
            collect_flow_positions(child, ids);
        }
    }
}

impl Track {
    pub fn sample(&self, time: Fraction) -> Result<Value, String> {
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
        };
        match (&self.from, &self.to) {
            (Value::Number(a), Value::Number(b)) if a.unit == b.unit => {
                Ok(Value::Number(crate::Number {
                    value: math::lerp(a.value, b.value, p)?,
                    unit: a.unit,
                }))
            }
            (Value::Vec2(a), Value::Vec2(b)) if a.unit == b.unit => Ok(Value::Vec2(Vec2 {
                values: [
                    math::float_lerp(a.values[0], b.values[0], p)?,
                    math::float_lerp(a.values[1], b.values[1], p)?,
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
