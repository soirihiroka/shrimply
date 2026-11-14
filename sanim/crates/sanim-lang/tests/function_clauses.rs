use sanim_core::{Fraction, Type, Unit, Value, math::domain::NumericDomain};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;

fn run(source: &str) -> Result<Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}

#[test]
fn numeric_clauses_dispatch_at_closed_and_open_boundaries() {
    let prepared=run("fn classify(i:Int < 1):String{return \"low\"}\nfn classify(i:Int >= 1):String{return \"high\"}\nlet a=classify(-2)\nlet b=classify(0)\nlet c=classify(1)\nlet d=classify(8)").unwrap();
    for name in ["a", "b"] {
        assert_eq!(prepared.bindings[name], Value::String("low".into()));
    }
    for name in ["c", "d"] {
        assert_eq!(prepared.bindings[name], Value::String("high".into()));
    }
}

#[test]
fn fixed_string_clauses_have_separate_bodies_and_durations() {
    let prepared=run("fn delay(time:\"1s\"):Unit{wait(1s)}\nfn delay(time:\"2s\"):Unit{wait(2s)}\ndelay(\"1s\")\ndelay(\"2s\")").unwrap();
    assert_eq!(prepared.animation.duration, Fraction::from(3));
}

#[test]
fn overlapping_and_uncovered_domains_explain_the_failure() {
    let error = run("fn pick(n:Number < 2):Unit{wait(0s)}\nfn pick(n:Number >= 1):Unit{wait(0s)}")
        .unwrap_err();
    assert!(error.message.contains("ambiguous clauses"), "{error}");
    let error=run("fn first(values:List<Number>):Unit{wait(0s)}\nfn first(values:List<String>):Unit{wait(0s)}").unwrap_err();
    assert!(
        error.message.contains("ambiguous clauses"),
        "empty lists must not make dispatch ambiguous: {error}"
    );
    let error = run(
        "fn pick(n:Int < 0):Unit{wait(0s)}\nfn pick(n:Int > 0):Unit{wait(0s)}\nmut n=0\npick(n)",
    )
    .unwrap_err();
    assert!(error.message.contains("clause"), "{error}");
    let error = run(
        "fn pick(n:Int < 0):Unit{wait(0s)}\nfn pick(n:Int >= 0):Unit{wait(0s)}\nmut n=0.5\npick(n)",
    )
    .unwrap_err();
    assert!(error.message.contains("clause"), "{error}");
}

#[test]
fn numeric_annotations_compose_and_enforce_integer_values() {
    run("type Small=Int >= -2 & Number < 3\nlet a:Small=-2\nlet b:Small=2").unwrap();
    run("type Nonzero=Int - (Int == 0)\nlet negative:Nonzero=-1\nlet positive:Nonzero=1").unwrap();
    assert!(run("type Nonzero=Int - (Int == 0)\nlet value:Nonzero=0").is_err());
    run("type Whole=(Int < 1) | (Int >= 1)\nlet original:Int=4\nlet copied:Whole=original")
        .unwrap();
    for source in [
        "let value:Int=0.5",
        "let value:Number > 2=2",
        "type Empty=Int > 0 < 1",
        "let value:Duration < 2=1s",
    ] {
        assert!(run(source).is_err(), "{source}");
    }
}

#[test]
fn inferred_read_integer_schema_survives_mutable_bindings() {
    let mut files = MemorySources::default();
    files.insert("number.json", "4");
    files.insert(
        "main.sanim",
        "mut value=read<Int>(\"number.json\")\nvalue=0.5",
    );
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(error.message.contains("Int"), "{error}");
}

#[test]
fn known_arguments_specialize_the_static_result_type() {
    run("fn category(n:Number < 0):\"negative\"{return \"negative\"}\nfn category(n:Number >= 0):\"nonnegative\"{return \"nonnegative\"}\nlet selected:\"negative\"=category(-1)").unwrap();
}

