use sanim_core::{Type, value_budget::Budget};
use std::{collections::BTreeMap, sync::Arc};

#[test]
fn captured_types_charge_once_with_temporary_owners_and_cow_checkpoints() {
    let types = Arc::new(BTreeMap::from([("T".into(), Type::String)]));
    let temporary_owner = types.clone();
    let mut budget = Budget::default();
    let mut remaining = 16_384;
    budget.charge_types(&types, &mut remaining).unwrap();
    let charged = remaining;
    assert!(
        charged < 16_384,
        "temporary owners must not bypass the charge"
    );
    let mut checkpoint = budget.clone();
    checkpoint
        .charge_types(&temporary_owner, &mut remaining)
        .unwrap();
    assert_eq!(remaining, charged);
    assert_eq!(checkpoint.metadata_bytes(), budget.metadata_bytes());

    let independent = Arc::new(types.as_ref().clone());
    checkpoint
        .charge_types(&independent, &mut remaining)
        .unwrap();
    assert!(
        remaining < charged,
        "distinct allocations need distinct receipts"
    );
    assert!(checkpoint.metadata_bytes() > budget.metadata_bytes());
    budget.charge_types(&types, &mut remaining).unwrap();
    let before = remaining;
    budget.charge_types(&independent, &mut remaining).unwrap();
    assert!(
        remaining < before,
        "checkpoint receipts must remain isolated by COW"
    );
}

#[test]
fn oversized_captured_types_fail_without_publishing_a_receipt() {
    let types = Arc::new(BTreeMap::from([("T".into(), Type::String)]));
    let mut budget = Budget::default();
    let mut remaining = 1;
    assert!(budget.charge_types(&types, &mut remaining).is_err());
    assert_eq!(remaining, 1);
    assert_eq!(budget.metadata_bytes(), 0);
}
