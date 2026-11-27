use sanim_core::{Fraction, ListLength, Number, Type, Unit, Value};
use sanim_lang::{
    Limits,
    expression::{Bindings, CheckedExpression, Interface},
};

const NUMBER: Type = Type::Numeric(Unit::Number);

fn check(source: &str, result: Type) -> CheckedExpression {
    CheckedExpression::check("test", source, Interface::new(result), Limits::default())
        .unwrap_or_else(|error| panic!("{error}"))
}

fn run(source: &str, result: Type) -> Value {
    check(source, result)
        .evaluate(Bindings::new(), &|name, _| {
            panic!("unexpected host call {name}")
        })
        .unwrap()
}

fn scalar(value: i64) -> Value {
    Value::Number(Number::scalar(value))
}

#[test]
fn newlines_follow_the_grammar_and_semicolons_force_boundaries() {
    let source = concat!(
        "fn twice(x: Number): Number { return x * 2 }\r\n",
        "let values = [8, 9]\r\n",
        "let n:\r\nNumber = twice\r\n",
        "    (values\r\n",
        "      [0]) # the next line continues the expression\r\n",
        "      +\r\n",
        "        2\r\n",
        "let rec = {a = n}\r\n",
        "let a = rec\r\n",
        "    .a;\r\n",
        "  (+a)\r\n",
    );
    assert_eq!(run(source, NUMBER), scalar(18));

    let formatted = sanim_lang::formatter::format_source(
        "test",
        source,
        Default::default(),
        Limits::default().max_parse_steps,
    )
    .unwrap();
    assert_eq!(run(&formatted, NUMBER), scalar(18));
    assert!(formatted.contains(".a;"));
}

