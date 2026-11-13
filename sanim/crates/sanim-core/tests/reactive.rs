use sanim_core::{
    Animation, Color, Easing, Element, Fraction, Geometry, Layout, Number, Property, Reveal,
    SampleTimes, TimelineDiff, Unit, Value, Vec2,
    reactive::{Node, ReactiveGraph},
};
use std::sync::Arc;

#[test]
fn font_budget_shares_checkpoints_without_retaining_assets_or_confusing_cow_updates() {
    use sanim_core::value_budget::{Budget, charge_with_budget};
    let mut bytes = Arc::new(vec![0u8; 1024]);
    let weak = Arc::downgrade(&bytes);
    let original = Value::Font(sanim_core::Font::Asset(bytes.clone()));
    let mut budget = Budget::default();
    let (mut remaining, mut items) = (8192usize, 64usize);
    charge_with_budget(&original, &mut remaining, &mut items, 8, &mut budget).unwrap();
    let after_first = remaining;
    let checkpoint = budget.clone();
    let metadata = checkpoint.metadata_bytes();
    charge_with_budget(&original, &mut remaining, &mut items, 8, &mut budget).unwrap();
    assert_eq!(after_first - remaining, std::mem::size_of::<Value>());
    Arc::make_mut(&mut bytes)[0] = 1;
    let changed = Value::Font(sanim_core::Font::Asset(bytes));
    let before_change = remaining;
    charge_with_budget(&changed, &mut remaining, &mut items, 8, &mut budget).unwrap();
    assert!(before_change - remaining >= 1024);
    assert!(budget.metadata_bytes() > metadata);
    assert_eq!(checkpoint.metadata_bytes(), metadata);
    drop(original);
    assert!(
        weak.upgrade().is_none(),
        "budget guards must not retain font bytes"
    );
}

#[test]
fn failed_font_charges_cannot_register_unpaid_payloads() {
    use sanim_core::value_budget::{Budget, charge_with_budget};
    let font = Value::Font(sanim_core::Font::Asset(Arc::new(vec![0u8; 1024])));
    let mut budget = Budget::default();
    let (mut bytes, mut items) = (std::mem::size_of::<Value>(), 4);
    assert!(charge_with_budget(&font, &mut bytes, &mut items, 8, &mut budget).is_err());
    assert_eq!(budget.metadata_bytes(), 0);
    let (mut bytes, mut items) = (4096usize, 4usize);
    charge_with_budget(&font, &mut bytes, &mut items, 8, &mut budget).unwrap();
    assert!(4096 - bytes >= 1024);
}

#[test]
fn shared_font_assets_are_budgeted_once_inside_persistent_configuration_values() {
    let bytes = Arc::new(vec![0u8; 32 * 1024]);
    let fonts = Value::List(sanim_core::List::typed(
        sanim_core::Type::Font,
        (0..128)
            .map(|_| Value::Font(sanim_core::Font::Asset(bytes.clone())))
            .collect(),
    ));
    let mut remaining_bytes = 128 * 1024;
    let mut remaining_values = 256;
    sanim_core::value_budget::charge(&fonts, &mut remaining_bytes, &mut remaining_values, 8)
        .unwrap();
    assert!(remaining_bytes > 32 * 1024);
}

fn numeric(value: i64, unit: Unit) -> Value {
    Value::Number(Number {
        value: Fraction::from(value),
        unit,
    })
}
fn element(id: &str, geometry: Geometry) -> Element {
    Element {
        id: id.into(),
        geometry: Arc::new(geometry),
        position: Vec2::zero(Unit::Length),
        rotation: Fraction::from(0),
        scale: Fraction::from(1),
        opacity: Fraction::from(1),
        fill: Color::parse("blue3").unwrap().into(),
        stroke: None,
        reveal: Reveal::default(),
        horizontal: Default::default(),
        layout: Layout::default(),
    }
}
fn oscillator() -> (ReactiveGraph, usize) {
    let mut graph = ReactiveGraph::default();
    graph.set_cell(1, numeric(0, Unit::Angle)).unwrap();
    let phase = graph.insert_node(Node::Cell(1), 100).unwrap();
    let sine = graph
        .insert_node(
            Node::Math {
                name: "sin".into(),
                args: vec![phase],
            },
            100,
        )
        .unwrap();
    let quarter = graph
        .insert_node(
            Node::Constant(Value::Number(Number {
                value: Fraction::new(1u64, 4u64),
                unit: Unit::Number,
            })),
            100,
        )
        .unwrap();
    let product = graph
        .insert_node(
            Node::Binary {
                op: "*".into(),
                left: sine,
                right: quarter,
            },
            100,
        )
        .unwrap();
    let one = graph
        .insert_node(Node::Constant(numeric(1, Unit::Number)), 100)
        .unwrap();
    let size = graph
        .insert_node(
            Node::Binary {
                op: "+".into(),
                left: one,
                right: product,
            },
            100,
        )
        .unwrap();
    graph
        .animate_cell(
            1,
            Fraction::from(0),
            Fraction::from(10),
            numeric(1800, Unit::Angle),
            Easing::Linear,
        )
        .unwrap();
    (graph, size)
}
fn scalar(value: &Value) -> f64 {
    sanim_core::math::to_f64(value.number().unwrap().value).unwrap()
}

