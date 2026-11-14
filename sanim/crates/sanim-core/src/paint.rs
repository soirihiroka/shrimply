//! A shape has exactly one native paint; gradients remain typed, inspectable values.
use crate::{Color, Fraction, List, Number, Type, Unit, Value, math};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
pub const MAX_GRADIENT_STOPS: usize = 64;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub offset: Fraction,
    pub color: Color,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Paint {
    Solid(Color),
    LinearGradient {
        angle: Fraction,
        stops: Arc<[GradientStop]>,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub paint: Paint,
    pub width: Fraction,
}
impl From<Color> for Paint {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}
impl Paint {
    pub fn gradient_type() -> Type {
        Type::Record(
            [
                ("angle".to_string(), Type::Numeric(Unit::Angle)),
                (
                    "stops".to_string(),
                    Type::List(
                        Box::new(Type::Record(
                            [
                                ("offset".to_string(), Type::Numeric(Unit::Number)),
                                ("color".to_string(), Type::Color),
                            ]
                            .into_iter()
                            .collect(),
                        )),
                        crate::ListLength::Unknown,
                    ),
                ),
            ]
            .into_iter()
            .collect(),
        )
    }
    pub fn ty() -> Type {
        Type::union([Type::Color, Self::gradient_type()]).expect("distinct paint types")
    }
    pub fn from_value(value: &Value) -> Result<Self, String> {
        if let Value::Color(color) = value {
            return Ok(Self::Solid(color.clone()));
        }
        let Value::Record(fields) = value else {
            return Err("fill requires Color(...) or LinearGradient(...)".into());
        };
        if !Self::gradient_type().accepts(&value.ty()) {
            return Err(format!(
                "fill requires Color or LinearGradient, found {}",
                value.ty()
            ));
        }
        let angle = fields["angle"].number()?.value;
        if !math::finite(&angle) {
            return Err("gradient angle must be finite".into());
        }
        let rows = fields["stops"].list()?;
        if !(2..=MAX_GRADIENT_STOPS).contains(&rows.len()) {
            return Err(format!(
                "LinearGradient requires 2 to {MAX_GRADIENT_STOPS} color stops"
            ));
        }
        let mut stops = Vec::with_capacity(rows.len());
        let mut previous = None;
        for (index, row) in rows.iter().enumerate() {
            let Value::Record(fields) = row else {
                unreachable!("typed stop")
            };
            let offset = fields["offset"].number()?.value;
            if !math::finite(&offset)
                || !(Fraction::from(0)..=Fraction::from(1)).contains(&offset)
                || previous.is_some_and(|previous| offset < previous)
            {
                return Err(format!(
                    "gradient stop {} requires an offset in [0,1], in nondecreasing order",
                    index + 1
                ));
            }
            let Value::Color(color) = &fields["color"] else {
                unreachable!("typed stop")
            };
            if color.0.iter().any(|channel| {
                !math::finite(channel) || !(Fraction::from(0)..=Fraction::from(1)).contains(channel)
            }) {
                return Err(format!(
                    "gradient stop {} has an invalid color channel",
                    index + 1
                ));
            }
            stops.push(GradientStop {
                offset,
                color: color.clone(),
            });
            previous = Some(offset);
        }
        if stops[0].offset != Fraction::from(0)
            || stops.last().expect("gradient stops").offset != Fraction::from(1)
        {
            return Err("gradient stops must start at offset 0 and end at offset 1".into());
        }
        Ok(Self::LinearGradient {
            angle,
            stops: stops.into(),
        })
    }
    pub fn value(&self) -> Value {
        match self {
            Self::Solid(color) => Value::Color(color.clone()),
            Self::LinearGradient { angle, stops } => Value::Record(
                [
                    (
                        "angle".to_string(),
                        Value::Number(Number {
                            value: *angle,
                            unit: Unit::Angle,
                        }),
                    ),
                    (
                        "stops".to_string(),
                        Value::List(List::from(
                            stops
                                .iter()
                                .map(|stop| {
                                    Value::Record(
                                        [
                                            (
                                                "offset".to_string(),
                                                Value::Number(Number {
                                                    value: stop.offset,
                                                    unit: Unit::Number,
                                                }),
                                            ),
                                            ("color".to_string(), Value::Color(stop.color.clone())),
                                        ]
                                        .into_iter()
                                        .collect(),
                                    )
                                })
                                .collect::<Vec<_>>(),
                        )),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
        }
    }
}
