//! Editable typed keys compile once into shared, seekable F-curve segments.
use crate::{Fraction, List, ListLength, Number, Type, Unit, Value, math};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
pub const MAX_KEYS: usize = 4096;
pub const MAX_CHANNELS: usize = 256;
const MAX_COMPONENTS: usize = 65_536;
const VALUE_DEPTH: usize = 2;
const ARC_HEADER_BYTES: usize = 2 * std::mem::size_of::<usize>();
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Bezier,
    Linear,
    Hold,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tangents {
    Auto,
    AutoClamped,
    Manual,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handle {
    pub time: Fraction,
    pub value: Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyframe {
    pub time: Fraction,
    pub value: Value,
    pub handle_in: Option<Handle>,
    pub handle_out: Option<Handle>,
    pub interpolation: Interpolation,
    pub tangents: Tangents,
}
#[derive(Clone, Debug, PartialEq)]
struct Segment {
    outgoing: Fraction,
    incoming: Fraction,
    values: Arc<[[Fraction; 4]]>,
    interpolation: Interpolation,
}
#[derive(Clone, Debug)]
pub struct Keyframed {
    pub keys: Arc<[Keyframe]>,
    segments: Arc<[Segment]>,
    units: Arc<[Unit]>,
    ty: Type,
    revision: u64,
    constant: bool,
    value_bytes: usize,
}
use crate::math::arithmetic as calc;
fn components(value: &Value) -> Result<Vec<Number>, String> {
    let ty = value.ty();
    super::values::checked_value(&ty)?;
    crate::validate_value(&ty, value)?;
    let values = match value {
        Value::Number(n) => vec![*n],
        Value::List(items) if !items.is_empty() && items.len() <= MAX_CHANNELS => items
            .iter()
            .map(Value::number)
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(format!(
                "keyframe value must be a Number, Length, Angle, Percent, Duration, Rate, or a fixed numeric List of 1..={MAX_CHANNELS} values"
            ));
        }
    };
    if values.iter().any(|v| !math::finite(&v.value)) {
        return Err("keyframe values must be finite".into());
    }
    Ok(values)
}
fn offsets(handle: &Handle, units: &[Unit], scalar: bool) -> Result<Vec<Fraction>, String> {
    if scalar != matches!(handle.value, Value::Number(_)) {
        return Err(
            "keyframe handle value offset must have the same scalar/List shape as its key value"
                .into(),
        );
    }
    let values = components(&handle.value)?;
    if values.len() != units.len() || values.iter().zip(units).any(|(v, u)| v.unit != *u) {
        return Err(
            "keyframe handle value offset must preserve its key's List length and coordinate units"
                .into(),
        );
    }
    if !math::finite(&handle.time) {
        return Err("keyframe handle time offsets must be finite".into());
    }
    Ok(values.iter().map(|n| n.value).collect())
}
impl Keyframed {
    pub fn new(keys: Arc<[Keyframe]>) -> Result<Self, String> {
        if !(2..=MAX_KEYS).contains(&keys.len()) {
            return Err(format!("Curve(keys=...) requires 2..={MAX_KEYS} keyframes"));
        }
        if keys[0].time != 0.into() {
            return Err("the first Curve keyframe must start at 0s".into());
        }
        math::temporal::nonnegative("curve", keys[keys.len() - 1].time)?;
        let scalar = matches!(keys[0].value, Value::Number(_));
        let mut values = Vec::with_capacity(keys.len());
        let units = components(&keys[0].value)?
            .iter()
            .map(|n| n.unit)
            .collect::<Vec<_>>();
        if keys
            .len()
            .checked_mul(units.len())
            .is_none_or(|n| n > MAX_COMPONENTS)
        {
            return Err("Curve keyframes exceed the compiled component budget".into());
        }
        for (index, key) in keys.iter().enumerate() {
            if !math::finite(&key.time) || index > 0 && key.time <= keys[index - 1].time {
                return Err(format!(
                    "Curve keyframe {} must have a finite time strictly after its predecessor; duplicate times are not allowed",
                    index + 1
                ));
            }
            if scalar != matches!(key.value, Value::Number(_)) {
                return Err("Curve keyframes must preserve scalar/List shape".into());
            }
            let numbers = components(&key.value)?;
            if numbers.len() != units.len() || numbers.iter().zip(&units).any(|(n, u)| n.unit != *u)
            {
                return Err(format!(
                    "Curve keyframe {} changes value units or List length",
                    index + 1
                ));
            }
            for handle in [&key.handle_in, &key.handle_out].into_iter().flatten() {
                offsets(handle, &units, scalar)?;
            }
            if key.handle_in.as_ref().is_some_and(|h| h.time > 0.into())
                || key.handle_out.as_ref().is_some_and(|h| h.time < 0.into())
            {
                return Err(
                    "incoming handle time offsets must be <=0s; outgoing offsets must be >=0s"
                        .into(),
                );
            }
            values.push(numbers.iter().map(|n| n.value).collect::<Vec<_>>());
        }
        let mut slopes = Vec::with_capacity(keys.len() - 1);
        for i in 0..keys.len() - 1 {
            let dt = calc("-", keys[i + 1].time, keys[i].time)?;
            slopes.push(
                values[i + 1]
                    .iter()
                    .zip(&values[i])
                    .map(|(b, a)| calc("/", calc("-", *b, *a)?, dt))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        let tangent = |index: usize, channel: usize| -> Result<Fraction, String> {
            if index == 0 {
                return Ok(slopes[0][channel]);
            }
            if index == keys.len() - 1 {
                return Ok(slopes[index - 1][channel]);
            }
            math::curve::tangent(
                [
                    (keys[index - 1].time, values[index - 1][channel]),
                    (keys[index].time, values[index][channel]),
                    (keys[index + 1].time, values[index + 1][channel]),
                ],
                keys[index].tangents == Tangents::AutoClamped,
            )
        };
        let mut segments = Vec::with_capacity(keys.len() - 1);
        for i in 0..keys.len() - 1 {
            let dt = calc("-", keys[i + 1].time, keys[i].time)?;
            let third = calc("/", dt, 3.into())?;
            let out = keys[i].handle_out.as_ref();
            let incoming = keys[i + 1].handle_in.as_ref();
            if keys[i].interpolation == Interpolation::Bezier
                && ((keys[i].tangents == Tangents::Manual && out.is_none())
                    || (keys[i + 1].tangents == Tangents::Manual && incoming.is_none()))
            {
                return Err(format!(
                    "manual tangents require outgoing/incoming handles for Bézier segment {}",
                    i + 1
                ));
            }
            let ot = out.map_or(third, |h| h.time);
            let it = incoming.map_or(-third, |h| h.time);
            if ot > dt || it < -dt || ot > calc("+", dt, it)? {
                return Err(format!(
                    "Bézier segment {} time handles must remain ordered inside their keyframe interval",
                    i + 1
                ));
            }
            let out_values = out.map(|h| offsets(h, &units, scalar)).transpose()?;
            let in_values = incoming.map(|h| offsets(h, &units, scalar)).transpose()?;
            let mut control = Vec::with_capacity(units.len());
            for channel in 0..units.len() {
                let a = values[i][channel];
                let b = values[i + 1][channel];
                let out_delta = if let Some(v) = &out_values {
                    v[channel]
                } else {
                    calc("*", tangent(i, channel)?, ot)?
                };
                let in_delta = if let Some(v) = &in_values {
                    v[channel]
                } else {
                    calc("*", tangent(i + 1, channel)?, it)?
                };
                control.push([a, calc("+", a, out_delta)?, calc("+", b, in_delta)?, b]);
            }
            segments.push(Segment {
                outgoing: calc("/", ot, dt)?,
                incoming: calc("/", calc("+", dt, it)?, dt)?,
                values: control.into(),
                interpolation: keys[i].interpolation,
            });
        }
        let ty = if scalar {
            Type::Numeric(units[0])
        } else {
            Type::List(
                Box::new(Type::union(units.iter().copied().map(Type::Numeric))?),
                ListLength::Exact(units.len()),
            )
        };
        static REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 63);
        let revision = REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let constant = keys.windows(2).all(|keys| keys[0].value == keys[1].value)
            && segments.iter().all(|segment| {
                segment.interpolation != Interpolation::Bezier
                    || segment
                        .values
                        .iter()
                        .all(|values| values.iter().all(|v| *v == values[0]))
            });
        // Reuse the canonical collection/type accounting, including lazy list metadata.
        // Numeric keys have at most one List level; the compiled Curve owns this charge once.
        let mut remaining = usize::MAX;
        let mut items = usize::MAX;
        let mut resources = crate::value_budget::Budget::default();
        for key in keys.iter() {
            for value in std::iter::once(&key.value)
                .chain(key.handle_in.as_ref().map(|handle| &handle.value))
                .chain(key.handle_out.as_ref().map(|handle| &handle.value))
            {
                crate::value_budget::charge_with_budget(
                    value,
                    &mut remaining,
                    &mut items,
                    VALUE_DEPTH,
                    &mut resources,
                )?;
                if let Value::List(list) = value {
                    crate::type_budget::charge(&list.element_type, &mut remaining)?;
                    remaining = remaining
                        .checked_sub(ARC_HEADER_BYTES)
                        .ok_or("Curve key type accounting overflow")?;
                }
            }
        }
        crate::type_budget::charge(&ty, &mut remaining)?;
        let value_bytes = usize::MAX - remaining;
        Ok(Self {
            value_bytes,
            keys,
            segments: segments.into(),
            units: units.into(),
            ty,
            revision,
            constant,
        })
    }
    /// Compare compiled values, so automatic tangents invalidate their affected neighbors.
    /// A source edit to an unused handle does not invalidate an identical sampled curve.
    pub fn changed_progress(&self, next: &Self) -> Option<Fraction> {
        if self.revision == next.revision {
            return None;
        }
        if self.ty != next.ty
            || self.units != next.units
            || self.keys[0].value != next.keys[0].value
        {
            return Some(0.into());
        }
        let normalize = |time, duration| math::arithmetic("/", time, duration).ok();
        let same_duration = self.duration() == next.duration();
        for index in 0..self.segments.len().max(next.segments.len()) {
            let a = self.segments.get(index);
            let b = next.segments.get(index);
            let start = || {
                self.keys
                    .get(index)
                    .map(|key| normalize(key.time, self.duration()).unwrap_or(0.into()))
                    .into_iter()
                    .chain(
                        next.keys
                            .get(index)
                            .map(|key| normalize(key.time, next.duration()).unwrap_or(0.into())),
                    )
                    .min()
                    .unwrap_or(0.into())
            };
            let Some((a, b)) = a.zip(b) else {
                return Some(start());
            };
            let same_interval = [index, index + 1].into_iter().all(|index| {
                if same_duration {
                    self.keys[index].time == next.keys[index].time
                } else {
                    normalize(self.keys[index].time, self.duration())
                        .zip(normalize(next.keys[index].time, next.duration()))
                        .is_some_and(|(a, b)| a == b)
                }
            });
            let same_values = a.interpolation == b.interpolation
                && match a.interpolation {
                    Interpolation::Hold => a
                        .values
                        .iter()
                        .zip(b.values.iter())
                        .all(|(a, b)| a[0] == b[0]),
                    Interpolation::Linear => a
                        .values
                        .iter()
                        .zip(b.values.iter())
                        .all(|(a, b)| a[0] == b[0] && a[3] == b[3]),
                    Interpolation::Bezier => a == b,
                };
            if !same_interval || !same_values {
                return Some(start());
            }
        }
        (self.keys.last().expect("bounded keys").value
            != next.keys.last().expect("bounded keys").value)
            .then_some(1.into())
    }
    pub fn is_constant(&self) -> bool {
        self.constant
    }
    pub fn value_type(&self) -> Type {
        self.ty.clone()
    }
    pub fn duration(&self) -> Fraction {
        self.keys[self.keys.len() - 1].time
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn resident_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + (4 + self.segments.len()) * ARC_HEADER_BYTES
            + self.keys.len() * std::mem::size_of::<Keyframe>()
            + self.segments.len() * std::mem::size_of::<Segment>()
            + self.segments.len() * self.units.len() * std::mem::size_of::<[Fraction; 4]>()
            + self.units.len() * std::mem::size_of::<Unit>()
            + self.value_bytes
    }
    pub fn sample(&self, time: Fraction) -> Result<Value, String> {
        math::temporal::viewport(time)?;
        if time <= 0.into() {
            return Ok(self.keys[0].value.clone());
        }
        if time >= self.duration() {
            return Ok(self.keys[self.keys.len() - 1].value.clone());
        }
        let index = self.keys.partition_point(|key| key.time <= time) - 1;
        if time == self.keys[index].time
            || self.segments[index].interpolation == Interpolation::Hold
        {
            return Ok(self.keys[index].value.clone());
        }
        let p = calc(
            "/",
            calc("-", time, self.keys[index].time)?,
            calc("-", self.keys[index + 1].time, self.keys[index].time)?,
        )?;
        let segment = &self.segments[index];
        let t = if segment.interpolation == Interpolation::Bezier {
            math::curve::parameter(p, segment.outgoing, segment.incoming)?
        } else {
            p
        };
        let values = segment
            .values
            .iter()
            .zip(self.units.iter())
            .map(|(v, u)| {
                Ok(Value::Number(Number {
                    value: if segment.interpolation == Interpolation::Linear {
                        math::visual_lerp(v[0], v[3], t)?
                    } else {
                        math::curve::bezier(*v, t)?
                    },
                    unit: *u,
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        if matches!(self.ty, Type::List(..)) {
            Ok(Value::List(List::from(values)))
        } else {
            Ok(values.into_iter().next().expect("scalar channel"))
        }
    }
}

impl PartialEq for Keyframed {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision || self.keys == other.keys
    }
}