#[test]
fn shared_signal_is_evaluated_once_per_sample_and_seek_order_is_irrelevant() {
    let (graph, size) = oscillator();
    for (time, expected) in [
        (Fraction::new(1u64, 2u64), 1.25),
        (Fraction::from(10), 1.),
        (Fraction::new(3u64, 2u64), 0.75),
        (Fraction::from(0), 1.),
        (Fraction::from(5), 1.),
    ] {
        let mut sample = graph.sampler(time).unwrap();
        assert!((scalar(&sample.value(size).unwrap()) - expected).abs() < 1e-8);
        let evaluated = sample.evaluated_nodes();
        assert_eq!(evaluated, graph.node_count());
        sample.value(size).unwrap();
        assert_eq!(sample.evaluated_nodes(), evaluated);
    }
}

#[test]
fn retained_templates_format_shared_values_and_seek_without_source_evaluation() {
    use sanim_core::TemplatePart;
    let mut graph = ReactiveGraph::default();
    graph.set_cell(1, numeric(1, Unit::Number)).unwrap();
    graph.set_cell(2, Value::String("03d".into())).unwrap();
    let value = graph.insert_node(Node::Cell(1), 100).unwrap();
    let format = graph.insert_node(Node::Cell(2), 100).unwrap();
    let parts = vec![
        TemplatePart::Text("value=".into()),
        TemplatePart::Hole {
            value,
            format: Some(format),
            conversion: None,
        },
        TemplatePart::Text("\n# literal".into()),
    ];
    let text = graph
        .insert_node(
            Node::Template {
                parts: parts.into(),
            },
            100,
        )
        .unwrap();
    graph
        .animate_cell(
            1,
            0.into(),
            1.into(),
            numeric(3, Unit::Number),
            Easing::Linear,
        )
        .unwrap();
    graph
        .write_cell(2, 1.into(), Value::String("b".into()))
        .unwrap();
    let restored: ReactiveGraph =
        serde_json::from_slice(&serde_json::to_vec(&graph).unwrap()).unwrap();
    for (time, expected) in [
        (Fraction::new(1u64, 2u64), "value=002\n# literal"),
        (1.into(), "value=11\n# literal"),
        (0.into(), "value=001\n# literal"),
    ] {
        let mut sampler = restored.sampler(time).unwrap();
        assert_eq!(sampler.value(text).unwrap().string().unwrap(), expected);
        let evaluated = sampler.evaluated_nodes();
        sampler.value(text).unwrap();
        assert_eq!(sampler.evaluated_nodes(), evaluated);
    }
    let dependencies = restored.cell_dependencies(text).unwrap();
    assert_eq!(dependencies.len(), 2);
    assert!(dependencies.contains(&1) && dependencies.contains(&2));
}

