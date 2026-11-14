use sanim_core::{Type, Value};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;
fn files(source: &str) -> MemorySources {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    files
}
fn run(source: &str) -> Prepared {
    Engine::default()
        .prepare("main.sanim", &files(source), &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
}
#[test]
fn literal_parameters_records_aliases_and_string_operations_are_static() {
    let prepared = run(r#"
type Phase = "before" | r"after"
type Sample = {variant:Phase, label:String}
fn choose(variant:Phase):String { return variant + "!" }
let phase="before"
let chosen=choose(phase)
let sample:Sample={variant:"after",label:chosen}
let comparison=phase=="after"
mut next=phase
next="after"
wait(1s)
"#);
    assert_eq!(prepared.bindings["chosen"].string().unwrap(), "before!");
    assert_eq!(prepared.bindings["next"].string().unwrap(), "after");
    assert_eq!(
        prepared.types["Phase"].string_literals().unwrap(),
        vec!["after", "before"]
    );
    assert_eq!(
        prepared.bindings["phase"].ty(),
        Type::String,
        "runtime strings retain one primitive representation"
    );
}
#[test]
fn bad_literals_and_unquoted_choices_have_actionable_diagnostics() {
    for source in [
        "fn choose(phase:\"before\"|\"after\"):String {return phase}\nlet value=choose(\"later\")",
        "type Sample={variant:\"before\"|\"after\"}\nlet sample:Sample={variant:\"later\"}",
    ] {
        let error = Engine::default()
            .prepare("main.sanim", &files(source), &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.code, "S106");
        assert!(
            error.message.contains("\"before\"")
                && error.message.contains("\"after\"")
                && error.message.contains("\"later\""),
            "{error}"
        );
    }
    for source in [
        "fn choose(phase:\"before\"|\"after\"):String {return phase}\nlet value=choose(before)",
        "type Sample={variant:\"before\"|\"after\"}\nlet sample:Sample={variant:before}",
    ] {
        let error = Engine::default()
            .prepare("main.sanim", &files(source), &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.code, "S120");
        assert!(
            error.message.contains("Did you mean \"before\"?"),
            "{error}"
        );
    }
    let error=Engine::default().prepare("main.sanim",&files("let before=1\nfn choose(phase:\"before\"|\"after\"):String{return phase}\nlet value=choose(before)"),&BTreeMap::new()).unwrap_err();
    assert_eq!(error.code, "S106");
    assert!(!error.message.contains("Did you mean"));
}
#[test]
fn reflected_defaults_widen_but_explicit_choices_validate_overrides() {
    let source = r#"
let initial="Hello"
let title=reflect(initial,key="title")
let direct=reflect("Hello",key="direct")
let phase:"before"|"after"=reflect("before",key="phase")
wait(1s)
"#;
    let files = files(source);
    let prepared = Engine::default()
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([
                ("title".into(), serde_json::json!("Other")),
                ("direct".into(), serde_json::json!("Another")),
                ("phase".into(), serde_json::json!("after")),
            ]),
        )
        .unwrap();
    assert_eq!(prepared.reflected[0].ty.as_ref(), &Type::String);
    assert_eq!(prepared.reflected[1].ty.as_ref(), &Type::String);
    assert_eq!(prepared.bindings["phase"].string().unwrap(), "after");
    let error = Engine::default()
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("phase".into(), serde_json::json!("later"))]),
        )
        .unwrap_err();
    assert!(
        error.message.contains("before") && error.message.contains("after"),
        "{error}"
    );
}
#[test]
fn imported_data_checks_literal_fields_and_nested_containers() {
    let mut files = files(
        "type Row={variant:\"before\"|\"after\"}\nlet rows=read<List<Row>>(\"rows.json\")\nwait(1s)",
    );
    files.insert("rows.json", r#"[{"variant":"before"},{"variant":"after"}]"#);
    assert!(
        Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .is_ok()
    );
    files.insert("rows.json", r#"[{"variant":"later"}]"#);
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("variant")
            && error.message.contains("before")
            && error.message.contains("after"),
        "{error}"
    );
    let literal = Type::StringLiteral("before".into());
    assert!(sanim_core::validate_value(&literal, &Value::String("after".into())).is_err());
    assert!(
        sanim_core::validate_value(
            &Type::List(Box::new(literal), sanim_core::ListLength::Unknown),
            &Value::List(sanim_core::List::typed(
                Type::String,
                [Value::String("after".into())].into_iter().collect()
            ))
        )
        .is_err()
    );
}
#[test]
fn type_strings_cannot_interpolate_and_format_round_trips() {
    let source = "type Phase=\"before\"|r\"after\"\nlet phase:Phase=\"before\"";
    let formatted =
        sanim_lang::formatter::format_source("main.sanim", source, Default::default(), 100_000)
            .unwrap();
    assert!(
        Engine::default()
            .prepare("main.sanim", &files(&formatted), &BTreeMap::new())
            .is_ok()
    );
    let error = Engine::default()
        .prepare(
            "main.sanim",
            &files("type Phase=\"be{value}\""),
            &BTreeMap::new(),
        )
        .unwrap_err();
    assert_eq!(error.code, "S002");
    assert!(error.message.contains("interpolation"));
}

#[test]
fn immutable_discriminants_narrow_records_and_both_branches() {
    let prepared = run(r#"
type Entry={variant:"before",left:Number}|{variant:"after",right:Number}
fn amount(entry:Entry):Number {
    if entry.variant=="before" {return entry.left} else {return entry.right}
}
fn exact(value:"before"|"after"):"before" {
    if value!="after" {return value} else {return "before"}
}
let answer=amount({variant:"after",right:7})
let phase=exact("before")
"#);
    assert_eq!(
        prepared.bindings["answer"].number().unwrap().value,
        7.into()
    );
    assert_eq!(prepared.bindings["phase"].string().unwrap(), "before");
}

#[test]
fn retained_literal_union_cells_and_signal_function_schemas_remain_exact() {
    let prepared = run(r#"
type Mode="hidden"|"scroll"
fn live(sig value:Mode):sig Mode {return value}
mut mode:Mode="hidden"
let mirrored=live(mode)
let panel=Box(overflow_y=mode,width=100pt,height=100pt)
let via_function=Box(overflow_y=mirrored,width=100pt,height=100pt)
add(panel)
add(via_function)
wait(100ms)
mode="scroll"
wait(1s)
"#);
    let before = prepared
        .animation
        .sample(sanim_core::Fraction::from(0))
        .unwrap();
    let after = prepared
        .animation
        .sample(sanim_core::Fraction::from(1))
        .unwrap();
    assert_eq!(before.len(), 2);
    for element in &before {
        assert_eq!(element.layout.overflow_y, sanim_core::Overflow::Hidden);
    }
    for element in &after {
        assert_eq!(element.layout.overflow_y, sanim_core::Overflow::Scroll);
    }
}

#[test]
fn emitted_literal_type_source_round_trips_braces_escapes_and_controls() {
    let value = "{before}\"\\\n\r\t\u{8}\u{c}\0😊";
    let quoted = sanim_core::template::quote_string(value);
    let displayed = Type::StringLiteral(value.into()).to_string();
    assert_eq!(displayed, quoted);
    let prepared = run(&format!("type Tag={quoted}\nlet tag:Tag={quoted}"));
    assert_eq!(prepared.bindings["tag"].string().unwrap(), value);
}

#[test]
fn retained_native_configuration_accepts_signal_literal_unions() {
    let prepared = run(r#"
type Tint="blue3"|"orange3"
fn palette(sig value:Tint):sig Tint {return value}
mut tint:Tint="blue3"
let live=palette(tint)
let marker=Circle(fill=Color(live))
add(marker)
wait(100ms)
tint="orange3"
wait(1s)
"#);
    let before = prepared
        .animation
        .sample(sanim_core::Fraction::from(0))
        .unwrap();
    let after = prepared
        .animation
        .sample(sanim_core::Fraction::from(1))
        .unwrap();
    assert_eq!(
        before[0].fill,
        sanim_core::Color::parse("blue3").unwrap().into()
    );
    assert_eq!(
        after[0].fill,
        sanim_core::Color::parse("orange3").unwrap().into()
    );
}

#[test]
fn retained_overflow_shorthand_preserves_static_and_live_axis_overrides() {
    let prepared = run(r#"
type Mode="auto"|"scroll"
mut mode:Mode="auto"
mut axis:"hidden"|"scroll"="hidden"
let fixed_x=Box(overflow=mode,overflow_x="hidden",width=100pt,height=100pt)
let fixed_y=Box(overflow=mode,overflow_y="hidden",width=100pt,height=100pt)
let both_fixed=Box(overflow=mode,overflow_x="hidden",overflow_y="hidden",width=100pt,height=100pt)
let live_x=Box(overflow=mode,overflow_x=axis,width=100pt,height=100pt)
add(fixed_x)
add(fixed_y)
add(both_fixed)
add(live_x)
wait(100ms)
mode="scroll"
axis="scroll"
wait(1s)
"#);
    use sanim_core::Overflow::{Auto, Hidden, Scroll};
    for (time, expected) in [
        (
            0,
            [
                (Hidden, Auto),
                (Auto, Hidden),
                (Hidden, Hidden),
                (Hidden, Auto),
            ],
        ),
        (
            1,
            [
                (Hidden, Scroll),
                (Scroll, Hidden),
                (Hidden, Hidden),
                (Scroll, Scroll),
            ],
        ),
    ] {
        let sampled = prepared
            .animation
            .sample(sanim_core::Fraction::from(time))
            .unwrap();
        assert_eq!(sampled.len(), expected.len());
        for (element, (x, y)) in sampled.iter().zip(expected) {
            assert_eq!(
                (element.layout.overflow_x, element.layout.overflow_y),
                (x, y),
                "{} at {time}s",
                element.id
            );
        }
    }
}

#[test]
fn retained_overflow_can_become_visible_and_release_scroll_evidence() {
    let prepared = run(r#"
mut mode:"auto"|"visible"="auto"
let panel=Box(overflow=mode,width=100pt,height=100pt,scroll=Vec2(10pt,20pt))
add(panel)
wait(100ms)
mode="visible"
wait(1s)
"#);
    let before = prepared
        .animation
        .sample(sanim_core::Fraction::from(0))
        .unwrap();
    assert!(before[0].layout.scroll.is_some());
    let after = prepared
        .animation
        .sample(sanim_core::Fraction::from(1))
        .unwrap();
    assert_eq!(after[0].layout.overflow_x, sanim_core::Overflow::Visible);
    assert_eq!(after[0].layout.overflow_y, sanim_core::Overflow::Visible);
    assert!(after[0].layout.scroll.is_none());
    assert_eq!(after[0].layout.scroll_range, [0.; 2]);
}

#[test]
fn a_visible_axis_resets_its_offset_without_erasing_the_still_clipped_axis() {
    let prepared = run(r#"
mut mode:"auto"|"visible"="auto"
let panel=Box(overflow=mode,overflow_y="hidden",width=100pt,height=100pt,scroll=Vec2(10pt,20pt),children=[Rect(width=300pt,height=400pt)])
add(panel)
wait(100ms)
mode="visible"
wait(1s)
"#);
    let before = prepared
        .animation
        .layout(0.into(), 200.into(), 200.into())
        .unwrap();
    assert_eq!(before[0].layout.scroll.unwrap().values, [10., 20.]);
    assert!(
        before[0]
            .layout
            .scroll_range
            .iter()
            .all(|range| *range > 0.)
    );
    let after = prepared
        .animation
        .layout(1.into(), 200.into(), 200.into())
        .unwrap();
    assert_eq!(after[0].layout.overflow_x, sanim_core::Overflow::Visible);
    assert_eq!(after[0].layout.overflow_y, sanim_core::Overflow::Hidden);
    assert_eq!(after[0].layout.scroll.unwrap().values, [0., 20.]);
    assert_eq!(after[0].layout.scroll_range[0], 0.);
    assert!(after[0].layout.scroll_range[1] > 0.);
}
