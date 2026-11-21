use sanim_core::{
    Animation, Color, Easing, Element, Fraction, Geometry, Layout, Motion, Number, Property, Unit,
    Value, Vec2,
    math::{
        cycle::Cycle,
        temporal::{Interval, Window},
    },
};
use std::sync::Arc;
fn interval(start: u64, width: u64, period: Option<u64>, until: Option<u64>) -> Interval {
    Interval {
        start: start.into(),
        end: (start + width).into(),
        interpolation: None,
        cycle: period.map(|period| {
            let cycle = Cycle::new(0.into(), period.into()).unwrap();
            until.map_or(cycle, |until| cycle.limited(until.into()).unwrap())
        }),
    }
}
#[test]
fn periodic_writer_overlap_respects_phases_points_and_finite_limits() {
    assert!(
        interval(0, 1, Some(2), None)
            .overlaps(interval(5000, 1, None, None))
            .unwrap()
    );
    assert!(
        !interval(0, 1, Some(2), None)
            .overlaps(interval(5001, 1, None, None))
            .unwrap()
    );
    assert!(
        !interval(0, 1, Some(2), Some(4))
            .overlaps(interval(4, 1, None, None))
            .unwrap()
    );
    assert!(
        interval(0, 0, Some(2), Some(4))
            .overlaps(interval(4, 0, None, None))
            .unwrap()
    );
    assert!(
        !interval(0, 1, Some(4), None)
            .overlaps(interval(1, 1, Some(6), None))
            .unwrap()
    );
    assert!(
        interval(0, 1, Some(4), None)
            .overlaps(interval(2, 1, Some(6), None))
            .unwrap()
    );
    assert!(
        !interval(0, 1, None, None)
            .overlaps(interval(1, 0, None, None))
            .unwrap()
    );
    assert!(
        interval(0, 1, None, None)
            .overlaps(interval(0, 0, None, None))
            .unwrap()
    );
    assert!(
        interval(0, 1, Some(1000), Some(1_000_001))
            .overlaps(interval(1, 1, Some(1001), None))
            .unwrap_err()
            .contains("work budget")
    );
    let tiny = Interval {
        start: Fraction::new(1u64, 3u64),
        end: Fraction::new(2u64, 3u64),
        cycle: Some(Cycle::new(0.into(), 1.into()).unwrap()),
        interpolation: None,
    };
    assert!(
        tiny.overlaps(Interval {
            start: Fraction::new(4u64, 3u64),
            end: Fraction::new(3u64, 2u64),
            cycle: None,
            interpolation: None
        })
        .unwrap()
    );
}
#[test]
fn exact_solver_matches_small_exhaustive_occurrence_oracle() {
    fn occurrences(a: Interval) -> Vec<(Fraction, Fraction)> {
        let mut result = vec![];
        let step = a.cycle.map(|c| c.end() - c.start()).unwrap_or(100.into());
        let until = a.cycle.and_then(|c| c.until()).unwrap_or(24.into());
        let mut start = a.start;
        while start <= until {
            let end = (start + (a.end - a.start)).min(until);
            if start < end || a.start == a.end {
                result.push((start, end));
            }
            start += step;
        }
        result
    }
    let mut pulses = vec![];
    for period in 1..=3 {
        for start in 0..period {
            for width in 0..=period - start {
                for until in [None, Some(2), Some(7)] {
                    pulses.push(interval(start, width, Some(period), until));
                }
            }
        }
    }
    for &a in &pulses {
        for &b in &pulses {
            let expected = occurrences(a).iter().any(|&(s, e)| {
                occurrences(b).iter().any(|&(t, u)| {
                    if s == e {
                        if t == u { s == t } else { t <= s && s < u }
                    } else if t == u {
                        s <= t && t < e
                    } else {
                        s < u && t < e
                    }
                })
            });
            assert_eq!(a.overlaps(b).unwrap(), expected, "{a:?} vs {b:?}");
        }
    }
}
fn element(id: &str, children: Vec<Element>) -> Element {
    Element {
        id: id.into(),
        geometry: Arc::new(Geometry::Box {
            radius: Default::default(),
            children: children.into_iter().collect(),
        }),
        position: Vec2::zero(Unit::Length),
        rotation: 0.into(),
        scale: 1.into(),
        opacity: 1.into(),
        fill: Color::parse("blue3").unwrap().into(),
        stroke: None,
        reveal: Default::default(),
        horizontal: Default::default(),
        layout: Layout::default(),
    }
}
fn motion(element: &Element, property: Property, from: u64, to: u64) -> Motion {
    Motion {
        element: element.clone(),
        property,
        from: Some(Value::Number(Number {
            value: from.into(),
            unit: if property == Property::Rotation {
                Unit::Angle
            } else {
                Unit::Number
            },
        })),
        to: Value::Number(Number {
            value: to.into(),
            unit: if property == Property::Rotation {
                Unit::Angle
            } else {
                Unit::Number
            },
        }),
        reveal: None,
        horizontal: None,
        easing: None,
    }
}
#[test]
fn ancestor_reveal_and_centered_growth_check_repeated_occurrences() {
    let child = element("child", vec![]);
    let parent = element("parent", vec![child.clone()]);
    let cycle = Cycle::new(0.into(), 2.into()).unwrap();
    let mut scene = Animation::default();
    scene
        .schedule_in_window(
            &[motion(&parent, Property::Reveal, 0, 1)],
            0.into(),
            1.into(),
            Easing::Linear,
            Window {
                cycle: Some(cycle),
                clip: None,
            },
        )
        .unwrap();
    let before = scene.clone();
    assert!(
        scene
            .schedule(
                &[motion(&child, Property::Reveal, 0, 1)],
                2.into(),
                1.into(),
                Easing::Linear
            )
            .unwrap_err()
            .contains("ancestor/descendant")
    );
    assert_eq!(scene, before);
    let mut scene = Animation::default();
    scene
        .schedule_in_window(
            &[motion(&child, Property::Scale, 0, 1)],
            0.into(),
            1.into(),
            Easing::Linear,
            Window {
                cycle: Some(cycle),
                clip: None,
            },
        )
        .unwrap();
    assert!(
        scene
            .schedule(
                &[motion(&child, Property::Rotation, 0, 90)],
                2.into(),
                1.into(),
                Easing::Linear
            )
            .is_err()
    );
    let mut scene = Animation::default();
    scene
        .schedule_in_window(
            &[motion(&child, Property::Rotation, 0, 90)],
            0.into(),
            1.into(),
            Easing::Linear,
            Window {
                cycle: Some(cycle),
                clip: None,
            },
        )
        .unwrap();
    assert!(
        scene
            .schedule(
                &[motion(&child, Property::Scale, 0, 1)],
                2.into(),
                1.into(),
                Easing::Linear
            )
            .is_err()
    );
    let mut scene = Animation::default();
    scene
        .schedule_in_window(
            &[motion(&parent, Property::Reveal, 0, 1)],
            0.into(),
            1.into(),
            Easing::Linear,
            Window {
                cycle: Some(cycle.limited(2.into()).unwrap()),
                clip: None,
            },
        )
        .unwrap();
    scene
        .schedule(
            &[motion(&child, Property::Reveal, 0, 1)],
            2.into(),
            1.into(),
            Easing::Linear,
        )
        .unwrap();
}
#[test]
#[ignore = "manual temporal overlap duration-independence benchmark"]
fn overlap_lookup_benchmark() {
    let periodic = interval(0, 1, Some(2), None);
    const LOOKUPS: usize = 100_000;
    for start in [2, 500_000] {
        let fixed = interval(start, 1, None, None);
        let began = std::time::Instant::now();
        for _ in 0..LOOKUPS {
            assert!(
                std::hint::black_box(periodic)
                    .overlaps(std::hint::black_box(fixed))
                    .unwrap()
            );
        }
        eprintln!(
            "{LOOKUPS} periodic/fixed overlaps at {start}s: {:?}",
            began.elapsed()
        );
    }
}
