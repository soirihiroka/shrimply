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
let typeface = read<Font>("not-installed.ttf")
let title = <Label font={typeface}>Native text is deferred</Label>
FadeIn(title, duration=250ms)
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
    sources.insert("main.sanim", "let title=Text(\"text\")\nadd(title)");
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
    sources.insert("main.sanim", script(0));
    let mut engine = Engine::default();
    let initial = engine
        .duration("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(initial.statistics.executed_queries, 1);
    let start = Instant::now();
    for edit in 1..=EDITS {
        sources.insert("main.sanim", script(edit));
        let changed = engine
            .duration("main.sanim", &sources, &BTreeMap::new())
            .unwrap();
        assert_eq!(changed.duration, Fraction::new(edit + 10, 1000u64));
        assert_eq!(changed.statistics.executed_queries, 0);
        assert_eq!(changed.statistics.decoded_assets, 0);
        assert!(changed.statistics.reused_queries > 0);
        black_box(changed);
    }
    let elapsed = start.elapsed();
    eprintln!(
        "headless duration: {EDITS} unique edits with {ROWS} CSV rows, {elapsed:?} total, {:?}/edit; zero repeated queries/decodes",
        elapsed / EDITS as u32
    );
}
