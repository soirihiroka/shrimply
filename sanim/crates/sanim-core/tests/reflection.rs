use sanim_core::{
    Span, Type,
    reflection::{Options, Schema, display_name},
};
#[test]
fn variable_names_become_readable_labels_without_rewriting_acronyms() {
    for (name, label) in [
        ("color1", "Color 1"),
        ("grid_line_width", "Grid Line Width"),
        ("RGBColor2", "RGB Color 2"),
        ("HTTP_serverURL", "HTTP Server URL"),
        ("_private_value", "Private Value"),
        ("__", "__"),
    ] {
        assert_eq!(display_name(name), label);
    }
}
#[test]
fn hosts_share_canonical_and_legacy_key_resolution() {
    let root = Options::default()
        .resolve("scene.sanim", "tint", Type::Color, Span::default())
        .unwrap();
    let imported = Options::default()
        .resolve("theme.sanim", "tint", Type::Color, Span::default())
        .unwrap();
    let schema = Schema::new("scene.sanim", [root.clone(), imported.clone()]).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        schema.declaration("tint").unwrap(),
        &root
    ));
    assert!(std::sync::Arc::ptr_eq(
        schema.declaration("theme.sanim::tint").unwrap(),
        &imported
    ));
    assert!(
        schema
            .declaration("missing")
            .unwrap_err()
            .message
            .contains("scene.sanim::tint")
    );
}

#[test]
fn exact_display_magnitudes_and_units_roundtrip_through_the_shared_numeric_grammar() {
    use sanim_core::Number;
    for literal in [
        "5",
        "3/7",
        "2pt",
        "4px",
        "250ms",
        "1/3s",
        "45deg",
        "30000/1001fps",
        "12.5%",
    ] {
        let number = Number::parse(literal).unwrap();
        let (value, suffix) = number.display_parts().unwrap();
        assert_eq!(
            Number::parse(&format!("{value}{suffix}")).unwrap(),
            number,
            "{literal}"
        );
    }
    let (value, suffix) = Number::parse("50%").unwrap().display_parts().unwrap();
    assert_eq!(value, sanim_core::Fraction::from(50));
    assert_eq!(suffix, "%");
}
