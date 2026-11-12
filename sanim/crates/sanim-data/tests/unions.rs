use sanim_core::{Type, Unit};
use sanim_data::{Limits, to_json, validate};

#[test]
fn union_validation_accepts_each_json_alternative_without_a_value_wrapper() {
    let ty = Type::union([Type::Numeric(Unit::Number), Type::String]).unwrap();
    for json in [serde_json::json!(42), serde_json::json!("text")] {
        let value = validate(&json, &ty, Limits::default()).unwrap();
        assert!(ty.accepts(&value.ty()));
        assert_eq!(to_json(&value).unwrap(), json);
    }
    let error = validate(&serde_json::json!(true), &ty, Limits::default()).unwrap_err();
    assert!(error.contains("union"), "{error}");
    assert!(
        error.contains("Number") && error.contains("String"),
        "{error}"
    );
}

#[test]
fn list_union_items_retain_the_declared_element_schema_even_when_empty() {
    let element = Type::union([Type::Numeric(Unit::Number), Type::String]).unwrap();
    let ty = Type::List(Box::new(element));
    for json in [serde_json::json!([]), serde_json::json!([1, "text"])] {
        let value = validate(&json, &ty, Limits::default()).unwrap();
        assert_eq!(value.ty(), ty);
        assert_eq!(to_json(&value).unwrap(), json);
    }
}

#[test]
fn failed_union_branches_cannot_refund_the_decoding_work_budget() {
    let ty = Type::union([Type::String, Type::Numeric(Unit::Number)]).unwrap();
    let error = validate(
        &serde_json::json!(1),
        &ty,
        Limits {
            max_values: 1,
            ..Limits::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("value limit"), "{error}");
}
