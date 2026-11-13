use sanim_core::{Fraction, Number, Value};
use sanim_lang::{Engine, FileSources, MemorySources, Reload, Session, Sources};
use std::collections::BTreeMap;

fn run(text: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", text);
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}

#[test]
fn recursion_cannot_hide_in_function_aliases() {
    let e = run("fn recurse(): Number { let alias = recurse; return alias() }\n").unwrap_err();
    assert_eq!(e.code, "S111");
}
#[test]
fn default_arguments_participate_in_cycle_checking() {
    let e = run("fn recurse(value: Number = recurse()): Number { return value }\n").unwrap_err();
    assert_eq!(e.code, "S111");
}
#[test]
fn percentage_components_keep_units() {
    let p = run("let p = Vec2(50%, 25%)\nlet x: Percent = p.x\n").unwrap();
    assert_eq!(
        p.bindings["x"].number().unwrap().unit,
        sanim_core::Unit::Percent
    );
}
#[test]
fn point_parameter_units_can_be_explicit() {
    let p=run("fn x_coordinate(p: Vec2<Length>): Length { return p.x }\nlet x = x_coordinate(Vec2(10pt, 20pt))\n").unwrap();
    assert_eq!(p.bindings["x"].number().unwrap().value, Fraction::from(10));
}
#[test]
fn percentage_cannot_be_used_as_length() {
    assert_eq!(
        run("let p = Vec2(50%, 25%)\nlet x: Length = p.x\n")
            .unwrap_err()
            .code,
        "S106"
    );
}
#[test]
fn oversized_expression_is_rejected_without_stack_overflow() {
    let text = format!("let result = {}\n", vec!["1"; 10_000].join(" + "));
    assert!(run(&text).is_err());
}

#[test]
fn enum_case_qualifier_must_match() {
    let e=run("enum First { A }\nenum Second { A }\nfn f(value: First): Number { match value { case Second.A { return 1 } } }\n").unwrap_err();
    assert_eq!(e.code, "S106");
}
#[test]
fn floating_layout_supports_values_beyond_fraction_products() {
    let p = run("let marker = Circle(position: Vec2(18446744073709551565%, 0%))\nadd(marker)\n")
        .unwrap();
    assert!(
        p.animation
            .layout(
                Fraction::from(0),
                Fraction::from(u64::MAX),
                Fraction::from(100)
            )
            .is_ok()
    );
}
#[test]
fn assertion_overflow_is_not_a_proof() {
    let p=run("let marker = Circle(position: Vec2(18446744073709551615pt, 10pt))\ncheck(inside_canvas(marker))\n").unwrap();
    assert_ne!(p.checks[0].evidence, sanim_core::verify::Evidence::Proved);
}
#[test]
fn malformed_programs_never_panic() {
    let alphabet = b"let fn (){}[]<>:;=,+-*/%#\"\\\n abc123";
    let mut seed = 0x5a17u64;
    for _ in 0..1000 {
        let mut text = String::new();
        for _ in 0..48 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            text.push(alphabet[(seed as usize) % alphabet.len()] as char);
        }
        let _result = run(&text);
    }
}
#[test]
fn changing_decode_limits_invalidates_asset_cache() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let data = read<List<Number>>(\"values.json\")\n",
    );
    files.insert("values.json", "[1,2,3]");
    let mut engine = Engine::default();
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    engine.limits.data.max_values = 2;
    assert_eq!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S306"
    );
}
#[test]
fn changing_iteration_limits_invalidates_pure_cache() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let values = range(10)\n");
    let mut engine = Engine::default();
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    engine.limits.max_iterations = 2;
    assert!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .is_err()
    );
}
#[test]
fn changing_limits_triggers_session_validation() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "repeat 4 { wait(1s) }\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    session.engine.limits.max_iterations = 2;
    assert!(matches!(session.poll(&files), Reload::Failed { .. }));
}
#[test]
fn changed_read_schema_is_not_cached_as_old_type() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let data = read<List<Number>>(\"values.json\")\n",
    );
    files.insert("values.json", "[1,2]");
    let mut engine = Engine::default();
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    files.insert(
        "main.sanim",
        "let data = read<List<String>>(\"values.json\")\n",
    );
    assert_eq!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S306"
    );
}
#[test]
fn reflected_nonfinite_coordinates_are_rejected() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let location=reflect(Vec2(10pt, 20pt))\n");
    let input = BTreeMap::from([(
        "location".into(),
        Value::Vec2(sanim_core::Vec2 {
            values: [f64::NAN, 0.0],
            unit: sanim_core::Unit::Length,
        }),
    )]);
    assert_eq!(
        Engine::default()
            .prepare("main.sanim", &files, &input)
            .unwrap_err()
            .code,
        "S302"
    );
}
#[test]
fn source_changes_during_prepare_discard_revision() {
    use std::cell::Cell;
    struct Changing {
        reads: Cell<usize>,
    }
    impl Sources for Changing {
        fn read(&self, _: &str) -> Result<Vec<u8>, String> {
            let n = self.reads.get();
            self.reads.set(n + 1);
            Ok(if n == 0 {
                b"wait(1s)\n".to_vec()
            } else {
                b"wait(2s)\n".to_vec()
            })
        }
    }
    let source = Changing {
        reads: Cell::new(0),
    };
    assert_eq!(
        Engine::default()
            .prepare("main.sanim", &source, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S203"
    );
}
#[test]
fn all_shipped_examples_prepare() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let sources = FileSources::new(root).unwrap();
    for path in ["hello.sanim", "data.sanim", "timer.sanim", "query.sanim"] {
        let p = Engine::default()
            .prepare(path, &sources, &BTreeMap::new())
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(!p.animation.layers.is_empty());
    }
}
#[test]
fn unknown_override_does_not_change_cache_validity() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let count=reflect(1)\nwait(count * 1s)\n");
    let mut engine = Engine::default();
    let good = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let bad = BTreeMap::from([("unknown".into(), Value::Number(Number::scalar(9)))]);
    assert!(engine.prepare("main.sanim", &files, &bad).is_err());
    assert_eq!(
        good.fingerprint,
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap()
            .fingerprint
    );
}

