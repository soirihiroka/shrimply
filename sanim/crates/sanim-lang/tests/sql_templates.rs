use sanim_core::{Type, Value};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;
fn prepare(code: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", code);
    Engine::default().prepare("main.sanim", &sources, &BTreeMap::new())
}
fn check(code: &str) {
    prepare(code).unwrap();
}
#[test]
fn expected_type_preserves_templates_for_any_function_and_infers_strings_by_default() {
    let result = prepare(
        r#"
fn display(message:Template):String {return str(message)}
let count=7
let rendered="count={count:03d}"
let preserved:Template="count={count:03d}"
assert(rendered=="count=007")
assert(str(preserved)==rendered)
assert(display("count={count:03d}")==rendered)
let literals:List<Template>=["one","two={count}"]
assert(str(unwrap(literals[0],preserved))=="one")
"#,
    )
    .unwrap();
    assert_eq!(result.bindings["rendered"].ty(), Type::String);
    assert_eq!(result.bindings["preserved"].ty(), Type::Template);
}
#[test]
fn template_values_render_only_in_a_string_context_including_functions_and_builtins() {
    let p = prepare(
        r#"
fn as_text(message:String):String {return message}
fn return_text(message:Template):String {return message}
let value=7
let template:Template="count={value:03d}"
let rendered=as_text(template)
let label=Text(template)
let label_again=Text(as_text(template))
let via_return=return_text(template)
let preserved=Sql("SELECT {value}")
assert(rendered=="count=007")
assert(via_return==rendered)
assert(str(template)==rendered)
assert(query<Number>(preserved,tables={})==[7])
"#,
    )
    .unwrap();
    assert_eq!(p.bindings["rendered"].ty(), Type::String);
    assert_eq!(p.bindings["template"].ty(), Type::Template);
}
#[test]
fn identical_literal_text_in_different_expected_contexts_cannot_collide_in_value_cache() {
    let p = prepare(
        r#"
let first="same literal"
let second:Template="same literal"
let third="same literal"
let fourth:Template="same literal"
assert(first==third)
assert(str(second)==str(fourth))
"#,
    )
    .unwrap();
    assert!(matches!(p.bindings["first"], Value::String(_)));
    assert!(matches!(p.bindings["second"], Value::Template(_)));
    assert!(matches!(p.bindings["fourth"], Value::Template(_)));
}
#[test]
fn direct_sql_and_reusable_statements_bind_scalars_without_rendering_the_values() {
    let prepared=prepare(r####"
let rows=[{value=1,name="one"},{value=2,name="two"},{value=3,name="three"}]
let minimum=2
let template:Template="SELECT value FROM samples WHERE value >= {minimum} ORDER BY value"
let statement=Sql(template)
assert(query<Number>(template,tables={samples=rows})==[2,3])
assert(query<Number>(statement,tables={samples=rows})==[2,3])
assert(query<Number>("SELECT value FROM samples WHERE value < {minimum}",tables={samples=rows})==[1])
"####).unwrap();
    assert_eq!(prepared.statistics.executed_queries, 2);
    assert_eq!(prepared.statistics.reused_queries, 1);
}

#[test]
fn templates_without_table_inputs_use_the_same_bounded_sqlite_executor() {
    check(
        r#"
let number=4
assert(query<Number>("SELECT {number}+1")==[5])
assert(query<Number>("SELECT 2",tables={})==[2])
"#,
    );
}
#[test]
fn scalar_holes_preserve_nullable_values_units_and_exact_fraction_text() {
    check(
        r####"
let distance=2.5pt
let present:Option<Length>=Some(distance)
let absent:Option<Length>=None
let flag=true
let ratio=Fraction(1,3)
assert(query<Length>("SELECT {distance}+1")==[3.5pt])
assert(query<Option<Length>>("SELECT {present}")==[Some(distance)])
assert(query<Option<Length>>("SELECT {absent}")==[None])
assert(query<Bool>("SELECT {flag}")==[true])
assert(query<Fraction>("SELECT {ratio}")==[ratio])
assert(query<String>(r"""SELECT '{"key":"{literal}"}'""")==[r'{"key":"{literal}"}'])
"####,
    );
}
#[test]
fn malicious_string_values_are_sqlite_parameters_and_never_sql_syntax() {
    check(
        r#"
let injected="x' OR 1=1; DROP TABLE samples; --"
let rows=[{name="normal",value=1},{name=injected,value=2}]
let statement=Sql("SELECT value FROM samples WHERE name={injected}")
assert(query<Number>(statement,tables={samples=rows})==[2])
assert(query<Number>("SELECT COUNT(*) FROM samples",tables={samples=rows})==[2])
let none:Option<Number>=None
assert(query<Number>("SELECT COUNT(*) FROM samples WHERE {none} IS NULL",tables={samples=rows})==[2])
"#,
    );
}
#[test]
fn identifiers_are_explicit_and_missing_identifiers_do_not_become_quoted_strings() {
    check(
        r#"
let rows=[{value=4},{value=8}]
let table=SqlIdent("samples")
let column=SqlIdent("value")
assert(query<Number>("SELECT {column} FROM {table} ORDER BY {column}",tables={samples=rows})==[4,8])
"#,
    );
    for (code, expected) in [
        (
            r#"let rows=[{value=1}];let table="samples";let result=query("SELECT * FROM {table}",tables={samples=rows})"#,
            "SqlIdent",
        ),
        (
            r#"let rows=[{value=1}];let field=SqlIdent("absent");let result=query("SELECT {field} FROM samples",tables={samples=rows})"#,
            "no such column",
        ),
        (
            r#"let rows=[{value=1}];let field=SqlIdent("value\" FROM samples; DROP TABLE samples; --");let result=query("SELECT {field} FROM samples",tables={samples=rows})"#,
            "no such column",
        ),
    ] {
        let error = prepare(code).unwrap_err();
        assert!(error.message.contains(expected), "{error}");
    }
}
#[test]
fn rendered_strings_legacy_params_and_sql_formats_have_actionable_static_errors() {
    for (code, expected) in [
        (
            r#"let sql="SELECT 1";let rows=query(sql,tables={})"#,
            "already rendered String",
        ),
        (
            r#"let sql=Sql("SELECT 1"+" + 2");let rows=query(sql,tables={})"#,
            "Template",
        ),
        (
            r#"let n=1;let rows=query("SELECT {n}",tables={},params={n=1})"#,
            "unknown argument 'params'",
        ),
        (
            r#"let n=1;let rows=query("SELECT {n:03d}",tables={})"#,
            "without formatting",
        ),
        (
            r#"let n=[1,2];let rows=query("SELECT {n}",tables={})"#,
            "scalar values",
        ),
    ] {
        let error = prepare(code).unwrap_err();
        assert!(error.message.contains(expected), "{error}");
    }
}
#[test]
fn quoted_and_commented_holes_fail_instead_of_silently_losing_bindings() {
    for code in [
        r#"let n=1;let rows=query("SELECT '{n}'",tables={})"#,
        r#"let n=1;let rows=query("SELECT 1 -- {n}",tables={})"#,
    ] {
        let error = prepare(code).unwrap_err();
        assert!(
            error.message.contains("outside SQL quotes")
                || error.message.contains("without SQL quotes"),
            "{error}"
        );
    }
}
#[test]
fn structured_templates_and_immutable_statements_capture_values_and_signals_update() {
    check(
        r#"
mut minimum=1
let rows=[{value=1},{value=2},{value=3}]
let frozen=Sql("SELECT value FROM samples WHERE value>={minimum} ORDER BY value")
sig live:Template="SELECT value FROM samples WHERE value>={minimum} ORDER BY value"
minimum=3
assert(query<Number>(frozen,tables={samples=rows})==[1,2,3])
assert(query<Number>(live,tables={samples=rows})==[3])
"#,
    );
}
#[test]
fn templates_preserve_timeline_effects_of_each_hole_in_source_order() {
    let p = prepare(
        r#"
mut count=0
fn next():Number {wait(100ms);count=count+1;return count}
let statement=Sql("SELECT {next()}+{next()}")
assert(query<Number>(statement,tables={})==[3])
wait(100ms)
"#,
    )
    .unwrap();
    assert_eq!(p.animation.duration, sanim_core::Fraction::new(3u64, 10u64));
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"
fn next():Number {wait(100ms);return 1}
let statement:Template="SELECT {next()}+{next()}"
wait(100ms)
"#,
    );
    let report = Engine::default()
        .duration("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(report.duration, sanim_core::Fraction::new(3u64, 10u64));
}
#[test]
fn changes_to_bound_values_reuse_the_database_and_invalidate_results_correctly() {
    let mut sources = MemorySources::default();
    let code = |minimum| {
        format!(
            r#"let rows=[{{value=1}},{{value=2}},{{value=3}}]
let minimum={minimum}
let statement=Sql("SELECT value FROM samples WHERE value>={{minimum}} ORDER BY value")
let result=query<Number>(statement,tables={{samples=rows}})
"#
        )
    };
    sources.insert("main.sanim", code(1));
    let mut engine = Engine::default();
    let first = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(first.bindings["result"].list().unwrap().len(), 3);
    sources.insert("main.sanim", code(3));
    let changed = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(changed.bindings["result"].list().unwrap().len(), 1);
    assert_eq!(changed.statistics.reused_databases, 1);
    assert_eq!(changed.statistics.executed_queries, 1);
}

#[test]
fn template_holes_observe_signal_returning_calls_as_typed_values() {
    check(
        r####"
fn twice(sig value:Number):sig Number {return value*2}
fn format_value():sig String {return "03d"}
mut source=3
assert("value={twice(source)}"=="value=6")
assert("{twice(source):{format_value()}}"=="006")
let frozen:Template="SELECT {twice(source)}"
sig live:Template="SELECT {twice(source)}"
source=4
assert(query<Number>(frozen)==[6])
assert(query<Number>(live)==[8])
assert(query<Number>("SELECT {twice(source)}")==[8])
"####,
    );
}
