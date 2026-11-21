#![cfg(not(feature = "native-text"))]
use sanim_core::{Font, Fraction, Type, Value, code::Language};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn data_dependent_duration_does_not_require_font_assets_or_text_layout() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"
type Row = {delay: Number}
let rows = read<List<Row>>("delays.csv")
let typeface = Font("sans-serif")
let title = <Label font={typeface}>Native text is deferred</Label>
title.fade_in(duration=250ms)
for row in rows { wait(row.delay * 1ms) }
"#,
    );
    sources.insert("delays.csv", "delay\n5\n10\n");
    let mut engine = Engine::default();
    let first = engine
        .duration("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(first.duration, Fraction::new(265u64, 1000u64));
    sources.insert("delays.csv", "delay\n20\n30\n");
    let changed = engine
        .duration("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(changed.duration, Fraction::new(3u64, 10u64));
    assert_eq!(changed.statistics.decoded_assets, 1);
}

#[test]
fn duration_projection_preserves_nested_layout_and_does_not_pollute_real_preparation() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim",r#"
fn<ref> intro():Animation {
    let typeface=read<Font>("unavailable.ttf")
    let title=<Label font={typeface}>Deferred visual target</Label>
    return Animation(children=[title.fade_in(duration=250ms),title,wait(100ms)],flow="sequence",duration=1s)
}
intro()
"#);
    let mut engine = Engine::default();
    engine.preparation = sanim_lang::Preparation::Demand;
    let measured = engine
        .duration("main.sanim", &sources, &Default::default())
        .unwrap();
    assert_eq!(measured.duration, Fraction::from(1));
    assert_eq!(measured.statistics.decoded_assets, 0);
    // A temporal-only plan must never escape through expression or prefix caches
    // as a supposedly prepared visual scene in the same demand-mode engine.
    assert!(
        engine
            .prepare("main.sanim", &sources, &Default::default())
            .is_err()
    );
    assert_eq!(
        engine
            .duration("main.sanim", &sources, &Default::default())
            .unwrap()
            .duration,
        Fraction::from(1)
    );
}

#[test]
fn geometry_dependent_durations_require_real_metrics_instead_of_substitutes() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "let title=<Label>Measured width</Label>\nif title.width == 1pt {wait(1s)} else {wait(2s)}",
    );
    let error = Engine::default()
        .duration("main.sanim", &sources, &Default::default())
        .unwrap_err();
    assert!(error.message.contains("native text layout"), "{error}");
}

#[test]
fn visual_children_keep_zero_time_slots_for_temporal_gaps() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"
let a=<Label>First</Label>
let b=<Label>Second</Label>
Animation(children=[a,b],flow="sequence",gap=250ms)
"#,
    );
    assert_eq!(
        Engine::default()
            .duration("main.sanim", &sources, &Default::default())
            .unwrap()
            .duration,
        Fraction::new(1u64, 4u64)
    );
}

#[test]
fn named_fixed_visual_lists_project_their_known_length_without_font_layout() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"
let children=[<Label>First</Label>,<Label>Second</Label>]
Animation(children=children,flow="sequence",gap=100ms)
wait(1s)
"#,
    );
    assert_eq!(
        Engine::default()
            .duration("main.sanim", &sources, &Default::default())
            .unwrap()
            .duration,
        Fraction::new(11u64, 10u64)
    );
}

#[test]
fn projected_literal_child_lists_respect_host_limits_before_allocating_slots() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"Animation(children=[<Label>a</Label>,<Label>b</Label>,<Label>c</Label>])"#,
    );
    let mut engine = Engine::default();
    engine.limits.data.max_values = 2;
    let error = engine
        .duration("main.sanim", &sources, &Default::default())
        .unwrap_err();
    assert!(error.message.contains("temporal child list"), "{error}");
}

