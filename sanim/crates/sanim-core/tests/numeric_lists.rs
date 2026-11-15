use sanim_core::{Fraction, List, ListLength, Number, Type, Unit, Value, math::operations};
fn list(values: &[i64], unit: Unit) -> Value {
    Value::List(List::typed(
        Type::Numeric(unit),
        values
            .iter()
            .map(|value| {
                Value::Number(Number {
                    value: Fraction::from(*value),
                    unit,
                })
            })
            .collect(),
    ))
}
#[test]
fn numeric_lists_are_pointwise_and_keep_exact_dimensions_and_units() {
    let result = operations::binary(
        "+",
        list(&[1, 2, 3, 4, 5], Unit::Length),
        list(&[5, 4, 3, 2, 1], Unit::Length),
    )
    .unwrap();
    assert_eq!(result, list(&[6, 6, 6, 6, 6], Unit::Length));
    assert_eq!(
        result.ty(),
        Type::List(Box::new(Type::Numeric(Unit::Length)), ListLength::Exact(5))
    );
    assert!(
        operations::binary("+", list(&[1, 2], Unit::Length), list(&[3], Unit::Length)).is_err()
    );
    assert!(
        operations::binary(
            "+",
            list(&[1, 2], Unit::Length),
            list(&[3, 4], Unit::Duration)
        )
        .is_err()
    );
    assert_eq!(
        operations::binary(
            "*",
            list(&[2, 4], Unit::Length),
            Value::Number(Number::scalar(2))
        )
        .unwrap(),
        list(&[4, 8], Unit::Length)
    );
}
#[test]
fn list_concatenation_is_explicit_and_distinct_from_numeric_addition() {
    assert_eq!(
        operations::binary(
            "concat",
            list(&[1, 2], Unit::Number),
            list(&[3], Unit::Number)
        )
        .unwrap(),
        list(&[1, 2, 3], Unit::Number)
    );
    let words = Value::List(List::typed(Type::String, vec![Value::String("a".into())]));
    assert!(operations::binary("+", words.clone(), words.clone()).is_err());
    assert_eq!(
        operations::binary("concat", words.clone(), words)
            .unwrap()
            .list()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn numeric_list_noops_share_the_original_cow_storage() {
    let original = list(&[1, 2, 3], Unit::Number);
    let result =
        operations::binary("*", original.clone(), Value::Number(Number::scalar(1))).unwrap();
    let (Value::List(a), Value::List(b)) = (&original, &result) else {
        panic!("List")
    };
    assert!(a.values.ptr_eq(&b.values));
}
#[test]
fn list_math_uses_the_same_typed_registry_for_all_dimensions() {
    use sanim_core::math::linear::list_function;
    assert_eq!(
        list_function("dot")
            .unwrap()
            .evaluate(&[
                list(&[1, 2, 3], Unit::Number),
                list(&[4, 5, 6], Unit::Number)
            ])
            .unwrap(),
        Value::Number(Number::scalar(32))
    );
    assert_eq!(
        list_function("norm")
            .unwrap()
            .evaluate(&[list(&[3, 4], Unit::Number)])
            .unwrap(),
        Value::Number(Number::scalar(5))
    );
    assert_eq!(
        list_function("cross")
            .unwrap()
            .evaluate(&[
                list(&[1, 0, 0], Unit::Number),
                list(&[0, 1, 0], Unit::Number)
            ])
            .unwrap(),
        list(&[0, 0, 1], Unit::Number)
    );
}
#[test]
#[ignore = "manual numeric List COW benchmark"]
fn numeric_list_identity_benchmark() {
    const ITEMS: usize = 100_000;
    const ITERATIONS: usize = 20;
    let original = Value::List(List::typed(
        Type::Numeric(Unit::Number),
        (0..ITEMS)
            .map(|value| Value::Number(Number::scalar(value as i64)))
            .collect(),
    ));
    let started = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        let result = operations::binary(
            "*",
            std::hint::black_box(original.clone()),
            Value::Number(Number::scalar(1)),
        )
        .unwrap();
        let (Value::List(a), Value::List(b)) = (&original, &result) else {
            panic!("List")
        };
        assert!(a.values.ptr_eq(&b.values));
        std::hint::black_box(result);
    }
    println!(
        "{ITEMS} numeric values * 1, {ITERATIONS} immutable shared outputs: {:?}",
        started.elapsed()
    );
}
