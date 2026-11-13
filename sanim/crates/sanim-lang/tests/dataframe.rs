use sanim_core::Fraction;
use sanim_lang::{Engine, MemorySources, Preparation};
use std::collections::BTreeMap;

fn prepare(script: &str) -> sanim_lang::Prepared {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", script);
    Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap()
}
const INPUT: &str = r#"type Entry={kind:String,n:Number,value:Number}
let rows:List<Entry>=[{kind="b",n=2,value=20},{kind="a",n=1,value=10},{kind="b",n=3,value=30},{kind="a",n=4,value=8}]
let df=DataFrame(rows)
"#;
#[test]
fn typed_columns_filter_aggregate_sort_and_reuse_execute_one_sqlite_query() {
    let p = prepare(
        &[
            INPUT,
            r#"
let selected=df[(df["value"] >= 10) and (df["n"] < 4)]
let enriched=selected.assign({double=selected["value"]*2})
let summary=enriched.groupby("kind").agg({value="mean",double="sum"}).sort_values("kind")
let result=summary.to_list()
let reused=summary.to_list()
assert(len(result)==2)
assert(result==reused)
assert(unwrap(result[0],{kind="",value=None,double=0}).kind=="a")
assert(unwrap(unwrap(result[1],{kind="",value=None,double=0}).value,0)==25)
assert(unwrap(result[1],{kind="",value=None,double=0}).double==100)
wait(1s)
"#,
        ]
        .concat(),
    );
    assert_eq!(p.statistics.executed_queries, 1);
    assert_eq!(p.statistics.reused_queries, 1);
}
#[test]
fn projections_masks_reductions_and_head_tail_preserve_order() {
    let p = prepare(
        &[
            INPUT,
            r#"
let sorted=df.sort_values("value",ascending=false)
let tail=sorted.tail(2).select(["n","value"]).to_list()
let result=df["value"].to_list()
let average=df["value"].mean()
let sum=df["value"].sum()
assert(unwrap(tail[0],{n=0,value=0}).value==10)
assert(unwrap(tail[1],{n=0,value=0}).value==8)
assert(unwrap(result[0],0)==20)
assert(unwrap(average,0)==17)
assert(sum==68)
assert(df.columns==["kind","n","value"])
wait(1s)
"#,
        ]
        .concat(),
    );
    assert_eq!(
        p.bindings["sum"].number().unwrap().value,
        Fraction::from(68)
    );
}
#[test]
fn nullable_columns_join_and_missing_value_operations_are_sqlite_backed() {
    prepare(
        r#"type Left={key:Number,value:Option<Number>}
let rows:List<Left>=[{key=1,value=Some(4)},{key=2,value=None},{key=3,value=Some(7)}]
let df=DataFrame(rows)
let missing=df[df["value"].isna()].to_list()
let filled=df.assign({value=df["value"].fillna(0)}).to_list()
let right=DataFrame([{key=1,label="first"},{key=3,label="third"}])
let joined=df.merge(right,on="key",how="left").to_list()
assert(len(missing)==1)
assert(unwrap(missing[0],{key=0,value=None}).key==2)
assert(unwrap(filled[1],{key=0,value=0}).value==0)
assert(unwrap(joined[1],{key=0,value=None,label=None}).label==None)
assert(unwrap(unwrap(joined[0],{key=0,value=None,label=None}).label,"")=="first")
wait(1s)
"#,
    );
}
#[test]
fn empty_typed_frames_keep_schema_and_correct_reduction_identities() {
    prepare(
        r#"type Entry={key:Number,value:Number}
let empty:List<Entry>=[]
let df=DataFrame(empty)
assert(len(df.to_list())==0)
assert(len(df)==0)
assert(len(df["value"])==0)
assert(df["value"].sum()==0)
assert(df["value"].count()==0)
assert(df["value"].mean()==None)
assert(len(df.groupby("key").sum().to_list())==0)
wait(1s)
"#,
    );
}
#[test]
fn invalid_columns_and_schemas_fail_before_sql_execution() {
    for (code, expected) in [
        (
            format!("{INPUT}\nlet result=df[\"absent\"]"),
            "available columns",
        ),
        (
            format!("{INPUT}\nlet result=df.groupby(\"absent\").sum()"),
            "available columns",
        ),
        (
            [INPUT, "\nlet result=df.agg({kind=\"mean\"})"].concat(),
            "not supported for String",
        ),
        (
            [INPUT, "\nlet result=df.rename({kind=\"n\"})"].concat(),
            "duplicate column",
        ),
    ] {
        let mut sources = MemorySources::default();
        sources.insert("main.sanim", code);
        let error = Engine::default()
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap_err();
        assert!(error.message.contains(expected), "{error}");
    }
}
#[test]
fn grouped_column_selection_cannot_silently_drop_groups() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        [
            INPUT,
            "let invalid=df.groupby(\"kind\")[\"value\"].mean()\n",
        ]
        .concat(),
    );
    let error = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S122");
    assert!(error.message.contains("grouped view"));
    assert!(error.message.contains("aggregate"));
}

