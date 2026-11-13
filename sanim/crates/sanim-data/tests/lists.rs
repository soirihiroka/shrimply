use sanim_core::{List, Type, Unit, Value};
use sanim_data::{Limits, decode, sqlite, to_json};
use std::collections::BTreeMap;

fn number() -> Type {
    Type::Numeric(Unit::Number)
}
fn rows() -> Type {
    Type::List(Box::new(Type::Record(BTreeMap::from([
        ("id".into(), Type::String),
        ("value".into(), number()),
    ]))))
}
fn bindings(name: &str, value: Value) -> Value {
    Value::Record([(name.to_owned(), value)].into_iter().collect())
}

#[test]
#[ignore = "microbenchmark: numerical array import into shared collection storage"]
fn collection_storage_benchmark() {
    const VALUES: u32 = 100_000;
    const RUNS: u32 = 3;
    let mut header = format!("{{'descr': '<f8', 'fortran_order': False, 'shape': ({VALUES},), }}");
    while (10 + header.len() + 1) % 16 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
    bytes.extend_from_slice(&u16::try_from(header.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(header.as_bytes());
    for n in 0..VALUES {
        bytes.extend_from_slice(&f64::from(n).to_le_bytes());
    }
    let ty = Type::Array(Box::new(number()));
    let started = std::time::Instant::now();
    for _ in 0..RUNS {
        let decoded = decode("numbers.npy", &bytes, &ty, Limits::default()).unwrap();
        let Value::Array { shape, values, .. } = decoded else {
            panic!("array")
        };
        assert_eq!(shape, vec![VALUES as usize]);
        assert_eq!(values.len(), VALUES as usize);
        assert_eq!(
            values[VALUES as usize - 1].number().unwrap().value,
            (VALUES - 1).into()
        );
        std::hint::black_box(values);
    }
    eprintln!(
        "NPY shared collection import: {} bytes / {VALUES} f64 values, {}us/import ({RUNS} runs)",
        bytes.len(),
        started.elapsed().as_micros() / u128::from(RUNS)
    );
}
fn query(sql: &str, inputs: &Value, result: &Type) -> Value {
    sqlite::query(sql, inputs, None, result, Limits::default()).unwrap()
}

#[test]
fn header_only_csv_is_empty_and_its_declared_columns_remain_queryable() {
    let input = decode("empty.csv", b"id,value\n", &rows(), Limits::default()).unwrap();
    assert_eq!(input.ty(), rows());
    assert_eq!(input.list().unwrap().len(), 0);
    let result = query(
        "SELECT COUNT(*) FROM samples",
        &bindings("samples", input),
        &Type::List(Box::new(number())),
    );
    assert_eq!(to_json(&result).unwrap(), serde_json::json!([0]));
    assert_eq!(result.ty(), Type::List(Box::new(number())));
}

#[test]
fn empty_json_yaml_and_toml_lists_keep_the_full_record_schema() {
    for (path, bytes) in [
        ("empty.json", b"[]".as_slice()),
        ("empty.yaml", b"[]".as_slice()),
    ] {
        let value = decode(path, bytes, &rows(), Limits::default()).unwrap();
        assert_eq!(value.ty(), rows());
        assert!(value.list().unwrap().is_empty());
    }
    let schema = Type::Record(BTreeMap::from([("samples".into(), rows())]));
    let Value::Record(fields) =
        decode("empty.toml", b"samples = []\n", &schema, Limits::default()).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(fields["samples"].ty(), rows());
    assert!(fields["samples"].list().unwrap().is_empty());
}

#[test]
fn sqlite_accepts_scalar_lists_and_can_query_its_dictionary_results() {
    let input = decode(
        "numbers.json",
        b"[1,2]",
        &Type::List(Box::new(number())),
        Limits::default(),
    )
    .unwrap();
    let sql_input = bindings("numbers", input);
    let dictionaries = query(
        "SELECT value AS n FROM numbers ORDER BY value",
        &sql_input,
        &Type::List(Box::new(Type::Dict)),
    );
    assert_eq!(
        to_json(&dictionaries).unwrap(),
        serde_json::json!([{"n":1},{"n":2}])
    );
    let total = query(
        "SELECT SUM(n) FROM previous",
        &bindings("previous", dictionaries),
        &Type::List(Box::new(number())),
    );
    assert_eq!(to_json(&total).unwrap(), serde_json::json!([3]));
    let empty = Value::List(List::typed(number(), vec![]));
    let count = query(
        "SELECT COUNT(*) FROM numbers",
        &bindings("numbers", empty),
        &Type::List(Box::new(number())),
    );
    assert_eq!(to_json(&count).unwrap(), serde_json::json!([0]));
}

#[test]
fn dictionary_sql_columns_use_the_actual_union_of_scalar_keys() {
    let input = decode(
        "rows.json",
        br#"[{"x":1},{"x":2,"label":"ok"}]"#,
        &Type::List(Box::new(Type::Dict)),
        Limits::default(),
    )
    .unwrap();
    let result = query(
        "SELECT SUM(x) AS total, COUNT(label) AS labeled FROM samples",
        &bindings("samples", input),
        &Type::List(Box::new(Type::Dict)),
    );
    assert_eq!(
        to_json(&result).unwrap(),
        serde_json::json!([{"total":3,"labeled":1}])
    );
    let empty = Value::List(List::typed(Type::Dict, vec![]));
    assert_eq!(empty.list().unwrap().len(), 0);
    let error = sqlite::query(
        "SELECT COUNT(*) FROM samples",
        &bindings("samples", empty),
        None,
        &Type::List(Box::new(Type::Dict)),
        Limits::default(),
    )
    .unwrap_err();
    assert!(error.contains("no column schema"), "{error}");
    assert!(error.contains("List<Row>"), "{error}");
}

#[test]
fn nested_dictionary_fields_fail_with_the_input_row_and_column() {
    let input = decode(
        "rows.json",
        br#"[{"x":{"nested":1}}]"#,
        &Type::List(Box::new(Type::Dict)),
        Limits::default(),
    )
    .unwrap();
    let error = sqlite::query(
        "SELECT x FROM samples",
        &bindings("samples", input),
        None,
        &Type::List(Box::new(Type::Dict)),
        Limits::default(),
    )
    .unwrap_err();
    assert!(error.contains("samples"), "{error}");
    assert!(error.contains("row 0, column 'x'"), "{error}");
    assert!(error.contains("scalar"), "{error}");
}

#[test]
#[ignore = "microbenchmark: actual in-memory SQLite list registration and query"]
fn sqlite_list_benchmark() {
    let count = 10_000_u32;
    let input = Value::List(List::typed(
        number(),
        (0..count)
            .map(|value| Value::Number(sanim_core::Number::scalar(i64::from(value))))
            .collect(),
    ));
    let inputs = bindings("samples", input);
    let result_ty = Type::List(Box::new(number()));
    let repeats = 10;
    let start = std::time::Instant::now();
    for _ in 0..repeats {
        let total = query("SELECT SUM(value) FROM samples", &inputs, &result_ty);
        assert_eq!(
            total.list().unwrap()[0].number().unwrap().value,
            sanim_core::Fraction::from(u64::from(count) * (u64::from(count) - 1) / 2)
        );
    }
    eprintln!(
        "actual in-memory SQLite registration+SUM over {count} List items: {:.1} us/query ({repeats} queries)",
        start.elapsed().as_micros() as f64 / f64::from(repeats)
    );
}
