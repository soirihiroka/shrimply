use sanim_core::{List, Type, Unit, Value, math};
use std::sync::Arc;

fn number() -> Type {
    Type::Numeric(Unit::Number)
}

#[test]
fn exact_length_validation_shares_values_and_rejects_wrong_lengths() {
    let value = Value::List(List::typed(
        number(),
        (0..1024)
            .map(|value| Value::Number(sanim_core::Number::scalar(value)))
            .collect(),
    ));
    let exact = Type::List(Box::new(number()), sanim_core::ListLength::Exact(1024));
    let Value::List(before) = &value else {
        unreachable!()
    };
    let refined = value.with_type(&exact).unwrap();
    let Value::List(after) = &refined else {
        unreachable!()
    };
    assert!(before.values.ptr_eq(&after.values));
    assert!(Arc::ptr_eq(&before.element_type, &after.element_type));
    assert!(
        value
            .with_type(&Type::List(
                Box::new(number()),
                sanim_core::ListLength::Exact(1023)
            ))
            .is_err()
    );
    assert_eq!(refined.ty(), exact);
}

#[test]
#[ignore = "manual exact/unknown List metadata performance measurement"]
fn sized_list_metadata_benchmark() {
    use std::{hint::black_box, time::Instant};
    const ITEMS: usize = 100_000;
    const READS: usize = 100_000;
    let value = Value::List(List::typed(
        number(),
        (0..ITEMS)
            .map(|value| Value::Number(sanim_core::Number::scalar(value as i64)))
            .collect(),
    ));
    for length in [
        sanim_core::ListLength::Unknown,
        sanim_core::ListLength::Exact(ITEMS),
    ] {
        let schema = Type::List(Box::new(number()), length);
        let start = Instant::now();
        for _ in 0..READS {
            black_box(value.with_type(black_box(&schema)).unwrap());
        }
        eprintln!(
            "{length:?}: {READS} shared schema applications over {ITEMS} values: {:?}",
            start.elapsed()
        );
    }
}

#[test]
fn contextual_empty_lists_retain_schema_without_adding_values_or_copying_payloads() {
    let original = Value::List(vec![].into());
    let expected = Type::List(Box::new(number()), sanim_core::ListLength::Unknown);
    let refined = original.with_type(&expected).unwrap();
    assert!(expected.accepts(&refined.ty()));
    let (Value::List(before), Value::List(after)) = (&original, &refined) else {
        unreachable!()
    };
    assert_eq!(after.len(), 0);
    assert!(before.values.is_empty() && after.values.is_empty());
    assert_eq!(before.element_type.as_ref(), &Type::Unknown);
    let cloned = refined.with_type(&expected).unwrap();
    let Value::List(cloned) = cloned else {
        unreachable!()
    };
    assert!(after.values.is_empty() && cloned.values.is_empty());
    assert!(Arc::ptr_eq(&after.element_type, &cloned.element_type));
}

#[test]
fn nested_list_refinement_preserves_original_values() {
    let original = Value::List(vec![Value::List(vec![].into())].into());
    let expected = Type::List(
        Box::new(Type::List(
            Box::new(number()),
            sanim_core::ListLength::Unknown,
        )),
        sanim_core::ListLength::Unknown,
    );
    let refined = original.with_type(&expected).unwrap();
    assert!(expected.accepts(&refined.ty()));
    assert_eq!(
        original.ty(),
        Type::List(
            Box::new(Type::List(
                Box::new(Type::Unknown),
                sanim_core::ListLength::Exact(0)
            )),
            sanim_core::ListLength::Exact(1)
        )
    );
    let Value::List(outer) = refined else {
        unreachable!()
    };
    assert_eq!(
        outer[0].ty(),
        Type::List(Box::new(number()), sanim_core::ListLength::Exact(0))
    );
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
        .with_type(&Type::List(
            Box::new(Type::union([number(), Type::String]).unwrap()),
            sanim_core::ListLength::Unknown,
        ))
        .unwrap();
    assert_ne!(exact.ty(), wider.ty());
    assert_eq!(
        math::operations::binary("==", wider, exact).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_edits_keep_old_versions_and_share_untouched_tree_chunks() {
    let original = List::typed(
        number(),
        (0..10_000)
            .map(|n| Value::Number(sanim_core::Number::scalar(n)))
            .collect(),
    );
    let mut appended = original.clone();
    appended
        .values
        .push_back(Value::Number(sanim_core::Number::scalar(10_000)));
    assert_eq!(original.len(), 10_000);
    assert_eq!(appended.len(), 10_001);
    assert!(std::ptr::eq(&original[0], &appended[0]));
    let mut removed = appended.clone();
    removed.values.remove(5_000);
    assert_eq!(appended[5_000].number().unwrap().value, 5_000.into());
    assert_eq!(removed[5_000].number().unwrap().value, 5_001.into());
    assert!(std::ptr::eq(&original[0], &removed[0]));
}

#[test]
fn arrays_and_lists_use_the_same_shared_collection_storage() {
    let list = List::typed(
        number(),
        (0..1_000)
            .map(|n| Value::Number(sanim_core::Number::scalar(n)))
            .collect(),
    );
    let array = Value::Array {
        element_type: list.element_type.clone(),
        shape: vec![100, 10],
        values: list.values.clone(),
    };
    assert!(array.list().unwrap().ptr_eq(&list.values));
    let restored: Value = serde_json::from_slice(&serde_json::to_vec(&array).unwrap()).unwrap();
    assert_eq!(restored, array);
    assert_eq!(restored.list().unwrap()[999], list[999]);
}

#[test]
#[ignore = "microbenchmark: persistent edits and sequential collection traversal"]
fn collection_storage_benchmark() {
    const VALUES: u32 = 100_000;
    const EDITS: u32 = 1_000;
    let values: Vec<_> = (0..VALUES)
        .map(|n| Value::Number(sanim_core::Number::scalar(i64::from(n))))
        .collect();
    let started = std::time::Instant::now();
    let original = List::typed(number(), values.clone());
    let build = started.elapsed();
    let started = std::time::Instant::now();
    let mut current = original.clone();
    let mut snapshots = Vec::new();
    for n in 0..EDITS {
        let mut next = current.clone();
        next.values
            .push_back(Value::Number(sanim_core::Number::scalar(i64::from(n))));
        snapshots.push(current);
        current = next;
    }
    let edits = started.elapsed();
    assert_eq!(snapshots[0].len(), VALUES as usize);
    assert_eq!(current.len(), (VALUES + EDITS) as usize);
    let started = std::time::Instant::now();
    let mut count = 0;
    for _ in 0..10 {
        for value in original.values.iter() {
            std::hint::black_box(value);
            count += 1;
        }
    }
    let persistent_scan = started.elapsed();
    let started = std::time::Instant::now();
    for _ in 0..10 {
        for value in values.iter() {
            std::hint::black_box(value);
        }
    }
    let contiguous_scan = started.elapsed();
    eprintln!(
        "shared collection storage: {VALUES} value build={}us; {EDITS} retained append snapshots={}us ({}ns/edit); {count} borrowed visits persistent={}us contiguous={}us",
        build.as_micros(),
        edits.as_micros(),
        edits.as_nanos() / u128::from(EDITS),
        persistent_scan.as_micros(),
        contiguous_scan.as_micros()
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
    let ty = Type::List(Box::new(number()), sanim_core::ListLength::Unknown);
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
    assert!(original.values.ptr_eq(&cloned.values));
}
