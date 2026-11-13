use sanim_core::{
    Animation, Color, Direction, Easing, Element, Fraction, Geometry, Layout, List, ListEdit,
    SampleTimes, TimelineDiff, Type, Unit, Value, Vec2,
};
use std::sync::Arc;

fn tile(id: &str) -> Element {
    Element {
        id: id.into(),
        geometry: Arc::new(Geometry::Rectangle {
            width: 10.into(),
            height: 10.into(),
            children: Default::default(),
            radius: Default::default(),
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

#[test]
fn nested_future_views_keep_their_clock_and_sample_their_actual_transition() {
    let mut animation = animation(&[]);
    let mut inner = tile("inner_view");
    inner.geometry = Arc::new(Geometry::Box {
        radius: Default::default(),
        children: Default::default(),
    });
    inner.layout = Layout {
        direction: Some(Direction::Row),
        width: Some(sanim_core::Number {
            value: 40.into(),
            unit: sanim_core::Unit::Length,
        }),
        height: Some(sanim_core::Number {
            value: 20.into(),
            unit: sanim_core::Unit::Length,
        }),
        ..Default::default()
    };
    animation.bind_list(2, &inner, items(&[])).unwrap();
    let outer = animation.sample_list(1, 0.into()).unwrap().unwrap();
    let edit = outer
        .edit(ListEdit::Append(Value::Element(Arc::new(inner))))
        .unwrap();
    animation
        .write_list_edit(1, 0.into(), 0.into(), &edit, Easing::Linear)
        .unwrap();
    let edit = items(&[])
        .edit(ListEdit::Append(Value::Element(Arc::new(tile(
            "nested_child",
        )))))
        .unwrap();
    animation
        .write_list_edit(2, 1.into(), 2.into(), &edit, Easing::Linear)
        .unwrap();
    let middle = Fraction::new(3u64, 2u64);
    assert_eq!(
        SampleTimes::new(&animation).canonical(middle).unwrap(),
        middle
    );
    let outer_children = children(&animation, middle, 100);
    let Geometry::Box { children, .. } = outer_children[0].geometry.as_ref() else {
        unreachable!()
    };
    assert_eq!(children[0].id, "nested_child");
    assert_eq!(children[0].opacity, Fraction::new(1u64, 2u64));
}

#[test]
fn populated_nested_views_keep_independent_ownership_and_cycles_fail_before_commit() {
    let mut animation = animation(&[]);
    let initial = items(&["nested_initial"]);
    let mut inner = tile("populated_inner");
    inner.geometry = Arc::new(Geometry::Box {
        radius: Default::default(),
        children: initial
            .values
            .iter()
            .map(|value| match value {
                Value::Element(element) => element.as_ref().clone(),
                _ => unreachable!(),
            })
            .collect(),
    });
    inner.layout = Layout {
        direction: Some(Direction::Row),
        width: Some(sanim_core::Number {
            value: 40.into(),
            unit: sanim_core::Unit::Length,
        }),
        height: Some(sanim_core::Number {
            value: 20.into(),
            unit: sanim_core::Unit::Length,
        }),
        ..Default::default()
    };
    animation.bind_list(2, &inner, initial).unwrap();
    let empty = animation.sample_list(1, 0.into()).unwrap().unwrap();
    let edit = empty
        .edit(ListEdit::Append(Value::Element(Arc::new(inner))))
        .unwrap();
    animation
        .write_list_edit(1, 0.into(), 0.into(), &edit, Easing::Linear)
        .unwrap();
    let outer = children(&animation, 0.into(), 100);
    let Geometry::Box { children, .. } = outer[0].geometry.as_ref() else {
        unreachable!()
    };
    assert_eq!(children[0].id, "nested_initial");
    let decoded: Animation =
        serde_json::from_slice(&serde_json::to_vec(&animation).unwrap()).unwrap();
    assert_eq!(decoded, animation);

    let mut cyclic = Animation::default();
    let mut a = tile("a_view");
    a.geometry = Arc::new(Geometry::Box {
        radius: Default::default(),
        children: Default::default(),
    });
    a.layout = Layout {
        direction: Some(Direction::Row),
        width: Some(sanim_core::Number {
            value: 40.into(),
            unit: sanim_core::Unit::Length,
        }),
        height: Some(sanim_core::Number {
            value: 20.into(),
            unit: sanim_core::Unit::Length,
        }),
        ..Default::default()
    };
    let mut b = a.clone();
    b.id = "b_view".into();
    cyclic.bind_list(1, &a, items(&[])).unwrap();
    cyclic.bind_list(2, &b, items(&[])).unwrap();
    let edit = items(&[])
        .edit(ListEdit::Append(Value::Element(Arc::new(a))))
        .unwrap();
    cyclic
        .write_list_edit(2, 0.into(), 0.into(), &edit, Easing::Linear)
        .unwrap();
    let before = cyclic.clone();
    let edit = items(&[])
        .edit(ListEdit::Append(Value::Element(Arc::new(b))))
        .unwrap();
    assert!(
        cyclic
            .write_list_edit(1, 0.into(), 0.into(), &edit, Easing::Linear)
            .unwrap_err()
            .contains("cyclic")
    );
    assert_eq!(cyclic, before);
}

#[test]
fn dynamic_view_nesting_is_bounded_before_sampling_can_recurse() {
    let mut animation = Animation::default();
    let mut views = Vec::new();
    for index in 0..=sanim_core::reactive::MAX_REACTIVE_DEPTH {
        let mut view = tile(&format!("depth_{index}"));
        view.geometry = Arc::new(Geometry::Box {
            radius: Default::default(),
            children: Default::default(),
        });
        view.layout = Layout {
            direction: Some(Direction::Row),
            width: Some(sanim_core::Number {
                value: 40.into(),
                unit: sanim_core::Unit::Length,
            }),
            height: Some(sanim_core::Number {
                value: 20.into(),
                unit: sanim_core::Unit::Length,
            }),
            ..Default::default()
        };
        animation
            .bind_list(index as u64, &view, items(&[]))
            .unwrap();
        views.push(view);
    }
    for index in 0..sanim_core::reactive::MAX_REACTIVE_DEPTH {
        let edit = items(&[])
            .edit(ListEdit::Append(Value::Element(Arc::new(
                views[index + 1].clone(),
            ))))
            .unwrap();
        let before = animation.clone();
        let result =
            animation.write_list_edit(index as u64, 0.into(), 0.into(), &edit, Easing::Linear);
        if index + 1 == sanim_core::reactive::MAX_REACTIVE_DEPTH {
            assert!(result.unwrap_err().contains("depth limit"));
            assert_eq!(animation, before);
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn entry_and_exit_scaling_keep_rect_and_group_slot_centers_fixed() {
    for grouped in [false, true] {
        let mut item = tile("fixed_center");
        if grouped {
            item.geometry = Arc::new(Geometry::Box {
                radius: Default::default(),
                children: vec![tile("face")].into(),
            });
            item.layout.width = Some(sanim_core::Number {
                value: 10.into(),
                unit: sanim_core::Unit::Length,
            });
            item.layout.height = Some(sanim_core::Number {
                value: 10.into(),
                unit: sanim_core::Unit::Length,
            });
        }
        let mut animation = animation(&[]);
        let edit = items(&[])
            .edit(ListEdit::Append(Value::Element(Arc::new(item))))
            .unwrap();
        animation
            .write_list_edit(1, 0.into(), 1.into(), &edit, Easing::Linear)
            .unwrap();
        let full = children(&animation, 1.into(), 100)[0].clone();
        for time in [Fraction::new(1u64, 4u64), Fraction::new(1u64, 2u64)] {
            let partial = children(&animation, time, 100)[0].clone();
            let center = |element: &Element| {
                let shift = if grouped {
                    5. * sanim_core::math::to_f64(element.scale).unwrap()
                } else {
                    0.
                };
                [
                    element.position.values[0] + shift,
                    element.position.values[1] + shift,
                ]
            };
            assert_eq!(center(&partial), center(&full));
        }
    }
}

#[test]
fn serialized_list_metadata_is_rejected_when_it_disagrees_with_typed_snapshots() {
    let mut animation = animation(&["a"]);
    let source = animation.sample_list(1, 0.into()).unwrap().unwrap();
    let edit = source
        .edit(ListEdit::Append(Value::Element(Arc::new(tile("b")))))
        .unwrap();
    animation
        .write_list_edit(1, 1.into(), 2.into(), &edit, Easing::Linear)
        .unwrap();
    let valid = serde_json::to_value(&animation.list_views).unwrap();
    let mut invalid = Vec::new();
    let mut value = valid.clone();
    value["events"] = serde_json::json!(0);
    invalid.push(value);
    let mut value = valid.clone();
    value["owners"]["a"] = serde_json::json!(99);
    invalid.push(value);
    let mut value = valid.clone();
    value["channels"]["1"]["initial"]["ids"] = serde_json::json!([]);
    invalid.push(value);
    let mut value = valid.clone();
    value["channels"]["1"]["transitions"][0]["start"] =
        serde_json::to_value(Fraction::from(-1)).unwrap();
    invalid.push(value);
    let mut value = valid.clone();
    value["catalog"]["a"]["id"] = serde_json::json!("forged");
    invalid.push(value);
    for value in invalid {
        assert!(serde_json::from_value::<sanim_core::list_views::ListViews>(value).is_err());
    }
    assert_eq!(
        serde_json::from_value::<sanim_core::list_views::ListViews>(valid).unwrap(),
        animation.list_views
    );
}

#[test]
fn sealed_edits_validate_only_changes_and_reject_duplicates_or_stale_sources_atomically() {
    let mut animation = animation(&["a"]);
    let initial = animation.sample_list(1, 0.into()).unwrap().unwrap();
    let later = initial
        .edit(ListEdit::Append(Value::Element(Arc::new(tile("b")))))
        .unwrap();
    animation
        .write_list_edit(1, 2.into(), 3.into(), &later, Easing::Linear)
        .unwrap();
    let before = animation.clone();
    let earlier = initial
        .edit(ListEdit::Append(Value::Element(Arc::new(tile("c")))))
        .unwrap();
    let error = animation
        .write_list_edit(1, 0.into(), 1.into(), &earlier, Easing::Linear)
        .unwrap_err();
    assert!(error.contains("chronological"), "{error}");
    assert_eq!(animation, before);
    let current = animation.sample_list(1, 3.into()).unwrap().unwrap();
    let duplicate = current
        .edit(ListEdit::Append(Value::Element(Arc::new(tile("a")))))
        .unwrap();
    let error = animation
        .write_list_edit(1, 3.into(), 4.into(), &duplicate, Easing::Linear)
        .unwrap_err();
    assert!(error.contains("duplicate"), "{error}");
    assert_eq!(animation, before);
    let error = animation
        .write_list_edit(1, 3.into(), 4.into(), &earlier, Easing::Linear)
        .unwrap_err();
    assert!(error.contains("source"), "{error}");
    assert_eq!(animation, before);
}

#[test]
#[ignore = "measured by make list-view-bench"]
fn retained_list_edit_benchmark() {
    const INITIAL: usize = 1_000;
    const EDITS: usize = 30;
    let names: Vec<_> = (0..INITIAL).map(|index| format!("base_{index}")).collect();
    let refs: Vec<_> = names.iter().map(String::as_str).collect();
    let base = animation(&refs);
    let mut strict = base.clone();
    let start = std::time::Instant::now();
    for index in 0..EDITS {
        let source = strict.sample_list(1, index.into()).unwrap().unwrap();
        let edit = source
            .edit(ListEdit::Append(Value::Element(Arc::new(tile(&format!(
                "new_{index}"
            ))))))
            .unwrap();
        strict
            .write_list(
                1,
                index.into(),
                (index + 1).into(),
                edit.output().clone(),
                Easing::Linear,
            )
            .unwrap();
    }
    let strict_us = start.elapsed().as_micros();
    let mut retained = base;
    let start = std::time::Instant::now();
    for index in 0..EDITS {
        let source = retained.sample_list(1, index.into()).unwrap().unwrap();
        let edit = source
            .edit(ListEdit::Append(Value::Element(Arc::new(tile(&format!(
                "new_{index}"
            ))))))
            .unwrap();
        retained
            .write_list_edit(1, index.into(), (index + 1).into(), &edit, Easing::Linear)
            .unwrap();
    }
    let retained_us = start.elapsed().as_micros();
    assert_eq!(
        strict.sample_list(1, EDITS.into()).unwrap(),
        retained.sample_list(1, EDITS.into()).unwrap()
    );
    eprintln!(
        "bound_list_initial={INITIAL} edits={EDITS} strict_us={strict_us} retained_us={retained_us}"
    );
}
fn items(names: &[&str]) -> List {
    List::typed(
        Type::Element,
        names
            .iter()
            .map(|name| Value::Element(Arc::new(tile(name))))
            .collect(),
    )
}
fn animation(names: &[&str]) -> Animation {
    let initial = items(names);
    let mut view = tile("view");
    view.geometry = Arc::new(Geometry::Box {
        radius: Default::default(),
        children: names.iter().map(|name| tile(name)).collect(),
    });
    view.layout = Layout {
        direction: Some(Direction::Row),
        width: Some(sanim_core::Number {
            value: 100.into(),
            unit: sanim_core::Unit::Length,
        }),
        height: Some(sanim_core::Number {
            value: 20.into(),
            unit: sanim_core::Unit::Length,
        }),
        ..Layout::default()
    };
    let mut animation = Animation::default();
    animation.bind_list(1, &view, initial).unwrap();
    animation.add(view, 0.into()).unwrap();
    animation
}
fn children(animation: &Animation, time: Fraction, width: u32) -> Vec<Element> {
    let roots = animation.layout(time, width.into(), 100.into()).unwrap();
    let Geometry::Box { children, .. } = roots[0].geometry.as_ref() else {
        unreachable!()
    };
    children.iter().cloned().collect()
}

#[test]
fn removals_overlay_old_slots_while_survivors_reflow_and_seeks_are_stateless() {
    let mut animation = animation(&["a", "b", "c"]);
    animation
        .write_list(1, 1.into(), 2.into(), items(&["b", "c"]), Easing::Linear)
        .unwrap();
    let middle = children(&animation, Fraction::new(3u64, 2u64), 100);
    assert_eq!(
        middle
            .iter()
            .map(|child| child.id.as_str())
            .collect::<Vec<_>>(),
        ["b", "c", "a"]
    );
    assert_eq!(middle[0].position.values[0], 10.);
    assert_eq!(middle[1].position.values[0], 20.);
    assert_eq!(middle[2].opacity, Fraction::new(1u64, 2u64));
    let after = children(&animation, 2.into(), 100);
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].position.values[0], 5.);
    assert_eq!(children(&animation, 0.into(), 100).len(), 3);
    assert_eq!(
        animation
            .sample_list(1, Fraction::new(3u64, 2u64))
            .unwrap()
            .unwrap(),
        items(&["a", "b", "c"])
    );
    assert_eq!(
        animation.sample_list(1, 2.into()).unwrap().unwrap(),
        items(&["b", "c"])
    );
}

#[test]
fn insertion_reorder_empty_and_resize_use_stable_identity() {
    let mut animation = animation(&["a", "b"]);
    animation
        .write_list(
            1,
            0.into(),
            1.into(),
            items(&["c", "b", "a"]),
            Easing::Linear,
        )
        .unwrap();
    let middle = children(&animation, Fraction::new(1u64, 2u64), 100);
    assert_eq!(middle[0].id, "c");
    assert_eq!(middle[0].opacity, Fraction::new(1u64, 2u64));
    assert_eq!(middle[1].position.values[0], 15.);
    assert_eq!(middle[2].position.values[0], 15.);
    // Parent resizing must keep local destination slots rather than embedding pixels.
    let wide = children(&animation, Fraction::new(1u64, 2u64), 200);
    assert_eq!(middle, wide);
    animation
        .write_list(1, 1.into(), 2.into(), items(&[]), Easing::Linear)
        .unwrap();
    assert!(children(&animation, 2.into(), 100).is_empty());
    assert_eq!(
        animation
            .sample_list(1, 2.into())
            .unwrap()
            .unwrap()
            .element_type
            .as_ref(),
        &Type::Element
    );
    assert_eq!(animation.element_count(), 4);
}

#[test]
fn timed_lists_preserve_prefix_cache_and_reject_overlapping_or_redefined_items_transactionally() {
    let mut animation = animation(&["a"]);
    animation
        .write_list(1, 2.into(), 3.into(), items(&["a", "b"]), Easing::Smooth)
        .unwrap();
    let old = animation.clone();
    animation
        .write_list(1, 4.into(), 5.into(), items(&["b"]), Easing::Linear)
        .unwrap();
    let diff = TimelineDiff::between(&old, &animation);
    assert!(
        diff.invalidated
            .iter()
            .all(|range| range.start >= Fraction::from(4))
    );
    assert_eq!(diff.invalidated[0].start, Fraction::from(4));
    let times = SampleTimes::new(&animation);
    assert_eq!(
        times.canonical(Fraction::new(5u64, 2u64)).unwrap(),
        Fraction::new(5u64, 2u64)
    );
    assert_eq!(
        times.canonical(Fraction::new(7u64, 2u64)).unwrap(),
        Fraction::from(3)
    );
    let future = animation.find("b").unwrap().clone();
    let move_item = sanim_core::Motion {
        element: future,
        property: sanim_core::Property::Position,
        to: Value::Vec2(Vec2::zero(Unit::Length)),
        from: None,
        reveal: None,
        horizontal: None,
        easing: None,
    };
    assert!(
        animation
            .schedule(&[move_item], 5.into(), 1.into(), Easing::Linear)
            .unwrap_err()
            .contains("flow layout")
    );
    let before = animation.clone();
    assert!(
        animation
            .write_list(
                1,
                Fraction::new(5u64, 2u64),
                4.into(),
                items(&[]),
                Easing::Linear
            )
            .unwrap_err()
            .contains("overlapping")
    );
    assert_eq!(animation, before);
    let mut replacement = tile("a");
    replacement.fill = Color::parse("red3").unwrap().into();
    assert!(
        animation
            .write_list(
                1,
                5.into(),
                6.into(),
                List::typed(Type::Element, vec![Value::Element(Arc::new(replacement))]),
                Easing::Linear
            )
            .unwrap_err()
            .contains("definition")
    );
    assert_eq!(animation, before);
    let roundtrip: Animation =
        serde_json::from_slice(&serde_json::to_vec(&animation).unwrap()).unwrap();
    assert_eq!(roundtrip, animation);
    assert_eq!(
        children(&roundtrip, Fraction::new(9u64, 2u64), 100),
        children(&animation, Fraction::new(9u64, 2u64), 100)
    );
}

#[test]
fn generic_list_events_include_explicit_future_noops_and_unbound_lists_do_not_animate_frames() {
    let mut animation = Animation::default();
    let empty = List::typed(Type::Numeric(Unit::Number), vec![]);
    let one = List::typed(
        Type::Numeric(Unit::Number),
        vec![Value::Number(sanim_core::Number::scalar(1))],
    );
    animation.set_list(7, empty.clone()).unwrap();
    animation
        .write_list(7, 2.into(), 2.into(), empty.clone(), Easing::Linear)
        .unwrap();
    animation
        .write_list(7, 0.into(), 0.into(), one.clone(), Easing::Linear)
        .unwrap();
    assert_eq!(animation.sample_list(7, 1.into()).unwrap().unwrap(), one);
    assert_eq!(animation.sample_list(7, 2.into()).unwrap().unwrap(), empty);
    assert_eq!(
        SampleTimes::new(&animation).canonical(1.into()).unwrap(),
        Fraction::from(0)
    );
}

#[test]
#[ignore = "manual indexed list-state performance measurement"]
fn retained_list_lookup_benchmark() {
    let mut animation = Animation::default();
    let list = |number| {
        List::typed(
            Type::Numeric(Unit::Number),
            vec![Value::Number(sanim_core::Number::scalar(number))],
        )
    };
    animation.set_list(7, list(0)).unwrap();
    const EVENTS: u64 = 2_048;
    const SAMPLES: u64 = 100_000;
    for index in 0..EVENTS {
        animation
            .write_list(
                7,
                index.into(),
                index.into(),
                list(index as i64),
                Easing::Linear,
            )
            .unwrap();
    }
    let started = std::time::Instant::now();
    for index in 0..SAMPLES {
        std::hint::black_box(
            animation
                .sample_list(7, Fraction::from(index % EVENTS))
                .unwrap(),
        );
    }
    println!(
        "list_events={EVENTS} samples={SAMPLES} ns_per_lookup={}",
        started.elapsed().as_nanos() / u128::from(SAMPLES)
    );
}

#[test]
fn spring_list_seeks_preserve_outgoing_items_until_the_actual_interval_end() {
    let mut animation = animation(&["a", "b"]);
    animation
        .write_list(1, 0.into(), 1.into(), items(&["b"]), Easing::Spring)
        .unwrap();
    for time in [
        Fraction::new(123_456_789u64, 1_000_000_000u64),
        Fraction::new(99u64, 100u64),
    ] {
        let sampled = children(&animation, time, 100);
        assert_eq!(sampled.len(), 2);
        let outgoing = sampled.iter().find(|element| element.id == "a").unwrap();
        assert!(outgoing.opacity > 0.into() && outgoing.opacity <= 1.into());
        assert!(outgoing.scale >= Fraction::new(23u64, 25u64));
    }
    assert_eq!(children(&animation, 1.into(), 100).len(), 1);
}
