use sanim_core::{Fraction, Type, Unit, Value};
use sanim_data::{Limits, decode};
use std::{
    collections::BTreeMap,
    io::{Cursor, Write},
};

fn number() -> Type {
    Type::Numeric(Unit::Number)
}
fn table() -> Type {
    Type::Table(Box::new(Type::Record(BTreeMap::from([
        ("id".into(), Type::String),
        ("value".into(), number()),
    ]))))
}
#[test]
fn csv_validates_columns_and_numbers() {
    let value = decode(
        "data.csv",
        b"id,value\na,1.25\nb,-2\n",
        &table(),
        Limits::default(),
    )
    .unwrap();
    let rows = value.list().unwrap();
    assert_eq!(rows.len(), 2);
    let Value::Record(first) = &rows[0] else {
        panic!()
    };
    assert_eq!(
        first["value"].number().unwrap().value,
        Fraction::new(5u64, 4u64)
    );
}
#[test]
fn csv_quoted_fields_include_newlines_and_quotes() {
    let value = decode(
        "data.csv",
        b"id,value\r\n\"a,\"\"b\"\"\nline\",2\r\n",
        &table(),
        Limits::default(),
    )
    .unwrap();
    let Value::Record(row) = &value.list().unwrap()[0] else {
        panic!()
    };
    assert_eq!(row["id"], Value::String("a,\"b\"\nline".into()));
}
#[test]
fn csv_duplicate_header_fails() {
    assert!(
        decode("a.csv", b"id,id\na,b\n", &table(), Limits::default())
            .unwrap_err()
            .contains("duplicate")
    );
}
#[test]
fn csv_missing_header_field_fails() {
    assert!(
        decode("a.csv", b"id\na\n", &table(), Limits::default())
            .unwrap_err()
            .contains("missing CSV column 'value'")
    );
}
#[test]
fn csv_unknown_column_fails() {
    assert!(
        decode(
            "a.csv",
            b"id,value,extra\na,1,2\n",
            &table(),
            Limits::default()
        )
        .unwrap_err()
        .contains("unknown CSV column")
    );
}
#[test]
fn csv_unclosed_quote_fails() {
    assert!(
        decode(
            "a.csv",
            b"id,value\n\"oops,1\n",
            &table(),
            Limits::default()
        )
        .is_err()
    );
}
#[test]
fn csv_bad_number_has_field_path() {
    assert!(decode("a.csv", b"id,value\na,nope\n", &table(), Limits::default()).is_err());
}
#[test]
fn csv_ragged_row_fails() {
    assert!(
        decode("a.csv", b"id,value\na\n", &table(), Limits::default())
            .unwrap_err()
            .contains("row 2")
    );
}
#[test]
fn json_schema_rejects_extra_fields() {
    let ty = Type::Record(BTreeMap::from([("name".into(), Type::String)]));
    assert!(
        decode(
            "a.json",
            br#"{"name":"a","extra":1}"#,
            &ty,
            Limits::default()
        )
        .unwrap_err()
        .contains("unknown field")
    );
}
#[test]
fn json_optional_values() {
    let ty = Type::List(Box::new(Type::Option(Box::new(number()))));
    let value = decode("a.json", b"[null,2]", &ty, Limits::default()).unwrap();
    assert_eq!(value.list().unwrap()[0], Value::None);
}
#[test]
fn json_exponent_is_exact() {
    let value = decode("a.json", b"1.25e2", &number(), Limits::default()).unwrap();
    assert_eq!(value.number().unwrap().value, Fraction::from(125));
}
#[test]
fn toml_nested_schema() {
    let ty = Type::Record(BTreeMap::from([(
        "theme".into(),
        Type::Record(BTreeMap::from([("tint".into(), Type::Color)])),
    )]));
    assert!(matches!(
        decode(
            "theme.toml",
            b"[theme]\ntint = '#FF0000'\n",
            &ty,
            Limits::default()
        )
        .unwrap(),
        Value::Record(_)
    ));
}
#[test]
fn byte_limit_is_checked_before_parse() {
    let limits = Limits {
        max_bytes: 2,
        ..Limits::default()
    };
    assert!(
        decode("a.json", b"[1,2]", &Type::List(Box::new(number())), limits)
            .unwrap_err()
            .contains("byte limit")
    );
}
#[test]
fn decoded_value_limit_is_enforced() {
    let limits = Limits {
        max_values: 2,
        ..Limits::default()
    };
    assert!(decode("a.json", b"[1,2]", &Type::List(Box::new(number())), limits).is_err());
}
#[test]
fn nesting_limit_is_enforced() {
    let limits = Limits {
        max_depth: 0,
        ..Limits::default()
    };
    assert!(decode("a.json", b"[1]", &Type::List(Box::new(number())), limits).is_err());
}
#[test]
fn unsupported_format_fails() {
    assert!(
        decode("a.xlsx", b"anything", &number(), Limits::default())
            .unwrap_err()
            .contains("unsupported")
    );
}

