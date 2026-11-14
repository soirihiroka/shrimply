use sanim_core::{Color, Fraction, Type, Unit, Value};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::{collections::BTreeMap, sync::Arc};

fn run(source: &str) -> Prepared {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_or_else(|e| panic!("{e}"))
}
#[test]
fn declarations_expose_config_key_display_label_type_and_source_location() {
    let p = run(r#"
let color1=reflect(Color(), label="Display Color")
let accent:Color=reflect(Color("blue3"), key="palette.accent", label="Accent Color")
let grid_lineWidth2:Length=reflect(2pt)
"#);
    let color = &p.reflected[0];
    assert_eq!(p.source, "main.sanim");
    assert_eq!(color.key, "main.sanim::color1");
    assert_eq!(color.name, "color1");
    assert_eq!(color.label, "Display Color");
    assert_eq!(color.source, "main.sanim");
    assert_eq!(color.span.line, 2);
    assert_eq!(color.ty.as_ref(), &Type::Color);
    assert_eq!(color.default, color.value);
    assert_eq!(p.reflected[1].key, "palette.accent");
    assert_eq!(p.reflected[1].label, "Accent Color");
    assert_eq!(p.reflected[2].label, "Grid Line Width 2");
    let json = serde_json::to_value(color).unwrap();
    assert_eq!(json["key"], "main.sanim::color1");
    assert_eq!(json["label"], "Display Color");
    assert!(
        json.get("declaration").is_none(),
        "metadata is flattened for hosts"
    );
}
#[test]
fn default_keys_use_normalized_relative_paths_for_root_and_imports() {
    let mut files = MemorySources::default();
    files.insert(
        "nested/main.sanim",
        "import \"../theme.sanim\" as theme\nlet primaryColor1=reflect(theme.tint)\n",
    );
    files.insert("theme.sanim", "let tint=reflect(Color(\"blue3\"))\n");
    let p = Engine::default()
        .prepare("./nested/main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(p.reflected[0].key, "theme.sanim::tint");
    assert_eq!(p.reflected[1].key, "nested/main.sanim::primaryColor1");
    assert_eq!(p.reflected[1].label, "Primary Color 1");
}
#[test]
fn canonical_overrides_work_and_bare_root_names_are_rejected() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let count=reflect(2)\nrepeat count {wait(1s)}\n",
    );
    let mut engine = Engine::default();
    let rejected = engine
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("count".into(), serde_json::json!(4))]),
        )
        .unwrap_err();
    assert!(rejected.message.contains("unknown reflected input 'count'"));
    assert!(rejected.message.contains("main.sanim::count"));
    let canonical = engine
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("main.sanim::count".into(), serde_json::json!(4))]),
        )
        .unwrap();
    let repeated = engine
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("main.sanim::count".into(), serde_json::json!(4))]),
        )
        .unwrap();
    assert_eq!(repeated.fingerprint, canonical.fingerprint);
    assert_eq!(repeated.animation, canonical.animation);
    assert_eq!(canonical.animation.duration, Fraction::from(4));
    assert_eq!(canonical.reflected[0].key, "main.sanim::count");
    assert!(repeated.statistics.restored_statements > 0);
    let report = engine
        .duration_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("main.sanim::count".into(), serde_json::json!(3))]),
        )
        .unwrap();
    assert_eq!(report.duration, Fraction::from(3));
}
#[test]
fn explicit_key_survives_variable_rename_and_key_defaults_do_not_depend_on_labels() {
    let mut files = MemorySources::default();
    let mut engine = Engine::default();
    let inputs = BTreeMap::from([("theme.accent".into(), serde_json::json!("green3"))]);
    files.insert(
        "main.sanim",
        "let old_color=reflect(Color(\"red3\"), label=\"Old Label\", key=\"theme.accent\")\n",
    );
    let old = engine.prepare_json("main.sanim", &files, &inputs).unwrap();
    files.insert(
        "main.sanim",
        "let new_color=reflect(Color(\"red3\"), label=\"New Label\", key=\"theme.accent\")\n",
    );
    let new = engine.prepare_json("main.sanim", &files, &inputs).unwrap();
    assert_eq!(old.reflected[0].key, new.reflected[0].key);
    assert_eq!(old.reflected[0].value, new.reflected[0].value);
    assert_eq!(new.reflected[0].label, "New Label");
    assert_eq!(new.reflected[0].name, "new_color");
    assert!(!new.bindings.contains_key("old_color"));
}
#[test]
fn metadata_changes_restore_before_binding_and_refresh_the_reflection_without_visual_changes() {
    let mut files = MemorySources::default();
    let mut engine = Engine::default();
    let script = |label: &str| {
        format!(
            "wait(1s)\nlet tint=reflect(Color(\"red3\"), label=\"{label}\", key=\"accent\")\nlet marker=Circle(id=\"dot\",fill=tint)\nadd(marker)\nwait(2s)\n"
        )
    };
    let inputs = BTreeMap::from([("accent".into(), serde_json::json!("blue3"))]);
    files.insert("main.sanim", script("Before"));
    let old = engine.prepare_json("main.sanim", &files, &inputs).unwrap();
    let prefix = engine.execution_state(1).unwrap();
    files.insert("main.sanim", script("After"));
    let updated = engine.prepare_json("main.sanim", &files, &inputs).unwrap();
    assert_eq!(updated.statistics.restored_statements, 1);
    assert!(Arc::ptr_eq(&prefix, &engine.execution_state(1).unwrap()));
    assert_eq!(updated.reflected[0].label, "After");
    assert_eq!(
        updated.reflected[0].default,
        Value::Color(Color::parse("red3").unwrap())
    );
    assert_eq!(
        updated.reflected[0].value,
        Value::Color(Color::parse("blue3").unwrap())
    );
    assert_eq!(old.animation, updated.animation);
    let fresh = Engine::default()
        .prepare_json("main.sanim", &files, &inputs)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&updated.reflected).unwrap(),
        serde_json::to_value(&fresh.reflected).unwrap()
    );
}
#[test]
fn duplicate_keys_are_rejected_across_files_before_loading_defaults() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","import \"theme\" as theme\nlet tint:Color=reflect(read<Color>(\"missing.json\"), key=\"accent\")\n");
    files.insert("theme.sanim", "let tint=reflect(Color(), key=\"accent\")\n");
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("duplicate reflection key 'accent'"),
        "{error}"
    );
    assert!(error.message.contains("theme.sanim::tint"));
    assert!(error.message.contains("main.sanim::tint"));
    assert_eq!(error.source, "main.sanim");
    assert_eq!(error.span.line, 2);
}
#[test]
fn metadata_and_override_errors_explain_how_to_fix_the_declaration() {
    for (source, expected) in [
        (
            "let tint=reflect(Color(), label=12)",
            "must be a string literal",
        ),
        (
            "let tint=reflect(Color(), label=\"First\", label=\"Again\")",
            "supplied more than once",
        ),
        (
            "let tint=reflect(Color(), name=\"Label\")",
            "supported options are key and label",
        ),
        (
            "let tint=reflect(Color(), key=\"\")",
            "key must be nonempty",
        ),
        (
            "let tint=reflect(Color(), key=\" accent \")",
            "surrounding whitespace",
        ),
        (
            "let tint=reflect(Color(), label=\"  \")",
            "label must contain visible text",
        ),
        (
            "let tint=reflect(Color(), key=\"accent\", \"Label\")",
            "metadata must use",
        ),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert!(error.message.contains(expected), "{error}");
        assert_eq!(error.source, "main.sanim");
        assert!(error.span.column > 0);
    }
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let count=reflect(1)\n");
    let unknown_alias = BTreeMap::from([
        ("count".into(), serde_json::json!(2)),
        ("main.sanim::count".into(), serde_json::json!(3)),
    ]);
    let error = Engine::default()
        .prepare_json("main.sanim", &files, &unknown_alias)
        .unwrap_err();
    assert!(error.message.contains("unknown reflected input 'count'"));
    assert!(error.message.contains("main.sanim::count"));
    let error = Engine::default()
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("missing".into(), serde_json::json!(2))]),
        )
        .unwrap_err();
    assert!(error.message.contains("main.sanim::count"));
}
#[test]
fn compound_types_keep_full_schema_and_validate_all_nested_values() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","type Settings={palette:List<Color>,offset:Vec2<Length>}\nlet settings:Settings=reflect({palette=[Color()],offset=Vec2(0pt,0pt)}, label=\"Plot Style\", key=\"plot.style\")\n");
    let inputs = BTreeMap::from([(
        "plot.style".into(),
        serde_json::json!({"palette":["red3","blue3"],"offset":["2pt","4pt"]}),
    )]);
    let p = Engine::default()
        .prepare_json("main.sanim", &files, &inputs)
        .unwrap();
    assert!(
        matches!(p.reflected[0].ty.as_ref(),Type::Record(fields) if matches!(fields["palette"],Type::List(_, _)))
    );
    let bad = BTreeMap::from([(
        "plot.style".into(),
        serde_json::json!({"palette":["red3",99],"offset":["2pt","4pt"]}),
    )]);
    let error = Engine::default()
        .prepare_json("main.sanim", &files, &bad)
        .unwrap_err();
    assert!(error.message.contains("plot.style"));
    assert!(error.message.contains("Plot Style"));
}

