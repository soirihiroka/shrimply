use sanim_core::{Fraction, TimelineEffect, Type};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;
fn sources(source: &str, assets: &[(&str, &str)]) -> MemorySources {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    for (path, text) in assets {
        files.insert(*path, *text);
    }
    files
}
fn compare(source: &str, assets: &[(&str, &str)], expected: Fraction) {
    let files = sources(source, assets);
    let full = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_or_else(|e| panic!("full: {e}\n{source}"));
    let fast = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap_or_else(|e| panic!("duration: {e}\n{source}"));
    assert_eq!(full.animation.duration, expected, "full: {source}");
    assert_eq!(
        fast.duration, full.animation.duration,
        "duration mismatch: {source}"
    );
}
#[test]
fn exact_rational_sums_and_parallel_plays_match() {
    compare(
        "let a=Circle()\nlet b=Rect()\nFadeIn([a, b], duration=100ms, ease=\"linear\")\nrepeat 3{wait(1s/3)}",
        &[],
        Fraction::new(11u64, 10u64),
    );
}
#[test]
fn timer_padding_and_early_returns_match() {
    compare("timer 3 every 1s{wait(200ms)}", &[], Fraction::from(3));
    compare(
        "fn stop():Unit{timer 1 every 10s{return wait(1s)}}\nstop()",
        &[],
        Fraction::from(1),
    );
    compare(
        "fn stop():Unit{timer 4 every 10s{return wait(1s)}}\nstop()",
        &[],
        Fraction::from(1),
    );
}
#[test]
fn equal_time_branches_preserve_early_return_control() {
    for (flag, expected) in [("true", 0), ("false", 1)] {
        compare(
            &format!(
                "fn pause(flag:Bool):Unit{{if flag{{return wait(0s)}};wait(1s)}}\npause({flag})"
            ),
            &[],
            Fraction::from(expected),
        );
    }
}
#[test]
fn empty_and_nonempty_zero_time_loops_preserve_returns() {
    for (items, expected) in [("[]", 1), ("[1]", 0)] {
        compare(
            "let items=read<List<Number>>(\"items.json\")\nfn pause():Unit{for item in items{return wait(0s)};wait(1s)}\npause()",
            &[("items.json", items)],
            Fraction::from(expected),
        );
    }
}
#[test]
fn imported_helper_arguments_and_data_dependencies_resolve() {
    compare(
        "import \"utils\" as utils\nlet length=read<Duration>(\"duration.json\")\nutils.pause(length)",
        &[
            ("utils.sanim", "fn pause(delay:Duration):Unit{wait(delay)}"),
            ("duration.json", "\"125ms\""),
        ],
        Fraction::new(1u64, 8u64),
    );
}
#[test]
fn value_return_helpers_keep_transitive_data_dependencies() {
    compare(
        "let samples=read<List<Number>>(\"samples.json\")\nfn count():Number{let size=len(samples);return size}\nwait(count()*1s)",
        &[("samples.json", "[1,2,3]")],
        Fraction::from(3),
    );
}
#[test]
fn function_parameters_are_not_later_global_constants() {
    compare(
        "fn pause(delay:Duration):Unit{wait(delay)}\nlet delay=10s\npause(1s)",
        &[],
        Fraction::from(1),
    );
    compare(
        "if true{let delay=1s;wait(delay)}\nlet delay=10s",
        &[],
        Fraction::from(1),
    );
}
#[test]
fn default_argument_effects_follow_source_evaluation_order() {
    compare(
        "fn make():Element{wait(200ms);return Circle()}\nfn pause(marker:Element=make()):Unit{wait(300ms)}\npause()",
        &[],
        Fraction::new(1u64, 2u64),
    );
    compare(
        "fn make():Element{wait(200ms);return Circle()}\nfn pause(marker:Element=make()):Unit{wait(300ms)}\nlet marker=Circle()\npause(marker)",
        &[],
        Fraction::new(3u64, 10u64),
    );
}
#[test]
fn short_circuit_calls_do_not_add_unexecuted_time() {
    compare(
        "fn tick():Bool{wait(1s);return true}\nlet a=false and tick()\nlet b=true or tick()\nlet c=true and tick()\nlet d=false or tick()",
        &[],
        Fraction::from(2),
    );
}
#[test]
fn match_payloads_and_early_returns_match() {
    compare(
        "enum Choice{Stop,Continue}\nfn pause(choice:Choice):Unit{match choice{case Choice.Stop{return wait(0s)} case Choice.Continue{wait(0s)}};wait(1s)}\npause(Choice.Stop)",
        &[],
        Fraction::from(0),
    );
    compare(
        "enum Delay{Value(Duration)}\nlet delay=Delay.Value(250ms)\nmatch delay{case Delay.Value(value){wait(value)}}",
        &[],
        Fraction::new(1u64, 4u64),
    );
}
#[test]
fn duration_analysis_skips_unneeded_geometry_and_assets() {
    let files = sources(
        "let font=read<Font>(\"unused.ttf\")\nfn caption():Element{return Text(\"x\",font=font)}\nrepeat 100000{let marker=Circle();FadeIn(marker, duration=1ms, ease=\"linear\")}",
        &[],
    );
    let report = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(report.duration, Fraction::from(100));
    assert!(!report.dependencies.contains_key("unused.ttf"));
    assert!(
        report.statistics.operations < 100,
        "{:?}",
        report.statistics
    );
}
#[test]
fn every_checked_node_and_function_has_a_duration_effect() {
    let source = "fn pause():Unit{let value=abs(-2);wait(200ms)}\npause()";
    let files = sources(source, &[]);
    let full = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let Type::Function(signature) = &full.symbols["pause"] else {
        panic!()
    };
    assert_eq!(
        signature.duration,
        TimelineEffect::Fixed(Fraction::new(1u64, 5u64))
    );
    let nodes = &full.durations["main.sanim"];
    let module = sanim_lang::parser::parse("main.sanim", source, 10000).unwrap();
    for statement in &module.statements {
        let statements =
            if let sanim_lang::ast::StatementKind::Function { body, .. } = &statement.kind {
                body.as_slice()
            } else {
                std::slice::from_ref(statement)
            };
        for statement in statements {
            assert!(nodes.iter().any(|node| node.span == statement.span));
            statement.visit_expressions(&mut |expr| {
                assert!(
                    nodes.iter().any(|node| node.span == expr.span),
                    "missing timing metadata for {:?}",
                    expr.kind
                )
            });
        }
    }
    for builtin in sanim_lang::prelude::builtins() {
        assert_eq!(
            builtin.parameters[builtin.timing.parameter].name,
            "duration"
        );
        assert_eq!(builtin.timeline_duration(), TimelineEffect::Unknown);
    }
}
#[test]
fn duration_limits_and_iteration_limits_fail_in_both_paths() {
    for source in [
        "wait(7200s+1ms)",
        "wait(7199s);wait(2s)",
        "repeat 100001{wait(1ms)}",
        "repeat 1000000000000{let unused=1}",
        "timer 1 every 1s{wait(2s)}",
    ] {
        let files = sources(source, &[]);
        for duration_only in [false, true] {
            let mut engine = Engine::default();
            let result = if duration_only {
                engine
                    .duration("main.sanim", &files, &BTreeMap::new())
                    .map(|_| ())
            } else {
                engine
                    .prepare("main.sanim", &files, &BTreeMap::new())
                    .map(|_| ())
            };
            assert!(result.is_err(), "{source} duration_only={duration_only}");
        }
    }
    compare("wait(7200s)", &[], Fraction::from(7200));
}
#[test]
fn import_scene_effects_and_positive_timing_cannot_hide_behind_fixed_duration() {
    for imported in [
        "let invalid=wait(1s)",
        "add(Circle())",
        "fn side():Unit{add(Circle())}\nside()",
    ] {
        let files = sources("import \"bad\" as bad", &[("bad.sanim", imported)]);
        assert_eq!(
            Engine::default()
                .duration("main.sanim", &files, &BTreeMap::new())
                .unwrap_err()
                .code,
            "S115"
        );
    }
}