#[test]
fn demand_mode_does_not_force_unused_method_chains_or_source_files() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"type Entry={value:Number}
let df=DataFrame(read<List<Entry>>("absent.csv"))
let preview=df.head(5).sort_values("value").to_list()
wait(1s)
"#,
    );
    let mut engine = Engine::default();
    engine.preparation = Preparation::Demand;
    let p = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(p.statistics.decoded_assets, 0);
    assert_eq!(p.statistics.executed_queries, 0);
    assert!(!p.bindings.contains_key("preview"));
}
#[test]
fn frame_methods_have_timeline_effects_without_importing_or_materializing_for_duration() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"type Entry={value:Number}
let df=DataFrame(read<List<Entry>>("absent.csv"))
let preview=df.head(5,duration=500ms).to_list(duration=250ms)
wait(1s)
"#,
    );
    let report = Engine::default()
        .duration("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(report.duration, Fraction::new(7u64, 4u64));
    assert_eq!(report.statistics.executed_queries, 0);
}
#[test]
fn column_expressions_cannot_silently_align_unrelated_rows() {
    let script = [
        INPUT,
        r#"
let filtered=df[df["n"]>2]
let bad=filtered.assign({value=df["value"]})
wait(1s)
"#,
    ]
    .concat();
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", script);
    let error = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("different DataFrame row plans"),
        "{error}"
    );
}

#[test]
fn column_arithmetic_handles_missing_values_strings_and_nonintegral_remainders() {
    prepare(
        r#"type Entry={value:Option<Number>,name:String}
let entries:List<Entry>=[{value=Some(5.5),name="first"},{value=None,name="missing"}]
let df=DataFrame(entries)
let assigned=df.assign(modulo=df["value"]%2,label=df["name"]+"!").to_list()
let kept=df[df["value"]>2].to_list()
assert(len(kept)==1)
for row in assigned {
    if row.name=="first" { assert(unwrap(row.modulo,0)==1.5); assert(row.label=="first!") }
    else { assert(row.modulo==None) }
}
wait(1s)
"#,
    );
}
#[test]
fn different_queries_reuse_imported_sqlite_tables_and_reloads_follow_data_versions() {
    let mut sources = MemorySources::default();
    sources.insert("samples.csv", "kind,value\na,2\nb,4\n");
    let make = |count| {
        format!(
            r#"type Entry={{kind:String,value:Number}}
let df=DataFrame(read<List<Entry>>("samples.csv"))
let first=df.head({count}).to_list()
let last=df.tail({count}).to_list()
assert(len(first)=={count})
wait(1s)
"#
        )
    };
    sources.insert("main.sanim", make(1));
    let mut engine = Engine::default();
    let first = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(first.statistics.executed_queries, 2);
    assert_eq!(first.statistics.reused_databases, 1);
    sources.insert("main.sanim", make(2));
    let changed = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(changed.statistics.executed_queries, 2);
    assert_eq!(changed.statistics.reused_databases, 2);
    sources.insert("samples.csv", "kind,value\nc,9\nd,10\n");
    let refreshed = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(refreshed.statistics.executed_queries, 2);
    assert_eq!(refreshed.statistics.reused_databases, 1);
    let sanim_core::Value::Record(row) = &refreshed.bindings["first"].list().unwrap()[0] else {
        panic!("record")
    };
    assert_eq!(row["kind"].string().unwrap(), "c");
}
#[test]
fn data_materialization_inside_loops_has_actionable_static_performance_advice() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        [
            INPUT,
            r#"
repeat 3 { let _values=df.head(2).to_list() }
wait(1s)
"#,
        ]
        .concat(),
    );
    let report = Engine::default().lint("main.sanim", &sources).unwrap();
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "P101"
            && diagnostic
                .message
                .contains("DataFrame/Series materialization")
    }));
}