#[test]
fn explicit_keys_are_independent_of_other_inputs_variable_names() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "import \"theme\" as theme\nlet tint=reflect(Color(\"red3\"))\n",
    );
    files.insert(
        "theme.sanim",
        "let accent=reflect(Color(\"blue3\"),key=\"tint\")\n",
    );
    let prepared = Engine::default()
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("tint".into(), serde_json::json!("green3"))]),
        )
        .unwrap();
    assert_eq!(
        prepared.bindings["tint"],
        Value::Color(Color::parse("red3").unwrap())
    );
    assert_eq!(
        prepared
            .reflected
            .iter()
            .find(|input| input.key == "tint")
            .unwrap()
            .value,
        Value::Color(Color::parse("green3").unwrap())
    );
}

#[test]
fn null_is_an_optional_value_and_omission_resets_to_the_default() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let maybe_value:Option<Number>=reflect(None, label=\"Optional Count\")\n",
    );
    let mut engine = Engine::default();
    let null = BTreeMap::from([("main.sanim::maybe_value".into(), serde_json::Value::Null)]);
    let p = engine.prepare_json("main.sanim", &files, &null).unwrap();
    assert_eq!(p.reflected[0].value, Value::None);
    let value = engine
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("main.sanim::maybe_value".into(), serde_json::json!(4))]),
        )
        .unwrap();
    assert!(
        matches!(&value.reflected[0].value,Value::Some(inner) if inner.number().unwrap().value==Fraction::from(4))
    );
    let reset = engine
        .prepare_json("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(reset.reflected[0].value, Value::None);
    files.insert("main.sanim", "let count:Number=reflect(1)\n");
    let error = engine
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("main.sanim::count".into(), serde_json::Value::Null)]),
        )
        .unwrap_err();
    assert!(error.message.contains("expected Number"), "{error}");
}