#[test]
fn retained_properties_share_scene_sampling_and_preserve_static_geometry() {
    let (mut graph, size) = oscillator();
    graph.bind("circle".into(), Property::Scale, size).unwrap();
    graph
        .bind("rectangle".into(), Property::Scale, size)
        .unwrap();
    let mut animation = Animation::default();
    animation.reactive = graph;
    animation
        .add(
            element(
                "circle",
                Geometry::Circle {
                    radius: Fraction::from(10),
                },
            ),
            Fraction::from(0),
        )
        .unwrap();
    animation
        .add(
            element(
                "rectangle",
                Geometry::Rectangle {
                    width: Fraction::from(20),
                    height: Fraction::from(10),
                    children: Default::default(),
                    radius: Fraction::from(0).into(),
                },
            ),
            Fraction::from(0),
        )
        .unwrap();
    let frame = animation.sample(Fraction::new(1u64, 2u64)).unwrap();
    assert_eq!(frame[0].scale, Fraction::new(5u64, 4u64));
    assert_eq!(frame[1].scale, frame[0].scale);
    assert!(Arc::ptr_eq(
        &frame[0].geometry,
        &animation.layers[0].element.geometry
    ));
    assert_eq!(
        animation
            .sample_property(
                &animation.layers[0].element,
                Property::Scale,
                Fraction::new(1u64, 2u64)
            )
            .unwrap()
            .number()
            .unwrap()
            .value,
        frame[0].scale
    );
}

#[test]
fn retained_lengths_update_geometry_without_mutating_original_definitions() {
    let mut graph = ReactiveGraph::default();
    graph.set_cell(1, numeric(10, Unit::Length)).unwrap();
    let size = graph.insert_node(Node::Cell(1), 10).unwrap();
    graph.bind("circle".into(), Property::Radius, size).unwrap();
    graph.bind("rect".into(), Property::Width, size).unwrap();
    graph.bind("rect".into(), Property::Height, size).unwrap();
    graph
        .animate_cell(
            1,
            Fraction::from(0),
            Fraction::from(2),
            numeric(20, Unit::Length),
            Easing::Linear,
        )
        .unwrap();
    let mut animation = Animation::default();
    animation.reactive = graph;
    animation
        .add(
            element(
                "circle",
                Geometry::Circle {
                    radius: Fraction::from(10),
                },
            ),
            Fraction::from(0),
        )
        .unwrap();
    animation
        .add(
            element(
                "rect",
                Geometry::Rectangle {
                    width: Fraction::from(10),
                    height: Fraction::from(10),
                    children: Default::default(),
                    radius: Fraction::from(0).into(),
                },
            ),
            Fraction::from(0),
        )
        .unwrap();
    let frame = animation.sample(Fraction::from(1)).unwrap();
    assert!(
        matches!(frame[0].geometry.as_ref(),Geometry::Circle{radius} if *radius==Fraction::from(15))
    );
    assert!(
        matches!(frame[1].geometry.as_ref(),Geometry::Rectangle {width,height, .. } if *width==Fraction::from(15)&&*height==Fraction::from(15))
    );
    assert!(
        matches!(animation.layers[0].element.geometry.as_ref(),Geometry::Circle{radius} if *radius==Fraction::from(10))
    );
}

#[test]
fn instantaneous_writes_take_effect_at_the_exact_boundary_and_preserve_old_samples() {
    let mut graph = ReactiveGraph::default();
    graph.set_cell(1, numeric(1, Unit::Number)).unwrap();
    graph
        .write_cell(1, Fraction::from(2), numeric(3, Unit::Number))
        .unwrap();
    graph
        .write_cell(1, Fraction::from(2), numeric(4, Unit::Number))
        .unwrap();
    assert_eq!(
        scalar(&graph.sample_cell(1, Fraction::from(1)).unwrap()),
        1.
    );
    assert_eq!(
        scalar(&graph.sample_cell(1, Fraction::from(2)).unwrap()),
        4.
    );
    graph
        .animate_cell(
            1,
            Fraction::from(2),
            Fraction::from(4),
            numeric(8, Unit::Number),
            Easing::Linear,
        )
        .unwrap();
    assert_eq!(
        scalar(&graph.sample_cell(1, Fraction::from(3)).unwrap()),
        6.
    );
    assert_eq!(
        scalar(&graph.sample_cell(1, Fraction::from(4)).unwrap()),
        8.
    );
    assert!(
        graph
            .write_cell(1, Fraction::from(3), numeric(0, Unit::Number))
            .unwrap_err()
            .contains("overlapping")
    );
    graph
        .write_cell(1, Fraction::from(4), numeric(9, Unit::Number))
        .unwrap();
    assert_eq!(
        scalar(&graph.sample_cell(1, Fraction::from(4)).unwrap()),
        9.
    );
}

