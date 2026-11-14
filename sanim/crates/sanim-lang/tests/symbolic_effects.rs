use sanim_core::{Fraction, TimelineEffect, Type};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn files(source: &str) -> MemorySources {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    files
}
fn compare(source: &str, expected: Fraction) {
    let files = files(source);
    let full = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let fast = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(full.animation.duration, expected, "{source}");
    assert_eq!(fast.duration, expected, "{source}");
}
#[test]
fn function_parameters_lengths_and_transitive_calls_preserve_formulas() {
    let source = "fn tick(count:Number):Unit{repeat count{wait(1ms)}}\nfn wrapper(count:Number):Unit{tick(count+1)}\nfn rows(items:List<Number>):Unit{for item in items{wait(250ms)}}\nwrapper(3)\nrows([1,2,3,4])";
    let prepared = Engine::default()
        .prepare("main.sanim", &files(source), &BTreeMap::new())
        .unwrap();
    for (name, symbol) in [
        ("tick", "count"),
        ("wrapper", "count"),
        ("rows", "len(items)"),
    ] {
        let signature = &prepared.symbols[name].signatures()[0];
        assert!(
            matches!(signature.duration, TimelineEffect::Symbolic(_)),
            "{name}: {}",
            signature.duration
        );
        assert!(signature.duration.to_string().contains(symbol));
    }
    compare(source, Fraction::new(251u64, 250u64));
    let report = Engine::default()
        .duration(
            "main.sanim",
            &files("fn tick(count:Number):Unit{repeat count{wait(1ms)}}\ntick(100000)"),
            &BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(report.duration, Fraction::from(100));
    assert!(
        report.statistics.operations < 100,
        "closed-form count must not iterate100000 times: {:?}",
        report.statistics
    );
}
#[test]
fn conditionals_keep_alternatives_and_early_returns_do_not_add_fallthrough_time() {
    let source = "fn choose(flag:Bool):Unit{if flag{return wait(1s)};return wait(2s)}\nchoose(true)\nchoose(false)";
    let prepared = Engine::default()
        .prepare("main.sanim", &files(source), &BTreeMap::new())
        .unwrap();
    let effect = &prepared.symbols["choose"].signatures()[0].duration;
    assert!(
        matches!(effect, TimelineEffect::Alternatives(_)),
        "{effect}"
    );
    assert_eq!(effect.to_string(), "1s | 2s");
    compare(source, Fraction::from(3));
}
#[test]
fn csv_length_resolves_without_rendering_and_keeps_its_file_dependency() {
    let source = "type Row={x:Number}\nlet samples=read<List<Row>>(\"samples.csv\")\nfn emit(items:List<Row>):Unit{for item in items{wait(250ms)}}\nemit(samples)";
    let mut files = files(source);
    files.insert("samples.csv", "x\n1\n2\n3\n");
    let report = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(report.duration, Fraction::new(3u64, 4u64));
    assert_eq!(report.statistics.decoded_assets, 1);
    assert!(report.dependencies.contains_key("samples.csv"));
}
#[test]
fn symbolic_specialization_still_checks_loop_domain_limits_and_overflow() {
    for count in ["0.5", "-1", "100001"] {
        let source =
            format!("fn tick(count:Number):Unit{{repeat count{{wait(1ms)}}}}\ntick({count})");
        assert!(
            Engine::default()
                .duration("main.sanim", &files(&source), &BTreeMap::new())
                .is_err(),
            "{source}"
        );
    }
    let source = "fn tick(step:Duration):Unit{wait(step)}\ntick(7201s)";
    assert!(
        Engine::default()
            .duration("main.sanim", &files(source), &BTreeMap::new())
            .unwrap_err()
            .message
            .contains("duration")
    );
}
#[test]
fn pure_zero_time_unused_map_does_not_force_unrelated_assets() {
    let source = "fn identity(value:Number):Number{return value}\nlet data=read<List<Number>>(\"absent.json\")\nlet ignored=data.map(identity)\nwait(1s)";
    let report = Engine::default()
        .duration("main.sanim", &files(source), &BTreeMap::new())
        .unwrap();
    assert_eq!(report.duration, Fraction::from(1));
    assert!(!report.dependencies.contains_key("absent.json"));
}
#[test]
fn overload_effects_specialize_known_literals_and_annotations_are_not_padding() {
    let source =
        "fn delay(time:\"1s\"):Unit{wait(1s)}\nfn delay(time:\"2s\"):Unit{wait(2s)}\ndelay(\"1s\")";
    let prepared = Engine::default()
        .prepare("main.sanim", &files(source), &BTreeMap::new())
        .unwrap();
    assert!(matches!(prepared.symbols["delay"], Type::Overloaded(_)));
    compare(source, Fraction::from(1));
    let error = Engine::default()
        .prepare(
            "main.sanim",
            &files("@duration(2s)\nfn wrong():Unit{wait(1s)}\nwrong()"),
            &BTreeMap::new(),
        )
        .unwrap_err();
    assert!(
        error.message.contains("duration") && error.message.contains("body"),
        "{error}"
    );
}

#[test]
fn reflected_duration_defaults_are_not_baked_into_static_formulas() {
    let source = "let step=reflect(1s,key=\"step\")\nwait(step)";
    let overrides = BTreeMap::from([(
        "step".to_string(),
        sanim_core::Value::Number(sanim_core::Number::parse("2s").unwrap()),
    )]);
    let report = Engine::default()
        .duration("main.sanim", &files(source), &overrides)
        .unwrap();
    assert_eq!(report.duration, Fraction::from(2));
}

#[test]
fn default_formulas_use_the_declarations_captured_module_constants() {
    compare(
        "let configured=1s\nfn pause(second:Duration=configured*2):Unit{wait(second)}\npause()",
        Fraction::from(2),
    );
}
#[test]
fn item_dependent_callback_time_is_evaluated_per_item_not_multiplied_as_one_parameter() {
    compare(
        "fn timed(value:Number):Number{wait(value*1s);return value}\nlet result=[1,2,3].map(timed)\nwait(1s)",
        Fraction::from(7),
    );
}

#[test]
fn annotation_body_contract_keeps_omitted_timed_default_effects_at_the_call_site() {
    compare(
        "fn input():Number{wait(1s);return 1}\n@duration(2s)\nfn job(value:Number=input()):Unit{wait(2s)}\njob()\nwait(1s)",
        Fraction::from(4),
    );
    compare(
        "fn input():Number{wait(1s);return 1}\n@duration(2s)\nfn job(value:Number=input()):Unit{wait(2s)}\njob(1)\nwait(1s)",
        Fraction::from(3),
    );
    compare(
        "@duration(step*2)\nfn job(step:Duration):Unit{wait(step);wait(step)}\njob(1s)",
        Fraction::from(2),
    );
}

#[test]
fn exact_sized_parameter_length_can_plan_without_reading_its_values() {
    let source = "fn rows(items:List<Number,3>):Unit{repeat len(items){wait(250ms)}}\nrows(read<List<Number,3>>(\"absent.json\"))";
    let report = Engine::default()
        .duration("main.sanim", &files(source), &BTreeMap::new())
        .unwrap();
    assert_eq!(report.duration, Fraction::new(3u64, 4u64));
    assert!(!report.dependencies.contains_key("absent.json"));
}
