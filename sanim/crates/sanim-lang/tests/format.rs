use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn str_defaults_and_explicit_formats_share_the_bounded_native_formatter() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "let default_text=str(0.5)\nlet decimal=str(0.5,format=\"\")\nlet binary=str(42,format=\"08b\")\nlet fraction=str(0.125,format=\".1%\")\nlet quoted=str(\"é\",format=\"\",conversion=\"a\")");
    let prepared = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    for (name, expected) in [
        ("default_text", "0.5"),
        ("decimal", "0.5"),
        ("binary", "00101010"),
        ("fraction", "12.5%"),
        ("quoted", "'\\xe9'"),
    ] {
        assert_eq!(prepared.bindings[name].string().unwrap(), expected);
    }
}

#[test]
fn bad_format_diagnostics_point_to_the_source_call() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "wait(1s)\nlet bad=str(0.5,format=\"b\")");
    let error = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.span.line, 2);
    assert!(error.message.contains("integral"), "{error}");
}

#[test]
fn lint_rejects_static_format_errors_without_reading_data() {
    struct Source;
    impl sanim_lang::Sources for Source {
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            assert_eq!(path, "main.sanim", "lint must never open a data resource");
            Ok(b"let rows=read<List<Number>>(\"missing.json\")\nlet label=str(\"text\",format=\".2f\")".to_vec())
        }
    }
    let error = Engine::default().lint("main.sanim", &Source).unwrap_err();
    assert_eq!(error.code, "S106");
    assert_eq!(error.span.line, 2);
    assert_eq!(error.source, "main.sanim");
    assert!(error.message.contains("String"), "{error}");
}

#[test]
fn dynamic_formats_keep_runtime_validation_and_known_bad_conversions_fail_during_lint() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "let spec=reflect(\".2f\")\nlet label=str(\"text\",format=spec)",
    );
    assert!(Engine::default().lint("main.sanim", &sources).is_ok());
    let error = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert!(error.message.contains("String"), "{error}");
    sources.insert(
        "main.sanim",
        "let spec=reflect(\".2f\")\nlet label=str(42,format=spec,conversion=\"invalid\")",
    );
    assert_eq!(
        Engine::default()
            .lint("main.sanim", &sources)
            .unwrap_err()
            .code,
        "S106"
    );
}
