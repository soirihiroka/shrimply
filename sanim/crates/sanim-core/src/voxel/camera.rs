//! Camera recipes and shared curves are immutable values built by canonical native operations.
use crate::{Fraction, List, ListLength, Number, Type, Unit, Value, math};
use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Camera,
    OrbitCamera,
    Curve,
    Keyframe,
    CurveAt,
    CurveSample,
    CurveDuration,
    CurveDirection,
    Pan,
    Offset,
    Dolly,
    DollyZoom,
    Zoom,
    Roll,
    LookAt,
}
const OPERATIONS: &[(Op, &str)] = &[
    (Op::Camera, "Camera"),
    (Op::OrbitCamera, "OrbitCamera"),
    (Op::Curve, "Curve"),
    (Op::Keyframe, "Keyframe"),
    (Op::CurveSample, "Curve.sample"),
    (Op::CurveDuration, "Curve.duration"),
    (Op::CurveAt, "Curve.at"),
    (Op::CurveDirection, "Curve.direction"),
    (Op::Pan, "Camera.pan"),
    (Op::Offset, "Camera.offset"),
    (Op::Dolly, "Camera.dolly"),
    (Op::DollyZoom, "Camera.dolly_zoom"),
    (Op::Zoom, "Camera.zoom"),
    (Op::Roll, "Camera.roll"),
    (Op::LookAt, "Camera.look_at"),
];
impl Op {
    pub fn all() -> impl Iterator<Item = Self> {
        OPERATIONS.iter().map(|(op, _)| *op)
    }
    pub fn name(self) -> &'static str {
        OPERATIONS
            .iter()
            .find(|(op, _)| *op == self)
            .expect("registered camera operation")
            .1
    }
    pub fn from_name(name: &str) -> Option<Self> {
        OPERATIONS
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(op, _)| *op)
    }
    pub fn is_method(self) -> bool {
        self.name().contains('.')
    }
}
fn numbers(length: usize) -> Type {
    Type::List(
        Box::new(Type::Numeric(Unit::Number)),
        ListLength::Exact(length),
    )
}
pub fn curve_type() -> Type {
    Type::Curve(Box::new(crate::curve::values::value_type()))
}
fn camera_curve_type() -> Type {
    Type::Curve(Box::new(super::config::coordinate_type()))
}
fn choices(values: &[&str]) -> Type {
    Type::union(values.iter().map(|v| Type::StringLiteral((*v).into()))).expect("camera choices")
}

