//! Immutable relative paths with a bounded cache for normalized and world-space sampling.
mod math;
use crate::layout::Coordinate;
use glam::DVec3;
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

pub const MAX_POINTS: usize = 256;
const SAMPLES_PER_SEGMENT: usize = 32;
const COMPILED_SIZES: usize = 2;
const ARC_HEADER_BYTES: usize = 2 * std::mem::size_of::<usize>();
const MAX_COORDINATE: f64 = (crate::MAX_DIMENSION * crate::MAX_DIMENSION) as f64;
pub type Position = [Coordinate; 3];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Linear,
    #[default]
    Centripetal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    #[serde(deserialize_with = "bounded_points")]
    points: Arc<[Position]>,
    interpolation: Interpolation,
    closed: bool,
}

fn bounded_points<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<[Position]>, D::Error> {
    struct Points;
    impl<'de> serde::de::Visitor<'de> for Points {
        type Value = Arc<[Position]>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {MAX_POINTS} curve control points")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            if sequence
                .size_hint()
                .is_some_and(|length| length > MAX_POINTS)
            {
                return Err(serde::de::Error::custom(
                    "Curve exceeds its control point budget",
                ));
            }
            let mut points = Vec::new();
            while let Some(point) = sequence.next_element()? {
                if points.len() == MAX_POINTS {
                    return Err(serde::de::Error::custom(
                        "Curve exceeds its control point budget",
                    ));
                }
                points.push(point);
            }
            Ok(points.into())
        }
    }
    deserializer.deserialize_seq(Points)
}

#[derive(Debug, Serialize)]
pub struct Path {
    definition: Definition,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    compiled: Mutex<VecDeque<([u32; 3], Arc<Compiled>)>>,
}
impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision || self.definition == other.definition
    }
}
impl<'de> Deserialize<'de> for Path {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            definition: Definition,
        }
        let Encoded { definition } = Encoded::deserialize(deserializer)?;
        Self::new(
            definition.points,
            definition.interpolation,
            definition.closed,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug)]
struct Entry {
    distance: f64,
    parameter: f64,
    up: DVec3,
}
#[derive(Debug)]
struct Compiled {
    points: Vec<DVec3>,
    entries: Vec<Entry>,
    interpolation: Interpolation,
    closed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub position: [f64; 3],
    pub target: [f64; 3],
    pub up: [f64; 3],
    pub distance: f64,
    pub length: f64,
}

impl Path {
    pub fn new(
        points: Arc<[Position]>,
        interpolation: Interpolation,
        closed: bool,
    ) -> Result<Self, String> {
        if !(2..=MAX_POINTS).contains(&points.len()) {
            return Err(format!("Curve requires 2..={MAX_POINTS} control points"));
        }
        for point in points.iter() {
            math::position(*point, [crate::MAX_DIMENSION; 3])?;
        }
        Ok(Self {
            definition: Definition {
                points,
                interpolation,
                closed,
            },
            revision: crate::revision(),
            compiled: Mutex::new(VecDeque::with_capacity(COMPILED_SIZES)),
        })
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Includes the maximum lazy lookup so memoization cannot grow outside its memory charge.
    pub fn resident_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + 2 * ARC_HEADER_BYTES
            + self.definition.points.len() * std::mem::size_of::<Position>()
            + COMPILED_SIZES
                * (std::mem::size_of::<([u32; 3], Arc<Compiled>)>()
                    + ARC_HEADER_BYTES
                    + std::mem::size_of::<Compiled>()
                    + self.definition.points.len() * std::mem::size_of::<DVec3>()
                    + (self.definition.points.len() * SAMPLES_PER_SEGMENT + 1)
                        * std::mem::size_of::<Entry>())
    }
    pub fn sample(
        &self,
        dimensions: [u32; 3],
        progress: f64,
        look_ahead: f64,
    ) -> Result<Sample, String> {
        if !progress.is_finite() || !look_ahead.is_finite() || look_ahead < 0.0 {
            return Err("Curve progress must be finite and look_ahead must be a finite nonnegative distance".into());
        }
        if dimensions.contains(&0) {
            return Err("Curve world dimensions must be nonzero".into());
        }
        let mut cache = self.compiled.lock().expect("camera path lookup poisoned");
        if let Some(index) = cache.iter().position(|(size, _)| *size == dimensions) {
            let entry = cache.remove(index).expect("known compiled curve size");
            cache.push_front(entry);
        } else {
            // Retire before allocating; sample under the lock so concurrent
            // callers cannot retain evicted tables outside the bounded cache.
            if cache.len() == COMPILED_SIZES {
                cache.pop_back();
            }
            let compiled = Arc::new(math::compile(&self.definition, dimensions)?);
            cache.push_front((dimensions, compiled));
        }
        Ok(math::sample(
            &cache.front().expect("compiled curve present").1,
            progress,
            look_ahead,
        ))
    }
}

pub fn resolve_position(position: Position, dimensions: [u32; 3]) -> Result<[f64; 3], String> {
    math::position(position, dimensions).map(|position| position.to_array())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fraction::Fraction;
    #[test]
    fn maximum_closed_curve_allocations_fit_the_lazy_charge_and_evict_old_tables() {
        let controls = (0..MAX_POINTS)
            .map(|index| {
                [
                    Coordinate::Percent(Fraction::from(index)),
                    Coordinate::Percent(Fraction::from(index % 7)),
                    Coordinate::Percent(Fraction::from(index % 11)),
                ]
            })
            .collect::<Arc<[_]>>();
        let path = Path::new(controls, Interpolation::Centripetal, true).unwrap();
        path.sample([100; 3], 0.5, 0.0).unwrap();
        path.sample([120; 3], 0.5, 0.0).unwrap();
        let evicted = {
            let cache = path.compiled.lock().unwrap();
            let actual = std::mem::size_of::<Path>()
                + 2 * ARC_HEADER_BYTES
                + path.definition.points.len() * std::mem::size_of::<Position>()
                + cache.capacity() * std::mem::size_of::<([u32; 3], Arc<Compiled>)>()
                + cache
                    .iter()
                    .map(|(_, compiled)| {
                        ARC_HEADER_BYTES
                            + std::mem::size_of::<Compiled>()
                            + compiled.points.capacity() * std::mem::size_of::<DVec3>()
                            + compiled.entries.capacity() * std::mem::size_of::<Entry>()
                    })
                    .sum::<usize>();
            assert!(
                actual <= path.resident_bytes(),
                "actual reserved cache bytes {actual} exceed the declared lazy charge {}",
                path.resident_bytes()
            );
            Arc::downgrade(&cache.back().unwrap().1)
        };
        path.sample([140; 3], 0.5, 0.0).unwrap();
        assert!(
            evicted.upgrade().is_none(),
            "evicted compiled tables must not remain retained"
        );
    }
}