fn npy(dtype: &str, shape: &str, payload: &[u8]) -> Vec<u8> {
    let header = format!("{{'descr': '{dtype}', 'fortran_order': False, 'shape': ({shape}), }}\n");
    let mut out = b"\x93NUMPY\x01\x00".to_vec();
    out.extend((header.len() as u16).to_le_bytes());
    out.extend(header.bytes());
    out.extend(payload);
    out
}
#[test]
fn npy_little_endian_integer_array() {
    let bytes = npy(
        "<i4",
        "2,",
        &[1i32.to_le_bytes(), (-2i32).to_le_bytes()].concat(),
    );
    let value = decode(
        "a.npy",
        &bytes,
        &Type::Array(Box::new(number())),
        Limits::default(),
    )
    .unwrap();
    let Value::Array { shape, values, .. } = value else {
        panic!()
    };
    assert_eq!(shape, vec![2]);
    assert_eq!(values[1].number().unwrap().value, Fraction::from(-2));
}
#[test]
fn npy_big_endian_float_array() {
    let bytes = npy(">f8", "1,", &1.5f64.to_be_bytes());
    let value = decode(
        "a.npy",
        &bytes,
        &Type::Array(Box::new(number())),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        value.list().unwrap()[0].number().unwrap().value,
        Fraction::new(3u64, 2u64)
    );
}
#[test]
fn npy_rejects_nan() {
    let bytes = npy("<f8", "1,", &f64::NAN.to_le_bytes());
    assert!(
        decode(
            "a.npy",
            &bytes,
            &Type::Array(Box::new(number())),
            Limits::default()
        )
        .unwrap_err()
        .contains("non-finite")
    );
}
#[test]
fn npy_rejects_object_arrays() {
    let bytes = npy("|O8", "1,", &[0; 8]);
    assert!(
        decode(
            "a.npy",
            &bytes,
            &Type::Array(Box::new(number())),
            Limits::default()
        )
        .unwrap_err()
        .contains("unsupported")
    );
}
#[test]
fn npy_shape_must_match_payload() {
    let bytes = npy("<i4", "2,", &[0; 4]);
    assert!(
        decode(
            "a.npy",
            &bytes,
            &Type::Array(Box::new(number())),
            Limits::default()
        )
        .unwrap_err()
        .contains("length")
    );
}
#[test]
fn npy_truncation_is_always_an_error() {
    let valid = npy("<i4", "1,", &1i32.to_le_bytes());
    for end in 0..valid.len() {
        assert!(
            decode(
                "a.npy",
                &valid[..end],
                &Type::Array(Box::new(number())),
                Limits::default()
            )
            .is_err(),
            "truncated at {end}"
        );
    }
}
#[test]
fn npz_decodes_typed_named_arrays() {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("points.npy", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&npy("<i4", "1,", &7i32.to_le_bytes()))
        .unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let ty = Type::Record(BTreeMap::from([(
        "points".into(),
        Type::Array(Box::new(number())),
    )]));
    assert!(matches!(
        decode("a.npz", &bytes, &ty, Limits::default()).unwrap(),
        Value::Record(_)
    ));
}

