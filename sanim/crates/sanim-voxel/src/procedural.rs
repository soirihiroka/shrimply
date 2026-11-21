//! Immutable, bounded scalar fields shared by native terrain, masks and scatter.
mod math;
mod raster;
pub use math::random;
pub use raster::{Heightmap, SurfacePoints};

use imbl::Vector;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{SeqAccess, Visitor},
    ser::SerializeSeq,
};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fmt, str::FromStr};

pub const MAX_FIELD_NODES: usize = 128;
pub const MAX_OCTAVES: u32 = 8;
pub const MAX_PROCEDURAL_WORK: usize = 64 * 1024 * 1024;
pub const MAX_SURFACE_POINTS: usize = 65_536;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axes {
    Xy,
    Xz,
    Yz,
    #[default]
    Xyz,
}
impl Axes {
    pub const ALL: [Self; 4] = [Self::Xy, Self::Xz, Self::Yz, Self::Xyz];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Xy => "xy",
            Self::Xz => "xz",
            Self::Yz => "yz",
            Self::Xyz => "xyz",
        }
    }
    pub const fn mask(self) -> [bool; 3] {
        match self {
            Self::Xy => [true, true, false],
            Self::Xz => [true, false, true],
            Self::Yz => [false, true, true],
            Self::Xyz => [true; 3],
        }
    }
}
impl FromStr for Axes {
    type Err = String;
    fn from_str(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|axes| axes.name() == name)
            .ok_or_else(|| {
                format!(
                    "unknown field axes '{name}'; use {}",
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoiseKind {
    Value,
    Gradient,
    #[default]
    Fbm,
    Ridged,
    Cellular,
}
impl NoiseKind {
    pub const ALL: [Self; 5] = [
        Self::Value,
        Self::Gradient,
        Self::Fbm,
        Self::Ridged,
        Self::Cellular,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Gradient => "gradient",
            Self::Fbm => "fbm",
            Self::Ridged => "ridged",
            Self::Cellular => "cellular",
        }
    }
}
impl FromStr for NoiseKind {
    type Err = String;
    fn from_str(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name() == name)
            .ok_or_else(|| {
                format!(
                    "unknown noise kind '{name}'; use {}",
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Noise {
    pub seed: u64,
    pub kind: NoiseKind,
    /// Wavelength in voxel-cell units.
    pub scale: f64,
    pub octaves: u32,
    pub persistence: f64,
    pub lacunarity: f64,
    pub axes: Axes,
}
impl Default for Noise {
    fn default() -> Self {
        Self {
            seed: 0,
            kind: NoiseKind::Fbm,
            scale: 32.0,
            octaves: 4,
            persistence: 0.5,
            lacunarity: 2.0,
            axes: Axes::default(),
        }
    }
}
impl Noise {
    fn octave_count(self) -> u32 {
        if matches!(self.kind, NoiseKind::Fbm | NoiseKind::Ridged) {
            self.octaves
        } else {
            1
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.scale.is_finite() || self.scale <= 0.0 {
            return Err("noise scale must be finite and positive".into());
        }
        if self.octaves == 0 || self.octaves > MAX_OCTAVES {
            return Err(format!("noise octaves must be within 1..={MAX_OCTAVES}"));
        }
        if !self.persistence.is_finite() || !(0.0..=1.0).contains(&self.persistence) {
            return Err("noise persistence must be finite within 0..=1".into());
        }
        if !self.lacunarity.is_finite() || !(1.0..=8.0).contains(&self.lacunarity) {
            return Err("noise lacunarity must be finite within 1..=8".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unary {
    Abs,
    Negate,
    Floor,
    Fract,
    Sin,
    Cos,
    Smoothstep,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Min,
    Max,
    Power,
    Modulo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Instruction {
    Axis(u8),
    Constant(f64),
    Noise {
        options: Noise,
        coordinates: [usize; 3],
    },
    Distance {
        coordinates: [usize; 3],
        center: [f64; 3],
        axes: [bool; 3],
    },
    Unary(Unary, usize),
    Binary(Binary, usize, usize),
    Clamp {
        value: usize,
        min: f64,
        max: f64,
    },
}
impl Instruction {
    fn references(&self) -> Vec<usize> {
        match self {
            Self::Axis(_) | Self::Constant(_) => Vec::new(),
            Self::Noise { coordinates, .. } | Self::Distance { coordinates, .. } => {
                coordinates.to_vec()
            }
            Self::Unary(_, at) | Self::Clamp { value: at, .. } => vec![*at],
            Self::Binary(_, a, b) => vec![*a, *b],
        }
    }
    fn remap(&self, map: &[usize]) -> Self {
        match self {
            Self::Axis(axis) => Self::Axis(*axis),
            Self::Constant(value) => Self::Constant(*value),
            Self::Noise {
                options,
                coordinates,
            } => Self::Noise {
                options: *options,
                coordinates: coordinates.map(|at| map[at]),
            },
            Self::Distance {
                coordinates,
                center,
                axes,
            } => Self::Distance {
                coordinates: coordinates.map(|at| map[at]),
                center: *center,
                axes: *axes,
            },
            Self::Unary(operation, at) => Self::Unary(*operation, map[*at]),
            Self::Binary(operation, a, b) => Self::Binary(*operation, map[*a], map[*b]),
            Self::Clamp { value, min, max } => Self::Clamp {
                value: map[*value],
                min: *min,
                max: *max,
            },
        }
    }
}

/// A single persistent SSA representation. Composition shares unchanged vector
/// branches, and repeated subexpressions are interned before native sampling.
#[derive(Clone, Debug)]
pub struct Field {
    code: Vector<Instruction>,
    fingerprint: [u8; 32],
    sample_work: usize,
}
impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code
    }
}
impl Serialize for Field {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.code.len()))?;
        for instruction in &self.code {
            seq.serialize_element(instruction)?;
        }
        seq.end()
    }
}
impl<'de> Deserialize<'de> for Field {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Program;
        impl<'de> Visitor<'de> for Program {
            type Value = Field;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    formatter,
                    "a nonempty field program of at most {MAX_FIELD_NODES} instructions"
                )
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Field, A::Error> {
                let mut code = Vector::new();
                while let Some(instruction) = sequence.next_element()? {
                    if code.len() == MAX_FIELD_NODES {
                        return Err(serde::de::Error::custom(
                            "density instruction budget exceeded",
                        ));
                    }
                    code.push_back(instruction);
                }
                Field::finish(code).map_err(serde::de::Error::custom)
            }
        }
        deserializer.deserialize_seq(Program)
    }
}

impl Field {
    fn finish(code: Vector<Instruction>) -> Result<Self, String> {
        if code.is_empty() || code.len() > MAX_FIELD_NODES {
            return Err(format!(
                "density program requires 1..={MAX_FIELD_NODES} instructions; split this field into simpler layers"
            ));
        }
        let mut work = code.len();
        for (index, instruction) in code.iter().enumerate() {
            if instruction.references().into_iter().any(|at| at >= index) {
                return Err("density instructions must reference earlier values; cycles and forward references are invalid".into());
            }
            match instruction {
                Instruction::Axis(axis) if *axis > 2 => {
                    return Err("density axis must be x, y, or z".into());
                }
                Instruction::Constant(value) if !value.is_finite() => {
                    return Err("density constants must be finite".into());
                }
                Instruction::Noise { options, .. } => {
                    options.validate()?;
                    work += if options.kind == NoiseKind::Cellular {
                        if options.axes == Axes::Xyz { 27 } else { 9 }
                    } else {
                        options.octave_count() as usize
                    };
                }
                Instruction::Distance { center, axes, .. }
                    if center.iter().any(|value| !value.is_finite()) || !axes.contains(&true) =>
                {
                    return Err(
                        "distance field needs finite center coordinates and at least one axis"
                            .into(),
                    );
                }
                Instruction::Clamp { min, max, .. }
                    if !min.is_finite() || !max.is_finite() || min > max =>
                {
                    return Err("density clamp requires finite min <= max".into());
                }
                _ => {}
            }
        }
        let encoded = serde_json::to_vec(&code).map_err(|error| error.to_string())?;
        Ok(Self {
            code,
            fingerprint: Sha256::digest(encoded).into(),
            sample_work: work,
        })
    }
    pub fn constant(value: f64) -> Result<Self, String> {
        Self::finish(std::iter::once(Instruction::Constant(value)).collect())
    }
    pub fn axis(axis: usize) -> Result<Self, String> {
        if axis > 2 {
            return Err("density axis must be x, y, or z".into());
        }
        Self::finish(std::iter::once(Instruction::Axis(axis as u8)).collect())
    }
    pub fn noise(options: Noise) -> Result<Self, String> {
        Self::finish(
            [
                Instruction::Axis(0),
                Instruction::Axis(1),
                Instruction::Axis(2),
                Instruction::Noise {
                    options,
                    coordinates: [0, 1, 2],
                },
            ]
            .into_iter()
            .collect(),
        )
    }
    pub fn distance(center: [f64; 3], axes: [bool; 3]) -> Result<Self, String> {
        Self::finish(
            [
                Instruction::Axis(0),
                Instruction::Axis(1),
                Instruction::Axis(2),
                Instruction::Distance {
                    coordinates: [0, 1, 2],
                    center,
                    axes,
                },
            ]
            .into_iter()
            .collect(),
        )
    }
    pub fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }
    pub fn instructions(&self) -> usize {
        self.code.len()
    }
    pub fn sample_work(&self) -> usize {
        self.sample_work
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.code.len() * std::mem::size_of::<Instruction>() * 2
    }
    pub fn unary(&self, operation: Unary) -> Result<Self, String> {
        let mut code = self.code.clone();
        code.push_back(Instruction::Unary(operation, code.len() - 1));
        Self::finish(code)
    }
    pub fn binary(&self, operation: Binary, other: &Self) -> Result<Self, String> {
        let mut builder = Builder::new(self.code.clone());
        let a = self.code.len() - 1;
        let b = builder.import(other, None)?;
        builder.push(Instruction::Binary(operation, a, b))?;
        Self::finish(builder.code)
    }
    pub fn clamp(&self, min: f64, max: f64) -> Result<Self, String> {
        let mut code = self.code.clone();
        code.push_back(Instruction::Clamp {
            value: code.len() - 1,
            min,
            max,
        });
        Self::finish(code)
    }
    pub fn smoothstep(&self, min: f64, max: f64) -> Result<Self, String> {
        if !min.is_finite() || !max.is_finite() || min >= max || !(max - min).is_finite() {
            return Err("density smoothstep requires finite min < max and a finite range".into());
        }
        self.clamp(min, max)?
            .binary(Binary::Subtract, &Self::constant(min)?)?
            .binary(Binary::Divide, &Self::constant(max - min)?)?
            .unary(Unary::Smoothstep)
    }
    pub fn transform(&self, offset: [f64; 3], scale: [f64; 3]) -> Result<Self, String> {
        if offset.iter().chain(&scale).any(|value| !value.is_finite()) || scale.contains(&0.0) {
            return Err(
                "density transform requires finite offsets and finite nonzero scales".into(),
            );
        }
        let mut builder = Builder::new(Vector::new());
        let mut axes = [0; 3];
        for axis in 0..3 {
            let input = builder.push(Instruction::Axis(axis as u8))?;
            let shift = builder.push(Instruction::Constant(offset[axis]))?;
            let shifted = builder.push(Instruction::Binary(Binary::Subtract, input, shift))?;
            let size = builder.push(Instruction::Constant(scale[axis]))?;
            axes[axis] = builder.push(Instruction::Binary(Binary::Divide, shifted, size))?;
        }
        let result = builder.import(self, Some(axes))?;
        Self::output(builder.code, result)
    }
    pub fn warp(&self, options: Noise, strength: f64) -> Result<Self, String> {
        options.validate()?;
        if !strength.is_finite() {
            return Err("domain warp strength must be finite".into());
        }
        if strength == 0.0 {
            return Ok(self.clone());
        }
        let mut builder = Builder::new(
            [
                Instruction::Axis(0),
                Instruction::Axis(1),
                Instruction::Axis(2),
            ]
            .into_iter()
            .collect(),
        );
        let magnitude = builder.push(Instruction::Constant(strength))?;
        let mut axes = [0, 1, 2];
        for (axis, target) in axes.iter_mut().enumerate() {
            if !options.axes.mask()[axis] {
                continue;
            }
            let options = Noise {
                seed: options.seed.wrapping_add(axis as u64),
                ..options
            };
            let noise = builder.push(Instruction::Noise {
                options,
                coordinates: [0, 1, 2],
            })?;
            let displacement =
                builder.push(Instruction::Binary(Binary::Multiply, noise, magnitude))?;
            *target = builder.push(Instruction::Binary(Binary::Add, axis, displacement))?;
        }
        let result = builder.import(self, Some(axes))?;
        Self::output(builder.code, result)
    }
    fn output(code: Vector<Instruction>, root: usize) -> Result<Self, String> {
        let mut reachable = [false; MAX_FIELD_NODES];
        let mut stack = vec![root];
        while let Some(at) = stack.pop() {
            if !reachable[at] {
                reachable[at] = true;
                stack.extend(code[at].references());
            }
        }
        if root + 1 == code.len() && reachable[..code.len()].iter().all(|value| *value) {
            return Self::finish(code);
        }
        let mut compact = Vector::new();
        let mut map = vec![0; code.len()];
        for (at, instruction) in code.iter().enumerate() {
            if reachable[at] {
                map[at] = compact.len();
                compact.push_back(instruction.remap(&map));
            }
        }
        Self::finish(compact)
    }
    pub fn sampler(&self) -> Sampler<'_> {
        Sampler {
            field: self,
            values: [0.0; MAX_FIELD_NODES],
        }
    }
    pub fn sample(&self, point: [f64; 3]) -> Result<f64, String> {
        self.sampler().sample(point)
    }
}

struct Builder {
    code: Vector<Instruction>,
    keys: HashMap<Vec<u8>, usize>,
}
impl Builder {
    fn new(code: Vector<Instruction>) -> Self {
        let keys = code
            .iter()
            .enumerate()
            .map(|(at, instruction)| {
                (
                    serde_json::to_vec(instruction).expect("finite validated instruction"),
                    at,
                )
            })
            .collect();
        Self { code, keys }
    }
    fn push(&mut self, instruction: Instruction) -> Result<usize, String> {
        let key = serde_json::to_vec(&instruction).map_err(|error| error.to_string())?;
        if let Some(at) = self.keys.get(&key) {
            return Ok(*at);
        }
        if self.code.len() == MAX_FIELD_NODES {
            return Err(format!(
                "density exceeds its {MAX_FIELD_NODES}-instruction budget; split it into layers"
            ));
        }
        let at = self.code.len();
        self.code.push_back(instruction);
        self.keys.insert(key, at);
        Ok(at)
    }
    fn import(&mut self, field: &Field, axes: Option<[usize; 3]>) -> Result<usize, String> {
        let mut map = Vec::with_capacity(field.code.len());
        for instruction in &field.code {
            let at = if let Instruction::Axis(axis) = instruction {
                if let Some(axes) = axes {
                    axes[*axis as usize]
                } else {
                    self.push(instruction.clone())?
                }
            } else {
                self.push(instruction.remap(&map))?
            };
            map.push(at);
        }
        Ok(*map.last().expect("nonempty validated field"))
    }
}

/// One scratch buffer is reused across all cells in a native raster operation.
pub struct Sampler<'a> {
    field: &'a Field,
    values: [f64; MAX_FIELD_NODES],
}
impl Sampler<'_> {
    pub fn sample(&mut self, point: [f64; 3]) -> Result<f64, String> {
        if point.iter().any(|value| !value.is_finite()) {
            return Err("density sample coordinates must be finite".into());
        }
        for (at, instruction) in self.field.code.iter().enumerate() {
            self.values[at] = math::evaluate(instruction, &self.values, point)?;
            if !self.values[at].is_finite() {
                return Err(format!(
                    "density instruction {at} produced a non-finite value at {point:?}; check divisions, powers, and field scale"
                ));
            }
        }
        Ok(self.values[self.field.code.len() - 1])
    }
}