#[test]
fn session_publishes_time_before_scene_validation() {
    let files = sources("add(Circle())\nwait(1s)", &[]);
    let mut session = sanim_lang::Session::new("main.sanim");
    session.engine.limits.max_elements = 0;
    let mut planned = None;
    let result = session.poll_with_duration(&files, |duration| planned = Some(duration));
    assert_eq!(planned, Some(Fraction::from(1)));
    assert!(matches!(result, sanim_lang::Reload::Failed { .. }));
}
#[test]
fn unused_reflected_defaults_do_not_load_irrelevant_timing_assets() {
    let files = sources(
        "let unused:Color=reflect(read<Color>(\"absent.json\"))\nwait(1s)",
        &[],
    );
    let report = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(report.duration, Fraction::from(1));
    assert!(!report.dependencies.contains_key("absent.json"));
}

#[test]
fn qualified_imports_propagate_value_and_timing_demands() {
    let assets = [
        (
            "base.sanim",
            "let delay=read<Duration>(\"delay.json\")\nlet values=read<List<Number>>(\"values.json\")\nfn count():Number{return len(values)}\nfn pause(length:Duration):Unit{wait(length)}",
        ),
        ("utils.sanim", "import \"base\" as base"),
        ("delay.json", "\"250ms\""),
        ("values.json", "[1,2,3]"),
    ];
    compare(
        "import \"base\" as base\nwait(base.delay)",
        &assets,
        Fraction::new(1u64, 4u64),
    );
    compare(
        "import \"base\" as base\nwait(base.count()*1s)",
        &assets,
        Fraction::from(3),
    );
    compare(
        "import \"utils\" as utils\nlet length=read<Duration>(\"delay.json\")\nutils.base.pause(length)",
        &assets,
        Fraction::new(1u64, 4u64),
    );
    compare(
        "import \"utils\" as utils\nwait(utils.base.count()*1s)",
        &assets,
        Fraction::from(3),
    );
}
#[test]
fn unused_reflected_overrides_are_validated_without_loading_defaults() {
    let files = sources(
        "let tint:Color=reflect(read<Color>(\"absent.json\"))\nwait(1s)",
        &[],
    );
    let report = Engine::default()
        .duration_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("main.sanim::tint".into(), serde_json::json!("blue"))]),
        )
        .unwrap();
    assert_eq!(report.duration, Fraction::from(1));
    assert!(
        Engine::default()
            .duration_json(
                "main.sanim",
                &files,
                &BTreeMap::from([("main.sanim::tint".into(), serde_json::json!(12))])
            )
            .is_err()
    );
}
#[test]
fn constant_boolean_short_circuiting_preserves_branch_time() {
    compare(
        "if false and true{wait(1s)}else{wait(2s)}",
        &[],
        Fraction::from(2),
    );
    compare(
        "if true or false{wait(1s)}else{wait(2s)}",
        &[],
        Fraction::from(1),
    );
}
#[test]
fn bounded_control_combinations_match_full_preparation() {
    for flag in [false, true] {
        for count in 0..4 {
            for repeats in 0..4 {
                let source = format!(
                    "fn phase(flag:Bool,count:Number,step:Duration):Unit{{repeat count{{if flag{{wait(step)}}else{{wait(step/2)}}}};timer 2 every 1s{{wait(100ms)}}}}\nrepeat {repeats}{{phase({flag},{count},200ms)}}"
                );
                let each = Fraction::from(2)
                    + Fraction::new(if flag { 2u64 } else { 1u64 }, 10u64) * Fraction::from(count);
                compare(&source, &[], each * Fraction::from(repeats));
            }
        }
    }
}

