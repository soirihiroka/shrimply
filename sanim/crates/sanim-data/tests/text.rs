use sanim_core::{Type, Value};
use sanim_data::{Limits, decode};

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