#[test]
fn channel_edits_preserve_earlier_frame_cache_intervals_and_active_times_are_not_collapsed() {
    let mut graph = ReactiveGraph::default();
    graph.set_cell(1, numeric(1, Unit::Number)).unwrap();
    let node = graph.insert_node(Node::Cell(1), 10).unwrap();
    graph.bind("circle".into(), Property::Scale, node).unwrap();
    let mut old = Animation::default();
    old.reactive = graph.clone();
    old.add(
        element(
            "circle",
            Geometry::Circle {
                radius: Fraction::from(1),
            },
        ),
        Fraction::from(0),
    )
    .unwrap();
    old.reactive
        .animate_cell(
            1,
            Fraction::from(5),
            Fraction::from(7),
            numeric(2, Unit::Number),
            Easing::Linear,
        )
        .unwrap();
    let mut new = old.clone();
    new.reactive = graph;
    new.reactive
        .animate_cell(
            1,
            Fraction::from(5),
            Fraction::from(7),
            numeric(3, Unit::Number),
            Easing::Linear,
        )
        .unwrap();
    let diff = TimelineDiff::between(&old, &new);
    assert!(!diff.affects(Fraction::from(4)));
    assert!(diff.affects(Fraction::from(5)));
    let times = SampleTimes::new(&new);
    assert_eq!(
        times.canonical(Fraction::from(4)).unwrap(),
        Fraction::from(0)
    );
    assert_eq!(
        times.canonical(Fraction::from(6)).unwrap(),
        Fraction::from(6)
    );
    assert_eq!(
        times.canonical(Fraction::from(8)).unwrap(),
        Fraction::from(7)
    );
}

#[test]
fn serde_rejects_forward_references_corrupt_types_and_unbounded_tracks() {
    let (graph, size) = oscillator();
    let json = serde_json::to_value(&graph).unwrap();
    assert!(json.get("metadata_bytes").is_none());
    assert!(json.get("track_count").is_none());
    let restored: ReactiveGraph = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(restored, graph);
    assert_eq!(
        restored
            .sampler(Fraction::from(1))
            .unwrap()
            .value(size)
            .unwrap(),
        graph
            .sampler(Fraction::from(1))
            .unwrap()
            .value(size)
            .unwrap()
    );
    let mut corrupt = json.clone();
    corrupt["nodes"][1]["operation"]["Math"]["args"] = serde_json::json!([size]);
    assert!(serde_json::from_value::<ReactiveGraph>(corrupt).is_err());
    let mut corrupt = json;
    corrupt["cells"]["1"]["tracks"][0]["end"] = serde_json::to_value(Fraction::from(7201)).unwrap();
    assert!(serde_json::from_value::<ReactiveGraph>(corrupt).is_err());
}

#[test]
fn typed_nodes_and_tracks_reject_invalid_inputs_without_committing_changes() {
    let mut graph = ReactiveGraph::default();
    graph.set_cell(1, numeric(1, Unit::Number)).unwrap();
    let node = graph.insert_node(Node::Cell(1), 1).unwrap();
    assert!(
        graph
            .insert_node(Node::Constant(numeric(1, Unit::Number)), 1)
            .is_err()
    );
    assert!(graph.bind("circle".into(), Property::Radius, node).is_err());
    let original = graph.clone();
    assert!(
        graph
            .write_cell(1, Fraction::from(0), numeric(1, Unit::Angle))
            .is_err()
    );
    assert!(
        graph
            .animate_cell(
                1,
                Fraction::from(0),
                Fraction::from(7201),
                numeric(2, Unit::Number),
                Easing::Linear
            )
            .is_err()
    );
    assert_eq!(graph, original);
    assert!(
        graph
            .insert_node(
                Node::Unary {
                    op: "-".into(),
                    value: 10
                },
                100
            )
            .is_err()
    );
}

#[test]
fn approximate_trigonometry_has_headroom_for_ordinary_followup_math() {
    for angle in [0, 90, 180, 450, 900, 1800] {
        let sine = sanim_core::math::functions::evaluate(
            "sin",
            &[Number {
                value: Fraction::from(angle),
                unit: Unit::Angle,
            }],
        )
        .unwrap();
        let scaled = sanim_core::math::operations::binary(
            "*",
            Value::Number(sine),
            Value::Number(Number {
                value: Fraction::new(1u64, 4u64),
                unit: Unit::Number,
            }),
        )
        .unwrap();
        let size =
            sanim_core::math::operations::binary("+", numeric(1, Unit::Number), scaled).unwrap();
        assert!((0.75..=1.25).contains(&scalar(&size)));
    }
}

