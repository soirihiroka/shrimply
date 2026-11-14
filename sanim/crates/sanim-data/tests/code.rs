use sanim_core::{
    Type, Value,
    code::{Content, Document, Language},
};
use sanim_data::{Limits, decode};

#[test]
fn declaration_names_and_exact_syntax_bytes_come_from_native_grammars() {
    for (path, source, expected) in [
        ("a.c", "int *factory() {\n return 0;\n}\n", "factory"),
        (
            "a.cpp",
            "int ns::push(int x) {\n return x;\n}\n",
            "ns::push",
        ),
        (
            "a.rs",
            "struct Queue;\nimpl Queue {\n fn push() {\n }\n}\n",
            "Queue",
        ),
        ("a.js", "const push = () => {\n return 1;\n};\n", "push"),
    ] {
        let document =
            sanim_data::code::parse(Document::read(path, source, Language::Auto).unwrap()).unwrap();
        let scope = document
            .scopes
            .iter()
            .find(|scope| scope.label.ends_with(&format!(":{expected}")))
            .unwrap_or_else(|| panic!("{path}: {:?}", document.scopes));
        assert!(scope.start_byte < scope.end_byte && scope.end_byte <= source.len());
        assert!(
            source.is_char_boundary(scope.start_byte) && source.is_char_boundary(scope.end_byte)
        );
    }
}

#[test]
fn every_bundled_grammar_has_real_highlights_and_folds() {
    for (path, source) in [
        ("a.py", "def main():\n    return 1\n"),
        ("a.rs", "fn main() {\n let x = 1;\n}\n"),
        ("a.js", "function main() {\n return 1;\n}\n"),
        ("a.ts", "function main(): number {\n return 1;\n}\n"),
        ("a.tsx", "function Main() {\n return <div>hi</div>;\n}\n"),
        ("a.c", "int main() {\n return 1;\n}\n"),
        ("a.cpp", "class Main {\n int value;\n};\n"),
        ("a.json", "{\n \"value\": 1\n}\n"),
        ("a.sh", "main() {\n echo ok\n}\n"),
        ("a.toml", "[table]\nvalue = 1\n"),
        ("a.yaml", "object:\n  value: 1\n"),
        ("a.sql", "SELECT\n  1;\n"),
    ] {
        let Value::Code(document) = decode(
            path,
            source.as_bytes(),
            &Type::Code(Language::Auto),
            Limits::default(),
        )
        .unwrap_or_else(|error| panic!("{path}: {error}")) else {
            panic!("Code")
        };
        assert!(document.syntax_parsed, "{path}");
        assert!(
            document.diagnostics.is_empty(),
            "{path}: {:?}",
            document.diagnostics
        );
        assert!(!document.highlights.is_empty(), "{path}");
        assert!(!document.scopes.is_empty(), "{path}");
    }
}
#[test]
fn patches_are_typed_change_documents_and_bad_source_is_safe_to_display() {
    let Value::Code(patch) = decode(
        "a.py.patch",
        b"--- a/a.py\n+++ b/a.py\n@@ -1 +1 @@\n-x = 1\n+x = 2\n",
        &Type::Code(Language::Auto),
        Limits::default(),
    )
    .unwrap() else {
        panic!("Code")
    };
    assert_eq!(patch.language, Language::Python);
    assert!(matches!(patch.content, Content::Patch(_)));
    assert!(patch.source().unwrap_err().contains(".patch"));
    let source = Document::read("a.py", "def invalid(:\n pass\n", Language::Python).unwrap();
    let parsed = sanim_data::code::parse(source).unwrap();
    assert!(!parsed.diagnostics.is_empty());
    assert!(!parsed.highlights.is_empty());
}
#[test]
fn source_scopes_are_stable_after_line_insertions_and_unicode_errors_use_columns() {
    let parse = |text| {
        sanim_data::code::parse(Document::read("stable.py", text, Language::Python).unwrap())
            .unwrap()
    };
    let first = parse("def hello():\n    return '世界'\n");
    let next = parse("# inserted\ndef hello():\n    return '世界'\n");
    assert_eq!(first.scopes[0].id, next.scopes[0].id);
    assert_eq!(first.scopes[0].start_line + 1, next.scopes[0].start_line);
    let invalid = parse("世界 = )\n");
    assert!(invalid.diagnostics.iter().all(|issue| issue.column <= 7));
}

#[test]
#[ignore = "run with make code-view-bench"]
fn code_view_benchmark() {
    use std::{hint::black_box, time::Instant};
    let source = (0..300)
        .map(|index| format!("def function_{index}(value):\n    return value + {index}\n\n"))
        .collect::<String>();
    let grammar = sanim_data::code::grammar(Language::Python)
        .unwrap()
        .unwrap();
    let first = Document::read("benchmark.py", &source, Language::Python).unwrap();
    let mut cold = Vec::new();
    let mut hot = Vec::new();
    for round in 0..9 {
        let changed = source.replacen(
            "return value + 0",
            &format!("return value + {}", round + 1),
            1,
        );
        let document = Document::read("benchmark.py", &changed, Language::Python).unwrap();
        black_box(sanim_data::code::parse_with(first.clone(), &grammar, true).unwrap());
        let start = Instant::now();
        let fresh = sanim_data::code::parse_with(document.clone(), &grammar, false).unwrap();
        cold.push(start.elapsed());
        let start = Instant::now();
        let updated = sanim_data::code::parse_with(document, &grammar, true).unwrap();
        hot.push(start.elapsed());
        assert_eq!(fresh.highlights, updated.highlights);
        assert_eq!(fresh.scopes, updated.scopes);
        black_box(updated);
    }
    let baseline = sanim_data::code::parse_with(first.clone(), &grammar, true).unwrap();
    let start = Instant::now();
    for index in 0..300 {
        let document =
            Document::read(&format!("view_{index}.py"), &source, Language::Python).unwrap();
        let cached = sanim_data::code::parse_with(document, &grammar, true).unwrap();
        assert!(baseline.syntax.shares_tree(&cached.syntax));
        black_box(cached);
    }
    eprintln!(
        "CodeView same completed syntax reused across 300 paths: {:?} total, {:?} per view",
        start.elapsed(),
        start.elapsed() / 300
    );
    cold.sort();
    hot.sort();
    eprintln!(
        "CodeView Python {} bytes / 300 functions, full parse+highlight+scope median {:?}, incremental median {:?}, ratio {:.3}x",
        source.len(),
        cold[4],
        hot[4],
        cold[4].as_secs_f64() / hot[4].as_secs_f64()
    );
}