#[test]
fn metadata_freshness_checks_cover_imports_and_default_data_without_repreparing() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "import \"theme\" as theme\nlet tint:Color=reflect(read<Color>(\"tint.json\"))\n",
    );
    files.insert("theme.sanim", "let size=1pt\n");
    files.insert("tint.json", "\"red3\"");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    prepared.validate_dependencies(&files).unwrap();
    files.insert("unrelated.json", "42");
    prepared.validate_dependencies(&files).unwrap();
    files.insert("theme.sanim", "let size=2pt\n");
    let error = prepared.validate_dependencies(&files).unwrap_err();
    assert_eq!(error.code, "S203");
    assert!(error.message.contains("theme.sanim"));
    files.insert("theme.sanim", "let size=1pt\n");
    files.insert("tint.json", "\"blue3\"");
    let error = prepared.validate_dependencies(&files).unwrap_err();
    assert_eq!(error.code, "S203");
    assert!(error.message.contains("tint.json"));
}

#[test]
fn expression_reflection_exposes_the_wrapped_value_before_composition() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let title=reflect(\"title\",label=\"Heading\")+\"test\"\n",
    );
    let mut engine = Engine::default();
    let default = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(default.bindings["title"], Value::String("titletest".into()));
    assert_eq!(default.reflected.len(), 1);
    let input = &default.reflected[0];
    assert_eq!(input.key, "main.sanim::title");
    assert_eq!(input.label, "Heading");
    assert_eq!(input.ty.as_ref(), &Type::String);
    assert_eq!(input.default, Value::String("title".into()));
    assert_eq!(input.value, input.default);
    let updated = engine
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("main.sanim::title".into(), serde_json::json!("edited"))]),
        )
        .unwrap();
    assert_eq!(
        updated.bindings["title"],
        Value::String("editedtest".into())
    );
    assert_eq!(updated.reflected[0].default, Value::String("title".into()));
    assert_eq!(updated.reflected[0].value, Value::String("edited".into()));
}

