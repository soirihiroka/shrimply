use sanim_core::{
    List, Number, Type, Unit, Value,
    dataframe::{Frame, Series},
};
use sanim_data::{Limits, dataframe, sqlite};
use std::{collections::BTreeMap, sync::Arc};
fn number(value: i64) -> Value {
    Value::Number(Number::scalar(value))
}
fn rows(count: usize) -> List {
    let schema = Type::Record(BTreeMap::<String, _>::from([
        ("key".into(), Type::Numeric(Unit::Number)),
        ("value".into(), Type::Numeric(Unit::Number)),
    ]));
    List::typed(
        schema,
        (0..count)
            .map(|index| {
                Value::Record(
                    BTreeMap::<String, _>::from([
                        ("key".into(), number((index % 5) as i64)),
                        ("value".into(), number(index as i64)),
                    ])
                    .into(),
                )
            })
            .collect(),
    )
}
#[test]
fn parameterized_column_plans_run_on_actual_sqlite() {
    let frame = Frame::new(rows(20)).unwrap();
    let mask = Series::binary(
        ">",
        Value::Series(Arc::new(frame.column("value").unwrap())),
        number(9),
    )
    .unwrap();
    let filtered = frame.filter(&mask).unwrap();
    let grouped = filtered
        .groupby(vec!["key".into()])
        .unwrap()
        .aggregate(&BTreeMap::<String, _>::from([(
            "value".into(),
            "sum".into(),
        )]))
        .unwrap();
    let query = dataframe::compile(&grouped, None, Limits::default()).unwrap();
    assert!(query.sql.contains("GROUP BY"));
    assert!(query.sql.contains(":p0"));
    let result = dataframe::collect(&grouped, None, Limits::default()).unwrap();
    assert_eq!(result.list().unwrap().len(), 5);
    let Value::Record(first) = &result.list().unwrap()[0] else {
        panic!("record")
    };
    assert_eq!(first["value"], number(25));
}
#[test]
fn plan_composition_preserves_shared_cow_source_without_materializing() {
    let original = rows(1000);
    let frame = Frame::new(original.clone()).unwrap();
    let projected = frame
        .select(&["value".into()])
        .unwrap()
        .limit(5, false)
        .unwrap();
    let query = dataframe::compile(&projected, None, Limits::default()).unwrap();
    let Value::Record(tables) = query.tables else {
        panic!("tables")
    };
    let Value::List(source) = &tables["input_0"] else {
        panic!("list")
    };
    assert!(source.shares_values(&original));
}
#[test]
fn quoted_identifiers_and_string_values_cannot_inject_sql() {
    let column = "x\";DROP TABLE data;--";
    let row = Value::Record(
        BTreeMap::<String, _>::from([(column.into(), Value::String("a' OR 1=1 --".into()))]).into(),
    );
    let frame = Frame::new(vec![row].into()).unwrap();
    let column = frame.column(column).unwrap();
    let mask = Series::binary(
        "==",
        Value::Series(Arc::new(column)),
        Value::String("different".into()),
    )
    .unwrap();
    let result =
        dataframe::collect(&frame.filter(&mask).unwrap(), None, Limits::default()).unwrap();
    assert_eq!(result.list().unwrap().len(), 0);
}
#[test]
fn reused_database_honors_exact_budgets_and_input_versions() {
    let frame = Frame::new(rows(40)).unwrap();
    let query = dataframe::compile(&frame, None, Limits::default()).unwrap();
    let mut cache = sqlite::Databases::default();
    cache.configure(1_000_000, 10);
    let request = || sqlite::Request {
        sql: &query.sql,
        tables: &query.tables,
        parameters: Some(&query.parameters),
        result: &query.result,
        limits: Limits::default(),
    };
    assert!(
        !cache
            .query(query.input_fingerprint.clone(), request())
            .unwrap()
            .reused_database
    );
    assert!(
        cache
            .query(query.input_fingerprint.clone(), request())
            .unwrap()
            .reused_database
    );
    let limits = Limits {
        max_sql_steps: 1,
        ..Limits::default()
    };
    let error = cache
        .query(
            query.input_fingerprint.clone(),
            sqlite::Request {
                limits,
                ..request()
            },
        )
        .unwrap_err();
    assert!(error.contains("different execution limits"));
    assert!(dataframe::collect(&frame, None, limits).is_err());
}
#[test]
fn deeply_chained_plans_fail_cleanly_before_unbounded_sql() {
    let mut frame = Frame::new(rows(3)).unwrap();
    loop {
        match frame.limit(3, false) {
            Ok(next) => frame = next,
            Err(error) => {
                assert!(error.contains("query plan exceeds"));
                break;
            }
        }
    }
}
#[test]
#[ignore]
fn dataframe_benchmark() {
    use std::{hint::black_box, time::Instant};
    let limits = Limits {
        max_sql_steps: 50_000_000,
        max_values: 2_000_000,
        ..Limits::default()
    };
    let source = rows(20_000);
    let frame = Frame::new(source).unwrap();
    let start = Instant::now();
    for _ in 0..1000 {
        let mut chain = frame.clone();
        for _ in 0..20 {
            chain = chain.limit(10_000, false).unwrap()
        }
        black_box(chain);
    }
    let compose = start.elapsed();
    let q = dataframe::compile(&frame, None, limits).unwrap();
    let request = || sqlite::Request {
        sql: &q.sql,
        tables: &q.tables,
        parameters: Some(&q.parameters),
        result: &q.result,
        limits,
    };
    let start = Instant::now();
    for _ in 0..10 {
        black_box(
            sqlite::query(&q.sql, &q.tables, Some(&q.parameters), &q.result, limits).unwrap(),
        );
    }
    let cold = start.elapsed();
    let mut cache = sqlite::Databases::default();
    cache.configure(limits.max_bytes, 10);
    cache.query(q.input_fingerprint.clone(), request()).unwrap();
    let start = Instant::now();
    for _ in 0..10 {
        assert!(
            black_box(cache.query(q.input_fingerprint.clone(), request()).unwrap()).reused_database
        )
    }
    let warm = start.elapsed();
    eprintln!(
        "DataFrame 20k rows: 1000 x 20-node lazy chains {compose:?}; 10 SQLite executions with fresh imports {cold:?}, reused imports {warm:?}; speedup {:.2}x",
        cold.as_secs_f64() / warm.as_secs_f64()
    );
    assert!(
        warm < cold,
        "SQLite snapshot reuse must improve execution time"
    );
}
