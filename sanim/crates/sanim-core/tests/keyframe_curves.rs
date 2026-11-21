use sanim_core::{
    Fraction, Number, Value,
    curve::{Curve, KeyInterpolation, Keyframe, Tangents},
};
use std::sync::Arc;
fn key(time: u64, value: i64) -> Keyframe {
    Keyframe {
        time: time.into(),
        value: Value::Number(Number::scalar(value)),
        handle_in: None,
        handle_out: None,
        interpolation: KeyInterpolation::Bezier,
        tangents: Tangents::AutoClamped,
    }
}
#[test]
fn auto_clamped_curves_never_overshoot_and_serialization_recompiles_validated_keys() {
    let curve = Curve::keyed(Arc::from([key(0, 0), key(1, 10), key(2, 1), key(3, 1)])).unwrap();
    for segment in 0u64..3 {
        for tick in 0u64..=100 {
            let time = Fraction::new(segment * 100 + tick, 100u64);
            let value = curve.sample_time(time).unwrap().number().unwrap().value;
            let (lo, hi) = match segment {
                0 => (0, 10),
                1 => (1, 10),
                _ => (1, 1),
            };
            assert!(value >= lo.into() && value <= hi.into(), "{time}: {value}");
        }
    }
    let saved = serde_json::to_vec(&curve).unwrap();
    let restored: Curve = serde_json::from_slice(&saved).unwrap();
    assert_eq!(curve, restored);
    assert_eq!(
        curve.sample_time(Fraction::new(3u64, 2u64)).unwrap(),
        restored.sample_time(Fraction::new(3u64, 2u64)).unwrap()
    );
}
#[test]
#[ignore = "run make curve-bench"]
fn keyframe_curve_benchmark() {
    for count in [16, 4096] {
        let curve = Curve::keyed(
            (0..count)
                .map(|i| key(i, i as i64 % 7))
                .collect::<Vec<_>>()
                .into(),
        )
        .unwrap();
        let start = std::time::Instant::now();
        let mut sum = 0.0;
        for i in 0..20_000u64 {
            sum += sanim_core::math::to_f64(
                curve
                    .sample_time(Fraction::new((i * 7919) % (count * 100), 100u64))
                    .unwrap()
                    .number()
                    .unwrap()
                    .value,
            )
            .unwrap();
        }
        println!(
            "keys={count} samples=20000 elapsed={:?} checksum={sum}",
            start.elapsed()
        );
    }
}

#[test]
fn clipped_equal_endpoint_curve_tracks_hold_the_actual_crop_and_roundtrip() {
    let curve = Arc::new(Curve::keyed(Arc::from([key(0, 0), key(1, 10), key(2, 0)])).unwrap());
    assert!(!curve.is_constant());
    let mut graph = sanim_core::reactive::ReactiveGraph::default();
    graph
        .set_typed_cell(
            1,
            sanim_core::Type::Numeric(sanim_core::Unit::Number),
            Value::Number(Number::scalar(0)),
        )
        .unwrap();
    graph
        .write_curve_cell_in_window(
            1,
            0.into(),
            2.into(),
            curve,
            sanim_core::math::temporal::Window {
                clip: Some((0.into(), 1.into())),
                cycle: None,
            },
        )
        .unwrap();
    assert_eq!(
        graph
            .sample_cell(1, 1.into())
            .unwrap()
            .number()
            .unwrap()
            .value,
        10.into()
    );
    assert_eq!(
        graph
            .sample_cell(1, Fraction::new(3u64, 2u64))
            .unwrap()
            .number()
            .unwrap()
            .value,
        10.into()
    );
    graph
        .write_cell(1, 2.into(), Value::Number(Number::scalar(20)))
        .unwrap();
    graph.validate().unwrap();
    let encoded = serde_json::to_vec(&graph).unwrap();
    let restored: sanim_core::reactive::ReactiveGraph = serde_json::from_slice(&encoded).unwrap();
    for time in [Fraction::new(1u64, 2u64), 2.into(), 1.into(), 0.into()] {
        assert_eq!(
            graph.sample_cell(1, time).unwrap(),
            restored.sample_cell(1, time).unwrap()
        );
    }
}