#[test]
#[ignore = "microbenchmark: actual retained graph and geometry samples across ten seconds"]
fn retained_frame_benchmark() {
    let (mut graph, size) = oscillator();
    graph.bind("circle".into(), Property::Scale, size).unwrap();
    graph
        .bind("rectangle".into(), Property::Scale, size)
        .unwrap();
    let mut animation = Animation::default();
    animation.reactive = graph;
    animation
        .add(
            element(
                "circle",
                Geometry::Circle {
                    radius: Fraction::from(10),
                },
            ),
            Fraction::from(0),
        )
        .unwrap();
    animation
        .add(
            element(
                "rectangle",
                Geometry::Rectangle {
                    width: Fraction::from(20),
                    height: Fraction::from(10),
                    children: Default::default(),
                    radius: Fraction::from(0).into(),
                },
            ),
            Fraction::from(0),
        )
        .unwrap();
    const PASSES: u64 = 30;
    const LAST_FRAME: u64 = 300;
    const RATE: u64 = 30;
    let started = std::time::Instant::now();
    for _ in 0..PASSES {
        for frame in 0..=LAST_FRAME {
            std::hint::black_box(animation.sample(Fraction::new(frame, RATE)).unwrap());
        }
    }
    let frames = PASSES * (LAST_FRAME + 1);
    println!(
        "retained_frames={frames} elapsed_us={} ns_per_frame={} graph_nodes={} metadata_bytes={}",
        started.elapsed().as_micros(),
        started.elapsed().as_nanos() / u128::from(frames),
        animation.reactive.node_count(),
        animation.reactive.metadata_bytes()
    );
}

#[test]
fn rounded_rectangle_radius_samples_are_nonnegative_and_keep_the_same_outer_bounds() {
    let mut graph = ReactiveGraph::default();
    graph.set_cell(1, numeric(0, Unit::Length)).unwrap();
    let radius = graph.insert_node(Node::Cell(1), 10).unwrap();
    graph
        .bind("rectangle".into(), Property::Radius, radius)
        .unwrap();
    graph
        .animate_cell(
            1,
            Fraction::from(0),
            Fraction::from(2),
            numeric(10, Unit::Length),
            Easing::Linear,
        )
        .unwrap();
    let mut animation = Animation::default();
    animation.reactive = graph;
    animation
        .add(
            element(
                "rectangle",
                Geometry::Rectangle {
                    width: Fraction::from(20),
                    height: Fraction::from(20),
                    children: Default::default(),
                    radius: Fraction::from(0).into(),
                },
            ),
            Fraction::from(0),
        )
        .unwrap();
    let sharp = animation.sample(Fraction::from(0)).unwrap();
    let rounded = animation.sample(Fraction::from(1)).unwrap();
    assert_eq!(
        rounded[0].property(Property::Radius).unwrap(),
        numeric(5, Unit::Length)
    );
    assert_eq!(
        sanim_core::verify::bounds(&sharp[0]).unwrap(),
        sanim_core::verify::bounds(&rounded[0]).unwrap()
    );
    let original = rounded[0].geometry.clone();
    let mut rejected = rounded[0].clone();
    assert!(
        rejected
            .set(Property::Radius, numeric(-1, Unit::Length))
            .is_err()
    );
    assert!(Arc::ptr_eq(&original, &rejected.geometry));
}

