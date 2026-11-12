use sanim_core::{Type, Value};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;
fn run(text: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", text);
    sources.insert("rows.csv", "name,value\na,1\nb,2\na,3\n");
    Engine::default().prepare("main.sanim", &sources, &BTreeMap::new())
}
#[test]
fn mixed_dictionary_and_nested_access() {
    let p=run("let d: Dict = {\"name\": \"sanim\", \"count\": 3, \"nested\": {\"items\": [4,5]}}\nlet title = decode<String>(d[\"name\"])\nlet amount = decode<Number>(d[\"nested\"][\"items\"][1])\nassert(title == \"sanim\")\nassert(amount == 5)\n").unwrap();
    assert_eq!(p.bindings["d"].ty(), Type::Dict);
}
#[test]
fn dictionaries_do_not_have_dot_fields() {
    let e = run("let d = {\"name\": \"sanim\"}\nlet s = d.name\n").unwrap_err();
    assert_eq!(e.code, "S121");
    assert!(e.message.contains("brackets"));
}
#[test]
fn dictionaries_have_no_generic_parameter() {
    assert!(run("let d: Dict<Number> = {\"a\": 1}\n").is_err());
}
#[test]
fn first_missing_key_is_reported_without_evaluating_later_access() {
    let e = run("let d = {\"present\": 1}\nlet s = d[\"key1\"][\"key2\"]\n").unwrap_err();
    assert_eq!(e.code, "S308");
    assert!(e.message.contains("key1"));
    assert!(!e.message.contains("key2"));
    assert!(e.message.contains("present"));
    assert_eq!(e.span.line, 2);
}
#[test]
fn wrong_nested_shape_has_path() {
    let e = run("let d = {\"name\": \"sanim\"}\nlet s = d[\"name\"][\"child\"]\n").unwrap_err();
    assert_eq!(e.code, "S308");
    assert!(e.message.contains("name") && e.message.contains("object or array"));
}
#[test]
fn optional_lookup_has_explicit_fallback() {
    run("let d = {\"a\": 1}\nlet n = decode<Number>(unwrap(get(d, \"missing\"), Data(42)))\nassert(n == 42)\nassert(len(keys(d)) == 1)\n").unwrap();
}
#[test]
fn dictionary_duplicate_keys_fail() {
    assert!(
        run("let d = {\"a\": 1, \"a\": 2}\n")
            .unwrap_err()
            .message
            .contains("duplicate dictionary")
    );
}
#[test]
fn dynamic_conversion_checks_actual_values() {
    assert!(
        run("let d = {\"count\": \"wrong\"}\nlet n = decode<Number>(d[\"count\"])\n")
            .unwrap_err()
            .message
            .contains("expected")
    );
}
#[test]
fn sql_default_result_is_dictionary_rows() {
    let p=run("type Sample = {name: String, value: Number}\nlet samples = read<Table<Sample>>(\"rows.csv\")\nlet grouped = query(\"SELECT name, SUM(value) AS total FROM samples GROUP BY name ORDER BY name\", tables: {samples: samples})\nfor row in grouped { let total = decode<Number>(row[\"total\"]); assert(total >= 2) }\n").unwrap();
    assert_eq!(
        p.bindings["grouped"].ty(),
        Type::Table(Box::new(Type::Dict))
    );
    assert_eq!(p.bindings["grouped"].list().unwrap().len(), 2);
}
#[test]
fn sql_can_validate_typed_rows_and_named_parameters() {
    run("type Sample = {name: String, value: Number}\ntype Summary = {name: String, total: Number}\nlet samples = read<Table<Sample>>(\"rows.csv\")\nlet result = query<Summary>(\"SELECT name, SUM(value) AS total FROM samples WHERE value >= :minimum GROUP BY name\", tables: {samples: samples}, params: {\"minimum\": 2})\nassert(len(result) == 2)\n").unwrap();
}
#[test]
fn sql_null_requires_optional_typed_result() {
    let e=run("type Sample = {name: String, value: Number}\ntype Result = {total: Number}\nlet samples = read<Table<Sample>>(\"rows.csv\")\nlet result = query<Result>(\"SELECT NULL AS total FROM samples LIMIT 1\", tables: {samples: samples})\n").unwrap_err();
    assert!(e.message.contains("Option<"));
}
#[test]
fn sql_rejects_bad_columns_even_when_result_is_empty() {
    let e=run("type Sample = {name: String, value: Number}\ntype Result = {wrong: Number}\nlet samples = read<Table<Sample>>(\"rows.csv\")\nlet result = query<Result>(\"SELECT value FROM samples WHERE 0\", tables: {samples: samples})\n").unwrap_err();
    assert!(e.message.contains("do not match"));
}
#[test]
fn sql_rejects_mutation_and_ambient_functions() {
    for sql in [
        "DELETE FROM samples",
        "ATTACH DATABASE '/tmp/forbidden.db' AS other",
        "SELECT random() AS value FROM samples",
        "SELECT datetime('now') AS value FROM samples",
    ] {
        let e=run(&format!("type Sample = {{name: String, value: Number}}\nlet samples = read<Table<Sample>>(\"rows.csv\")\nlet result = query({sql:?}, tables: {{samples: samples}})\n")).unwrap_err();
        assert_eq!(e.code, "S309");
    }
}
#[test]
fn yaml_toml_and_json_use_same_dynamic_access() {
    for (path, data) in [
        ("data.yaml", "settings:\n  count: 3\n"),
        ("data.yml", "settings: {count: 3}\n"),
        ("data.toml", "[settings]\ncount = 3\n"),
        ("data.json", "{\"settings\":{\"count\":3}}"),
    ] {
        let mut sources = MemorySources::default();
        sources.insert("main.sanim",format!("let d = read<Dict>({path:?})\nlet n = decode<Number>(d[\"settings\"][\"count\"])\nassert(n == 3)\n"));
        sources.insert(path, data);
        Engine::default()
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap();
    }
}
#[test]
fn scalar_data_is_not_a_dictionary() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "let d = read<Dict>(\"data.json\")\n");
    sources.insert("data.json", "3");
    assert!(
        Engine::default()
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .is_err()
    );
}
#[test]
fn unstructured_value_can_be_reflected() {
    let p = run("reflect settings: Dict = {\"amount\": 3, \"enabled\": true}\n").unwrap();
    assert!(matches!(p.bindings["settings"], Value::Dict(_)));
    assert!(p.reflected[0].ty.reflectable());
}

