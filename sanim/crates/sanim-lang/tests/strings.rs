use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn check(source: &str) {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
}

#[test]
fn strings_interpolate_expressions_and_preserve_literal_spacing_and_unicode() {
    check(
        r#"
let n = 2
let r = 3
let text = "n={n}, sum={n + 1}, half={0.5}  τέλος"
assert(text == "n=2, sum=3, half=0.5  τέλος")
assert('r={r}' == 'r=3')
assert("{{name}}={n}" == r"{name}=2")
assert("{ decode<Number>({'key': 5}['key']) }" == "5")
assert("{str("nested")}" == "nested")
"#,
    );
}

#[test]
fn interpolation_follows_signals_while_let_preserves_its_original_value() {
    check(
        r#"
mut a = 1
sig text = "a={a}"
let original = "a={a}"
assert(text == "a=1")
a = 2
assert(text == "a=2")
assert(original == "a=1")
"#,
    );
}

#[test]
fn raw_strings_disable_interpolation_and_keep_the_normal_escape_rules() {
    check(
        r#"
let text = r"{missing}\nquoted=\"value\""
assert(text == r'{missing}\nquoted="value"')
let json = r"{\"n\":1}"
assert(json == r'{"n":1}')
"#,
    );
}

#[test]
fn triple_quotes_preserve_multiline_text_quotes_and_reactive_interpolation() {
    check(
        r####"
mut n = 3
sig message = """value={n:04d}
  "double", 'single', ""two"", # literal text
"""
assert(message == "value=0003\n  \"double\", 'single', \"\"two\"\", # literal text\n")
n = 12
assert(message == "value=0012\n  \"double\", 'single', \"\"two\"\", # literal text\n")
assert('''first
second''' == "first\nsecond")
assert(r"""{"unchanged": true}
# stays text""" == r"{\"unchanged\": true}\n# stays text")
assert("""""" == "")
assert('''''' == "")
"####,
    );
}

#[test]
fn multiline_sql_uses_the_existing_typed_sqlite_query_api() {
    check(
        r####"
let rows = [{value=3}, {value=1}, {value=2}]
let minimum = 2
let sql = Sql("""
    SELECT "value"
    FROM samples
    WHERE value >= {minimum}
    ORDER BY value
""")
let result = query<Number>(sql, tables={samples=rows})
assert(result == [2, 3])
"####,
    );
}

#[test]
fn multiline_strings_keep_crlf_contents_and_correct_following_diagnostics() {
    check("let text=\"\"\"a\r\nb\"\"\"\r\nassert(text==\"a\\r\\nb\")");
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "let text=\"\"\"a\r\nb\"\"\"\r\nlet broken=missing",
    );
    let error = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.span.line, 3, "{error}");
    let error = sanim_lang::parser::parse("main.sanim", "let text=\"\"\"unclosed\ntext", 100_000)
        .unwrap_err();
    assert!(error.message.contains("expected closing \"\"\""), "{error}");
    assert_eq!(error.span.line, 1);
}

#[test]
fn format_specifications_conversions_and_nested_width_share_the_str_api() {
    check(
        r#"
let value = 12.345
let width = 8
let precision = 2
assert("{value:.2f}" == "12.35")
assert("{value:{width}.{precision}f}" == "   12.35")
assert("{0.125:.1%}" == "12.5%")
assert("{255:#06x}" == "0x00ff")
assert("{'hello'!r}" == "'hello'")
assert("{'é'!a}" == r"'\\xe9'")
assert("{'hello'!s:>7}" == "  hello")
"#,
    );
}

#[test]
fn multiline_interpolation_uses_the_same_expression_parser_and_timeline_order() {
    check(
        r#"
fn step(value: Number): Number {
    wait(100ms)
    return value
}
let text = "first={step(1)}, second={
    step(2) + 3
}"
assert(text == "first=1, second=5")
wait(100ms)
"#,
    );
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "fn step(): Number { wait(100ms)\nreturn 1 }\nlet text=\"{step()}{step()}\"\nassert(text==\"11\")");
    let prepared = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        prepared.animation.duration,
        sanim_core::Fraction::new(1u64, 5u64)
    );
}

#[test]
fn malformed_interpolation_has_a_source_location_and_actionable_reason() {
    for (source, expected) in [
        ("let text=\"value={}\"", "empty string interpolation"),
        ("let text=\"value={1\"", "close string interpolation"),
        ("let text=\"value=}\"", "unmatched '}'"),
        ("let text=\"value={1!z}\"", "supported conversions"),
        ("let text=\"value={unknown}\"", "unknown"),
        ("import \"{1}\" as library", "import path must be a literal"),
        (
            "let n=1\nlet v=reflect(1,label=\"Value {n}\")",
            "must be a string literal",
        ),
    ] {
        let mut sources = MemorySources::default();
        sources.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap_err();
        assert!(error.message.contains(expected), "{source}: {error}");
        assert_eq!(error.source, "main.sanim");
        assert!(error.span.line > 0);
    }
}