#[test]
fn expression_reflection_accepts_native_values_and_stable_explicit_keys() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let color=reflect(Color(),label=\"Display Color\",key=\"theme.accent\")\nadd(Circle(id=\"dot\",fill=color))\n");
    let p = Engine::default()
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("theme.accent".into(), serde_json::json!("blue3"))]),
        )
        .unwrap();
    assert_eq!(p.reflected.len(), 1);
    assert_eq!(p.reflected[0].key, "theme.accent");
    assert_eq!(p.reflected[0].label, "Display Color");
    assert_eq!(p.reflected[0].ty.as_ref(), &Type::Color);
    assert_eq!(
        p.reflected[0].default,
        Value::Color(Color::parse("red3").unwrap())
    );
    assert_eq!(
        p.animation.layers[0].element.fill,
        sanim_core::Paint::Solid(Color::parse("blue3").unwrap())
    );
}

#[test]
fn removed_reflection_declaration_syntax_is_rejected() {
    for source in [
        "reflect color=Color()",
        "reflect(\"Display Color\") color=Color()",
        "reflect(key=\"theme.accent\") color=Color()",
        "reflect() color=Color()",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.code, "S002", "{error}");
        assert_eq!(error.source, "main.sanim");
        assert!(error.span.column > 0);
    }
}

#[test]
fn contextual_types_resolve_optional_and_empty_expression_defaults() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let maybe:Option<Number>=reflect(None)\nlet values:List<Number>=reflect([])\n",
    );
    let mut engine = Engine::default();
    let default = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        default.reflected[0].ty.as_ref(),
        &Type::Option(Box::new(Type::Numeric(Unit::Number)))
    );
    assert_eq!(
        default.reflected[1].ty.as_ref(),
        &Type::List(
            Box::new(Type::Numeric(Unit::Number)),
            sanim_core::ListLength::Unknown
        )
    );
    assert_eq!(default.reflected[0].default, Value::None);
    let overridden = engine
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([
                ("main.sanim::maybe".into(), serde_json::json!(4)),
                ("main.sanim::values".into(), serde_json::json!([1, 2])),
            ]),
        )
        .unwrap();
    assert!(
        matches!(&overridden.bindings["maybe"], Value::Some(value) if value.number().unwrap().value==Fraction::from(4))
    );
    assert!(matches!(&overridden.bindings["values"], Value::List(values) if values.len()==2));
}

#[test]
fn nested_record_and_constructor_arguments_infer_distinct_field_keys() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let style={fill=reflect(Color(\"red3\")),radius=reflect(12pt)}\nlet marker=Circle(id=\"dot\",fill=reflect(Color(\"orange3\")),radius=style.radius)\nadd(marker)\n");
    let inputs = BTreeMap::from([
        ("main.sanim::style.fill".into(), serde_json::json!("green3")),
        ("main.sanim::style.radius".into(), serde_json::json!("20pt")),
        ("main.sanim::marker.fill".into(), serde_json::json!("blue3")),
    ]);
    let p = Engine::default()
        .prepare_json("main.sanim", &files, &inputs)
        .unwrap();
    let keys: Vec<_> = p.reflected.iter().map(|input| input.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "main.sanim::style.fill",
            "main.sanim::style.radius",
            "main.sanim::marker.fill"
        ]
    );
    assert_eq!(
        p.animation.layers[0].element.fill,
        sanim_core::Paint::Solid(Color::parse("blue3").unwrap())
    );
    let Value::Record(style) = &p.bindings["style"] else {
        panic!("expected style record")
    };
    assert_eq!(style["fill"], Value::Color(Color::parse("green3").unwrap()));
    assert_eq!(style["radius"].number().unwrap().value, Fraction::from(20));
}