#[test]
fn duration_accepts_valid_overrides_for_deferred_visual_reflections() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"
let title=<Label fill={reflect(Color("red"),key="ink")}>Deferred color</Label>
title.fade_in(duration=250ms)
"#,
    );
    let overrides = [(
        "ink".to_string(),
        Value::Color(sanim_core::Color::parse("blue").unwrap()),
    )]
    .into_iter()
    .collect();
    assert_eq!(
        Engine::default()
            .duration("main.sanim", &sources, &overrides)
            .unwrap()
            .duration,
        Fraction::new(1u64, 4u64)
    );
    for (key, value) in [
        (
            "absent",
            Value::Color(sanim_core::Color::parse("blue").unwrap()),
        ),
        ("ink", Value::String("invalid Color value".into())),
    ] {
        let overrides = [(key.to_string(), value)].into_iter().collect();
        assert!(
            Engine::default()
                .duration("main.sanim", &sources, &overrides)
                .is_err()
        );
    }
}

#[test]
fn headless_data_sql_and_real_syntax_trees_remain_available() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"
type Row = {value: Number}
let rows = read<List<Row>>("values.yaml")
let settings = read<Dict>("settings.toml")
let text = read<String>("notes.txt")
let df = DataFrame(rows)
let result = df[df["value"] > 1].to_list()
assert(len(result)==2)
assert(text=="real text\n")
assert(decode<Number>(settings["count"])==3)
wait(len(result)*1s)
"#,
    );
    sources.insert("values.yaml", "- value: 1\n- value: 2\n- value: 3\n");
    sources.insert("settings.toml", "count = 3\n");
    sources.insert("notes.txt", "real text\n");
    let prepared = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.animation.duration, Fraction::from(2));
    assert_eq!(prepared.statistics.executed_queries, 1);
    let Value::Code(code) = sanim_data::decode(
        "source.py",
        b"def example():\n    return 42\n",
        &Type::Code(Language::Python),
        Default::default(),
    )
    .unwrap() else {
        panic!("parsed code")
    };
    assert!(code.syntax_parsed);
    assert!(!code.scopes.is_empty());
    assert!(!code.highlights.is_empty());
}

#[test]
fn unavailable_geometry_capabilities_return_errors_instead_of_fake_metrics() {
    assert!(sanim_data::font::system_font_epoch().is_none());
    let font = Font::family("sans-serif", "sans-serif").unwrap();
    assert!(
        sanim_data::font::cached_shape(&font, "text", Fraction::from(12), false, false)
            .err()
            .unwrap()
            .contains("native-text")
    );
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "let title=Text(\"text\")\nAnimation(children=[title])",
    );
    let error = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert!(error.message.contains("native text layout"), "{error}");
}

#[test]
#[ignore]
fn headless_duration_reload_benchmark() {
    use std::{fmt::Write, hint::black_box, time::Instant};
    const EDITS: u64 = 200;
    const ROWS: u64 = 10_000;
    let mut sources = MemorySources::default();
    let mut csv = String::from("value\n");
    for value in 0..ROWS {
        writeln!(csv, "{value}").unwrap();
    }
    sources.insert("values.csv", csv);
    let script = |tail| {
        format!(
            r#"type Row = {{value:Number}}
let rows=read<List<Row>>("values.csv")
let values=DataFrame(rows).head(10).to_list()
wait(len(values)*1ms)
wait({tail}ms)
"#
        )
    };
    sources.insert("main.sanim", format!("{}\nAnimation()", script(0)));
    let mut engine = Engine::default();
    let initial = engine
        .duration("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(initial.statistics.executed_queries, 1);
    let start = Instant::now();
    for edit in 1..=EDITS {
        sources.insert("main.sanim", format!("{}\nAnimation()", script(edit)));
        let changed = engine
            .duration("main.sanim", &sources, &BTreeMap::new())
            .unwrap();
        assert_eq!(changed.duration, Fraction::new(edit + 10, 1000u64));
        assert_eq!(changed.statistics.executed_queries, 0);
        assert_eq!(changed.statistics.decoded_assets, 0);
        assert!(
            changed.statistics.reused_queries > 0 || changed.statistics.restored_statements > 0
        );
        black_box(changed);
    }
    let elapsed = start.elapsed();
    eprintln!(
        "headless duration: {EDITS} unique edits with {ROWS} CSV rows, {elapsed:?} total, {:?}/edit; zero repeated queries/decodes",
        elapsed / EDITS as u32
    );
}