#[test]
fn value_helpers_can_return_whole_imported_records() {
    compare(
        "import \"base\" as base\nif str(base)==\"unused\"{wait(1s)}else{wait(2s)}",
        &[
            ("base.sanim", "let delay=read<Duration>(\"delay.json\")"),
            ("delay.json", "\"250ms\""),
        ],
        Fraction::from(2),
    );
}

#[test]
fn reveals_and_growth_have_the_same_bounded_duration_in_both_passes() {
    compare(
        "let shape=Circle()\nCreate(shape, duration=100ms, ease=\"linear\")\nUncreate(shape, duration=200ms, ease=\"linear\")\nWrite(shape, duration=300ms, ease=\"linear\")\nUnwrite(shape, duration=400ms, ease=\"linear\")\nGrowFromCenter(shape, duration=500ms, ease=\"linear\")\nShrinkToCenter(shape, duration=600ms, ease=\"linear\")",
        &[],
        Fraction::new(21u64, 10u64),
    );
}

#[test]
fn ordinary_calls_have_optional_exact_duration_effects() {
    compare(
        "fn value(x:Number=abs(-1,duration=200ms)):Number{return x}\nlet a=value()\nwait(a*100ms)",
        &[],
        Fraction::new(3u64, 10u64),
    );
    compare(
        "fn value(x:Number=abs(-1,duration=200ms)):Number{return x}\nlet a=value(1)\nwait(a*100ms)",
        &[],
        Fraction::new(1u64, 10u64),
    );
    compare(
        "type Pair={x:Number,y:Number}\nlet pair=Pair(x=1,y=2,duration=300ms)\nwait(pair.x*100ms)",
        &[],
        Fraction::new(2u64, 5u64),
    );
    compare(
        "type Window={duration:Duration}\nlet window=Window(duration=300ms)\nwait(window.duration)",
        &[],
        Fraction::new(3u64, 10u64),
    );
    compare(
        "let x=abs(-2,duration=300ms)\nlet color=Color(\"red3\",duration=200ms)\nwait(x*100ms)",
        &[],
        Fraction::new(7u64, 10u64),
    );
    compare(
        "fn twice(value:Number):Number{return value*2}\nlet x=twice(3,duration=400ms)\nwait(x*100ms)",
        &[],
        Fraction::from(1),
    );
    compare(
        "fn pulse():Unit{wait(200ms)}\npulse(duration=500ms)",
        &[],
        Fraction::new(1u64, 2u64),
    );
    compare(
        "fn pause(duration:Duration):Unit{wait(duration)}\npause(duration=300ms)",
        &[],
        Fraction::new(3u64, 10u64),
    );
    compare(
        "let delay=read<Duration>(\"delay.json\")\nlet x=abs(-2,duration=delay)\nwait(x*100ms)",
        &[("delay.json", "\"300ms\"")],
        Fraction::new(1u64, 2u64),
    );
}
#[test]
fn timed_call_envelopes_reject_overflow_and_too_short_bodies() {
    for source in [
        "let x=abs(-1,duration=7201s)",
        "fn pulse():Unit{wait(2s)}\npulse(duration=1s)",
        "let x=abs(-1,duration=-1s)",
        "let x=abs(-1,duration=1pt)",
    ] {
        let files = sources(source, &[]);
        for duration_only in [false, true] {
            let result = if duration_only {
                Engine::default()
                    .duration("main.sanim", &files, &BTreeMap::new())
                    .map(|_| ())
            } else {
                Engine::default()
                    .prepare("main.sanim", &files, &BTreeMap::new())
                    .map(|_| ())
            };
            assert!(result.is_err(), "{source}");
        }
    }
}
#[test]
fn zero_duration_calls_remain_pure_and_timed_cache_hits_preserve_time() {
    compare(
        "import \"utils\" as utils",
        &[("utils.sanim", "let idle=wait(0s)")],
        Fraction::from(0),
    );
    compare(
        "let x=reflect(abs(-2,duration=0s))\nsig y=sin(0deg,duration=0s)\nrepeat 3{let timed=abs(-2,duration=100ms)}",
        &[],
        Fraction::new(3u64, 10u64),
    );
    compare(
        "import \"utils\" as utils\nwait(utils.value*100ms)",
        &[("utils.sanim", "let value=abs(-2,duration=0s)")],
        Fraction::new(1u64, 5u64),
    );
    let files = sources("let x=reflect(abs(-2,duration=1s))", &[]);
    assert!(
        Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .is_err()
    );
}
