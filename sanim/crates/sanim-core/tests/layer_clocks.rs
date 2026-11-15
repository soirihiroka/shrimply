use sanim_core::{
    Animation, Color, Direction, Easing, Element, Fraction, Geometry, Layout, List, ListEdit,
    Motion, Number, Property, SampleTimes, Type, Unit, Value, Vec2, reactive::Node,
};
use std::sync::Arc;

fn scalar(value: i64) -> Value {
    Value::Number(Number::scalar(value))
}
fn tile(id: &str) -> Element {
    Element {
        id: id.into(),
        geometry: Arc::new(Geometry::Box {
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
        layout: Layout {
            width: Some(Number {
                value: 10.into(),
                unit: Unit::Length,
            }),
            height: Some(Number {
                value: 10.into(),
                unit: Unit::Length,
            }),
            ..Default::default()
        },
    }
}
fn fade(animation: &mut Animation, element: Element, start: u64, duration: u64) {
    animation
        .schedule(
            &[Motion {
                element,
                property: Property::Opacity,
                to: scalar(0),
                from: None,
                reveal: None,
                horizontal: None,
                easing: None,
            }],
            start.into(),
            duration.into(),
            Easing::Linear,
        )
        .unwrap();
}

#[test]
fn unrelated_layer_motion_does_not_change_a_static_layers_clock() {
    let mut animation = Animation::default();
    animation.add(tile("background"), 0.into()).unwrap();
    let foreground = tile("foreground");
    animation.add(foreground.clone(), 0.into()).unwrap();
    fade(&mut animation, foreground, 1, 2);
    let clocks = SampleTimes::for_layers(&animation);
    assert_eq!(clocks.len(), 2);
    assert_eq!(clocks[0].constant_time(2.into()).unwrap(), Some(0.into()));
    assert_eq!(clocks[1].constant_time(2.into()).unwrap(), None);
    assert_eq!(clocks[1].constant_time(3.into()).unwrap(), Some(3.into()));
    assert_eq!(
        SampleTimes::new(&animation).canonical(2.into()).unwrap(),
        Fraction::from(2)
    );
    assert_eq!(
        SampleTimes::new(&animation).canonical(4.into()).unwrap(),
        Fraction::from(3)
    );
    assert!(clocks[0].constant_time(Fraction::from(-1)).is_err());
}

#[test]
fn layer_clocks_follow_shared_reactive_dependencies_and_mount_births() {
    let mut animation = Animation::default();
    animation.reactive.set_cell(1, scalar(1)).unwrap();
    animation
        .reactive
        .animate_cell(1, 1.into(), 7.into(), scalar(0), Easing::Linear)
        .unwrap();
    let node = animation.reactive.insert_node(Node::Cell(1), 10).unwrap();
    for id in ["first", "second"] {
        animation
            .reactive
            .bind(id.into(), Property::Opacity, node)
            .unwrap();
    }
    animation.add(tile("first"), 0.into()).unwrap();
    animation.add(tile("second"), 5.into()).unwrap();
    animation.add(tile("unrelated"), 0.into()).unwrap();
    let clocks = SampleTimes::for_layers(&animation);
    assert_eq!(clocks[0].constant_time(3.into()).unwrap(), None);
    assert_eq!(clocks[1].constant_time(3.into()).unwrap(), Some(0.into()));
    assert_eq!(clocks[1].constant_time(5.into()).unwrap(), None);
    assert_eq!(clocks[1].constant_time(8.into()).unwrap(), Some(7.into()));
    assert_eq!(clocks[2].constant_time(6.into()).unwrap(), Some(0.into()));
}

#[test]
fn nested_future_live_items_keep_their_parent_layer_clock_active() {
    let mut animation = Animation::default();
    animation.add(tile("background"), 0.into()).unwrap();
    let empty = List::typed(Type::Element, vec![]);
    let mut outer = tile("outer");
    outer.layout.direction = Some(Direction::Column);
    let mut inner = tile("inner");
    inner.layout.direction = Some(Direction::Column);
    animation.bind_list(1, &outer, empty.clone()).unwrap();
    animation.bind_list(2, &inner, empty.clone()).unwrap();
    animation.add(outer, 0.into()).unwrap();
    let edit = empty
        .edit(ListEdit::Append(Value::Element(Arc::new(inner))))
        .unwrap();
    animation
        .write_list_edit(1, 1.into(), 2.into(), &edit, Easing::Linear)
        .unwrap();
    let edit = empty
        .edit(ListEdit::Append(Value::Element(Arc::new(tile("future")))))
        .unwrap();
    animation
        .write_list_edit(2, 2.into(), 2.into(), &edit, Easing::Linear)
        .unwrap();
    animation.reactive.set_cell(3, scalar(1)).unwrap();
    animation
        .reactive
        .animate_cell(3, 3.into(), 4.into(), scalar(0), Easing::Linear)
        .unwrap();
    let node = animation.reactive.insert_node(Node::Cell(3), 10).unwrap();
    animation
        .reactive
        .bind("future".into(), Property::Opacity, node)
        .unwrap();
    let clocks = SampleTimes::for_layers(&animation);
    assert_eq!(
        clocks[0].constant_time(Fraction::new(3u64, 2u64)).unwrap(),
        Some(0.into())
    );
    assert_eq!(
        clocks[1].constant_time(Fraction::new(3u64, 2u64)).unwrap(),
        None
    );
    assert_eq!(
        clocks[1].constant_time(Fraction::new(7u64, 2u64)).unwrap(),
        None
    );
    assert_eq!(
        SampleTimes::new(&animation)
            .constant_time(Fraction::new(7u64, 2u64))
            .unwrap(),
        None
    );
}

#[test]
fn reused_layout_roots_skip_native_work_and_share_their_existing_geometry() {
    let mut animation = Animation::default();
    let mut background = tile("background");
    background.geometry = Arc::new(Geometry::Box {
        children: [tile("label")].into_iter().collect(),
        radius: Default::default(),
    });
    animation.add(background, 0.into()).unwrap();
    let foreground = tile("foreground");
    animation.add(foreground.clone(), 0.into()).unwrap();
    fade(&mut animation, foreground, 1, 2);
    let initial = animation.layout(0.into(), 100.into(), 100.into()).unwrap();
    let expected = animation.layout(2.into(), 100.into(), 100.into()).unwrap();
    let mut visited = Vec::new();
    let reused = animation
        .layout_with_reuse(
            2.into(),
            100.into(),
            100.into(),
            |element, _| {
                visited.push(element.id.clone());
                Ok(())
            },
            |index, _| (index == 0).then(|| initial[0].clone()),
        )
        .unwrap();
    assert_eq!(reused, expected);
    assert!(Arc::ptr_eq(&initial[0].geometry, &reused[0].geometry));
    assert_eq!(visited, ["foreground"]);
    assert!(
        animation
            .layout_with_reuse(
                Fraction::from(-1),
                100.into(),
                100.into(),
                |_, _| Ok(()),
                |_, _| Some(initial[0].clone())
            )
            .is_err()
    );
}

#[test]
fn code_presentation_events_only_advance_their_mounted_layer_clock() {
    use sanim_core::code::{
        Document, Language,
        view::{Options, PresentationEvent, State, Theme},
    };
    let mut state = State {
        document: Arc::new(Document::read("code.txt", "old", Language::Plain).unwrap()),
        options: Arc::new(Options {
            font: Default::default(),
            font_size: 12.into(),
            line_height: 16.into(),
            auto_line_height: false,
            tab_size: 4,
            line_numbers: false,
            scope_indicators: false,
            theme: Arc::new(Theme::default()),
            spans: vec![],
        }),
        folded: Default::default(),
        selection: Arc::from([]),
        end: 3.into(),
        events: Default::default(),
    };
    let before = state.snapshot();
    state.document = Arc::new(Document::read("code.txt", "new", Language::Plain).unwrap());
    state.events.insert(
        2.into(),
        PresentationEvent {
            cycle: None,
            interpolation: None,
            start: 2.into(),
            end: 3.into(),
            easing: Easing::Linear,
            animation: Default::default(),
            from: before,
            to: state.snapshot(),
        },
    );
    let mut code = tile("code");
    code.geometry = Arc::new(Geometry::Code {
        document: state.document.clone(),
        options: state.options.clone(),
        children: Default::default(),
        pending: false,
        source_changed: false,
        presentation: None,
    });
    let mut animation = Animation::default();
    animation.add(tile("background"), 0.into()).unwrap();
    animation.add(code, 1.into()).unwrap();
    animation.code_views.insert("code".into(), Arc::new(state));
    let clocks = SampleTimes::for_layers(&animation);
    assert_eq!(
        clocks[0].constant_time(Fraction::new(5u64, 2u64)).unwrap(),
        Some(0.into())
    );
    assert_eq!(
        clocks[1].constant_time(Fraction::new(5u64, 2u64)).unwrap(),
        None
    );
    assert_eq!(clocks[1].constant_time(3.into()).unwrap(), Some(3.into()));
    assert_eq!(
        clocks[1].constant_time(Fraction::new(3u64, 2u64)).unwrap(),
        Some(1.into())
    );
}

#[test]
fn motions_finished_before_mount_and_orphan_bindings_do_not_add_layer_work() {
    let mut animation = Animation::default();
    let late = tile("late");
    animation.add(late.clone(), 5.into()).unwrap();
    fade(&mut animation, late, 1, 2);
    animation.reactive.set_cell(99, scalar(1)).unwrap();
    animation
        .reactive
        .animate_cell(99, 0.into(), 10.into(), scalar(0), Easing::Linear)
        .unwrap();
    let node = animation.reactive.insert_node(Node::Cell(99), 10).unwrap();
    animation
        .reactive
        .bind("orphan".into(), Property::Opacity, node)
        .unwrap();
    let clocks = SampleTimes::for_layers(&animation);
    assert_eq!(clocks[0].constant_time(6.into()).unwrap(), Some(5.into()));
    assert_eq!(clocks[0].constant_time(2.into()).unwrap(), Some(0.into()));
    let mut native_calls = 0;
    let hidden = animation
        .layout_with_reuse(
            2.into(),
            100.into(),
            100.into(),
            |_, _| {
                native_calls += 1;
                Ok(())
            },
            |_, _| panic!("unborn roots cannot reuse a visible snapshot"),
        )
        .unwrap();
    assert!(hidden.is_empty());
    assert_eq!(native_calls, 0);
}