#[test]
fn parser_limits_are_enforced_on_cached_modules() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let value = 1 + 2\n");
    let mut engine = Engine::default();
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    engine.limits.max_parse_steps = 2;
    assert_eq!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S002"
    );
}
#[test]
fn flow_position_conflicts_fail_explicitly() {
    assert!(run("let child = Circle(id: \"child\")\nadd(Row(children: [child]))\nMoveTo(child, Vec2(10pt,0pt), duration=1s, ease=\"linear\")\n").unwrap_err().message.contains("flow layout controls"));
}

#[test]
fn shallow_multiline_source_does_not_recurse_per_newline() {
    let source = format!(
        "let values: List<Number> = [\n{}1\n]\n",
        "\n // blank\n".repeat(20_000)
    );
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
}

#[test]
fn registered_types_functions_and_constants_are_automatically_protected() {
    for name in sanim_lang::prelude::types()
        .keys()
        .copied()
        .chain(sanim_lang::prelude::builtins().iter().map(|b| b.name))
        .chain(
            sanim_core::math::functions::CONSTANTS
                .iter()
                .map(|(n, _)| *n),
        )
    {
        assert!(sanim_lang::prelude::protected(name), "{name}");
        let mut files = MemorySources::default();
        files.insert("main.sanim", format!("let {name}=1"));
        assert_eq!(
            Engine::default()
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap_err()
                .code,
            "S101",
            "{name}"
        );
    }
}

#[test]
fn functions_cannot_bypass_static_call_graph_in_temporary_values() {
    for source in [
        "fn identity<T>(value:T):T{return value}\nfn recurse():Number{return identity(recurse)()}\nlet x=recurse()",
        "fn recurse():Number{return unwrap(Some(recurse),recurse)()}\nlet x=recurse()",
        "fn recurse():Number{return ({callback:recurse}).callback()}\nlet x=recurse()",
        "fn identity<T>(value:T):T{return value}\nfn number_value():Number{return 1}\nidentity(number_value)",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        assert_eq!(
            Engine::default()
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap_err()
                .code,
            "S111",
            "{source}"
        );
    }
}

#[test]
fn resolved_alias_expansion_has_a_global_byte_budget() {
    let mut source = String::from("type A0 = {left:Number,right:Number}\n");
    for index in 1..40 {
        source.push_str(&format!(
            "type A{index} = {{left:A{},right:A{}}}\n",
            index - 1,
            index - 1
        ));
    }
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S105");
    assert!(error.message.contains("budget"), "{error}");
}

#[test]
fn enum_namespaces_cannot_bypass_type_budget() {
    let source = format!(
        "enum Many {{ {} }}",
        (0..10_000)
            .map(|i| format!("V{i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S105");
    assert!(error.message.contains("budget"));
}
#[test]
fn inferred_generic_expansion_cannot_bypass_type_budget() {
    let mut source = String::from(
        "fn wrap<T>(value:Option<T>):Option<{left:T,right:T}>{return None}\nlet a0:Option<Number>=None\n",
    );
    for i in 1..40 {
        source.push_str(&format!("let a{i}=wrap(a{})\n", i - 1));
    }
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(error.message.contains("budget"), "{error}");
}
