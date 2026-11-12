use sanim_core::{List, Type, Unit, Value, math};
use std::sync::Arc;

fn number() -> Type {
    Type::Numeric(Unit::Number)
}

#[test]
fn contextual_empty_lists_retain_schema_without_adding_values_or_copying_payloads() {
    let original = Value::List(vec![].into());
    let expected = Type::List(Box::new(number()));
    let refined = original.with_type(&expected).unwrap();
    assert_eq!(refined.ty(), expected);
    let (Value::List(before), Value::List(after)) = (&original, &refined) else {
        unreachable!()
    };
    assert_eq!(after.len(), 0);
    assert!(Arc::ptr_eq(&before.values, &after.values));
    assert_eq!(before.element_type.as_ref(), &Type::Unknown);
    let cloned = refined.with_type(&expected).unwrap();
    let Value::List(cloned) = cloned else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(&after.values, &cloned.values));
    assert!(Arc::ptr_eq(&after.element_type, &cloned.element_type));
}

#[test]
fn nested_list_refinement_preserves_original_values() {
    let original = Value::List(vec![Value::List(vec![].into())].into());
    let expected = Type::List(Box::new(Type::List(Box::new(number()))));
    let refined = original.with_type(&expected).unwrap();
    assert_eq!(refined.ty(), expected);
    assert_eq!(
        original.ty(),
        Type::List(Box::new(Type::List(Box::new(Type::Unknown))))
    );
    let Value::List(outer) = refined else {
        unreachable!()
    };
    assert_eq!(outer[0].ty(), Type::List(Box::new(number())));
    assert_eq!(outer[0].list().unwrap().len(), 0);
    assert!(Value::Bool(true).with_type(&number()).is_err());
}

#[test]
fn concatenating_empty_typed_lists_keeps_the_element_schema() {
    let values = Value::List(List::typed(number(), vec![]));
    let result = math::operations::binary("+", values.clone(), Value::List(vec![].into())).unwrap();
    assert_eq!(result.ty(), values.ty());
    assert!(result.list().unwrap().is_empty());
}

#[test]
fn assignable_list_schemas_do_not_change_value_equality() {
    let exact = Value::List(List::typed(
        number(),
        vec![Value::Number(sanim_core::Number::scalar(1))],
    ));
    let wider = exact
        .with_type(&Type::List(Box::new(
            Type::union([number(), Type::String]).unwrap(),
        )))
        .unwrap();
    assert_ne!(exact.ty(), wider.ty());
    assert_eq!(
        math::operations::binary("==", wider, exact).unwrap(),
        Value::Bool(true)
    );
}

#[test]
#[ignore = "microbenchmark: typed list COW context/clone"]
fn typed_list_benchmark() {
    let count = 100_000_u32;
    let value = Value::List(List::typed(
        number(),
        (0..count)
            .map(|number| Value::Number(sanim_core::Number::scalar(i64::from(number))))
            .collect(),
    ));
    let ty = Type::List(Box::new(number()));
    let start = std::time::Instant::now();
    for _ in 0..count {
        std::hint::black_box(value.with_type(&ty).unwrap());
    }
    eprintln!(
        "typed List context/COW clone over {count} values: {:.1} ns/op ({count} clones)",
        start.elapsed().as_nanos() as f64 / f64::from(count)
    );
    let Value::List(original) = &value else {
        unreachable!()
    };
    let Value::List(cloned) = value.with_type(&ty).unwrap() else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(&original.values, &cloned.values));
}