#[test]
fn changed_keyframes_preserve_exact_prefix_and_account_for_neighbor_tangents() {
    for (interpolation, expected) in [
        (KeyInterpolation::Linear, 3u64),
        (KeyInterpolation::Bezier, 2),
        (KeyInterpolation::Hold, 4),
    ] {
        let keys = (0..=4)
            .map(|i| {
                let mut k = key(i, i as i64 * 10);
                k.interpolation = interpolation;
                k
            })
            .collect::<Vec<_>>();
        let old = Curve::keyed(keys.clone().into()).unwrap();
        let mut edited = keys;
        edited[4].value = Value::Number(Number::scalar(50));
        let new = Curve::keyed(edited.into()).unwrap();
        assert_eq!(
            old.changed_progress(&new),
            Some(Fraction::new(expected, 4u64))
        );
        for tick in 0..expected * 10 {
            assert_eq!(
                old.sample_time(Fraction::new(tick, 10u64)).unwrap(),
                new.sample_time(Fraction::new(tick, 10u64)).unwrap()
            );
        }
    }
}

#[test]
fn curve_prefix_changes_map_through_clipped_track_windows_and_exact_retiming() {
    let old = Arc::new(
        Curve::keyed(
            (0..=4)
                .map(|i| {
                    let mut k = key(i, i as i64);
                    k.interpolation = KeyInterpolation::Linear;
                    k
                })
                .collect::<Vec<_>>()
                .into(),
        )
        .unwrap(),
    );
    let mut keys = (0..=4)
        .map(|i| {
            let mut k = key(i, i as i64);
            k.interpolation = KeyInterpolation::Linear;
            k
        })
        .collect::<Vec<_>>();
    keys[4].value = Value::Number(Number::scalar(9));
    let new = Arc::new(Curve::keyed(keys.into()).unwrap());
    let mut graph = sanim_core::reactive::ReactiveGraph::default();
    graph
        .set_typed_cell(
            1,
            sanim_core::Type::Numeric(sanim_core::Unit::Number),
            Value::Number(Number::scalar(0)),
        )
        .unwrap();
    graph
        .write_curve_cell_in_window(
            1,
            2.into(),
            8.into(),
            old,
            sanim_core::math::temporal::Window {
                clip: Some((3.into(), 9.into())),
                cycle: None,
            },
        )
        .unwrap();
    let encoded = serde_json::to_value(&graph).unwrap();
    let tracks = encoded["cells"]["1"]["tracks"].as_array().unwrap();
    let a: sanim_core::Track = serde_json::from_value(tracks[0].clone()).unwrap();
    let mut b = a.clone();
    b.curve = Some(new);
    assert_eq!(a.changed_from(&b), Some(8.into()));
    b.end = 7.into();
    let mut a = a;
    a.end = 7.into();
    assert_eq!(
        a.changed_from(&b),
        None,
        "changes beyond a clipped writer cannot invalidate its frames"
    );
    a.cycle = Some(sanim_core::math::cycle::Cycle::new(0.into(), 10.into()).unwrap());
    b.cycle = a.cycle;
    assert_eq!(
        a.changed_from(&b),
        None,
        "an invisible edit stays invisible in every repeated crop"
    );
}

#[test]
#[ignore = "run make curve-prefix-bench"]
fn keyframe_prefix_diff_benchmark() {
    let keys = (0..4096).map(|i| key(i, i as i64 % 7)).collect::<Vec<_>>();
    let old = Curve::keyed(keys.clone().into()).unwrap();
    let mut edited = keys;
    edited[4095].value = Value::Number(Number::scalar(20));
    let new = Curve::keyed(edited.into()).unwrap();
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        std::hint::black_box(old.changed_progress(&old));
    }
    let unchanged = start.elapsed();
    let start = std::time::Instant::now();
    for _ in 0..100 {
        std::hint::black_box(old.changed_progress(&new));
    }
    println!(
        "curve_keys=4096 shared_identity_1000={unchanged:?} late_diff_100={:?} first_change={:?}",
        start.elapsed(),
        old.changed_progress(&new)
    );
}

