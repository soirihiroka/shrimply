use sanim_core::{plan::*, *};
use std::sync::Arc;
fn hold(seconds: u64) -> Arc<plan::Animation> {
    Arc::new(plan::Animation::new(Node::Hold(Some(seconds.into()))))
}
fn container(children: Vec<Arc<plan::Animation>>, flow: Flow) -> plan::Animation {
    let mut p = plan::Animation::new(Node::Container(children.into_iter().collect()));
    p.style.flow = flow;
    p
}
#[test]
fn same_cow_plan_resolves_once_per_placement_and_viewport() {
    let child = hold(1);
    let mut root = container(vec![child.clone(), child], Flow::Sequence);
    root.style.justify = Justify::SpaceBetween;
    let a = root.layout(Some(10.into()), 100).unwrap();
    assert_eq!(a.placements[1].start, Fraction::from(0));
    assert_eq!(a.placements[2].start, Fraction::from(9));
    assert!(Arc::ptr_eq(
        &a.placements[1].animation,
        &a.placements[2].animation
    ));
    let b = root.layout(Some(4.into()), 100).unwrap();
    assert_eq!(b.placements[2].start, Fraction::from(3));
    assert_eq!(root.measure().unwrap(), Extent::Finite(2.into()));
}
#[test]
fn padding_gap_margin_and_relative_offset_are_exact() {
    let mut child = hold(1).as_ref().clone();
    child.style.margin_start = Fraction::new(1u64, 4u64);
    child.style.margin_end = Fraction::new(1u64, 4u64);
    child.style.offset = Fraction::new(1u64, 2u64);
    let mut root = container(vec![Arc::new(child), hold(2)], Flow::Sequence);
    root.style.padding_start = 1.into();
    root.style.padding_end = 1.into();
    root.style.gap = Fraction::new(1u64, 2u64);
    let result = root.layout(None, 100).unwrap();
    assert_eq!(result.duration, Some(6.into()));
    assert_eq!(result.placements[1].start, Fraction::new(7u64, 4u64));
    assert_eq!(result.placements[2].start, Fraction::from(3));
}
#[test]
fn matching_parent_uses_content_duration_and_requires_real_constraint() {
    let mut child = hold(1).as_ref().clone();
    child.style.duration = TimeDimension::MatchParent;
    child.style.margin_start = 1.into();
    child.style.margin_end = 1.into();
    let mut root = container(vec![Arc::new(child)], Flow::Parallel);
    root.style.padding_start = 1.into();
    root.style.padding_end = 1.into();
    let result = root.layout(Some(10.into()), 100).unwrap();
    assert_eq!(result.placements[1].duration, Some(6.into()));
    assert!(root.layout(None, 100).is_err());
}
#[test]
fn absolute_children_do_not_grow_the_temporal_viewport() {
    let mut side = hold(5).as_ref().clone();
    side.style.position_mode = PositionMode::Absolute;
    let root = container(vec![Arc::new(side), hold(2)], Flow::Sequence);
    let result = root.layout(None, 100).unwrap();
    assert_eq!(result.duration, Some(2.into()));
    assert_eq!(result.placements[2].start, Fraction::from(0));
    assert_eq!(root.measure().unwrap(), Extent::Finite(2.into()));
}
#[test]
fn infinite_cycles_are_compact_and_budgeted() {
    let cycle = plan::Animation::new(Node::Repeat {
        animation: hold(2),
        count: None,
    });
    let result = cycle.layout(None, 100).unwrap();
    assert_eq!(result.duration, None);
    assert_eq!(result.placements.len(), 2);
    let result = cycle.layout(Some(100.into()), 100).unwrap();
    assert_eq!(result.duration, Some(100.into()));
    assert_eq!(result.placements.len(), 2);
    let huge = plan::Animation::new(Node::Repeat {
        animation: hold(1),
        count: Some(MAX_PLAN_NODES + 1),
    });
    assert!(huge.layout(None, MAX_PLAN_NODES).is_err());
    let invalid = plan::Animation::new(Node::Repeat {
        animation: hold(0),
        count: None,
    });
    assert!(invalid.measure().is_err());
}
#[test]
fn viewport_clips_overflow_instead_of_rescaling_work() {
    let root = container(vec![hold(10)], Flow::Sequence);
    let layout = root.layout(Some(2.into()), 100).unwrap();
    assert_eq!(layout.duration, Some(2.into()));
    assert_eq!(layout.placements[1].duration, Some(10.into()));
    assert_eq!(layout.placements[1].clip, Some((0.into(), 2.into())));
}
#[test]
fn shared_budget_charges_plan_tree_once_and_does_not_keep_it_alive() {
    let root = Arc::new(container(vec![hold(1), hold(2)], Flow::Sequence));
    let weak = Arc::downgrade(&root);
    let mut budget = value_budget::Budget::default();
    let mut bytes = 100_000;
    let mut items = 100;
    let value = Value::Animation(root.clone());
    value_budget::charge_with_budget(&value, &mut bytes, &mut items, 100, &mut budget).unwrap();
    let first = bytes;
    value_budget::charge_with_budget(&value, &mut bytes, &mut items, 100, &mut budget).unwrap();
    assert!(first - bytes < std::mem::size_of::<Value>() * 2);
    drop(value);
    drop(root);
    assert!(weak.upgrade().is_none());
}
#[test]
#[ignore = "manual temporal layout benchmark"]
fn temporal_layout_benchmark() {
    let children = (0..1000).map(|_| hold(1)).collect();
    let mut root = container(children, Flow::Sequence);
    root.style.justify = Justify::SpaceBetween;
    let mut samples = Vec::new();
    for _ in 0..100 {
        let started = std::time::Instant::now();
        let layout = std::hint::black_box(&root)
            .layout(Some(2000.into()), 2000)
            .unwrap();
        assert_eq!(layout.placements.len(), 1001);
        samples.push(started.elapsed().as_micros());
    }
    samples.sort();
    println!(
        "temporal layout 1000 placements / 100 viewport relayouts: median={}us p95={}us",
        samples[50], samples[95]
    );
}
#[test]
fn absolute_infinite_content_is_clipped_by_the_finite_flow_box() {
    let mut infinite = plan::Animation::new(Node::Repeat {
        animation: hold(2),
        count: None,
    });
    infinite.style.position_mode = PositionMode::Absolute;
    let root = container(vec![hold(10), Arc::new(infinite)], Flow::Sequence);
    let layout = root.layout(None, 100).unwrap();
    assert_eq!(layout.duration, Some(10.into()));
    assert_eq!(layout.placements[2].start, Fraction::from(0));
    assert_eq!(layout.placements[2].clip, Some((0.into(), 10.into())));
    assert_eq!(root.measure().unwrap(), Extent::Finite(10.into()));
}
#[test]
fn parallel_alignment_uses_each_childs_resolved_extent() {
    let mut root = container(vec![hold(1), hold(2)], Flow::Parallel);
    root.style.justify = Justify::End;
    let layout = root.layout(Some(10.into()), 100).unwrap();
    assert_eq!(layout.placements[1].start, Fraction::from(9));
    assert_eq!(layout.placements[2].start, Fraction::from(8));
}
#[test]
fn finite_prefix_uses_visible_intervals_before_indefinite_holds() {
    let mut first = plan::Animation::new(Node::Hold(Some(Fraction::new(1u64, 10u64))));
    first.style.overflow = Overflow::Visible;
    let root = container(
        vec![
            Arc::new(first),
            Arc::new(plan::Animation::new(Node::Hold(None))),
        ],
        Flow::Sequence,
    );
    let result = root.layout(None, 100).unwrap();
    assert_eq!(result.duration, None);
    assert_eq!(result.finite_prefix, Fraction::new(1u64, 10u64));
    let mut cropped = container(vec![hold(4)], Flow::Parallel);
    cropped.style.duration = TimeDimension::Fixed(1.into());
    let cycle = plan::Animation::new(Node::Repeat {
        animation: Arc::new(container(vec![Arc::new(cropped), hold(1)], Flow::Sequence)),
        count: None,
    });
    assert_eq!(
        cycle.layout(None, 100).unwrap().finite_prefix,
        Fraction::from(2)
    );
}
#[test]
fn unused_trailing_gap_does_not_overflow_the_two_hour_horizon() {
    let mut root = container(vec![hold(math::MAX_TIMELINE_SECONDS)], Flow::Sequence);
    root.style.gap = 1.into();
    assert_eq!(
        root.layout(None, 100).unwrap().duration,
        Some(math::MAX_TIMELINE_SECONDS.into())
    );
}
#[test]
fn external_constraint_does_not_leak_through_an_intrinsic_auto_parent() {
    let mut matching = hold(1).as_ref().clone();
    matching.style.duration = TimeDimension::MatchParent;
    let inner = container(vec![Arc::new(matching)], Flow::Parallel);
    let root = container(vec![Arc::new(inner)], Flow::Parallel);
    assert!(
        root.layout(Some(10.into()), 100)
            .unwrap_err()
            .to_string()
            .contains("auto parent")
    );
}
#[test]
fn repeat_occupancy_includes_child_margins_gaps_and_outer_padding() {
    let mut child = hold(1).as_ref().clone();
    child.style.margin_start = 1.into();
    child.style.margin_end = 1.into();
    let child = Arc::new(child);
    let finite = plan::Animation::new(Node::Repeat {
        animation: child.clone(),
        count: Some(2),
    });
    assert_eq!(finite.measure().unwrap(), Extent::Finite(6.into()));
    let placed = finite.layout(None, 100).unwrap();
    assert_eq!(placed.placements[1].start, Fraction::from(1));
    assert_eq!(placed.placements.len(), 2);
    assert_eq!(placed.placements[0].cycle.unwrap().until(), Some(6.into()));
    assert_eq!(
        placed.placements[0]
            .cycle
            .unwrap()
            .time(Fraction::new(9u64, 2u64))
            .unwrap(),
        Fraction::new(3u64, 2u64)
    );
    let mut spaced = plan::Animation::new(Node::Repeat {
        animation: child.clone(),
        count: Some(3),
    });
    spaced.style.gap = 2.into();
    spaced.style.padding_start = 1.into();
    spaced.style.padding_end = 1.into();
    assert_eq!(spaced.measure().unwrap(), Extent::Finite(15.into()));
    let placed = spaced.layout(None, 100).unwrap();
    assert_eq!(placed.placements.len(), 2);
    assert_eq!(placed.placements[0].cycle.unwrap().until(), Some(14.into()));
    assert_eq!(
        placed.placements[0]
            .cycle
            .unwrap()
            .time(Fraction::new(25u64, 2u64))
            .unwrap(),
        Fraction::new(5u64, 2u64)
    );
    let mut infinite = plan::Animation::new(Node::Repeat {
        animation: child,
        count: None,
    });
    infinite.style.gap = 1.into();
    let placed = infinite.layout(None, 100).unwrap();
    assert_eq!(placed.placements[0].cycle.unwrap().end(), Fraction::from(4));
    assert_eq!(placed.finite_prefix, Fraction::from(4));
}
#[test]
fn infinite_flow_allows_absolute_siblings_without_advancing_them() {
    let mut side = hold(1).as_ref().clone();
    side.style.position_mode = PositionMode::Absolute;
    let root = container(
        vec![
            Arc::new(plan::Animation::new(Node::Hold(None))),
            Arc::new(side),
        ],
        Flow::Sequence,
    );
    let result = root.layout(None, 100).unwrap();
    assert_eq!(result.duration, None);
    assert_eq!(result.placements[2].start, Fraction::from(0));
}
#[test]
fn zero_length_writes_have_closed_point_boundaries() {
    let window = math::temporal::Window {
        clip: Some((0.into(), 1.into())),
        cycle: None,
    };
    assert!(window.interval(1.into(), 0.into()).unwrap().is_some());
    assert!(window.interval(1.into(), 1.into()).unwrap().is_none());
    assert!(window.interval(2.into(), 0.into()).unwrap().is_none());
    let point = math::temporal::Window {
        clip: Some((0.into(), 0.into())),
        cycle: None,
    };
    assert!(point.interval(0.into(), 0.into()).unwrap().is_some());
    assert!(point.interval(0.into(), 1.into()).unwrap().is_none());
}
#[test]
fn millions_of_finite_repetitions_compile_one_bounded_cycle() {
    let body = Arc::new(plan::Animation::new(Node::Hold(Some(Fraction::new(
        1u64, 1000u64,
    )))));
    let plan = plan::Animation::new(Node::Repeat {
        animation: body,
        count: Some(7_000_000),
    });
    let result = plan.layout(None, 10).unwrap();
    assert_eq!(result.duration, Some(7000.into()));
    assert_eq!(result.placements.len(), 2);
    assert_eq!(
        result.placements[0].cycle.unwrap().until(),
        Some(7000.into())
    );
}
#[test]
fn parent_constraint_failures_are_typed() {
    let mut root = hold(1).as_ref().clone();
    root.style.duration = TimeDimension::MatchParent;
    assert!(matches!(
        root.layout(None, 100),
        Err(LayoutError::MissingParent)
    ));
    let nested = container(vec![Arc::new(root)], Flow::Parallel);
    assert!(matches!(
        container(vec![Arc::new(nested)], Flow::Parallel).layout(Some(10.into()), 100),
        Err(LayoutError::CyclicParent)
    ));
    let mut invalid = hold(1).as_ref().clone();
    invalid.style.duration = TimeDimension::Fixed((-1).into());
    assert!(matches!(
        invalid.layout(None, 100),
        Err(LayoutError::Invalid(_))
    ));
}
