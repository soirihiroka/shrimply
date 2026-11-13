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
fn hosts_resolve_only_canonical_keys() {
    let root = Options::default()
        .resolve("scene.sanim", "tint", Type::Color, Span::default())
        .unwrap();
    let imported = Options::default()
        .resolve("theme.sanim", "tint", Type::Color, Span::default())
        .unwrap();
    let schema = Schema::new("scene.sanim", [root.clone(), imported.clone()]).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        schema.declaration("scene.sanim::tint").unwrap(),
        &root
    ));
    assert!(std::sync::Arc::ptr_eq(
        schema.declaration("theme.sanim::tint").unwrap(),
        &imported
    ));
    assert!(
        schema
            .declaration("tint")
            .unwrap_err()
            .message
            .contains("scene.sanim::tint")
    );
    assert!(
        schema
            .declaration("tint")
            .unwrap_err()
            .message
            .contains("unknown reflected input")
    );
}

#[test]
#[ignore = "measured by make reflection-schema-bench"]
fn canonical_registry_lookup_benchmark() {
    const INPUTS: usize = 1000;
    const LOOKUPS: usize = 100_000;
    let declarations = (0..INPUTS)
        .map(|index| {
            Options::default()
                .resolve(
                    "scene.sanim",
                    &format!("input_{index}"),
                    Type::Color,
                    Span::default(),
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let schema = Schema::new("scene.sanim", declarations.clone()).unwrap();
    let build_us = started.elapsed().as_micros();
    let started = std::time::Instant::now();
    for index in 0..LOOKUPS {
        std::hint::black_box(
            schema
                .declaration(std::hint::black_box(&declarations[index % INPUTS].key))
                .unwrap(),
        );
    }
    eprintln!(
        "canonical reflection schema: inputs={INPUTS} build_us={build_us} lookups={LOOKUPS} lookup_total_us={} ns_per_lookup={}",
        started.elapsed().as_micros(),
        started.elapsed().as_nanos() / LOOKUPS as u128
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
