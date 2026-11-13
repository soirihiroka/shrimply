use sanim_core::{
    Element, Geometry, Value,
    media::{Asset, Kind},
    value_budget::{Budget, charge_with_budget},
};
use std::sync::Arc;

fn value(asset: Arc<Asset>) -> Value {
    Value::Element(Arc::new(Element {
        id: "image".into(),
        geometry: Arc::new(Geometry::Media {
            asset,
            width: 120.into(),
            height: 72.into(),
        }),
        position: sanim_core::Vec2::zero(sanim_core::Unit::Length),
        rotation: 0.into(),
        scale: 1.into(),
        opacity: 1.into(),
        fill: sanim_core::Color::parse("light1").unwrap().into(),
        stroke: None,
        reveal: Default::default(),
        horizontal: Default::default(),
        layout: Default::default(),
    }))
}
#[test]
fn media_retention_uses_cow_guards_and_failed_charges_never_register_unpaid_assets() {
    let source = include_bytes!("../../../tests/fixtures/media-sample.png").to_vec();
    let asset = Arc::new(Asset::new(Kind::Bitmap, Arc::new(source.clone()), 160, 96).unwrap());
    let weak = Arc::downgrade(&asset);
    let first = value(asset.clone());
    let mut budget = Budget::default();
    let (mut bytes, mut items) = (100_000usize, 100usize);
    charge_with_budget(&first, &mut bytes, &mut items, 8, &mut budget).unwrap();
    let checkpoint = budget.clone();
    let before = bytes;
    charge_with_budget(&first, &mut bytes, &mut items, 8, &mut budget).unwrap();
    assert!(before - bytes < source.len());
    let second = value(Arc::new(
        Asset::new(Kind::Bitmap, Arc::new(source.clone()), 160, 96).unwrap(),
    ));
    let before = bytes;
    charge_with_budget(&second, &mut bytes, &mut items, 8, &mut budget).unwrap();
    assert!(before - bytes >= source.len());
    assert!(budget.metadata_bytes() > checkpoint.metadata_bytes());
    let mut unpaid = Budget::default();
    let mut insufficient = std::mem::size_of::<Element>() + std::mem::size_of::<Value>() + 100;
    assert!(charge_with_budget(&first, &mut insufficient, &mut items, 8, &mut unpaid).is_err());
    assert_eq!(unpaid.metadata_bytes(), 0);
    drop(first);
    drop(asset);
    assert!(
        weak.upgrade().is_none(),
        "retention guards never keep media payloads alive"
    );
}
#[test]
fn serialized_assets_cannot_supply_a_forged_content_fingerprint() {
    let asset = Asset::new(Kind::Svg, Arc::new(b"<svg/>".to_vec()), 1, 1).unwrap();
    let mut encoded = serde_json::to_value(&asset).unwrap();
    encoded["fingerprint"] = serde_json::json!("forged");
    let decoded: Asset = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.fingerprint(), sanim_core::digest(b"<svg/>"));
}