#[test]
fn generic_numeric_arithmetic_does_not_promise_literal_refinements() {
    let prepared=run("fn square<T:Numeric>(kind:\"a\",value:T):T{return value*value}\nfn square<T:Numeric>(kind:\"b\",value:T):T{return value*value}\nlet result=square(\"a\",2)").unwrap();
    assert_eq!(
        prepared.bindings["result"].number().unwrap().value,
        Fraction::from(4)
    );
    assert!(run("fn unsafe_square<T:Int < 2>(value:T):T{return value*value}").is_err());
}

#[test]
fn generic_and_signal_clauses_use_the_canonical_call_path() {
    let prepared=run("fn pick<T>(kind:\"left\",value:T):T{return value}\nfn pick<T>(kind:\"right\",value:T):T{return value}\nlet a=pick(\"left\",5)\nfn twice(sig value:Number < 0):sig Number{return value*2}\nfn twice(sig value:Number >= 0):sig Number{return value*2}\nmut input=2\nsig output=twice(input)\ninput=3\nlet b=output").unwrap();
    assert_eq!(
        prepared.bindings["a"].number().unwrap().value,
        Fraction::from(5)
    );
    assert_eq!(
        prepared.bindings["b"].number().unwrap().value,
        Fraction::from(6)
    );
}

#[test]
fn duration_contract_annotation_is_typed_and_pure() {
    run("@duration(1s)\nfn delay(time:\"one\"):Unit{wait(1s)}\n@duration(2s)\nfn delay(time:\"two\"):Unit{wait(2s)}\ndelay(\"one\")").unwrap();
    assert!(run("@duration(1) fn bad():Unit{wait(0s)}").is_err());
    assert!(run("@duration(wait(1s)) fn bad():Unit{wait(0s)}").is_err());
    assert!(run("@duration(1s) let value=1").is_err());
}

#[test]
fn formatter_round_trips_refinements_and_contracts() {
    let source = "@duration(1s) fn pause(i:Int<1):Unit{wait(1s)}\n@duration(2s) fn pause(i:Int>=1):Unit{wait(2s)}\npause(0)";
    let formatted =
        sanim_lang::formatter::format_source("main.sanim", source, Default::default(), 200_000)
            .unwrap();
    run(&formatted).unwrap();
    let twice =
        sanim_lang::formatter::format_source("main.sanim", &formatted, Default::default(), 200_000)
            .unwrap();
    assert_eq!(formatted, twice);
}

#[test]
#[ignore = "manual clause dispatch performance measurement"]
fn function_clause_dispatch_benchmark() {
    use std::time::Instant;
    const CALLS: usize = 1000;
    let body = (0..CALLS)
        .map(|index| format!("let result_{index}=pick({index})\n"))
        .collect::<String>();
    let single = format!("fn pick(n:Number):Number{{return n}}\n{body}");
    let guarded = format!(
        "fn pick(n:Int < 500):Number{{return n}}\nfn pick(n:Int >= 500):Number{{return n}}\n{body}"
    );
    for (name, source) in [("single", single), ("guarded", guarded)] {
        let start = Instant::now();
        let prepared = run(&source).unwrap();
        assert_eq!(
            prepared.bindings["result_999"].number().unwrap().value,
            Fraction::from(999)
        );
        eprintln!("{name}: {CALLS} calls in {:?}", start.elapsed());
    }
}

#[test]
fn exact_integer_interval_math_handles_negative_fractional_bounds() {
    let integer = NumericDomain::all(Unit::Number, true);
    let lower = integer
        .clone()
        .constrained(">", sanim_core::Number::parse("-0.5").unwrap())
        .unwrap();
    assert_eq!(lower.lower, Some((Fraction::from(0), true)));
    let upper = integer
        .constrained("<", sanim_core::Number::parse("-0.5").unwrap())
        .unwrap();
    assert_eq!(upper.upper, Some((Fraction::from(-1), true)));
    assert!(!Type::RefinedNumeric(lower).overlaps(&Type::RefinedNumeric(upper)));
}

#[test]
fn clause_overlap_analysis_obeys_the_existing_type_work_budget() {
    let source = (0..64)
        .map(|index| format!("fn choose(value:\"{index}\"):Number{{return {index}}}\n"))
        .collect::<String>();
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let mut engine = Engine::default();
    engine.limits.max_type_bytes = 4 * 1024;
    let error = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S105");
    assert!(error.message.contains("budget"), "{error}");
}