#[test]
fn return_newline_does_not_take_a_parenthesized_operand() {
    let error = CheckedExpression::check(
        "test",
        "return\n(1)",
        Interface::new(NUMBER),
        Limits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(error.span.line, 1);
    assert!(error.message.contains("Unit"), "{error}");

    let source = "let f = fn (): Unit { return # no value\n }; f(); 4";
    assert_eq!(run(source, NUMBER), scalar(4));
}

#[test]
fn optional_parentheses_and_else_if_work_with_quoted_punctuation() {
    let source = r#"
let text = "semicolon;\""
let message = { "key:;": text }
if false {
    return "first"
} else if (len(keys(message)) == 1) {
    return "{3:02d}: {text}"
} else {
    return "last"
}"#;
    assert_eq!(
        run(source, Type::String),
        Value::String("03: semicolon;\"".into())
    );
}

#[test]
fn vectors_use_checked_cardinality_and_pointwise_arithmetic() {
    let vector = Type::List(Box::new(NUMBER), ListLength::Exact(2));
    let result = run(
        "let p = [10, 20]\nlerp(p + [2, 0], [14, 24], 0.5)",
        vector.clone(),
    );
    assert_eq!(
        result.list().unwrap().iter().cloned().collect::<Vec<_>>(),
        [scalar(13), scalar(22)]
    );
    let error = CheckedExpression::check(
        "test",
        "[1, 2, 3]",
        Interface::new(vector),
        Limits::default(),
    )
    .err()
    .unwrap();
    assert!(error.message.contains("List<Number,2>"), "{error}");
}

#[test]
fn scene_and_resource_apis_are_rejected_in_every_checked_context() {
    for source in [
        "import \"./scene\" as scene\n0",
        "let scene = <Box/>\n0",
        "fn helper(): Animation { return wait(1s) }; 0",
        "let f = fn (): Number { let data = read<Number>(\"x.json\"); return data }; 0",
        "let configured = reflect(1)\n0",
        "sig phase = 0\n0",
    ] {
        let error =
            CheckedExpression::check("test", source, Interface::new(NUMBER), Limits::default())
                .err()
                .unwrap_or_else(|| panic!("checked restricted source: {source}"));
        assert_eq!(error.code, "S141", "{error}");
    }
}

#[test]
fn host_values_and_local_mutation_are_fresh_and_work_is_bounded() {
    let program = CheckedExpression::check(
        "test",
        "mut total = sample\nrepeat 2 { total = total + current() }\ntotal",
        Interface::new(NUMBER)
            .input("sample", NUMBER)
            .input("unused", Type::String)
            .function("current", NUMBER, []),
        Limits {
            max_operations: 100,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(program.inputs().collect::<Vec<_>>(), ["sample"]);
    for (input, sample, expected) in [(2, 4, 10), (5, 2, 9)] {
        let result = program
            .evaluate(
                Bindings::from(vec![("sample".to_owned(), scalar(input))]),
                &|name, _| {
                    assert_eq!(name, "current");
                    Ok(scalar(sample))
                },
            )
            .unwrap();
        assert_eq!(result, scalar(expected));
    }
    let invalid = program
        .evaluate(
            Bindings::from(vec![("sample".to_owned(), scalar(1))]),
            &|_, _| {
                Ok(Value::Number(Number {
                    value: Fraction::NaN,
                    unit: Unit::Number,
                }))
            },
        )
        .unwrap_err();
    assert!(invalid.message.contains("finite"), "{invalid}");
    let bounded = CheckedExpression::check(
        "test",
        "mut a = 1\nrepeat 1000 { a = a + 1 }\na",
        Interface::new(NUMBER),
        Limits {
            max_operations: 40,
            ..Limits::default()
        },
    )
    .unwrap();
    let error = bounded
        .evaluate(Bindings::new(), &|_, _| unreachable!())
        .unwrap_err();
    assert_eq!(error.code, "S303");
}

#[test]
fn temporary_collections_are_subject_to_value_limits() {
    let limits = Limits {
        data: sanim_data::Limits {
            max_values: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let expression =
        CheckedExpression::check("test", "len([1, 2])", Interface::new(NUMBER), limits).unwrap();
    let error = expression
        .evaluate(Bindings::new(), &|_, _| unreachable!())
        .unwrap_err();
    assert_eq!(error.code, "S303");
}

#[test]
fn fractional_frame_rates_stay_exact_through_drop_frame_boundaries() {
    let rate = Fraction::new(30_000_u64, 1_001_u64);
    let program = CheckedExpression::check(
        "test",
        "timecode(frame_index(time, fps), fps, true)",
        Interface::new(Type::String)
            .input("time", Type::Numeric(Unit::Duration))
            .input("fps", Type::Numeric(Unit::Rate)),
        Limits::default(),
    )
    .unwrap();
    for (rate, frame, output) in [
        (rate, 1799, "00:00:59;29"),
        (rate, 1800, "00:01:00;02"),
        (rate * Fraction::from(2), 3600, "00:01:00;04"),
    ] {
        let inputs = Bindings::from(vec![
            (
                "time".to_owned(),
                Value::Number(Number {
                    value: Fraction::from(frame) / rate,
                    unit: Unit::Duration,
                }),
            ),
            (
                "fps".to_owned(),
                Value::Number(Number {
                    value: rate,
                    unit: Unit::Rate,
                }),
            ),
        ]);
        assert_eq!(
            program.evaluate(inputs, &|_, _| unreachable!()).unwrap(),
            Value::String(output.into())
        );
    }
}

#[test]
fn long_else_if_chains_hit_the_parser_nesting_limit() {
    let source = format!(
        "if false {{ return 0 }}{} else {{ return 1 }}",
        " else if false { return 0 }".repeat(1000)
    );
    let error = sanim_lang::parser::parse("nested", &source, Limits::default().max_parse_steps)
        .unwrap_err();
    assert!(error.message.contains("nesting"), "{error}");
}

#[test]
fn markup_on_its_own_line_starts_a_scene_statement() {
    let source = "let scale = 1\n<Animation>{wait(scale * 1s)}</Animation>\nwait(0.5s)";
    let mut sources = sanim_lang::MemorySources::default();
    sources.insert("scene.sanim", source);
    let animation = sanim_lang::Engine::default()
        .prepare("scene.sanim", &sources, &std::collections::BTreeMap::new())
        .unwrap();
    assert_eq!(animation.animation.duration, Fraction::new(3_u64, 2_u64));
}

#[test]
fn an_incomplete_tag_after_an_expression_is_available_to_editor_recovery() {
    let source = "let size = 10pt\n<Bo";
    let recovery =
        sanim_lang::parser::parse_recovering("scene", source, Limits::default().max_parse_steps);
    assert_eq!(recovery.module.statements.len(), 1);
    assert_eq!(
        recovery.markup.last().map(|opening| opening.name.as_str()),
        Some("Bo")
    );
}