#[test]
fn inline_constructor_reflection_uses_an_explicit_key_without_a_binding() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "add(Circle(id=\"dot\",fill=reflect(Color(\"red3\"),key=\"dot.fill\")))\n",
    );
    let p = Engine::default()
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("dot.fill".into(), serde_json::json!("blue3"))]),
        )
        .unwrap();
    assert_eq!(p.reflected.len(), 1);
    assert_eq!(p.reflected[0].key, "dot.fill");
    assert_eq!(
        p.animation.layers[0].element.fill,
        sanim_core::Paint::Solid(Color::parse("blue3").unwrap())
    );
}

#[test]
fn expression_inputs_control_bounded_repeats_and_exact_duration_planning() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let count=reflect(2)\nlet step=reflect(100ms)\nrepeat count {wait(step)}\nwait(reflect(200ms,key=\"tail\"))\n");
    let inputs = BTreeMap::from([
        ("main.sanim::count".into(), serde_json::json!(3)),
        ("main.sanim::step".into(), serde_json::json!("250ms")),
        ("tail".into(), serde_json::json!("500ms")),
    ]);
    let mut engine = Engine::default();
    let duration = engine.duration_json("main.sanim", &files, &inputs).unwrap();
    let full = engine.prepare_json("main.sanim", &files, &inputs).unwrap();
    assert_eq!(duration.duration, Fraction::new(5u64, 4u64));
    assert_eq!(full.animation.duration, duration.duration);
    assert_eq!(full.reflected.len(), 3);
}

#[test]
fn warm_expression_reuse_preserves_reflection_registration_and_changed_overrides() {
    let mut files = MemorySources::default();
    let mut engine = Engine::default();
    files.insert(
        "main.sanim",
        "wait(1s)\nlet title=reflect(\"title\")+\"test\"\nwait(1s)\n",
    );
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    // Changing the first statement forces the expression to run again with warm value caches.
    files.insert(
        "main.sanim",
        "wait(2s)\nlet title=reflect(\"title\")+\"test\"\nwait(1s)\n",
    );
    let inputs = BTreeMap::from([("main.sanim::title".into(), serde_json::json!("new"))]);
    let warm = engine.prepare_json("main.sanim", &files, &inputs).unwrap();
    let fresh = Engine::default()
        .prepare_json("main.sanim", &files, &inputs)
        .unwrap();
    assert_eq!(warm.statistics.restored_statements, 0);
    assert_eq!(warm.reflected.len(), 1);
    assert_eq!(warm.bindings["title"], Value::String("newtest".into()));
    assert_eq!(warm.fingerprint, fresh.fingerprint);
    assert_eq!(
        serde_json::to_value(&warm.reflected).unwrap(),
        serde_json::to_value(&fresh.reflected).unwrap()
    );
}

#[test]
fn suffix_hot_edits_restore_exactly_one_reflection_entry() {
    let mut files = MemorySources::default();
    let mut engine = Engine::default();
    let prefix = "let title=reflect(\"title\",label=\"Heading\")+\"test\"\nwait(1s)\n";
    files.insert("main.sanim", format!("{prefix}wait(1s)\n"));
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let checkpoint = engine.execution_state(2).unwrap();
    files.insert("main.sanim", format!("{prefix}wait(2s)\n"));
    let warm = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let fresh = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(warm.statistics.restored_statements, 2);
    assert!(Arc::ptr_eq(
        &checkpoint,
        &engine.execution_state(2).unwrap()
    ));
    assert_eq!(warm.reflected.len(), 1);
    assert_eq!(warm.reflected[0].default, Value::String("title".into()));
    assert_eq!(warm.bindings["title"], Value::String("titletest".into()));
    assert_eq!(warm.fingerprint, fresh.fingerprint);
}

