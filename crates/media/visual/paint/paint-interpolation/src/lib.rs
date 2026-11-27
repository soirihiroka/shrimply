use std::sync::{Arc, LazyLock};

use glam::Vec2;
use serde::{Deserialize, Serialize};
use shrimply_timeline_value::{
    Interpolation, Time, TimelineExpressionValue, TimelineKeyframe, TimelineValueType,
    expression::{Fraction, List, ListLength, Number, Type, Unit, Value},
    expression_f32, expression_number,
};
use uuid::Uuid;

mod math;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PaintDrawing {
    pub strokes: Vec<PaintStroke>,
    pub fills: Vec<PaintFill>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaintDrawingKeyframe {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub time: Time,
    pub value: PaintDrawing,
    #[serde(default = "jump_interpolation")]
    pub interpolation_to_next: Interpolation,
}

const fn jump_interpolation() -> Interpolation {
    Interpolation::Jump
}

impl TimelineKeyframe<PaintDrawing> for PaintDrawingKeyframe {
    fn id(&self) -> Uuid {
        self.id
    }

    fn id_mut(&mut self) -> &mut Uuid {
        &mut self.id
    }

    fn time(&self) -> Time {
        self.time
    }

    fn time_mut(&mut self) -> &mut Time {
        &mut self.time
    }

    fn value(&self) -> &PaintDrawing {
        &self.value
    }

    fn value_mut(&mut self) -> &mut PaintDrawing {
        &mut self.value
    }
}

impl TimelineValueType for PaintDrawing {
    type Keyframe = PaintDrawingKeyframe;

    fn default_value() -> Self {
        Self::default()
    }

    fn keyframe(time: Time, value: Self) -> Self::Keyframe {
        PaintDrawingKeyframe {
            id: Uuid::new_v4(),
            time,
            value,
            interpolation_to_next: Interpolation::Jump,
        }
    }

    fn value_at(keyframes: &[Self::Keyframe], time: Time) -> Self {
        let Some(right_index) = keyframes.iter().position(|keyframe| keyframe.time > time) else {
            return keyframes
                .last()
                .map(|keyframe| keyframe.value.clone())
                .unwrap_or_default();
        };
        if right_index == 0 {
            return keyframes[0].value.clone();
        }
        let left = &keyframes[right_index - 1];
        let right = &keyframes[right_index];
        if left.interpolation_to_next == Interpolation::Jump {
            return left.value.clone();
        }
        let duration = right.time.signed_sub(left.time).as_secs_f64();
        if duration <= f64::EPSILON {
            return right.value.clone();
        }
        let progress = time.signed_sub(left.time).as_secs_f64() / duration;
        let progress = left.interpolation_to_next.value(progress);
        math::interpolate_drawing(&left.value, &right.value, progress.clamp(0.0, 1.0) as f32)
    }
}

const NUMBER: Type = Type::Numeric(Unit::Number);

fn record_type<const N: usize>(fields: [(&str, Type); N]) -> Type {
    Type::Record(
        fields
            .into_iter()
            .map(|(name, ty)| (name.to_owned(), ty))
            .collect(),
    )
}

fn list_type(element: Type) -> Type {
    Type::List(Box::new(element), ListLength::Unknown)
}

static POINT: LazyLock<Type> = LazyLock::new(|| {
    record_type([
        ("position", Vec2::expression_type()),
        ("pressure", Type::Option(Box::new(NUMBER))),
    ])
});

static STROKE: LazyLock<Type> = LazyLock::new(|| {
    record_type([
        ("points", list_type(POINT.clone())),
        ("width_scale", NUMBER),
        ("color_index", NUMBER),
    ])
});

static FILL: LazyLock<Type> = LazyLock::new(|| {
    record_type([
        ("seed", Vec2::expression_type()),
        ("loops", list_type(list_type(Vec2::expression_type()))),
        ("color_index", NUMBER),
    ])
});

impl TimelineExpressionValue for PaintDrawing {
    fn expression_type() -> Type {
        record_type([
            ("strokes", list_type(STROKE.clone())),
            ("fills", list_type(FILL.clone())),
        ])
    }