#[test]
fn multichannel_keys_and_both_handle_lists_charge_real_value_storage_once() {
    const KEYS: usize = 128;
    const CHANNELS: usize = 256;
    let values = || {
        Value::List(sanim_core::List::from(
            (0..CHANNELS)
                .map(|i| Value::Number(Number::scalar(i as i64)))
                .collect::<Vec<_>>(),
        ))
    };
    let keys = (0..KEYS)
        .map(|i| Keyframe {
            time: Fraction::from(i as u64),
            value: values(),
            handle_in: Some(sanim_core::curve::Handle {
                time: -Fraction::new(1u64, 3u64),
                value: values(),
            }),
            handle_out: Some(sanim_core::curve::Handle {
                time: Fraction::new(1u64, 3u64),
                value: values(),
            }),
            interpolation: KeyInterpolation::Linear,
            tangents: Tangents::AutoClamped,
        })
        .collect::<Vec<_>>();
    let minimum_values = keys
        .iter()
        .flat_map(|key| {
            std::iter::once(&key.value)
                .chain(key.handle_in.as_ref().map(|handle| &handle.value))
                .chain(key.handle_out.as_ref().map(|handle| &handle.value))
        })
        .map(|value| value.list().unwrap().len() * std::mem::size_of::<Value>())
        .sum::<usize>();
    let curve = Arc::new(Curve::keyed(keys.into()).unwrap());
    assert!(curve.resident_bytes() >= minimum_values + KEYS * std::mem::size_of::<Keyframe>());
    let mut bytes = curve.resident_bytes() * 2;
    let mut items = usize::MAX;
    let mut resources = sanim_core::value_budget::Budget::default();
    let value = Value::Curve(curve);
    sanim_core::value_budget::charge_with_budget(&value, &mut bytes, &mut items, 3, &mut resources)
        .unwrap();
    let before = bytes;
    sanim_core::value_budget::charge_with_budget(&value, &mut bytes, &mut items, 3, &mut resources)
        .unwrap();
    assert_eq!(
        before - bytes,
        std::mem::size_of::<Value>(),
        "shared Curve roots must not recharge their compiled/key payload"
    );
}

#[test]
fn keyframe_wire_rejects_forged_list_metadata() {
    for ty in [
        sanim_core::Type::String,
        sanim_core::Type::Numeric(sanim_core::Unit::Length),
    ] {
        let mut first = key(0, 0);
        first.value = Value::List(sanim_core::List::typed(
            ty,
            vec![Value::Number(Number::scalar(1))],
        ));
        assert!(Curve::keyed(Arc::from([first, key(1, 2)])).is_err());
    }
}

#[test]
fn static_native_curves_are_folded_once_with_a_reserved_native_budget() {
    use sanim_core::{
        Type, Unit,
        code::values::Native,
        reactive::{Node, ReactiveGraph},
        voxel::camera::Op,
    };
    let key = |time| {
        sanim_core::curve::values::keyframe(
            &[
                (
                    "time".to_owned(),
                    Value::Number(Number {
                        value: Fraction::from(time),
                        unit: Unit::Duration,
                    }),
                ),
                (
                    "value".to_owned(),
                    Value::Number(Number::scalar(time as i64)),
                ),
            ]
            .into_iter()
            .collect(),
        )
        .unwrap()
    };
    let mut graph = ReactiveGraph::default();
    let keys = graph
        .insert_node(
            Node::Constant(Value::List(sanim_core::List::typed(
                sanim_core::curve::values::keyframe_type(Type::Numeric(Unit::Number)),
                vec![key(0u64), key(1u64)],
            ))),
            100,
        )
        .unwrap();
    let op = || Node::NativeConfig {
        operation: Native::Camera(Op::Curve),
        arguments: [("keys".to_owned(), keys)].into_iter().collect(),
    };
    let mut work = 0;
    assert!(graph.insert_node_budgeted(op(), 100, &mut work, 0).is_err());
    assert_eq!(
        graph.node_count(),
        1,
        "failed reservations cannot publish a node"
    );
    let curve = graph
        .insert_node_budgeted(op(), 100, &mut work, 10_000)
        .unwrap();
    assert!(work > 0);
    assert_eq!(
        graph.node_type(curve),
        Some(&Type::Curve(Box::new(Type::Numeric(Unit::Number))))
    );
    let Value::Curve(a) = graph.sampler(0.into()).unwrap().value(curve).unwrap() else {
        panic!("Curve")
    };
    let Value::Curve(b) = graph.sampler(1.into()).unwrap().value(curve).unwrap() else {
        panic!("Curve")
    };
    assert!(
        Arc::ptr_eq(&a, &b),
        "a fresh frame sampler must reuse the compiled Curve allocation"
    );
    let restored: ReactiveGraph =
        serde_json::from_slice(&serde_json::to_vec(&graph).unwrap()).unwrap();
    assert_eq!(
        restored.sampler(1.into()).unwrap().value(curve).unwrap(),
        Value::Curve(a)
    );
}
