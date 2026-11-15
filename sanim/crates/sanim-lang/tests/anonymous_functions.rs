use sanim_core::{Fraction, Value};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;

fn run(source: &str) -> Result<Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{}\nAnimation()", source));
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}

#[test]
fn typed_anonymous_functions_call_and_capture_lexical_values() {
    let prepared=run("let offset=2\nlet double=fn(value:Number):Number{return value*2+offset}\nlet answer=double(20)\nassert(answer==42)").unwrap();
    assert_eq!(
        prepared.bindings["answer"],
        Value::Number(sanim_core::Number::scalar(42))
    );
}

#[test]
fn anonymous_callbacks_share_the_list_invoker_and_default_arguments() {
    run("let offset=2\nlet values=[1,2,3].map(fn(value:Number):Number{return value+offset})\nassert(values==[3,4,5])\nlet increment=fn(value:Number,step:Number=2):Number{return value+step}\nassert(increment(40)==42)").unwrap();
}

#[test]
fn captures_of_mutable_cells_remain_live_and_timeline_calls_are_bounded() {
    let source = "mut count=1\nlet read_count=fn<ref>():Number{return count}\nassert(read_count()==1)\ncount=2\nassert(read_count()==2)\nlet delay=fn(time:Duration):Animation{return wait(time)}\ndelay(1s)\ndelay(500ms)";
    let prepared = run(source).unwrap();
    assert_eq!(prepared.animation.duration, Fraction::new(3_u64, 2_u64));
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{}\nAnimation()", source));
    let duration = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(duration.duration, prepared.animation.duration);
}

#[test]
fn returned_functions_capture_each_factory_scope_without_aliasing() {
    run("fn make(offset:Number):fn(value:Number):Number{return fn(value:Number):Number{return value+offset}}\nlet first=make(1)\nlet second=make(2)\nassert(first(40)==41)\nassert(second(40)==42)\nassert(make(3)(39)==42)").unwrap();
}

#[test]
fn anonymous_signal_parameters_and_returns_keep_live_dependencies() {
    run("mut count=1\nlet double=fn<sig>(sig value:Number):sig Number{return value*2}\nlet live=double(count)\nassert(live==2)\ncount=2\nassert(live==4)").unwrap();
}

#[test]
fn callable_annotations_allow_higher_order_arguments_and_generic_captures() {
    run("fn apply(callback:fn(value:Number):Number,input:Number):Number{return callback(input)}\nfn capture<T>(offset:T):fn():T{return fn():T{return offset}}\nlet identity=fn<T>(value:T):T{return value}\nassert(identity(42)==42)\nassert(apply(fn(value:Number):Number{return value*2},21)==42)\nlet captured=capture(42)\nassert(captured()==42)").unwrap();
}

#[test]
fn escaped_mutable_captures_and_reduce_callbacks_remain_valid() {
    run("fn counter():fn<mut, ref>():Number{mut count=0;return fn<mut, ref>():Number{count=count+1;return count}}\nlet next=counter()\nassert(next()==1)\nassert(next()==2)\nlet total=[1,2,3].reduce(fn(sum:Number,value:Number):Number{return sum+value})\nassert(total==6)").unwrap();
}

#[test]
fn closure_duration_evaluates_local_duration_expressions_exactly() {
    let source =
        "let delay=fn<mut>(time:Duration):Unit{let length=time*2;wait(length)}\ndelay(500ms)";
    let prepared = run(source).unwrap();
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{}\nAnimation()", source));
    assert_eq!(
        Engine::default()
            .duration("main.sanim", &files, &BTreeMap::new())
            .unwrap()
            .duration,
        prepared.animation.duration
    );
}

#[test]
fn closure_checkpoints_restore_without_reexecuting_captured_prefix() {
    let prefix = "fn make(offset:Number):fn(value:Number):Number{return fn(value:Number):Number{return value+offset}}\nlet captured=make(2)\nlet numbers=[1,2,3].map(captured)\n";
    let mut engine = Engine::default();
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        format!("{prefix}assert(captured(40)==42)\nwait(1s)"),
    );
    let before = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    files.insert(
        "main.sanim",
        format!("{prefix}assert(captured(41)==43)\nwait(2s)"),
    );
    let after = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(after.statistics.restored_statements >= 2);
    assert_eq!(after.bindings["numbers"], before.bindings["numbers"]);
    assert_eq!(after.animation.duration, Fraction::from(2));
}