    fn expression_input(&self) -> Result<Value, String> {
        let strokes = expression_list(
            &STROKE,
            self.strokes.iter().map(|stroke| {
                let points = expression_list(
                    &POINT,
                    stroke.points.iter().map(|point| {
                        let pressure = point
                            .pressure
                            .map(expression_number)
                            .transpose()?
                            .map_or(Value::None, |value| Value::Some(Arc::new(value)));
                        Ok(record([
                            ("position", point.position.expression_input()?),
                            ("pressure", pressure),
                        ]))
                    }),
                )?;
                Ok(record([
                    ("points", points),
                    ("width_scale", expression_number(stroke.width_scale)?),
                    ("color_index", expression_index(stroke.color_index)),
                ]))
            }),
        )?;
        let boundary_type = Vec2::expression_type();
        let loops_type = list_type(boundary_type.clone());
        let fills = expression_list(
            &FILL,
            self.fills.iter().map(|fill| {
                let loops = expression_list(
                    &loops_type,
                    fill.loops.iter().map(|boundary| {
                        expression_list(
                            &boundary_type,
                            boundary
                                .iter()
                                .map(TimelineExpressionValue::expression_input),
                        )
                    }),
                )?;
                Ok(record([
                    ("seed", fill.seed.expression_input()?),
                    ("loops", loops),
                    ("color_index", expression_index(fill.color_index)),
                ]))
            }),
        )?;
        Ok(record([("strokes", strokes), ("fills", fills)]))
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        let Value::Record(output) = output else {
            return None;
        };
        let strokes = output.get("strokes")?.list().ok()?;
        let fills = output.get("fills")?.list().ok()?;
        if strokes.len() != self.strokes.len() || fills.len() != self.fills.len() {
            return None;
        }
        let mut drawing = self.clone();
        for (output, stroke) in strokes.iter().zip(&mut drawing.strokes) {
            let Value::Record(output) = output else {
                return None;
            };
            let points = output.get("points")?.list().ok()?;
            if points.len() != stroke.points.len() {
                return None;
            }
            stroke.width_scale = expression_f32(output.get("width_scale")?)?.max(0.0);
            stroke.color_index = index(output.get("color_index")?)?;
            for (output, point) in points.iter().zip(&mut stroke.points) {
                let Value::Record(output) = output else {
                    return None;
                };
                point.position = Vec2::ZERO.expression_output(output.get("position")?.clone())?;
                point.pressure = match output.get("pressure")? {
                    Value::None => None,
                    Value::Some(value) => Some(expression_f32(value)?.clamp(0.0, 1.0)),
                    _ => return None,
                };
            }
        }
        for (output, fill) in fills.iter().zip(&mut drawing.fills) {
            let Value::Record(output) = output else {
                return None;
            };
            fill.seed = Vec2::ZERO.expression_output(output.get("seed")?.clone())?;
            fill.color_index = index(output.get("color_index")?)?;
            let loops = output.get("loops")?.list().ok()?;
            if loops.len() != fill.loops.len() {
                return None;
            }
            for (output, boundary) in loops.iter().zip(&mut fill.loops) {
                let points = output.list().ok()?;
                if points.len() != boundary.len() {
                    return None;
                }
                for (output, point) in points.iter().zip(boundary) {
                    *point = Vec2::ZERO.expression_output(output.clone())?;
                }
            }
        }
        Some(drawing)
    }
}

fn record<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Record(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn expression_list(
    element: &Type,
    values: impl Iterator<Item = Result<Value, String>>,
) -> Result<Value, String> {
    Ok(Value::List(List::typed(
        element.clone(),
        values.collect::<Result<Vec<_>, _>>()?,
    )))
}

fn expression_index(index: usize) -> Value {
    Value::Number(Number {
        value: Fraction::from(index as u64),
        unit: Unit::Number,
    })
}

fn index(value: &Value) -> Option<usize> {
    value.number().ok()?.usize().ok()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaintStroke {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(default = "Uuid::new_v4")]
    pub correspondence_id: Uuid,
    #[serde(default = "default_stroke_width_scale")]
    pub width_scale: f32,
    #[serde(default)]
    pub color_index: usize,
    pub points: Vec<PaintPoint>,
}

impl PaintStroke {
    pub fn new(points: Vec<PaintPoint>, width_scale: f32, color_index: usize) -> Self {
        Self {
            id: Uuid::new_v4(),
            correspondence_id: Uuid::new_v4(),
            width_scale,
            color_index,
            points,
        }
    }
}

fn default_stroke_width_scale() -> f32 {
    1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaintPoint {
    pub position: Vec2,
    pub pressure: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaintFill {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(default = "Uuid::new_v4")]
    pub correspondence_id: Uuid,
    pub seed: Vec2,
    #[serde(default)]
    pub color_index: usize,
    #[serde(default)]
    pub loops: Vec<Vec<Vec2>>,
}

impl PaintFill {
    pub fn new(seed: Vec2, loops: Vec<Vec<Vec2>>, color_index: usize) -> Self {
        Self {
            id: Uuid::new_v4(),
            correspondence_id: Uuid::new_v4(),
            seed,
            color_index,
            loops,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, y: f32) -> PaintPoint {
        PaintPoint {
            position: Vec2::new(x, y),
            pressure: Some(0.5),
        }
    }

    fn stroke(points: &[(f32, f32)]) -> PaintStroke {
        PaintStroke::new(points.iter().map(|&(x, y)| point(x, y)).collect(), 1.0, 0)
    }

    fn midpoint(from: PaintDrawing, to: PaintDrawing) -> PaintDrawing {
        let mut left = PaintDrawing::keyframe(Time::ZERO, from);
        left.interpolation_to_next = Interpolation::Linear;
        PaintDrawing::value_at(
            &[left, PaintDrawing::keyframe(Time::from_fraction(2, 1), to)],
            Time::from_fraction(1, 1),
        )
    }

    #[test]
    fn expression_round_trips_and_rejects_topology_changes() {
        let drawing = PaintDrawing {
            strokes: vec![stroke(&[(0.0, 0.0), (2.0, 0.0)])],
            fills: vec![PaintFill::new(Vec2::ZERO, vec![vec![Vec2::X]], 0)],
        };
        let input = drawing.expression_input().unwrap();
        shrimply_timeline_value::expression::validate_value(
            &PaintDrawing::expression_type(),
            &input,
        )
        .unwrap();
        assert_eq!(drawing.expression_output(input).unwrap(), drawing);
        let empty = PaintDrawing::default().expression_input().unwrap();
        assert!(drawing.expression_output(empty).is_none());
    }

    #[test]
    fn jump_in_graph_holds_the_left_drawing() {
        let from = PaintDrawing {
            strokes: vec![stroke(&[(0.0, 0.0), (2.0, 0.0)])],
            fills: Vec::new(),
        };
        let drawing = PaintDrawing::value_at(
            &[
                PaintDrawing::keyframe(Time::ZERO, from.clone()),
                PaintDrawing::keyframe(
                    Time::from_fraction(2, 1),
                    PaintDrawing {
                        strokes: vec![stroke(&[(0.0, 2.0), (2.0, 2.0)])],
                        fills: Vec::new(),
                    },
                ),
            ],
            Time::from_fraction(1, 1),
        );

        assert_eq!(drawing, from);
    }

    #[test]
    fn morphs_positions_and_pressure_at_intermediate_time() {
        let drawing = midpoint(
            PaintDrawing {
                strokes: vec![stroke(&[(0.0, 0.0), (2.0, 0.0)])],
                fills: Vec::new(),
            },
            PaintDrawing {
                strokes: vec![PaintStroke::new(
                    vec![
                        PaintPoint {
                            position: Vec2::new(2.0, 2.0),
                            pressure: Some(1.0),
                        },
                        PaintPoint {
                            position: Vec2::new(3.0, 2.0),
                            pressure: Some(1.0),
                        },
                        PaintPoint {
                            position: Vec2::new(4.0, 2.0),
                            pressure: Some(1.0),
                        },
                    ],
                    1.0,
                    0,
                )],
                fills: Vec::new(),
            },
        );

        assert_eq!(drawing.strokes[0].points.len(), 3);
        assert!(drawing.strokes[0].points[0].position.y > 0.0);
        assert!(drawing.strokes[0].points[0].position.y < 2.0);
        assert_eq!(drawing.strokes[0].points[0].pressure, Some(0.75));
    }

    #[test]
    fn splits_lines_when_no_safe_merge_can_equalize_counts() {
        let drawing = midpoint(
            PaintDrawing {
                strokes: vec![stroke(&[(0.0, 0.0), (10.0, 0.0)])],
                fills: Vec::new(),
            },
            PaintDrawing {
                strokes: vec![
                    stroke(&[(0.0, 10.0), (4.0, 10.0)]),
                    stroke(&[(6.0, 10.0), (10.0, 10.0)]),
                ],
                fills: Vec::new(),
            },
        );

        assert_eq!(drawing.strokes.len(), 2);
        assert!(drawing.strokes.iter().all(|stroke| {
            stroke.points.len() >= 2 && stroke.points.iter().all(|point| point.position.is_finite())
        }));
    }

    #[test]
    fn creates_a_continuous_result_when_one_cel_is_empty() {
        let drawing = midpoint(
            PaintDrawing::default(),
            PaintDrawing {
                strokes: vec![stroke(&[(0.0, 0.0), (10.0, 0.0)])],
                fills: Vec::new(),
            },
        );

        assert_eq!(drawing.strokes.len(), 1);
        assert!(drawing.strokes[0].width_scale > 0.0);
        assert!(drawing.strokes[0].width_scale < 1.0);
    }

    #[test]
    fn near_closed_open_strokes_keep_their_extent() {
        let drawing = midpoint(
            PaintDrawing {
                strokes: vec![stroke(&[
                    (-0.1, 1.4),
                    (0.7, 1.1),
                    (1.1, 0.4),
                    (0.9, -0.7),
                    (0.1, -1.2),
                    (-0.8, -0.9),
                    (-1.1, -0.1),
                    (-0.8, 0.8),
                    (0.6, 1.0),
                ])],
                fills: Vec::new(),
            },
            PaintDrawing {
                strokes: vec![stroke(&[
                    (-0.7, 1.0),
                    (0.1, 1.4),
                    (0.9, 1.0),
                    (1.1, 0.1),
                    (0.6, -0.9),
                    (-0.3, -1.2),
                    (-1.0, -0.6),
                    (-1.1, 0.4),
                    (0.2, 1.2),
                ])],
                fills: Vec::new(),
            },
        );
        let points = &drawing.strokes[0].points;
        let minimum = points
            .iter()
            .map(|point| point.position)
            .reduce(Vec2::min)
            .unwrap();
        let maximum = points
            .iter()
            .map(|point| point.position)
            .reduce(Vec2::max)
            .unwrap();

        assert!((maximum.x - minimum.x) > 1.5);
        assert!((maximum.y - minimum.y) > 1.5);
    }
}