struct Field {
    name: &'static str,
    ty: Type,
    value: Value,
}
fn scalar(value: i64, unit: Unit) -> Value {
    Value::Number(Number {
        value: Fraction::from(value),
        unit,
    })
}
fn list(values: &[i64], unit: Unit) -> Value {
    Value::List(List::from(
        values.iter().map(|v| scalar(*v, unit)).collect::<Vec<_>>(),
    ))
}
fn coordinates(values: [(u64, u64); 3]) -> Value {
    Value::List(List::from(
        values
            .into_iter()
            .map(|(a, b)| {
                Value::Number(Number {
                    value: Fraction::new(a, b),
                    unit: Unit::Percent,
                })
            })
            .collect::<Vec<_>>(),
    ))
}
fn fields() -> &'static [Field] {
    static FIELDS: OnceLock<Vec<Field>> = OnceLock::new();
    FIELDS.get_or_init(|| {
        let num = Type::Numeric(Unit::Number);
        let angle = Type::Numeric(Unit::Angle);
        vec![
            Field {
                name: "position",
                ty: Type::Option(Box::new(super::config::coordinate_type())),
                value: Value::Some(Arc::new(coordinates([(6, 5), (7, 10), (6, 5)]))),
            },
            Field {
                name: "target",
                ty: Type::Option(Box::new(super::config::coordinate_type())),
                value: Value::Some(Arc::new(coordinates([(1, 2), (1, 4), (1, 2)]))),
            },
            Field {
                name: "path",
                ty: Type::Option(Box::new(camera_curve_type())),
                value: Value::None,
            },
            Field {
                name: "progress",
                ty: num.clone(),
                value: scalar(0, Unit::Number),
            },
            Field {
                name: "look_ahead",
                ty: num.clone(),
                value: scalar(4, Unit::Number),
            },
            Field {
                name: "up",
                ty: numbers(3),
                value: list(&[0, 1, 0], Unit::Number),
            },
            Field {
                name: "projection",
                ty: choices(&["perspective", "orthographic"]),
                value: Value::String("perspective".into()),
            },
            Field {
                name: "fov",
                ty: angle.clone(),
                value: scalar(45, Unit::Angle),
            },
            Field {
                name: "height",
                ty: num.clone(),
                value: scalar(100, Unit::Number),
            },
            Field {
                name: "roll",
                ty: angle.clone(),
                value: scalar(0, Unit::Angle),
            },
            Field {
                name: "zoom",
                ty: num.clone(),
                value: scalar(1, Unit::Number),
            },
            Field {
                name: "pan",
                ty: numbers(2),
                value: list(&[0, 0], Unit::Number),
            },
            Field {
                name: "offset",
                ty: numbers(3),
                value: list(&[0, 0, 0], Unit::Number),
            },
            Field {
                name: "dolly",
                ty: num.clone(),
                value: scalar(0, Unit::Number),
            },
            Field {
                name: "lock_framing",
                ty: Type::Bool,
                value: Value::Bool(false),
            },
            Field {
                name: "orbit",
                ty: Type::Bool,
                value: Value::Bool(false),
            },
            Field {
                name: "radius",
                ty: num,
                value: scalar(100, Unit::Number),
            },
            Field {
                name: "azimuth",
                ty: angle.clone(),
                value: scalar(45, Unit::Angle),
            },
            Field {
                name: "elevation",
                ty: angle,
                value: scalar(30, Unit::Angle),
            },
        ]
    })
}
pub fn camera_type() -> Type {
    Type::Record(
        fields()
            .iter()
            .map(|f| (f.name.into(), f.ty.clone()))
            .collect(),
    )
}
pub fn signature(op: Op) -> (Vec<(&'static str, Type, bool)>, Type) {
    let num = Type::Numeric(Unit::Number);
    match op {
        Op::Curve => (
            vec![
                (
                    "points",
                    Type::List(
                        Box::new(Type::List(
                            Box::new(
                                Type::union(
                                    [Unit::Number, Unit::Length, Unit::Percent, Unit::Angle]
                                        .map(Type::Numeric),
                                )
                                .expect("geometric units"),
                            ),
                            ListLength::Unknown,
                        )),
                        ListLength::Unknown,
                    ),
                    false,
                ),
                (
                    "keys",
                    Type::List(
                        Box::new(crate::curve::values::keyframe_type(
                            crate::curve::values::value_type(),
                        )),
                        ListLength::Unknown,
                    ),
                    false,
                ),
                ("interpolation", choices(&["linear", "centripetal"]), false),
                ("closed", Type::Bool, false),
            ],
            curve_type(),
        ),
        Op::Keyframe => (
            crate::curve::values::parameters(crate::curve::values::value_type()),
            crate::curve::values::keyframe_type(crate::curve::values::value_type()),
        ),
        Op::CurveSample => (
            vec![
                ("curve", curve_type(), true),
                ("time", Type::Numeric(Unit::Duration), true),
            ],
            crate::curve::values::value_type(),
        ),
        Op::CurveDuration => (
            vec![("curve", curve_type(), true)],
            Type::Numeric(Unit::Duration),
        ),
        Op::CurveAt | Op::CurveDirection => (
            vec![("curve", curve_type(), true), ("progress", num, true)],
            match curve_type() {
                Type::Curve(t) => *t,
                _ => unreachable!(),
            },
        ),
        Op::Camera | Op::OrbitCamera => {
            let parameters = fields()
                .iter()
                .filter(|f| {
                    let common = matches!(
                        f.name,
                        "target" | "up" | "projection" | "fov" | "height" | "roll" | "zoom"
                    );
                    common
                        || if op == Op::Camera {
                            matches!(f.name, "position" | "path" | "progress" | "look_ahead")
                        } else {
                            matches!(f.name, "radius" | "azimuth" | "elevation")
                        }
                })
                .map(|f| {
                    (
                        f.name,
                        if let Type::Option(t) = &f.ty {
                            *t.clone()
                        } else {
                            f.ty.clone()
                        },
                        false,
                    )
                })
                .collect();
            (parameters, camera_type())
        }
        _ => {
            let (name, ty) = match op {
                Op::Pan => ("by", numbers(2)),
                Op::Offset => ("by", numbers(3)),
                Op::LookAt => ("target", super::config::coordinate_type()),
                Op::Zoom => ("factor", num),
                Op::Roll => ("angle", Type::Numeric(Unit::Angle)),
                _ => ("by", num),
            };
            (
                vec![("camera", camera_type(), true), (name, ty, true)],
                camera_type(),
            )
        }
    }
}
pub fn checked_result(
    op: Op,
    actuals: &std::collections::BTreeMap<String, Type>,
    result: Type,
) -> Result<Type, String> {
    if op == Op::Keyframe {
        let value = actuals.get("value").ok_or("Keyframe requires value")?;
        crate::curve::values::checked_value(value)?;
        for name in ["handle_in", "handle_out"] {
            if let Some(actual) = actuals.get(name) {
                let wanted = crate::curve::values::handle_type(value.clone());
                if !wanted.accepts(actual) {
                    return Err(format!(
                        "Keyframe {name} must preserve the key value's units and List shape: expected {wanted}, found {actual}"
                    ));
                }
            }
        }
        return Ok(crate::curve::values::keyframe_type(value.clone()));
    }
    if op == Op::Curve {
        match (actuals.get("points"), actuals.get("keys")) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(
                    "Curve requires exactly one of points=... or keys=[Keyframe(...), ...]".into(),
                );
            }
            (None, Some(Type::List(key, _))) => {
                if actuals.contains_key("closed") || actuals.contains_key("interpolation") {
                    return Err("keyframed Curve uses each Keyframe's interpolation; closed/interpolation belong only to geometric points curves".into());
                }
                let Type::Record(fields) = key.as_ref() else {
                    return Err("Curve keys must be Keyframe(...) values".into());
                };
                let value = fields
                    .get("value")
                    .ok_or("Curve keys require Keyframe values")?;
                crate::curve::values::checked_value(value)?;
                return Ok(Type::Curve(Box::new(value.clone())));
            }
            (Some(Type::List(point, _)), None) => {
                if !matches!(point.as_ref(), Type::List(_, ListLength::Exact(2 | 3))) {
                    return Err(
                        "Curve control points must have a known List length of 2 or 3".into(),
                    );
                }
                return Ok(Type::Curve(point.clone()));
            }
            _ => return Err("Curve requires a List of control points or Keyframe values".into()),
        }
    }
    if matches!(op, Op::CurveAt | Op::CurveDirection | Op::CurveSample) {
        let Some(Type::Curve(point)) = actuals.get("curve") else {
            return Err("curve method requires Curve(...)".into());
        };
        return if matches!(op, Op::CurveAt | Op::CurveSample) {
            Ok(*point.clone())
        } else {
            match point.as_ref() {
                Type::List(_, length) => {
                    Ok(Type::List(Box::new(Type::Numeric(Unit::Number)), *length))
                }
                _ => Err("Curve direction requires numeric Lists".into()),
            }
        };
    }
    if op == Op::Camera && actuals.contains_key("path") && actuals.contains_key("position") {
        return Err("Camera accepts position or path, not both".into());
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SpecWire", into = "SpecWire")]
pub struct Spec {
    values: OrdMap<String, Value>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpecWire {
    values: OrdMap<String, Value>,
}
impl From<Spec> for SpecWire {
    fn from(spec: Spec) -> Self {
        Self {
            values: spec.values,
        }
    }
}
impl TryFrom<SpecWire> for Spec {
    type Error = String;
    fn try_from(wire: SpecWire) -> Result<Self, String> {
        Self::new(&Value::Record(wire.values))
    }
}
impl Spec {
    pub fn new(value: &Value) -> Result<Self, String> {
        crate::validate_value(&camera_type(), value)
            .map_err(|error| format!("camera requires Camera(...) or OrbitCamera(...): {error}"))?;
        let Value::Record(values) = value else {
            return Err("camera requires a typed Camera value".into());
        };
        let spec = Self {
            values: values.clone(),
        };
        spec.validate()?;
        Ok(spec)
    }
    fn validate(&self) -> Result<(), String> {
        for name in ["zoom", "height", "radius"] {
            if self.number(name)? <= 0.0 {
                return Err(format!("Camera {name} must be positive and finite"));
            }
        }
        if !(0.0..179.0).contains(&self.number("fov")?) || self.number("fov")? == 0.0 {
            return Err("Camera fov must be between 0deg and 179deg".into());
        }
        if self.number("look_ahead")? < 0.0 {
            return Err("Camera look_ahead cannot be negative".into());
        }
        Ok(())
    }
    pub fn value(&self) -> Value {
        Value::Record(self.values.clone())
    }
    pub fn curve(&self) -> Option<&Arc<crate::curve::Curve>> {
        match self.values.get("path") {
            Some(Value::Some(v)) => match v.as_ref() {
                Value::Curve(c) => Some(c),
                _ => None,
            },
            _ => None,
        }
    }
    pub fn resident_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.values.len() * (std::mem::size_of::<(String, Value)>() + 64)
    }
    pub(crate) fn number(&self, name: &str) -> Result<f64, String> {
        math::to_f64(self.values[name].number()?.value)
    }
    pub(crate) fn value_ref(&self, name: &str) -> &Value {
        &self.values[name]
    }
    pub fn resolve(
        &self,
        dimensions: [u32; 3],
        viewport: [u32; 2],
    ) -> Result<sanim_voxel::Camera, String> {
        math::camera::resolve(self, dimensions, viewport)
    }
}

pub fn execute(op: Op, args: &OrdMap<String, Value>) -> Result<Value, String> {
    if op == Op::Keyframe {
        return crate::curve::values::keyframe(args);
    }
    if op == Op::Curve && args.contains_key("keys") {
        if args.contains_key("points")
            || args.contains_key("closed")
            || args.contains_key("interpolation")
        {
            return Err(
                "Curve(keys=...) cannot also specify points, closed, or interpolation".into(),
            );
        }
        let keys = args["keys"].list()?;
        if keys.len() > crate::curve::MAX_KEYS {
            return Err("Curve exceeds its keyframe budget".into());
        }
        return Ok(Value::Curve(Arc::new(crate::curve::Curve::keyed(
            keys.iter()
                .map(crate::curve::values::parse)
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        )?)));
    }
    if op == Op::CurveDuration {
        let Value::Curve(curve) = &args["curve"] else {
            return Err("Curve.duration requires Curve(...)".into());
        };
        return Ok(crate::curve::values::duration_value(
            curve.duration().ok_or(
                "geometric points curves have no clock; use Keyframe timestamps to define duration",
            )?,
        ));
    }
    if op == Op::Curve {
        let interpolation = match args
            .get("interpolation")
            .map(Value::string)
            .transpose()?
            .unwrap_or("centripetal")
        {
            "linear" => sanim_voxel::camera::path::Interpolation::Linear,
            "centripetal" => Default::default(),
            _ => return Err("Curve interpolation must be linear or centripetal".into()),
        };
        let closed = matches!(args.get("closed"), Some(Value::Bool(true)));
        return Ok(Value::Curve(Arc::new(crate::curve::Curve::new(
            args.get("points").ok_or("Curve requires points or keys")?,
            interpolation,
            closed,
        )?)));
    }
    if matches!(op, Op::CurveAt | Op::CurveDirection | Op::CurveSample) {
        let Value::Curve(curve) = &args["curve"] else {
            return Err("curve operation requires Curve(...)".into());
        };
        return match op {
            Op::CurveAt => curve.at(args["progress"].number()?.value),
            Op::CurveSample => curve.sample_time(args["time"].number()?.value),
            _ => curve.direction(math::to_f64(args["progress"].number()?.value)?),
        };
    }
    let mut values = if matches!(op, Op::Camera | Op::OrbitCamera) {
        fields()
            .iter()
            .map(|f| (f.name.to_string(), f.value.clone()))
            .collect::<OrdMap<_, _>>()
    } else {
        let Value::Record(values) = &args["camera"] else {
            return Err("camera method requires a Camera value".into());
        };
        values.clone()
    };
    if matches!(op, Op::Camera | Op::OrbitCamera) {
        if args.contains_key("path") && args.contains_key("position") {
            return Err("Camera accepts position or path, not both; put positions into Curve control points".into());
        }
        for (name, value) in args {
            let value = if matches!(name.as_str(), "position" | "target" | "path") {
                Value::Some(Arc::new(value.clone()))
            } else {
                value.clone()
            };
            values.insert(name.clone(), value);
        }
        if args.contains_key("path") {
            values.insert("position".into(), Value::None);
            if !args.contains_key("target") {
                values.insert("target".into(), Value::None);
            }
        }
        values.insert("orbit".into(), Value::Bool(op == Op::OrbitCamera));
    } else {
        let field = match op {
            Op::Pan => "pan",
            Op::Offset => "offset",
            Op::Dolly | Op::DollyZoom => "dolly",
            Op::Zoom => "zoom",
            Op::Roll => "roll",
            Op::LookAt => "target",
            _ => unreachable!(),
        };
        let argument = match op {
            Op::LookAt => "target",
            Op::Zoom => "factor",
            Op::Roll => "angle",
            _ => "by",
        };
        if op == Op::LookAt {
            values.insert(field.into(), Value::Some(Arc::new(args[argument].clone())));
        } else {
            let value = math::operations::binary(
                if op == Op::Zoom { "*" } else { "+" },
                values[field].clone(),
                args[argument].clone(),
            )?;
            values.insert(field.into(), value);
        }
        if op == Op::DollyZoom {
            values.insert("lock_framing".into(), Value::Bool(true));
        }
    }
    let result = Value::Record(values);
    Spec::new(&result)?;
    Ok(result)
}

/// Reserve native preparation work before compiling a curve's immutable segments.
pub fn work(op: Op, args: &OrdMap<String, Value>) -> Result<usize, String> {
    const COMPILE_WORK_PER_COMPONENT: usize = 96;
    const SAMPLE_WORK_PER_CHANNEL: usize = 16;
    const SAMPLE_TIME_SOLVE_WORK: usize = 64;
    if op == Op::Curve
        && let Some(keys) = args.get("keys")
    {
        let keys = keys.list()?;
        if keys.len() > crate::curve::MAX_KEYS {
            return Err("Curve exceeds its keyframe budget".into());
        }
        let channels = keys
            .get(0)
            .and_then(|value| {
                if let Value::Record(fields) = value {
                    fields.get("value")
                } else {
                    None
                }
            })
            .map_or(1, |value| {
                if let Value::List(values) = value {
                    values.len()
                } else {
                    1
                }
            });
        return keys
            .len()
            .checked_mul(channels)
            .and_then(|n| n.checked_mul(COMPILE_WORK_PER_COMPONENT))
            .ok_or("Curve compilation work overflow".into());
    }
    if matches!(op, Op::CurveAt | Op::CurveSample | Op::CurveDirection)
        && let Some(Value::Curve(curve)) = args.get("curve")
    {
        return curve
            .dimension()
            .checked_mul(SAMPLE_WORK_PER_CHANNEL)
            .and_then(|n| n.checked_add(SAMPLE_TIME_SOLVE_WORK))
            .ok_or("Curve sampling work overflow".into());
    }
    Ok(0)
}