#[test]
fn same_revision_shares_native_tree_across_paths_and_preserves_old_tree_after_edit() {
    use std::sync::Arc;
    let text = "def shared_revision(value):\n    return value + 31\n";
    let first = sanim_data::code::parse(Document::read("left.py", text, Language::Python).unwrap())
        .unwrap();
    let other =
        sanim_data::code::parse(Document::read("right.py", text, Language::Python).unwrap())
            .unwrap();
    assert_eq!(other.path.as_ref(), "right.py");
    assert!(first.syntax.shares_tree(&other.syntax));
    assert!(Arc::ptr_eq(&first.highlights, &other.highlights));
    let tree = first.syntax.get::<tree_sitter::Tree>().unwrap();
    let original = tree.root_node().to_sexp();
    let changed = sanim_data::code::parse(
        Document::read(
            "left.py",
            &text.replace("31", "value * 32"),
            Language::Python,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!first.syntax.shares_tree(&changed.syntax));
    assert_eq!(
        tree.root_node().to_sexp(),
        original,
        "old snapshots retain their immutable native tree"
    );
    assert_ne!(
        original,
        changed
            .syntax
            .get::<tree_sitter::Tree>()
            .unwrap()
            .root_node()
            .to_sexp()
    );
}
#[test]
fn grammar_query_identity_and_concurrent_reuse_are_exact() {
    use std::sync::Arc;
    let one = Arc::new(
        sanim_data::code::Grammar::new(
            tree_sitter_python::LANGUAGE.into(),
            "(integer) @number",
            "(function_definition) @fold",
        )
        .unwrap(),
    );
    let two = sanim_data::code::Grammar::new(
        tree_sitter_python::LANGUAGE.into(),
        "(integer) @constant",
        "(function_definition) @fold",
    )
    .unwrap();
    let source = "def concurrent_revision():\n    return 576\n";
    let results = std::thread::scope(|scope| {
        (0..8)
            .map(|index| {
                let grammar = one.clone();
                scope.spawn(move || {
                    sanim_data::code::parse_with(
                        Document::read(&format!("parallel_{index}.py"), source, Language::Python)
                            .unwrap(),
                        &grammar,
                        true,
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    for result in &results[1..] {
        assert!(results[0].syntax.shares_tree(&result.syntax));
    }
    let changed = sanim_data::code::parse_with(
        Document::read("parallel_0.py", source, Language::Python).unwrap(),
        &two,
        true,
    )
    .unwrap();
    assert_eq!(results[0].highlights[0].kind.as_ref(), "number");
    assert_eq!(changed.highlights[0].kind.as_ref(), "constant");
}

#[test]
fn line_number_gutter_uses_native_digit_metrics_and_right_alignment() {
    use sanim_core::{
        Font, Fraction,
        code::view::{Options, Theme},
    };
    use std::sync::Arc;
    let document = Document::read("gutter.txt", &"word\n".repeat(110), Language::Plain).unwrap();
    for family in ["sans-serif", "monospace", "serif"] {
        let font = Font::family(family, family).unwrap();
        let size = Fraction::from(18);
        let options = Options {
            font: font.clone(),
            font_size: size,
            line_height: 27.into(),
            auto_line_height: true,
            tab_size: 4,
            line_numbers: true,
            scope_indicators: false,
            theme: Arc::new(Theme::default()),
            spans: vec![],
        };
        let prepared =
            sanim_data::code::view::prepare("code", &document, &options, &Default::default(), &[])
                .unwrap();
        let gap = 2.
            * sanim_data::font::cached_shape(&font, " ", size, false, false)
                .unwrap()
                .advance;
        let mut right: Option<f64> = None;
        for line in [1, 10, 100] {
            let number = prepared
                .tokens
                .iter()
                .find(|token| token.line == line && token.key.starts_with("line-number:"))
                .unwrap();
            let text = prepared
                .tokens
                .iter()
                .find(|token| token.line == line && token.key.starts_with("token:"))
                .unwrap();
            let label =
                if let sanim_core::Geometry::Text { text, .. } = number.element.geometry.as_ref() {
                    text
                } else {
                    panic!()
                };
            let advance = sanim_data::font::cached_shape(&font, label, size, false, false)
                .unwrap()
                .advance;
            let end = number.position.values[0] + advance;
            if let Some(previous) = right {
                assert!((end - previous).abs() < 1e-8, "{family}: digits must align");
            }
            right = Some(end);
            assert!(
                (text.position.values[0] - end - gap).abs() < 1e-8,
                "{family}: code needs a measured gutter gap"
            );
        }
    }
}
