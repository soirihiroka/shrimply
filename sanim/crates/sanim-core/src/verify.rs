//! Conservative geometric proofs. Unknown is never reported as proved.
use crate::{
    Animation, Element, Fraction, Geometry, Property, Vec2,
    math::{float_arithmetic as calc, to_f64},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub min: Vec2,
    pub max: Vec2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Evidence {
    Proved,
    Refuted,
    Unknown,
    Sampled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    pub evidence: Evidence,
    pub message: String,
    pub time: Option<Fraction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Relation {
    Above,
    Below,
    LeftOf,
    RightOf,
    Overlaps,
    Disjoint,
    Contains,
    Touches,
    InsideCanvas,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Predicate {
    pub left: Element,
    pub right: Option<Element>,
    pub relation: Relation,
    pub gap: Fraction,
    pub continuous: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Contract {
    pub predicate: Predicate,
    pub time: Fraction,
    pub required: bool,
    pub module: String,
    pub span: crate::Span,
    pub message: Option<String>,
}

fn test(relation: Relation, a: Bounds, b: Bounds, gap: f64) -> Option<bool> {
    Some(match relation {
        Relation::Above => calc("+", a.max.values[1], gap).ok()? <= b.min.values[1],
        Relation::Below => a.min.values[1] >= calc("+", b.max.values[1], gap).ok()?,
        Relation::LeftOf => calc("+", a.max.values[0], gap).ok()? <= b.min.values[0],
        Relation::RightOf => a.min.values[0] >= calc("+", b.max.values[0], gap).ok()?,
        Relation::Contains => inside(b, a),
        Relation::InsideCanvas => inside(a, b),
        Relation::Overlaps => {
            a.min.values[0] < b.max.values[0]
                && a.max.values[0] > b.min.values[0]
                && a.min.values[1] < b.max.values[1]
                && a.max.values[1] > b.min.values[1]
        }
        Relation::Disjoint => {
            a.max.values[0] <= b.min.values[0]
                || b.max.values[0] <= a.min.values[0]
                || a.max.values[1] <= b.min.values[1]
                || b.max.values[1] <= a.min.values[1]
        }
        Relation::Touches => {
            !test(Relation::Overlaps, a, b, gap)?
                && (a.max.values[0] == b.min.values[0]
                    || b.max.values[0] == a.min.values[0]
                    || a.max.values[1] == b.min.values[1]
                    || b.max.values[1] == a.min.values[1])
                && a.min.values[0] <= b.max.values[0]
                && a.max.values[0] >= b.min.values[0]
                && a.min.values[1] <= b.max.values[1]
                && a.max.values[1] >= b.min.values[1]
        }
    })
}

/// These predicates explicitly concern axis-aligned layout bounds, not pixel visibility.
pub fn evaluate(animation: &Animation, predicate: &Predicate, time: Fraction) -> Verification {
    let result = |evidence, message: String, time| Verification {
        evidence,
        message,
        time,
    };
    let sample = |element: &Element, time: Fraction| -> Result<Bounds, String> {
        if animation
            .layers
            .iter()
            .any(|v| v.element.find(&element.id).is_some() && v.element.id != element.id)
        {
            return Err("nested-object assertions require composed transform evidence".into());
        }
        let mut sampled =
            animation.sample_element(animation.find(&element.id).unwrap_or(element), time)?;
        if matches!(sampled.geometry.as_ref(), Geometry::Group { .. }) {
            return Err("group assertions require composed layout evidence".into());
        }
        crate::layout::resolve(
            &mut sampled,
            crate::math::to_f64(animation.logical_size().0)?,
            crate::math::to_f64(animation.logical_size().1)?,
        )?;
        bounds(&sampled)
    };
    let target = predicate
        .right
        .as_ref()
        .map(|v| format!("'{}'", v.id))
        .unwrap_or_else(|| "the canvas".into());
    let describe = format!(
        "'{}' must be {:?} {target} (layout bounds, gap {}pt)",
        predicate.left.id, predicate.relation, predicate.gap
    );
    let at = |t| -> Result<(Bounds, Bounds), String> {
        let a = sample(&predicate.left, t)?;
        let b = if let Some(right) = &predicate.right {
            sample(right, t)?
        } else {
            Bounds {
                min: Vec2::zero(crate::Unit::Length),
                max: Vec2 {
                    values: [
                        crate::math::to_f64(animation.logical_size().0)?,
                        crate::math::to_f64(animation.logical_size().1)?,
                    ],
                    unit: crate::Unit::Length,
                },
            }
        };
        Ok((a, b))
    };
    if !predicate.continuous {
        return match at(time) {
            Ok((a, b))
                if test(
                    predicate.relation,
                    a,
                    b,
                    to_f64(predicate.gap).unwrap_or(f64::NAN),
                )
                .is_none() =>
            {
                result(
                    Evidence::Unknown,
                    format!("{describe}; relationship arithmetic overflow"),
                    Some(time),
                )
            }
            Ok((a, b)) => result(
                if test(
                    predicate.relation,
                    a,
                    b,
                    to_f64(predicate.gap).unwrap_or(f64::NAN),
                ) == Some(true)
                {
                    Evidence::Proved
                } else {
                    Evidence::Refuted
                },
                format!(
                    "{describe}; found left={a:?}, right={b:?} at {time}s. Adjust positions, sizes, or assertion timing."
                ),
                Some(time),
            ),
            Err(e) => result(
                Evidence::Unknown,
                format!("{describe}; cannot verify: {e}"),
                Some(time),
            ),
        };
    }
    if animation.reactive.has_bindings() {
        return result(
            Evidence::Unknown,
            format!(
                "{describe}; continuous proof of retained reactive properties is not supported; use explicit frame checks"
            ),
            None,
        );
    }
    for element in std::iter::once(&predicate.left).chain(predicate.right.iter()) {
        if animation.tracks.get(&element.id).is_some_and(|v| {
            v.contains_key(&Property::Rotation)
                || v.contains_key(&Property::Scale)
                || v.contains_key(&Property::StretchX)
                || v.get(&Property::Position).is_some_and(|tracks| {
                    tracks
                        .iter()
                        .any(|track| track.easing == crate::Easing::Overshoot)
                })
        }) {
            return result(
                Evidence::Unknown,
                format!(
                    "{describe}; continuous proof supports bounded translations with fixed scale/rotation. Use explicit frame checks for this motion."
                ),
                None,
            );
        }
    }
    let mut times = vec![Fraction::from(0), animation.duration];
    for element in std::iter::once(&predicate.left).chain(predicate.right.iter()) {
        if let Some(tracks) = animation
            .tracks
            .get(&element.id)
            .and_then(|v| v.get(&Property::Position))
        {
            for track in tracks {
                times.extend([track.start, track.end]);
            }
        }
    }
    times.sort();
    times.dedup();
    let mut samples = vec![];
    for t in times {
        match at(t) {
            Ok((a, b)) => {
                if test(
                    predicate.relation,
                    a,
                    b,
                    to_f64(predicate.gap).unwrap_or(f64::NAN),
                )
                .is_none()
                {
                    return result(
                        Evidence::Unknown,
                        format!("{describe}; relationship arithmetic overflow"),
                        Some(t),
                    );
                }
                if test(
                    predicate.relation,
                    a,
                    b,
                    to_f64(predicate.gap).unwrap_or(f64::NAN),
                ) == Some(false)
                {
                    return result(
                        Evidence::Refuted,
                        format!("{describe}; counterexample at {t}s: left={a:?}, right={b:?}"),
                        Some(t),
                    );
                }
                samples.push((a, b));
            }
            Err(e) => return result(Evidence::Unknown, format!("{describe}; {e}"), Some(t)),
        }
    }
    for pair in samples.windows(2) {
        let ((a, b), (c, d)) = (pair[0], pair[1]);
        let union = |a: Bounds, b: Bounds| Bounds {
            min: Vec2 {
                values: [
                    a.min.values[0].min(b.min.values[0]),
                    a.min.values[1].min(b.min.values[1]),
                ],
                unit: crate::Unit::Length,
            },
            max: Vec2 {
                values: [
                    a.max.values[0].max(b.max.values[0]),
                    a.max.values[1].max(b.max.values[1]),
                ],
                unit: crate::Unit::Length,
            },
        };
        let intersection = |a: Bounds, b: Bounds| Bounds {
            min: Vec2 {
                values: [
                    a.min.values[0].max(b.min.values[0]),
                    a.min.values[1].max(b.min.values[1]),
                ],
                unit: crate::Unit::Length,
            },
            max: Vec2 {
                values: [
                    a.max.values[0].min(b.max.values[0]),
                    a.max.values[1].min(b.max.values[1]),
                ],
                unit: crate::Unit::Length,
            },
        };
        let proved = match predicate.relation {
            Relation::InsideCanvas => Some(inside(union(a, c), intersection(b, d))),
            Relation::Contains => Some(inside(union(b, d), intersection(a, c))),
            Relation::Overlaps => test(
                Relation::Overlaps,
                intersection(a, c),
                intersection(b, d),
                to_f64(predicate.gap).unwrap_or(f64::NAN),
            ),
            Relation::Touches => Some(a == c && b == d),
            relation => test(
                relation,
                union(a, c),
                union(b, d),
                to_f64(predicate.gap).unwrap_or(f64::NAN),
            ),
        };
        if proved != Some(true) {
            return result(
                Evidence::Unknown,
                format!(
                    "{describe}; endpoint checks passed but conservative continuous envelopes overlap. This is not a proof of failure."
                ),
                None,
            );
        }
    }
    result(
        Evidence::Proved,
        format!(
            "{describe}; proved over [0, {}]s using monotonic translation envelopes",
            animation.duration
        ),
        None,
    )
}

pub fn bounds(element: &Element) -> Result<Bounds, String> {
    if element.horizontal.scale != Fraction::from(1) {
        return Err("horizontal stretch bounds require renderer evidence".into());
    }
    bounds_at(element, element.position, element.rotation)
}

pub(crate) fn bounds_at(
    element: &Element,
    position: Vec2,
    rotation: Fraction,
) -> Result<Bounds, String> {
    if rotation != Fraction::from(0) {
        return Err("rotated bounds require renderer evidence".into());
    }
    let zero = 0.0_f64;
    let (min, max) = match element.geometry.as_ref() {
        Geometry::Circle { radius } => (
            Vec2 {
                values: [-to_f64(*radius)?, -to_f64(*radius)?],
                unit: crate::Unit::Length,
            },
            Vec2 {
                values: [to_f64(*radius)?, to_f64(*radius)?],
                unit: crate::Unit::Length,
            },
        ),
        Geometry::Rectangle { width, height, .. } => (
            Vec2 {
                values: [
                    -calc("/", to_f64(*width)?, 2.0)?,
                    -calc("/", to_f64(*height)?, 2.0)?,
                ],
                unit: crate::Unit::Length,
            },
            Vec2 {
                values: [
                    calc("/", to_f64(*width)?, 2.0)?,
                    calc("/", to_f64(*height)?, 2.0)?,
                ],
                unit: crate::Unit::Length,
            },
        ),
        Geometry::Line { end, width } => {
            let half = calc("/", to_f64(*width)?, 2.0)?;
            (
                Vec2 {
                    values: [
                        calc("-", zero.min(end.values[0]), half)?,
                        calc("-", zero.min(end.values[1]), half)?,
                    ],
                    unit: crate::Unit::Length,
                },
                Vec2 {
                    values: [
                        calc("+", zero.max(end.values[0]), half)?,
                        calc("+", zero.max(end.values[1]), half)?,
                    ],
                    unit: crate::Unit::Length,
                },
            )
        }
        Geometry::Polygon { points } => (
            Vec2 {
                values: [
                    points
                        .iter()
                        .map(|p| p.values[0])
                        .min_by(f64::total_cmp)
                        .ok_or("empty polygon")?,
                    points
                        .iter()
                        .map(|p| p.values[1])
                        .min_by(f64::total_cmp)
                        .ok_or("empty polygon")?,
                ],
                unit: crate::Unit::Length,
            },
            Vec2 {
                values: [
                    points
                        .iter()
                        .map(|p| p.values[0])
                        .max_by(f64::total_cmp)
                        .ok_or("empty polygon")?,
                    points
                        .iter()
                        .map(|p| p.values[1])
                        .max_by(f64::total_cmp)
                        .ok_or("empty polygon")?,
                ],
                unit: crate::Unit::Length,
            },
        ),
        Geometry::Group { children } => {
            let bounds = children.iter().map(bounds).collect::<Result<Vec<_>, _>>()?;
            (
                Vec2 {
                    values: [
                        bounds
                            .iter()
                            .map(|b| b.min.values[0])
                            .min_by(f64::total_cmp)
                            .ok_or("empty group")?,
                        bounds
                            .iter()
                            .map(|b| b.min.values[1])
                            .min_by(f64::total_cmp)
                            .ok_or("empty group")?,
                    ],
                    unit: crate::Unit::Length,
                },
                Vec2 {
                    values: [
                        bounds
                            .iter()
                            .map(|b| b.max.values[0])
                            .max_by(f64::total_cmp)
                            .ok_or("empty group")?,
                        bounds
                            .iter()
                            .map(|b| b.max.values[1])
                            .max_by(f64::total_cmp)
                            .ok_or("empty group")?,
                    ],
                    unit: crate::Unit::Length,
                },
            )
        }
        Geometry::Text { bounds, .. } => (bounds.min, bounds.max),
    };
    let x1 = calc(
        "+",
        calc("*", min.values[0], to_f64(element.scale)?)?,
        position.values[0],
    )?;
    let x2 = calc(
        "+",
        calc("*", max.values[0], to_f64(element.scale)?)?,
        position.values[0],
    )?;
    let y1 = calc(
        "+",
        calc("*", min.values[1], to_f64(element.scale)?)?,
        position.values[1],
    )?;
    let y2 = calc(
        "+",
        calc("*", max.values[1], to_f64(element.scale)?)?,
        position.values[1],
    )?;
    let values = [x1, x2, y1, y2];
    if element.scale != Fraction::from(0)
        && ((min.values[0] != max.values[0] && x1 == x2)
            || (min.values[1] != max.values[1] && y1 == y2))
    {
        return Err("geometry is too small relative to its coordinate magnitude for f64 bounds; reduce the coordinate magnitude or increase the size".into());
    }
    if values.iter().any(|v| !v.is_finite()) {
        return Err("bounds arithmetic overflow".into());
    }
    Ok(Bounds {
        min: Vec2 {
            values: [x1.min(x2), y1.min(y2)],
            unit: crate::Unit::Length,
        },
        max: Vec2 {
            values: [x1.max(x2), y1.max(y2)],
            unit: crate::Unit::Length,
        },
    })
}

pub fn inside(inner: Bounds, outer: Bounds) -> bool {
    inner.min.values[0] >= outer.min.values[0]
        && inner.min.values[1] >= outer.min.values[1]
        && inner.max.values[0] <= outer.max.values[0]
        && inner.max.values[1] <= outer.max.values[1]
}