#[test]
fn background_points_and_animations_insert_chronologically_and_rebase_future_starts() {
    let mut graph = ReactiveGraph::default();
    graph.set_cell(1, numeric(0, Unit::Number)).unwrap();
    graph
        .write_cell(1, Fraction::from(2), numeric(9, Unit::Number))
        .unwrap();
    graph
        .write_cell(1, Fraction::from(0), numeric(3, Unit::Number))
        .unwrap();
    graph
        .write_cell(1, Fraction::from(1), numeric(5, Unit::Number))
        .unwrap();
    graph
        .write_cell(1, Fraction::from(0), numeric(4, Unit::Number))
        .unwrap();
    for (time, expected) in [(2, 9.), (0, 4.), (1, 5.)] {
        assert_eq!(
            scalar(&graph.sample_cell(1, Fraction::from(time)).unwrap()),
            expected
        );
    }
    graph.validate().unwrap();
    let mut future = ReactiveGraph::default();
    future.set_cell(1, numeric(0, Unit::Number)).unwrap();
    future
        .animate_cell(
            1,
            Fraction::from(2),
            Fraction::from(4),
            numeric(10, Unit::Number),
            Easing::Linear,
        )
        .unwrap();
    future
        .write_cell(1, Fraction::from(0), numeric(4, Unit::Number))
        .unwrap();
    assert_eq!(
        scalar(&future.sample_cell(1, Fraction::from(2)).unwrap()),
        4.
    );
    assert_eq!(
        scalar(&future.sample_cell(1, Fraction::from(3)).unwrap()),
        7.
    );
    let before = future.clone();
    assert!(
        future
            .animate_cell(
                1,
                Fraction::from(1),
                Fraction::from(3),
                numeric(2, Unit::Number),
                Easing::Linear
            )
            .is_err()
    );
    assert_eq!(future, before);
    future
        .animate_cell(
            1,
            Fraction::from(0),
            Fraction::from(2),
            numeric(6, Unit::Number),
            Easing::Linear,
        )
        .unwrap();
    assert_eq!(
        scalar(&future.sample_cell(1, Fraction::from(2)).unwrap()),
        6.
    );
    assert_eq!(
        scalar(&future.sample_cell(1, Fraction::from(3)).unwrap()),
        8.
    );
    future.validate().unwrap();
}

#[test]
fn typed_vector_nodes_and_components_share_cells_and_validate_serialized_indices() {
    let mut graph = ReactiveGraph::default();
    graph.set_cell(1, numeric(10, Unit::Length)).unwrap();
    graph.set_cell(2, numeric(20, Unit::Length)).unwrap();
    let x = graph.insert_node(Node::Cell(1), 20).unwrap();
    let y = graph.insert_node(Node::Cell(2), 20).unwrap();
    let vector = graph
        .insert_node(Node::Vector { values: vec![x, y] }, 20)
        .unwrap();
    let component = graph
        .insert_node(Node::Component { vector, index: 0 }, 20)
        .unwrap();
    graph
        .animate_cell(
            1,
            Fraction::from(0),
            Fraction::from(2),
            numeric(30, Unit::Length),
            Easing::Linear,
        )
        .unwrap();
    let mut sample = graph.sampler(Fraction::from(1)).unwrap();
    assert_eq!(
        sample.value(vector).unwrap().vec2().unwrap().values,
        [20., 20.]
    );
    assert_eq!(sample.value(component).unwrap(), numeric(20, Unit::Length));
    assert_eq!(sample.evaluated_nodes(), 4);
    assert_eq!(
        graph
            .cell_dependencies(component)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        [1, 2]
    );
    let unitless = graph
        .insert_node(Node::Constant(numeric(0, Unit::Number)), 20)
        .unwrap();
    assert!(
        graph
            .insert_node(
                Node::Vector {
                    values: vec![x, unitless]
                },
                20
            )
            .is_err()
    );
    assert!(
        graph
            .insert_node(Node::Component { vector, index: 2 }, 20)
            .is_err()
    );
    let mut json = serde_json::to_value(&graph).unwrap();
    json["nodes"][component]["operation"]["Component"]["index"] = serde_json::json!(2);
    assert!(serde_json::from_value::<ReactiveGraph>(json).is_err());
}

#[test]
fn group_dimensions_are_layout_properties_with_clear_automatic_value_errors() {
    let mut group = element(
        "group",
        Geometry::Box {
            radius: Default::default(),
            children: imbl::Vector::new(),
        },
    );
    assert!(
        group
            .property(Property::Width)
            .unwrap_err()
            .contains("automatic")
    );
    let geometry = group.geometry.clone();
    group
        .set(Property::Width, numeric(100, Unit::Length))
        .unwrap();
    group
        .set(Property::Height, numeric(40, Unit::Length))
        .unwrap();
    assert_eq!(
        group.property(Property::Width).unwrap(),
        numeric(100, Unit::Length)
    );
    assert_eq!(
        group.property(Property::Height).unwrap(),
        numeric(40, Unit::Length)
    );
    assert!(Arc::ptr_eq(&geometry, &group.geometry));
    assert!(
        group
            .set(Property::Radius, numeric(1, Unit::Length))
            .is_err()
    );
}