#[test]
fn expression_reflection_rejects_nonstatic_metadata_and_nonreflectable_defaults() {
    for source in [
        "let caption=\"Heading\"\nlet title=reflect(\"title\",label=caption)",
        "let title=reflect(\"title\",key=\"theme.\"+\"title\")",
        "let title=reflect(\"title\",label=12)",
        "let title=reflect(\"title\",label=\"A\",label=\"B\")",
        "let title=reflect(\"title\",unknown=\"value\")",
        "let title=reflect(\"title\",key=\"\")",
        "let title=reflect(\"title\",label=\"  \")",
        "let title=reflect()",
        "let title=reflect(Circle())",
        "let title:Number=reflect(\"title\")",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.source, "main.sanim", "{error}");
        assert!(
            error.message.contains("reflect") || error.message.contains("Number"),
            "{error}"
        );
        assert!(error.span.column > 0, "{error}");
    }
}

#[test]
fn reflection_in_repeated_or_conditional_execution_is_rejected_before_evaluation() {
    for source in [
        "if true {let title=reflect(\"title\",key=\"title\")}",
        "repeat 2 {let title=reflect(\"title\",key=\"title\")}",
        "for i in range(2) {let title=reflect(\"title\",key=\"title\")}",
        "fn title():String{return reflect(\"title\",key=\"title\")}\nlet result=title()",
        "let selected=false and reflect(true)",
        "let value=reflect(reflect(1))",
        "fn computed():Number{wait(1s);return 1}\nlet value=reflect(computed())",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert!(error.message.contains("reflect"), "{error}");
        assert!(error.span.column > 0, "{error}");
    }
}

#[test]
fn expression_reflection_reports_duplicate_canonical_keys() {
    for source in [
        "let first=reflect(1,key=\"shared\")\nlet second=reflect(2,key=\"shared\")",
        "let first=reflect(1,key=\"main.sanim::second\")\nlet second=reflect(2)",
        "let condition=reflect(true)\nif reflect(true,key=\"main.sanim::condition\") {wait(1s)}",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert!(error.message.contains("key"), "{error}");
        assert!(error.message.contains("reflect"), "{error}");
    }
    let explicit = run("let total=reflect(1,key=\"first\")+reflect(2,key=\"second\")\n");
    assert_eq!(explicit.reflected.len(), 2);
    assert_eq!(
        explicit.bindings["total"].number().unwrap().value,
        Fraction::from(3)
    );
}

#[test]
fn multiple_operator_inputs_and_anonymous_constructor_fields_have_semantic_keys() {
    let p = run("let total=reflect(1)+reflect(2)\nadd(Circle(id=\"dot\",fill=reflect(Color())))\n");
    let keys: Vec<_> = p.reflected.iter().map(|input| input.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "main.sanim::total.left",
            "main.sanim::total.right",
            "main.sanim::Circle.fill"
        ]
    );
    assert_eq!(
        p.bindings["total"].number().unwrap().value,
        Fraction::from(3)
    );
    assert_eq!(
        p.animation.layers[0].element.fill,
        sanim_core::Paint::Solid(Color::parse("red3").unwrap())
    );
}

#[test]
fn expression_reflection_has_one_canonical_ast_node() {
    use sanim_lang::{
        ast::{ExprKind, StatementKind},
        parser,
    };
    for source in [
        "let color=reflect(Color(),label=\"Accent\",key=\"theme.accent\")",
        "let color=reflect(Color(),key=\"theme.accent\",label=\"Accent\")",
    ] {
        let module = parser::parse("main.sanim", source, 200_000).unwrap();
        let StatementKind::Bind { name, value, .. } = &module.statements[0].kind else {
            panic!("reflection should use an ordinary binding")
        };
        assert_eq!(name, "color");
        let ExprKind::Reflect { value, options } = &value.kind else {
            panic!("reflection should have one canonical expression representation")
        };
        assert_eq!(options.key.as_deref(), Some("theme.accent"));
        assert_eq!(options.label.as_deref(), Some("Accent"));
        assert!(
            matches!(&value.kind, ExprKind::Call { callee, .. } if matches!(&callee.kind, ExprKind::Name(name) if name=="Color"))
        );
    }
}