#[test]
fn data_roundtrips_exact_scalars_and_time() {
    run("let n=9007199254740993\nlet duration=1s/60\nassert(decode<Number>(Data(n)) == n)\nassert(decode<Duration>(Data(duration)) == duration)\nassert(decode<Color>(Data(Color(\"red\"))) == Color(\"red\"))\n").unwrap();
}

#[test]
fn dictionary_data_accepts_heterogeneous_arrays_and_null() {
    run("let d={\"items\": [1, \"two\", None, {\"enabled\": true}]}\nassert(decode<String>(d[\"items\"][1]) == \"two\")\nlet raw=Data([1, \"two\"])\nassert(decode<Number>(raw[0]) == 1)\n").unwrap();
}

#[test]
fn sql_preserves_large_integer_inputs() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","type Sample={value: Number}\nlet samples=read<Table<Sample>>(\"data.csv\")\nlet result=query(\"SELECT value FROM samples\",tables: {samples: samples})\nfor row in result { assert(decode<Number>(row[\"value\"]) == 9007199254740993) }\n");
    files.insert("data.csv", "value\n9007199254740993\n");
    Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
}
#[test]
fn sql_clock_keywords_are_rejected() {
    for sql in [
        "SELECT CURRENT_TIMESTAMP AS now FROM samples",
        "SELECT CURRENT_DATE AS today FROM samples",
        "SELECT CURRENT_TIME AS now FROM samples",
    ] {
        let e=run(&format!("type Sample = {{name: String, value: Number}}\nlet samples=read<Table<Sample>>(\"rows.csv\")\nlet result=query({sql:?},tables: {{samples: samples}})\n")).unwrap_err();
        assert_eq!(e.code, "S309");
    }
}
#[test]
fn warm_query_cache_reuses_results_but_data_changes_invalidate() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","type Sample = {value: Number}\nlet samples=read<Table<Sample>>(\"data.csv\")\nlet result=query(\"SELECT SUM(value) AS total FROM samples\",tables: {samples: samples})\n");
    files.insert("data.csv", "value\n1\n");
    let mut engine = Engine::default();
    let first = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(first.statistics.executed_queries, 1);
    let warm = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(warm.statistics.reused_queries, 1);
    files.insert("data.csv", "value\n2\n");
    let changed = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(changed.statistics.executed_queries, 1);
    assert_ne!(warm.bindings["result"], changed.bindings["result"]);
}