#[test]
fn json_decodes_fraction_vectors_and_matrices() {
    let limits = Limits::default();
    assert_eq!(
        decode("a.json", br#""1/3""#, &Type::Fraction, limits).unwrap(),
        Value::Fraction(Fraction::new(1u64, 3u64))
    );
    let v = decode("a.json", b"[1,2,3]", &Type::Vector(3, None), limits).unwrap();
    assert_eq!(v.vector().unwrap().0.len(), 3);
    let m = decode(
        "a.json",
        b"[[1,0,0,0],[0,1,0,0],[0,0,1,0],[0,0,0,1]]",
        &Type::Matrix4,
        limits,
    )
    .unwrap();
    assert!(matches!(m, Value::Matrix4(_)));
}
#[test]
fn npy_decodes_vectors_from_trailing_component_axis() {
    let payload = [1i32, 2, 3, 4, 5, 6]
        .into_iter()
        .flat_map(i32::to_le_bytes)
        .collect::<Vec<_>>();
    let bytes = npy("<i4", "2,3", &payload);
    let ty = Type::Array(Box::new(Type::Vector(3, None)));
    let value = decode("a.npy", &bytes, &ty, Limits::default()).unwrap();
    let Value::Array { shape, values, .. } = value else {
        panic!()
    };
    assert_eq!(shape, vec![2]);
    assert_eq!(values[1].vector().unwrap().0[2], 6.0);
}
#[test]
fn npy_decodes_matrix_shape() {
    let matrix = sanim_core::math::linear::identity();
    let payload = matrix
        .iter()
        .flatten()
        .flat_map(|v| (*v as i32).to_le_bytes())
        .collect::<Vec<_>>();
    let bytes = npy("<i4", "4,4", &payload);
    assert!(matches!(
        decode("a.npy", &bytes, &Type::Matrix4, Limits::default()).unwrap(),
        Value::Matrix4(_)
    ));
}

#[test]
fn yaml_aliases_respect_expanded_byte_budget() {
    let text = format!("a: &text {}\nb: [*text, *text, *text]\n", "x".repeat(100));
    let limits = Limits {
        max_bytes: 300,
        ..Limits::default()
    };
    let e = decode("data.yaml", text.as_bytes(), &Type::Dict, limits).unwrap_err();
    assert!(e.contains("expanded byte"), "{e}");
}
#[test]
fn yaml_duplicate_keys_and_multiple_documents_fail() {
    for text in ["a: 1\na: 2\n", "---\na: 1\n---\nb: 2\n"] {
        assert!(decode("data.yaml", text.as_bytes(), &Type::Dict, Limits::default()).is_err());
    }
}
#[test]
fn json_and_npy_vectors_retain_f64_range() {
    let vector = decode(
        "data.json",
        b"[1e100,1e-100]",
        &Type::Vector(2, None),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(vector.vector().unwrap().0, &[1e100, 1e-100]);
    let payload = [1e100_f64, 1e-100]
        .into_iter()
        .flat_map(f64::to_le_bytes)
        .collect::<Vec<_>>();
    let vector = decode(
        "data.npy",
        &npy("<f8", "2", &payload),
        &Type::Vector(2, None),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(vector.vector().unwrap().0, &[1e100, 1e-100]);
}
#[test]
fn sql_instruction_budget_interrupts_recursive_queries() {
    let tables = Value::Record(BTreeMap::new());
    let ty = Type::Table(Box::new(Type::Dict));
    let limits = Limits {
        max_sql_steps: 1000,
        ..Limits::default()
    };
    let e=sanim_data::sqlite::query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT SUM(x) AS total FROM n",&tables,None,&ty,limits).unwrap_err();
    assert!(e.contains("instruction budget"), "{e}");
}
#[test]
fn sql_rejects_multiple_statements() {
    let e = sanim_data::sqlite::query(
        "SELECT 1 AS value; SELECT 2 AS value",
        &Value::Record(BTreeMap::new()),
        None,
        &Type::Table(Box::new(Type::Dict)),
        Limits::default(),
    )
    .unwrap_err();
    assert!(e.to_ascii_lowercase().contains("multiple"), "{e}");
}