#[test]
fn anonymous_functions_preserve_diagnostics_and_signal_purity() {
    for (source, expected) in [
        (
            "let f=fn(value:Number):Number{return value}\nf(\"wrong\")",
            "expected Number",
        ),
        ("let f=fn():Number{let missing=1}", "must return"),
        (
            "let f=fn(Color:Number):Number{return Color}",
            "protected Sanim name",
        ),
        ("mut f=fn():Number{return 1}", "immutable let binding"),
        (
            "mut count=0\nsig values=[1].map(fn<mut, ref>(value:Number):Number{count=count+1;return value})",
            "sig captures must be pure",
        ),
        (
            "let value=reflect((fn<mut>():Number{check(true);return 1})())",
            "reflected default",
        ),
    ] {
        let error = run(source).unwrap_err();
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}

#[test]
fn anonymous_source_formats_and_reparses_without_semantic_changes() {
    let source =
        "let twice=fn(value:Number,step:Number=2):Number{return value*step}\nassert(twice(21)==42)";
    let formatted =
        sanim_lang::formatter::format_source("main.sanim", source, Default::default(), 1_000_000)
            .unwrap();
    run(&formatted).unwrap();
    assert_eq!(
        formatted,
        sanim_lang::formatter::format_source(
            "main.sanim",
            &formatted,
            Default::default(),
            1_000_000
        )
        .unwrap()
    );
}

#[test]
#[ignore = "performance measurement"]
fn closure_reload_benchmark() {
    use std::time::Instant;
    const EDITS: usize = 80;
    for (name, prefix) in [
        (
            "named",
            "fn double(value:Number):Number{return value*2}\nlet values=range(2000).map(double)\n",
        ),
        (
            "closure",
            "let factor=2\nlet double=fn(value:Number):Number{return value*factor}\nlet values=range(2000).map(double)\n",
        ),
    ] {
        let mut files = MemorySources::default();
        let mut engine = Engine::default();
        files.insert("main.sanim", format!("{prefix}wait(1ms)"));
        let start = Instant::now();
        let cold = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        let cold_us = start.elapsed().as_micros();
        let mut times = Vec::new();
        let mut max_bytes = 0;
        let mut min_bytes = usize::MAX;
        for edit in 0..EDITS {
            files.insert("main.sanim", format!("{prefix}wait({}ms)", edit + 2));
            let start = Instant::now();
            let updated = engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap();
            times.push(start.elapsed().as_micros());
            assert!(
                updated.statistics.restored_statements >= if name == "named" { 1 } else { 3 },
                "{name}: {:?}",
                updated.statistics
            );
            assert!(updated.statistics.operations < cold.statistics.operations / 100);
            if edit >= EDITS / 2 {
                min_bytes = min_bytes.min(updated.statistics.checkpoint_bytes);
                max_bytes = max_bytes.max(updated.statistics.checkpoint_bytes);
            }
            assert!(updated.statistics.checkpoint_bytes < engine.limits.max_cache_bytes);
        }
        times.sort_unstable();
        assert!(
            max_bytes - min_bytes < 4096,
            "suffix edits must not accumulate closure checkpoints: {min_bytes}..{max_bytes}"
        );
        println!(
            "{name}: cold_us={cold_us} suffix_median_us={} suffix_p95_us={} checkpoint_bytes={min_bytes}..{max_bytes} cold_ops={}",
            times[EDITS / 2],
            times[EDITS * 95 / 100],
            cold.statistics.operations
        );
    }
}

#[test]
fn write_only_captures_and_animated_cells_share_the_lexical_store() {
    run("mut count=0\nlet set=fn<mut>():Unit{count=42}\nset()\nassert(count==42)").unwrap();
}

#[test]
fn anonymous_function_scopes_are_available_to_the_native_code_view() {
    let prepared=run("let view=CodeView(code=r'''let double=fn(value:Number):Number {\n return value*2\n}''',lang=\"sanim\")\nAnimation(children=[view])").unwrap();
    let sanim_core::Geometry::Code { document, .. } =
        prepared.animation.find("view").unwrap().geometry.as_ref()
    else {
        panic!("CodeView")
    };
    assert!(document.diagnostics.is_empty());
    assert!(
        document
            .scopes
            .iter()
            .any(|scope| scope.label.contains("anonymous")
                && scope.start_line == 1
                && scope.end_line >= 3)
    );
}

#[test]
fn bounded_calls_reject_timeline_overflow_for_anonymous_callbacks() {
    let error = run(
        "let delay=fn(value:Number):Animation{return wait(60m)}\nAnimation(children=[1,2,3].map(delay),flow=\"sequence\")",
    )
    .unwrap_err();
    assert!(
        error.message.contains("two-hour")
            || error.message.contains("7200")
            || error.message.contains("2-hour"),
        "{error}"
    );
}

#[test]
fn forward_helper_effects_follow_inferred_closure_aliases() {
    run("mut count=0\nlet f=fn<mut, ref>():Number{return bump()}\nlet alias=f\nfn<mut, ref> bump():Number{count=count+1;return count}\nlet value=alias()\nassert(value==1)").unwrap();
    let source = "mut count=0\nlet f=fn<mut, ref>():Number{return bump()}\nlet alias=f\nfn<mut, ref> bump():Number{count=count+1;return count}\nsig value=alias()";
    let error = run(source).unwrap_err();
    assert_eq!(error.code, "S131", "{error}");
    assert!(error.message.contains("pure"), "{error}");
    run("let pause=fn<mut>():Unit{later()}\nfn<mut> later():Unit{wait(1s)}\npause()").unwrap();
}

#[test]
fn forward_effects_survive_generic_identity_and_named_function_values() {
    run("mut count=0\nfn identity<T>(value:T):T{return value}\nlet f=fn<mut, ref>():Number{return bump()}\nlet alias=identity(f)\nfn<mut, ref> bump():Number{count=count+1;return count}\nlet value=alias()\nassert(value==1)").unwrap();
    run("mut count=0\nlet f=bump\nfn<mut, ref> bump():Number{count=count+1;return count}\nlet value=f()\nassert(value==1)").unwrap();
}

#[test]
fn captured_generic_types_share_storage_and_obey_retained_budgets() {
    let fields = (0..64)
        .map(|i| format!("field_{i}:Number"))
        .collect::<Vec<_>>()
        .join(",");
    let types = format!("type Wide={{{fields}}}\n");
    let mut files = MemorySources::default();
    files.insert("main.sanim",format!("{types}fn helper():fn():Number{{return fn():Number{{return 1}}}}\nfn make<T>():fn():Number{{return helper()}}\nlet held=make<Wide>()\nAnimation()"));
    let mut engine = Engine::default();
    engine.limits.data.max_bytes = 2048;
    let error = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S303", "{error}");
    assert!(error.message.contains("budget"), "{error}");
    files.insert("main.sanim",format!("{types}fn<mut> make<T>():Unit{{repeat 50{{let next=fn():Number{{return 1}};assert(next()==1)}}}}\nmake<Wide>()\nAnimation()"));
    engine.limits.data.max_bytes = 128 * 1024;
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
}
