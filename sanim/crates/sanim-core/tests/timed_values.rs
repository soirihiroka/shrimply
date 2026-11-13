use sanim_core::{
    Easing, Fraction, Number, Type, Unit, Value, Vec3, Vec4,
    reactive::{Node, ReactiveGraph},
};
use std::sync::Arc;

fn number(value: i64) -> Value {
    Value::Number(Number {
        value: Fraction::from(value),
        unit: Unit::Number,
    })
}
fn transition(ty: Type, from: Value, to: Value) -> ReactiveGraph {
    let mut graph = ReactiveGraph::default();
    graph.set_typed_cell(1, ty, from).unwrap();
    graph
        .animate_cell(1, Fraction::from(1), Fraction::from(3), to, Easing::Linear)
        .unwrap();
    graph.validate().unwrap();
    graph
}
fn assert_discrete(ty: Type, from: Value, to: Value) {
    let graph = transition(ty, from.clone(), to.clone());
    // Seek in arbitrary order: sampling never mutates the checkpoint or another sample.
    for (time, expected) in [(3, &to), (0, &from), (2, &from), (4, &to), (1, &from)] {
        assert_eq!(
            &graph.sample_cell(1, Fraction::from(time)).unwrap(),
            expected
        );
    }
    let json = serde_json::to_value(&graph).unwrap();
    let restored: ReactiveGraph = serde_json::from_value(json).unwrap();
    assert_eq!(restored, graph);
}
#[test]
fn string_record_matrix_and_array_state_commit_at_the_end() {
    assert_discrete(
        Type::String,
        Value::String("before".into()),
        Value::String("after".into()),
    );
    let ty = Type::Record(
        [
            ("label".into(), Type::String),
            ("count".into(), Type::Numeric(Unit::Number)),
        ]
        .into(),
    );
    let record = |label: &str, count| {
        Value::Record(
            [
                ("label".to_owned(), Value::String(label.into())),
                ("count".to_owned(), number(count)),
            ]
            .into_iter()
            .collect(),
        )
    };
    assert_discrete(ty, record("one", 1), record("two", 2));
    let mut matrix = sanim_core::math::linear::identity();
    matrix[0][0] = 2.;
    assert_discrete(
        Type::Matrix4,
        Value::Matrix4(Box::new(sanim_core::math::linear::identity())),
        Value::Matrix4(Box::new(matrix)),
    );
    let array = |values: Vec<Value>| Value::Array {
        element_type: Arc::new(Type::Numeric(Unit::Number)),
        shape: vec![values.len()],
        values: values.into_iter().collect(),
    };
    assert_discrete(
        Type::Array(Box::new(Type::Numeric(Unit::Number))),
        array(vec![number(1)]),
        array(vec![number(2), number(3)]),
    );
}
#[test]
fn union_and_option_cells_preserve_the_declared_schema() {
    let numeric = Type::Numeric(Unit::Number);
    let union = Type::union([numeric.clone(), Type::String]).unwrap();
    assert_discrete(union.clone(), number(0), number(10));
    assert_discrete(union, number(0), Value::String("complete".into()));
    assert_discrete(
        Type::Option(Box::new(numeric)),
        Value::None,
        Value::Some(Arc::new(number(10))),
    );
    assert_discrete(Type::Bool, Value::Bool(false), Value::Bool(true));
}
#[test]
fn fractions_and_three_and_four_component_vectors_interpolate() {
    let fraction = transition(
        Type::Fraction,
        Value::Fraction(Fraction::from(1)),
        Value::Fraction(Fraction::from(3)),
    );
    assert_eq!(
        fraction.sample_cell(1, Fraction::from(2)).unwrap(),
        Value::Fraction(Fraction::from(2))
    );
    let vector3 = transition(
        Type::Vector(3, Some(Unit::Length)),
        Value::Vec3(Vec3 {
            values: [0., 2., 4.],
            unit: Unit::Length,
        }),
        Value::Vec3(Vec3 {
            values: [2., 4., 6.],
            unit: Unit::Length,
        }),
    );
    assert_eq!(
        vector3.sample_cell(1, Fraction::from(2)).unwrap(),
        Value::Vec3(Vec3 {
            values: [1., 3., 5.],
            unit: Unit::Length
        })
    );
    let vector4 = transition(
        Type::Vector(4, Some(Unit::Number)),
        Value::Vec4(Vec4 {
            values: [0., 2., 4., 6.],
            unit: Unit::Number,
        }),
        Value::Vec4(Vec4 {
            values: [2., 4., 6., 8.],
            unit: Unit::Number,
        }),
    );
    assert_eq!(
        vector4.sample_cell(1, Fraction::from(2)).unwrap(),
        Value::Vec4(Vec4 {
            values: [1., 3., 5., 7.],
            unit: Unit::Number
        })
    );
}
#[test]
fn arbitrary_background_source_order_rebases_discrete_state() {
    let mut graph = ReactiveGraph::default();
    graph
        .set_typed_cell(1, Type::String, Value::String("initial".into()))
        .unwrap();
    graph
        .animate_cell(
            1,
            Fraction::from(3),
            Fraction::from(4),
            Value::String("last".into()),
            Easing::Linear,
        )
        .unwrap();
    graph
        .write_cell(1, Fraction::from(1), Value::String("earlier".into()))
        .unwrap();
    assert_eq!(
        graph.sample_cell(1, Fraction::from(3)).unwrap(),
        Value::String("earlier".into())
    );
    assert_eq!(
        graph.sample_cell(1, Fraction::from(4)).unwrap(),
        Value::String("last".into())
    );
    graph.validate().unwrap();
}
#[test]
fn generic_state_never_weakens_property_expression_types_or_validation() {
    let mut graph = ReactiveGraph::default();
    graph
        .set_typed_cell(
            1,
            Type::union([Type::Numeric(Unit::Number), Type::String]).unwrap(),
            number(1),
        )
        .unwrap();
    assert!(graph.insert_node(Node::Cell(1), 100).is_err());
    assert!(
        graph
            .write_cell(1, Fraction::from(1), Value::Bool(true))
            .is_err()
    );
    assert!(
        graph
            .set_typed_cell(
                2,
                Type::Numeric(Unit::Number),
                Value::Number(Number {
                    value: Fraction::NaN,
                    unit: Unit::Number
                })
            )
            .is_err()
    );
    assert!(
        graph
            .set_typed_cell(
                2,
                Type::Vector(3, Some(Unit::Number)),
                Value::Vec3(Vec3 {
                    values: [0., f64::NAN, 0.],
                    unit: Unit::Number
                })
            )
            .is_err()
    );
    assert!(
        graph
            .set_typed_cell(2, Type::Option(Box::new(Type::Unknown)), Value::None)
            .is_err()
    );
    let json = serde_json::to_value(&graph).unwrap();
    let mut corrupt = json.clone();
    corrupt["cells"]["1"]["ty"] = serde_json::to_value(Type::Bool).unwrap();
    assert!(serde_json::from_value::<ReactiveGraph>(corrupt).is_err());
    assert!(serde_json::from_value::<ReactiveGraph>(json).is_ok());
}