#[test]
fn sqlite_vm_work_counts_toward_cumulative_preparation_budget_but_hits_do_not_execute() {
    let script = [
        INPUT,
        r#"
for count in range(12) { let _batch=df.head(count).to_list() }
wait(1s)
"#,
    ]
    .concat();
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", script);
    let prepared = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.statistics.executed_queries, 12);
    assert!(prepared.statistics.sql_instructions > 0);
    let mut engine = Engine::default();
    engine.limits.max_operations =
        prepared.statistics.operations - prepared.statistics.sql_instructions / 2;
    let error = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S303");
    assert!(error.message.contains("preparation exceeded"));
    sources.insert(
        "main.sanim",
        [
            INPUT,
            r#"
repeat 12 { let _batch=df.head(2).to_list() }
wait(1s)
"#,
        ]
        .concat(),
    );
    let hit = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(hit.statistics.executed_queries, 1);
    assert_eq!(hit.statistics.reused_queries, 11);
    assert!(hit.statistics.sql_instructions < prepared.statistics.sql_instructions);
}

#[test]
#[ignore]
fn dataframe_hot_reload_benchmark() {
    use std::{hint::black_box, time::Instant};
    let mut sources = MemorySources::default();
    let mut csv = String::from("key,value\n");
    for value in 0..20_000 {
        use std::fmt::Write;
        writeln!(csv, "{},{}", value % 5, value).unwrap();
    }
    sources.insert("samples.csv", csv);
    let script = |count| {
        format!(
            r#"type Entry={{key:Number,value:Number}}
let df=DataFrame(read<List<Entry>>("samples.csv"))
let rows=df.head({count}).to_list()
wait(1s)
"#
        )
    };
    let mut engine = Engine::default();
    sources.insert("main.sanim", script(100));
    engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    let start = Instant::now();
    for count in 101..111 {
        sources.insert("main.sanim", script(count));
        let updated = engine
            .prepare("main.sanim", &sources, &BTreeMap::new())
            .unwrap();
        assert!(updated.statistics.restored_statements > 0);
        assert_eq!(updated.statistics.decoded_assets, 0);
        assert_eq!(updated.statistics.reused_databases, 1);
        black_box(updated);
    }
    let hot = start.elapsed();
    let start = Instant::now();
    for count in 101..111 {
        sources.insert("main.sanim", script(count));
        black_box(
            Engine::default()
                .prepare("main.sanim", &sources, &BTreeMap::new())
                .unwrap(),
        );
    }
    let cold = start.elapsed();
    eprintln!(
        "DataFrame 20k source rows, 10 changed head queries: checkpoint + SQLite import reuse {hot:?}, fresh engines {cold:?}; speedup {:.2}x",
        cold.as_secs_f64() / hot.as_secs_f64()
    );
    assert!(
        hot < cold,
        "incremental queries must beat fresh preparation"
    );
}

#[test]
fn cheap_frame_reductions_and_materializing_series_have_distinct_performance_metadata() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "let df=DataFrame([{kind=\"a\",value=1}])\nlet plan=df.groupby(\"kind\").sum()\nlet result=df[\"value\"].sum()\nrepeat 2 {\nlet cheap=df.groupby(\"kind\").sum()\nlet expensive=df[\"value\"].sum()\n}\nwait(1s)\n");
    let report = Engine::default().lint("main.sanim", &sources).unwrap();
    let expensive_loop_calls = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "P101")
        .collect::<Vec<_>>();
    assert_eq!(expensive_loop_calls.len(), 1);
    assert_eq!(expensive_loop_calls[0].span.line, 6);
    let mut engine = Engine::default();
    engine.preparation = Preparation::Demand;
    let prepared = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert!(prepared.bindings.contains_key("plan"));
    assert!(!prepared.bindings.contains_key("result"));
    assert_eq!(prepared.statistics.executed_queries, 0);
}

#[test]
fn generic_frame_and_series_helpers_preserve_rows_nullability_and_units() {
    prepare(
        r#"
fn first<T>(data:DataFrame<T>):List<T> { return data.head(1).to_list() }
fn last<T>(data:DataFrame<T>):List<T> { return data.tail(1).to_list() }
fn values<T>(column:Series<T>):List<T> { return column.to_list() }
fn distances<T:{distance:Length}>(data:DataFrame<T>):List<Length> { return data["distance"].to_list() }
type Entry={name:String,amount:Option<Length>}
let rows:List<Entry>=[{name="first",amount=Some(12pt)},{name="last",amount=None}]
let data=DataFrame(rows)
let leading=first(data)
let trailing=last(data)
let amounts=values(data["amount"])
assert(unwrap(leading[0],{name="",amount=None}).name=="first")
assert(unwrap(unwrap(leading[0],{name="",amount=None}).amount,0pt)==12pt)
assert(unwrap(trailing[0],{name="",amount=None}).name=="last")
assert(unwrap(amounts[1],Some(0pt))==None)
let measured=distances(DataFrame([{distance=4pt,label="kept"}]))
assert(unwrap(measured[0],0pt)==4pt)
wait(1s)
"#,
    );
}
