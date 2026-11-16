//! Shared typed spatial paths and editable, immutable keyframed value curves.
mod keyframes;
pub mod values;
use crate::{List, ListLength, Number, Type, Unit, Value, math};
pub use keyframes::{
    Handle, Interpolation as KeyInterpolation, Keyframe, MAX_CHANNELS, MAX_KEYS, Tangents,
};
use sanim_voxel::{
    camera::path::{Interpolation, Path, Sample},
    layout::Coordinate,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "GeometricWire", into = "GeometricWire")]
struct Geometric {
    path: Arc<Path>,
    dimension: usize,
    units: [Unit; 3],
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeometricWire {
    path: Arc<Path>,
    dimension: usize,
    units: [Unit; 3],
}
impl From<Geometric> for GeometricWire {
    fn from(curve: Geometric) -> Self {
        Self {
            path: curve.path,
            dimension: curve.dimension,
            units: curve.units,
        }
    }
}
impl TryFrom<GeometricWire> for Geometric {
    type Error = String;
    fn try_from(value: GeometricWire) -> Result<Self, String> {
        if !(2..=3).contains(&value.dimension) || value.units.iter().any(|unit| !supported(*unit)) {
            return Err("Curve requires a fixed numeric List of length 2 or 3".into());
        }
        Ok(Self {
            path: value.path,
            dimension: value.dimension,
            units: value.units,
        })
    }
}
fn supported(unit: Unit) -> bool {
    matches!(
        unit,
        Unit::Number | Unit::Length | Unit::Percent | Unit::Angle
    )
}

impl Geometric {
    pub fn new(points: &Value, interpolation: Interpolation, closed: bool) -> Result<Self, String> {
        let points = points.list()?;
        if !(2..=sanim_voxel::camera::path::MAX_POINTS).contains(&points.len()) {
            return Err("Curve requires between 2 and 256 control points".into());
        }
        let dimension = points.get(0).expect("checked control point").list()?.len();
        if !(2..=3).contains(&dimension) {
            return Err("Curve control points must be fixed numeric Lists of length 2 or 3".into());
        }
        let mut units = [Unit::Number; 3];
        let mut coordinates = Vec::with_capacity(points.len());
        for (index, point) in points.iter().enumerate() {
            let point = point.list()?;
            if point.len() != dimension {
                return Err(format!(
                    "Curve control point {index} has {} coordinates; expected {dimension}",
                    point.len()
                ));
            }
            let mut coordinate = [Coordinate::Cells(crate::Fraction::from(0)); 3];
            for (axis, value) in point.iter().enumerate() {
                let number = value.number()?;
                if !supported(number.unit) {
                    return Err(
                        "Curve coordinates require numbers, lengths, percentages, or angles".into(),
                    );
                }
                if index == 0 {
                    units[axis] = number.unit;
                }
                if units[axis] != number.unit {
                    return Err(format!(
                        "Curve axis {axis} changes units at control point {index}; use the same unit along each axis"
                    ));
                }
                coordinate[axis] = if number.unit == Unit::Percent {
                    Coordinate::Percent(number.value * crate::Fraction::from(100))
                } else {
                    Coordinate::Cells(number.value)
                };
            }
            coordinates.push(coordinate);
        }
        Ok(Self {
            path: Arc::new(Path::new(coordinates.into(), interpolation, closed)?),
            dimension,
            units,
        })
    }
    pub fn revision(&self) -> u64 {
        self.path.revision()
    }
    pub fn resident_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.path.resident_bytes()
    }
    pub fn value_type(&self) -> Type {
        Type::List(
            Box::new(
                Type::union(
                    self.units[..self.dimension]
                        .iter()
                        .copied()
                        .map(Type::Numeric),
                )
                .expect("curve units"),
            ),
            ListLength::Exact(self.dimension),
        )
    }
    pub fn dimension(&self) -> usize {
        self.dimension
    }
    pub fn sample(&self, progress: f64) -> Result<Value, String> {
        let sample = self.path.sample([1; 3], progress, 0.0)?;
        let values = (0..self.dimension)
            .map(|axis| {
                Ok(Value::Number(Number {
                    value: math::functions::approximate(sample.position[axis])?,
                    unit: self.units[axis],
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Value::List(List::from(values)))
    }
    pub fn direction(&self, progress: f64) -> Result<Value, String> {
        let sample = self.path.sample([1; 3], progress, 0.0)?;
        let delta =
            std::array::from_fn::<_, 3, _>(|axis| sample.target[axis] - sample.position[axis]);
        let length = delta.iter().map(|axis| axis * axis).sum::<f64>().sqrt();
        Ok(Value::List(List::from(
            (0..self.dimension)
                .map(|axis| {
                    Ok(Value::Number(Number {
                        value: math::functions::approximate(delta[axis] / length)?,
                        unit: Unit::Number,
                    }))
                })
                .collect::<Result<Vec<_>, String>>()?,
        )))
    }
    pub fn sample_world(
        &self,
        dimensions: [u32; 3],
        progress: f64,
        look_ahead: f64,
    ) -> Result<Sample, String> {
        if self.dimension != 3
            || self
                .units
                .iter()
                .any(|unit| !matches!(unit, Unit::Number | Unit::Percent))
        {
            return Err(
                "a camera path requires Curve<List<Number | Percent, 3>>; use cells or percentages"
                    .into(),
            );
        }
        self.path.sample(dimensions, progress, look_ahead)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Definition", into = "Definition")]
pub struct Curve {
    kind: Kind,
}
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Path(Geometric),
    Keys(Arc<keyframes::Keyframed>),
}
#[derive(Serialize, Deserialize)]
enum Definition {
    Path(Geometric),
    Keys(#[serde(deserialize_with = "bounded_keys")] Arc<[Keyframe]>),
}
fn bounded_keys<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Arc<[Keyframe]>, D::Error> {
    struct Keys;
    impl<'de> serde::de::Visitor<'de> for Keys {
        type Value = Arc<[Keyframe]>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {MAX_KEYS} keyframes")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            if sequence.size_hint().is_some_and(|n| n > MAX_KEYS) {
                return Err(serde::de::Error::custom(
                    "Curve exceeds its keyframe budget",
                ));
            }
            let mut keys = Vec::new();
            while let Some(key) = sequence.next_element()? {
                if keys.len() == MAX_KEYS {
                    return Err(serde::de::Error::custom(
                        "Curve exceeds its keyframe budget",
                    ));
                }
                keys.push(key);
            }
            Ok(keys.into())
        }
    }
    deserializer.deserialize_seq(Keys)
}
impl From<Curve> for Definition {
    fn from(curve: Curve) -> Self {
        match curve.kind {
            Kind::Path(path) => Self::Path(path),
            Kind::Keys(keys) => Self::Keys(keys.keys.clone()),
        }
    }
}
impl TryFrom<Definition> for Curve {
    type Error = String;
    fn try_from(value: Definition) -> Result<Self, String> {
        Ok(match value {
            Definition::Path(path) => Self {
                kind: Kind::Path(path),
            },
            Definition::Keys(keys) => Self {
                kind: Kind::Keys(Arc::new(keyframes::Keyframed::new(keys)?)),
            },
        })
    }
}
impl Curve {
    pub fn new(points: &Value, interpolation: Interpolation, closed: bool) -> Result<Self, String> {
        Ok(Self {
            kind: Kind::Path(Geometric::new(points, interpolation, closed)?),
        })
    }
    pub fn keyed(keys: Arc<[Keyframe]>) -> Result<Self, String> {
        Ok(Self {
            kind: Kind::Keys(Arc::new(keyframes::Keyframed::new(keys)?)),
        })
    }
    /// Earliest normalized progress whose sampled result may differ.
    pub fn changed_progress(&self, next: &Self) -> Option<crate::Fraction> {
        match (&self.kind, &next.kind) {
            (Kind::Keys(a), Kind::Keys(b)) => a.changed_progress(b),
            (Kind::Path(a), Kind::Path(b)) if a == b => None,
            _ => Some(0.into()),
        }
    }
    pub fn is_constant(&self) -> bool {
        match &self.kind {
            Kind::Path(_) => false,
            Kind::Keys(keys) => keys.is_constant(),
        }
    }
    pub fn duration(&self) -> Option<crate::Fraction> {
        match &self.kind {
            Kind::Path(_) => None,
            Kind::Keys(keys) => Some(keys.duration()),
        }
    }
    pub fn revision(&self) -> u64 {
        match &self.kind {
            Kind::Path(path) => path.revision(),
            Kind::Keys(keys) => keys.revision(),
        }
    }
    pub fn resident_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.kind {
                Kind::Path(path) => path.resident_bytes(),
                Kind::Keys(keys) => keys.resident_bytes(),
            }
    }
    pub fn value_type(&self) -> Type {
        match &self.kind {
            Kind::Path(path) => path.value_type(),
            Kind::Keys(keys) => keys.value_type(),
        }
    }
    pub fn dimension(&self) -> usize {
        match &self.kind {
            Kind::Path(path) => path.dimension(),
            Kind::Keys(keys) => match keys.value_type() {
                Type::List(_, ListLength::Exact(n)) => n,
                _ => 1,
            },
        }
    }
    pub fn sample(&self, progress: f64) -> Result<Value, String> {
        if !progress.is_finite() {
            return Err("Curve progress must be finite".into());
        }
        match &self.kind {
            Kind::Path(path) => path.sample(progress),
            Kind::Keys(keys) => keys.sample(math::visual_product(
                keys.duration(),
                math::functions::approximate(progress.clamp(0.0, 1.0))?,
            )?),
        }
    }
    pub fn at(&self, progress: crate::Fraction) -> Result<Value, String> {
        if !math::finite(&progress) {
            return Err("Curve progress must be finite".into());
        }
        match &self.kind {
            Kind::Path(path) => path.sample(math::to_f64(progress)?),
            Kind::Keys(keys) => keys.sample(math::arithmetic(
                "*",
                keys.duration(),
                progress.max(0.into()).min(1.into()),
            )?),
        }
    }
    pub fn sample_time(&self, time: crate::Fraction) -> Result<Value, String> {
        match &self.kind{Kind::Keys(keys)=>keys.sample(time),Kind::Path(_)=>Err("Curve.sample(time) requires keyframes with timestamps; use .at(progress) for a geometric points curve".into())}
    }
    pub fn direction(&self, progress: f64) -> Result<Value, String> {
        match &self.kind{Kind::Path(path)=>path.direction(progress),Kind::Keys(_)=>Err("Curve.direction requires a geometric points curve; keyframed curves return ordinary typed values with .at(progress) or .sample(time)".into())}
    }
    pub fn sample_world(
        &self,
        dimensions: [u32; 3],
        progress: f64,
        look_ahead: f64,
    ) -> Result<Sample, String> {
        match &self.kind{Kind::Path(path)=>path.sample_world(dimensions,progress,look_ahead),Kind::Keys(_)=>Err("a keyframed camera curve drives Camera(position=curve.at(progress), target=...); Camera(path=...) requires a geometric points curve with a spatial direction".into())}
    }
}
