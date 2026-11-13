use sanim_core::{Type, Value};
use sanim_data::{Limits, decode};

#[test]
fn string_literal_reads_share_verbatim_text_rules_and_validate_allowed_values() {
    let before = Type::StringLiteral("before".into());
    let phase = Type::union([before.clone(), Type::StringLiteral("after".into())]).unwrap();
    for schema in [&before, &phase] {
        assert_eq!(
            decode("phase.txt", b"before", schema, Limits::default()).unwrap(),
            Value::String("before".into())
        );
        let error = decode("phase.txt", b"later", schema, Limits::default()).unwrap_err();
        assert!(
            error.contains("phase.txt") && error.contains("before"),
            "{error}"
        );
    }
    assert!(decode("phase.txt", b"after", &phase, Limits::default()).is_ok());
    assert!(
        decode("phase.txt", b"before\n", &phase, Limits::default()).is_err(),
        "literal reading must not silently trim source"
    );
    assert!(
        decode("phase.txt", &[0xff], &phase, Limits::default())
            .unwrap_err()
            .contains("UTF-8")
    );
}

#[test]
fn explicit_string_reads_verbatim_utf8_independent_of_extension() {
    let text = "Hello, 世界\n{braces} and \\slashes\n";
    for path in ["notes.txt", "README.md", "document", "raw.json"] {
        assert_eq!(
            decode(path, text.as_bytes(), &Type::String, Limits::default()).unwrap(),
            Value::String(text.into())
        );
    }
    assert!(
        decode("broken.txt", &[0xff], &Type::String, Limits::default())
            .unwrap_err()
            .contains("UTF-8")
    );
    assert!(
        decode(
            "large.txt",
            b"large",
            &Type::String,
            Limits {
                max_bytes: 2,
                ..Limits::default()
            }
        )
        .is_err()
    );
    assert!(
        decode(
            "text.txt",
            b"one value",
            &Type::String,
            Limits {
                max_values: 0,
                ..Limits::default()
            }
        )
        .is_err()
    );
}
