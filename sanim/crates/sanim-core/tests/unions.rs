use sanim_core::{Type, TypeBound, Unit, Value};
use std::collections::BTreeMap;

fn number() -> Type {
    Type::Numeric(Unit::Number)
}

#[test]
fn union_normalization_flattens_deduplicates_and_orders_equivalent_schemas() {
    let a = Type::union([number(), Type::String]).unwrap();
    let b = Type::union([Type::String, Type::Union(vec![number(), Type::String])]).unwrap();
    assert_eq!(a, b);
    assert_eq!(Type::union([number(), number()]).unwrap(), number());
    assert!(Type::union([]).is_err());
    assert!(a.accepts(&number()));
    assert!(a.accepts(&Type::String));
    assert!(!a.accepts(&Type::Bool));
    assert!(a.accepts(&Type::union([Type::String, number()]).unwrap()));
    assert!(!number().accepts(&a));
}

#[test]
fn union_predicates_substitution_and_constraints_cover_every_alternative() {
    let generic = Type::union([Type::Variable("T".into()), Type::String]).unwrap();
    let resolved = generic.substitute(&BTreeMap::from([("T".into(), number())]));
    assert_eq!(resolved, Type::union([number(), Type::String]).unwrap());
    assert!(resolved.readable());
    assert!(resolved.reflectable());
    assert!(resolved.data_convertible());
    assert!(
        !Type::union([Type::String, Type::Font])
            .unwrap()
            .reflectable()
    );
    assert!(TypeBound::Numeric.accepts(&Type::union([number(), Type::Fraction]).unwrap()));
    assert!(!TypeBound::Numeric.accepts(&resolved));
    assert!(Type::union([number(), Type::Unknown]).unwrap().unresolved());
}

#[test]
fn contextual_union_lists_keep_a_concrete_current_schema_and_original_payload() {
    let empty = Value::List(vec![].into());
    let ty = Type::union([
        Type::List(Box::new(number()), sanim_core::ListLength::Unknown),
        Type::List(Box::new(Type::String), sanim_core::ListLength::Unknown),
    ])
    .unwrap();
    let refined = empty.with_type(&ty).unwrap();
    assert!(ty.accepts(&refined.ty()));
    assert!(!refined.ty().unresolved());
    assert!(refined.list().unwrap().is_empty());
    assert!(empty.ty().unresolved());
}

#[test]
fn equality_of_erased_union_arms_is_total_without_weakening_static_checks() {
    use sanim_core::{Number, math::operations};
    let number = Value::Number(Number::scalar(1));
    let text = Value::String("one".into());
    assert!(operations::binary_type("==", &number.ty(), &text.ty()).is_err());
    assert_eq!(
        operations::binary("==", number.clone(), text.clone()).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        operations::binary("!=", number, text).unwrap(),
        Value::Bool(true)
    );
}