#[test]
fn anonymous_fields_accept_only_their_explicit_config_keys() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "add(Circle(id=\"first\",fill=reflect(Color(),key=\"palette.first\")))\nadd(Circle(id=\"second\",fill=reflect(Color(),key=\"palette.second\")))\n");
    let inputs = BTreeMap::from([
        ("palette.first".into(), serde_json::json!("blue3")),
        ("palette.second".into(), serde_json::json!("green3")),
    ]);
    let mut engine = Engine::default();
    let p = engine.prepare_json("main.sanim", &files, &inputs).unwrap();
    assert_eq!(p.reflected.len(), 2);
    assert_eq!(p.reflected[0].name, "Circle.fill");
    assert_eq!(p.reflected[1].name, "Circle.fill");
    assert_eq!(
        p.animation.layers[0].element.fill,
        sanim_core::Paint::Solid(Color::parse("blue3").unwrap())
    );
    assert_eq!(
        p.animation.layers[1].element.fill,
        sanim_core::Paint::Solid(Color::parse("green3").unwrap())
    );
    let error = engine
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("Circle.fill".into(), serde_json::json!("orange3"))]),
        )
        .unwrap_err();
    assert_eq!(error.code, "S302");
    assert!(error.message.contains("unknown reflected input"), "{error}");
    assert!(error.message.contains("palette.first"), "{error}");
    assert!(error.message.contains("palette.second"), "{error}");
}

#[test]
fn generic_arguments_infer_a_concrete_reflected_type_from_the_default() {
    for generic in ["T", "T:Reflectable"] {
        let mut files = MemorySources::default();
        files.insert(
            "main.sanim",
            format!("fn identity<{generic}>(value:T):T{{return value}}\nlet result=identity(reflect(2))\n"),
        );
        let mut engine = Engine::default();
        let default = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(default.reflected.len(), 1);
        assert_eq!(
            default.reflected[0].ty.as_ref(),
            &Type::Numeric(Unit::Number)
        );
        assert_eq!(
            default.bindings["result"].number().unwrap().value,
            Fraction::from(2)
        );
        let key = default.reflected[0].key.clone();
        let overridden = engine
            .prepare_json(
                "main.sanim",
                &files,
                &BTreeMap::from([(key, serde_json::json!(7))]),
            )
            .unwrap();
        assert_eq!(
            overridden.bindings["result"].number().unwrap().value,
            Fraction::from(7)
        );
        assert_eq!(
            overridden.reflected[0].default.number().unwrap().value,
            Fraction::from(2)
        );
    }
}

#[test]
fn generic_optional_defaults_require_a_concrete_inner_type_before_reflection() {
    for generic in ["T", "T:Reflectable"] {
        let mut files = MemorySources::default();
        files.insert(
            "main.sanim",
            format!("fn identity<{generic}>(value:Option<T>):Option<T>{{return value}}\nlet result=identity(reflect(None))\n"),
        );
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.code, "S113", "{error}");
        assert!(error.message.contains("static type"), "{error}");
        assert!(error.message.contains("annotation"), "{error}");
    }
    let explicit = run(
        "fn identity<T>(value:Option<T>):Option<T>{return value}\nlet result=identity<Number>(reflect(None))\n",
    );
    assert_eq!(
        explicit.reflected[0].ty.as_ref(),
        &Type::Option(Box::new(Type::Numeric(Unit::Number)))
    );
    assert_eq!(explicit.bindings["result"], Value::None);
}

#[test]
fn read_color_defaults_infer_reflection_schema_without_annotations() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let accent=reflect(read<Color>(\"accent.json\"))",
    );
    files.insert("accent.json", "\"red3\"");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.reflected[0].ty.as_ref(), &sanim_core::Type::Color);
    assert_eq!(
        prepared.bindings["accent"],
        Value::Color(Color::parse("red3").unwrap())
    );
}
