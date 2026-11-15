use sanim_core::{Fraction, Property, Value};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn sources(code: &str) -> MemorySources {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", code);
    sources
}

#[test]
fn forever_is_a_pure_plan_value_and_not_lexical_control_flow() {
    let files = sources(
        "fn stay():Animation{return wait(forever)}\nlet hold=stay()\nassert(true)\nreturn hold",
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(prepared.animation.indefinite);
    assert_eq!(prepared.animation.duration, Fraction::from(0));
    assert!(
        prepared.symbols["stay"].signatures()[0]
            .capabilities
            .is_pure()
    );
    let files = sources("let hold=wait(forever)\nassert(false)\nreturn hold");
    assert_eq!(
        Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S306"
    );
}

#[test]
fn cyclic_signal_plan_is_compact_and_external_relayout_reuses_it() {
    let files = sources(
        "mut phase=0deg\nsig size=64pt+16pt*sin(phase)\nlet shape=Box(width=size,height=size)\nlet pulse=animate(phase,to=360deg,duration=2s,ease=\"linear\").repeat(forever)\nreturn Animation(children=[shape,pulse])",
    );
    let mut engine = Engine::default();
    engine.available_duration = Some(10.into());
    let prepared = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(!prepared.animation.indefinite);
    assert_eq!(prepared.animation.duration, Fraction::from(10));
    let at = |time| {
        prepared.animation.sample(time).unwrap()[0]
            .property(Property::Width)
            .unwrap()
    };
    assert_eq!(at(Fraction::new(1u64, 2u64)), at(Fraction::new(5u64, 2u64)));
    let resized = prepared
        .relayout(&sanim_core::Viewport::default(), Some(20.into()))
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&prepared.plan, &resized.plan));
    assert_eq!(resized.statistics.operations, 0);
    assert_eq!(resized.animation.duration, Fraction::from(20));
}

#[test]
fn infinite_repeat_rejects_zero_cycles_and_finite_math_keeps_its_bound() {
    for code in [
        "return wait(0s).repeat(forever)",
        "return wait(120m1ms)",
        "repeat forever {}\nreturn wait(1s)",
    ] {
        let files = sources(code);
        assert!(
            Engine::default()
                .prepare("main.sanim", &files, &BTreeMap::new())
                .is_err(),
            "{code}"
        );
    }
    assert_eq!(Value::Forever.ty().to_string(), "Forever");
}

#[test]
#[ignore = "release performance/memory benchmark"]
fn reflow_reuses_builder_and_bounds_retained_revisions() {
    use std::{sync::Arc, time::Instant};
    let mut files = sources(
        "type Row={value:Number}\nlet samples=read<List<Row>>(\"samples.csv\")\nassert(len(samples)==32768)\nlet values=range(1000).map(fn(value:Number):Animation{return wait(1ms)})\nreturn Animation(children=values,flow=\"sequence\",justify=\"between\")",
    );
    files.insert(
        "samples.csv",
        format!(
            "value\n{}",
            (0..32768)
                .map(|value| format!("{value}\n"))
                .collect::<String>()
        ),
    );
    let mut engine = Engine::default();
    let prepared = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let weak = Arc::downgrade(&prepared.plan);
    let owners = Arc::strong_count(&prepared.plan);
    let mut reflow = Vec::new();
    let mut rebuilt = Vec::new();
    for step in 0..32 {
        let duration = Fraction::from(2 + step % 3);
        let start = Instant::now();
        let changed = prepared
            .relayout(&sanim_core::Viewport::default(), Some(duration))
            .unwrap();
        reflow.push(start.elapsed().as_micros());
        assert_eq!(changed.statistics.operations, 0);
        assert!(Arc::ptr_eq(&prepared.plan, &changed.plan));
        drop(changed);
        engine.available_duration = Some(duration);
        let start = Instant::now();
        let changed = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        rebuilt.push(start.elapsed().as_micros());
        drop(changed);
    }
    reflow.sort_unstable();
    rebuilt.sort_unstable();
    println!(
        "1000-node temporal viewport, 32768-row CSV: reflow median={}us p95={}us, cached builder prepare median={}us p95={}us",
        reflow[16], reflow[30], rebuilt[16], rebuilt[30]
    );
    assert!(
        reflow[16] < rebuilt[16],
        "plan-only reflow must beat source preparation on this workload"
    );
    assert_eq!(
        Arc::strong_count(&prepared.plan),
        owners,
        "completed reflows must not retain additional plan revisions"
    );
    drop(prepared);
    drop(engine);
    assert!(
        weak.upgrade().is_none(),
        "all cached plans release with their owner; no global retention"
    );
}

#[test]
fn root_collects_expression_results_once_in_execution_order() {
    let files = sources(
        r#"let item=Box()
let ignored=item.fade_in(duration=99s)
<Animation>{item.fade_in(duration=1s)}</Animation>
repeat 2 {wait(500ms)}
for n in range(2) {wait(250ms)}
item.fade_out(duration=1s)"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.animation.duration, Fraction::new(7u64, 2u64));
    let tracks = &prepared.animation.tracks["item"][&Property::Opacity];
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[1].start, Fraction::new(5u64, 2u64));
    assert_eq!(
        prepared
            .time_map
            .event_index()
            .neighbors(Fraction::new(5u64, 2u64)),
        [Some(Fraction::new(5u64, 2u64)); 2]
    );
}

#[test]
fn functions_and_imports_do_not_collect_discarded_plans() {
    let mut files = sources("import \"helper\" as helper\nhelper.ignored()\nwait(1s)");
    files.insert("helper.sanim", "wait(50s)\nfn ignored():Unit{wait(99s)}");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.animation.duration, Fraction::from(1));
    let lint = Engine::default().lint("main.sanim", &files).unwrap();
    assert!(
        lint.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "L003")
    );
    let files = sources("fn helper():Animation{return wait(1s)}");
    assert!(Engine::default().lint("main.sanim", &files).is_ok());
    assert_eq!(
        Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S134"
    );
}

#[test]
fn explicit_root_return_stops_entry_without_collecting_function_internals() {
    let files = sources(
        "fn step():Animation{wait(99s);return wait(2s)}\nwait(1s)\nreturn step()\nassert(false)",
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.animation.duration, Fraction::from(3));
}

#[test]
fn inline_code_construction_is_pure_but_used_source_reads_require_ref() {
    let files = sources(
        "fn panel():Animation{let view=CodeView(code=\"hello\",lang=\"plain\");return Animation(children=[view,view.style(line_numbers=true)])}\nreturn panel()",
    );
    Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let files = sources(
        "fn panel():Animation{return Animation(children=[CodeView(src=\"absent.py\")])}\nreturn panel()",
    );
    assert_eq!(
        Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S133"
    );
}

#[test]
fn cropped_code_successor_fails_before_a_visual_jump() {
    let files = sources(
        "let view=CodeView(code=\"hello\",lang=\"plain\")\nlet first=Animation(children=[view.style(line_numbers=true,duration=3s)],duration=1s)\nreturn Animation(children=[first,view.style(line_numbers=false,duration=1s)],flow=\"sequence\")",
    );
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("clipped before completion"),
        "{error}"
    );
}

#[test]
fn live_list_cell_plans_update_visual_snapshots() {
    let files = sources(
        "let one=Box(width=10pt,height=10pt)\nlet two=Box(width=10pt,height=10pt)\nmut items:List<Element>=[one]\nlet view=Box(children=items,flex_direction=\"row\")\nreturn Animation(children=[view,animate(items,to=[two],duration=2s)])",
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let (width, height) = prepared.animation.logical_size();
    let middle = prepared.animation.layout(1.into(), width, height).unwrap();
    assert_eq!(middle[0].geometry.children().unwrap().len(), 2);
    let end = prepared.animation.layout(2.into(), width, height).unwrap();
    let children = end[0].geometry.children().unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].id, "two");
}

#[test]
fn infinite_sources_remain_extendable_after_a_finite_host_crop() {
    use sanim_core::plan::Extent;
    for source in [
        "wait(forever)",
        "let item=Box()\nreturn item.fade_in(duration=1s).repeat(forever)",
    ] {
        let files = sources(source);
        let mut engine = Engine::default();
        engine.available_duration = Some(2.into());
        let prepared = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(prepared.natural_extent(), Extent::Infinite);
        assert_eq!(prepared.source_extent(), None);
        assert!(!prepared.animation.indefinite);
        let expanded = prepared
            .relayout(&sanim_core::Viewport::default(), Some(18000.into()))
            .unwrap();
        assert_eq!(expanded.animation.duration, Fraction::from(18000));
        assert_eq!(expanded.natural_extent(), Extent::Infinite);
        assert_eq!(expanded.source_extent(), None);
        assert!(std::sync::Arc::ptr_eq(&prepared.plan, &expanded.plan));
    }
    let files = sources("return wait(forever)");
    let mut engine = Engine::default();
    engine.available_duration = Some(18000.into());
    engine.limits.max_time_map_bytes = 4096;
    let report = engine
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(report.duration, Fraction::from(18000));
    assert_eq!(report.frames, 540000);
    let files = sources("wait(180m)");
    assert!(
        engine
            .duration("main.sanim", &files, &BTreeMap::new())
            .is_err()
    );
}

#[test]
fn finite_content_uses_resolved_host_bounds_for_end_alignment() {
    use sanim_core::plan::Extent;
    let files = sources(
        "let item=Box()\nreturn Animation(target=item,children=[item.fade_in(duration=1s,ease=\"linear\")],justify=\"end\")",
    );
    let mut engine = Engine::default();
    engine.available_duration = Some(6.into());
    let prepared = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.natural_extent(), Extent::Finite(1.into()));
    assert_eq!(prepared.source_extent(), Some(6.into()));
    assert_eq!(
        prepared.animation.tracks["item"][&Property::Opacity][0].start,
        Fraction::from(5)
    );
    let expanded = prepared
        .relayout(&sanim_core::Viewport::default(), Some(18000.into()))
        .unwrap();
    assert_eq!(
        expanded.animation.tracks["item"][&Property::Opacity][0].start,
        Fraction::from(17999)
    );
    assert_eq!(expanded.source_extent(), Some(18000.into()));
}

#[test]
fn missing_external_parent_and_cyclic_parent_constraints_have_distinct_diagnostics() {
    for (source, code) in [
        ("return Animation(duration=\"match_parent\")", "S138"),
        (
            "return Animation(children=[Animation(duration=\"match_parent\")])",
            "S139",
        ),
    ] {
        let files = sources(source);
        for measure in [true, false] {
            let mut engine = Engine::default();
            let error = if measure {
                engine
                    .duration("main.sanim", &files, &BTreeMap::new())
                    .unwrap_err()
            } else {
                engine
                    .prepare("main.sanim", &files, &BTreeMap::new())
                    .unwrap_err()
            };
            assert_eq!(error.code, code, "{error}");
            assert!(
                error.span.end > error.span.start,
                "root diagnostic retains the constructor location"
            );
        }
    }
}

#[test]
fn failed_reflows_remain_diagnostic_until_repaired_and_preserve_source_failures() {
    use sanim_lang::{Reload, Session};
    let source = "let item=Box()\nreturn Animation(children=[item.fade_in(duration=\"match_parent\")],duration=\"match_parent\")";
    let mut files = sources(source);
    let mut session = Session::new("main.sanim");
    session.engine.available_duration = Some(1.into());
    assert!(matches!(session.poll(&files), Reload::Updated { .. }));
    let plan = session.current().unwrap().plan.clone();
    let viewport = sanim_core::Viewport::default();
    assert!(matches!(
        session.relayout(&viewport, Some(7201.into())),
        Reload::Failed { .. }
    ));
    assert!(session.last_error().is_some());
    assert!(matches!(session.poll(&files), Reload::Unchanged));
    assert!(session.last_error().is_some());
    assert!(matches!(
        session.relayout(&viewport, Some(2.into())),
        Reload::Updated { .. }
    ));
    assert!(session.last_error().is_none());
    assert!(std::sync::Arc::ptr_eq(
        &plan,
        &session.current().unwrap().plan
    ));
    files.insert("main.sanim", "let invalid=(");
    assert!(matches!(session.poll(&files), Reload::Failed { .. }));
    let error = session.last_error().unwrap().clone();
    assert!(matches!(
        session.relayout(&viewport, Some(3.into())),
        Reload::Updated { .. }
    ));
    assert_eq!(session.last_error(), Some(&error));
    files.insert("main.sanim", source);
    session.poll(&files);
    assert!(session.last_error().is_none());
}

#[test]
fn imported_shared_plan_dags_do_not_expand_into_checkpoint_context_keys() {
    use std::time::{Duration, Instant};
    let mut files = sources("import \"library\" as library\nwait(1s)\n");
    let mut library = String::from("let p0=wait(1ms)\n");
    for depth in 1..=16 {
        library.push_str(&format!(
            "let p{depth}=Animation(children=[p{},p{}])\n",
            depth - 1,
            depth - 1
        ));
    }
    files.insert("library.sanim", library);
    let mut engine = Engine::default();
    engine.limits.max_cache_bytes = 65536;
    let started = Instant::now();
    let prepared = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "checkpoint identity must scale with physical source/value inputs, not repeated DAG occurrences"
    );
    assert!(prepared.statistics.operations < 256);
    assert!(prepared.statistics.checkpoint_bytes <= engine.limits.max_cache_bytes);
    files.insert(
        "main.sanim",
        "import \"library\" as library\nwait(1s)\nwait(1s)",
    );
    let changed = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(changed.statistics.restored_statements > 0);
    assert_eq!(changed.animation.duration, Fraction::from(2));
}

#[test]
fn imported_function_and_data_versions_invalidate_restored_semantic_contexts() {
    let mut files =
        sources("import \"library\" as library\nlet delay=library.value\nwait(delay*1s)");
    files.insert(
        "library.sanim",
        "fn compute():Number{return 1}\nlet value=compute()",
    );
    let mut engine = Engine::default();
    assert_eq!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap()
            .animation
            .duration,
        Fraction::from(1)
    );
    files.insert(
        "library.sanim",
        "fn compute():Number{return 2}\nlet value=compute()",
    );
    assert_eq!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap()
            .animation
            .duration,
        Fraction::from(2)
    );
    files.insert("library.sanim", "let value=read<Number>(\"value.json\")");
    files.insert("value.json", "3");
    assert_eq!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap()
            .animation
            .duration,
        Fraction::from(3)
    );
    files.insert("value.json", "4");
    assert_eq!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap()
            .animation
            .duration,
        Fraction::from(4)
    );
}

#[test]
fn restored_allocation_memos_preserve_the_cold_preparation_budget() {
    let prefix = "fn maker():fn(value:Number):Number{return fn(value:Number):Number{return value}}\nlet first=maker()\nassert(first(1)==1)\nwait(1s)\n";
    let expanded = format!("{prefix}let second=maker()\nassert(second(2)==2)\nwait(1s)");
    let mut files = sources(&expanded);
    let mut lower = 1024usize;
    let mut upper = 65536usize;
    while upper - lower > 1 {
        let middle = lower + (upper - lower) / 2;
        let mut engine = Engine::default();
        engine.limits.data.max_bytes = middle;
        if engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .is_ok()
        {
            upper = middle;
        } else {
            lower = middle;
        }
    }
    let mut cold = Engine::default();
    cold.limits.data.max_bytes = upper;
    cold.prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let mut engine = Engine::default();
    engine.limits.data.max_bytes = upper;
    files.insert("main.sanim", prefix);
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    files.insert("main.sanim", expanded);
    let changed = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(
        changed.statistics.restored_statements > 0,
        "regression must exercise a real restored prefix"
    );
    assert_eq!(changed.animation.duration, Fraction::from(2));
}

#[cfg(feature = "native-text")]
#[test]
fn reflow_rechecks_visual_contracts_with_the_canonical_native_text_resolver() {
    let files = sources(
        "mut words=\"a\"\nlet label=Label(words)\nassert(inside_canvas(label))\nreturn Animation(children=[label,animate(words,to=\"b\",duration=0s)])",
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let reflowed = prepared
        .relayout(&sanim_core::Viewport::default(), Some(1.into()))
        .unwrap();
    assert_eq!(reflowed.contracts.len(), 1);
}

#[test]
fn external_crops_preserve_infinite_source_prototypes_and_offset_phase() {
    let files = sources(
        "mut size=10pt\nlet shape=Box(width=size,height=10pt)\nreturn Animation(children=[shape,animate(size,to=30pt,duration=2s,ease=\"linear\")]).repeat(forever)",
    );
    let mut engine = Engine::default();
    engine.available_duration = Some(Fraction::new(1u64, 2u64));
    let prepared = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.animation.duration, Fraction::new(1u64, 2u64));
    for time in [Fraction::new(3u64, 2u64), Fraction::new(11u64, 2u64)] {
        let sampled = prepared.animation.sample(time).unwrap();
        assert_eq!(
            sampled[0].property(Property::Width).unwrap(),
            Value::Number(sanim_core::Number {
                value: 25.into(),
                unit: sanim_core::Unit::Length
            })
        );
    }
    assert_eq!(prepared.time_map.indexed_frames(), None);
    assert!(prepared.time_map.at_frame(165).is_some());
}

#[test]
fn unhosted_infinite_source_maps_resolve_late_holds_and_original_child_lines() {
    let files = sources("wait(forever)");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.time_map.indexed_frames(), None);
    assert_eq!(prepared.time_map.at_frame(3_000_000).unwrap().span.line, 1);
    let files = sources(
        "let item=Box()\nlet reveal=item.fade_in(duration=2s)\nreturn reveal.repeat(forever)",
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.time_map.at_frame(3030).unwrap().span.line, 2);
}

#[test]
fn reflection_infers_the_editable_duration_from_a_mixed_wait_context() {
    let files = sources("wait(reflect(200ms,key=\"tail\"))");
    let mut engine = Engine::default();
    let prepared = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        prepared.reflected[0].ty.as_ref(),
        &sanim_core::Type::Numeric(sanim_core::Unit::Duration)
    );
    let changed = engine
        .prepare(
            "main.sanim",
            &files,
            &BTreeMap::from([(
                "tail".into(),
                Value::Number(sanim_core::Number::parse("400ms").unwrap()),
            )]),
        )
        .unwrap();
    assert_eq!(changed.animation.duration, Fraction::new(2u64, 5u64));
}

#[test]
fn variable_length_numeric_cell_tracks_commit_at_the_endpoint() {
    let files = sources("mut values:List<Number>=[]\nreturn animate(values,to=[1,2],duration=1s)");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let sanim_core::plan::Node::Cell { cell, .. } = prepared.plan.node.as_ref() else {
        panic!("expected the returned cell plan")
    };
    let cell = *cell;
    assert_eq!(
        prepared
            .animation
            .reactive
            .sample_cell(cell, Fraction::new(1u64, 2u64))
            .unwrap()
            .list()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        prepared
            .animation
            .reactive
            .sample_cell(cell, 1.into())
            .unwrap()
            .list()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn demand_defers_unused_data_pipelines_only_for_pure_callbacks() {
    let files = sources(
        "fn identity(value:Number):Number{return value}\nlet unused=read<List<Number>>(\"absent.json\").map(identity)\nwait(1s)",
    );
    let mut engine = Engine::default();
    engine.preparation = sanim_lang::Preparation::Demand;
    let prepared = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(
        prepared
            .deferred_bindings
            .iter()
            .any(|binding| binding.name == "unused")
    );
    assert!(!prepared.dependencies.contains_key("absent.json"));
    assert!(
        Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .is_err()
    );
}

#[test]
#[ignore = "release selective-data-laziness benchmark"]
fn unused_pure_callback_pipeline_benchmark() {
    use std::time::Instant;
    let mut files = sources(
        "fn identity(value:Number):Number{return value}\nlet unused=read<List<Number>>(\"items.json\").map(identity)\nwait(1s)",
    );
    files.insert(
        "items.json",
        serde_json::to_string(&(0..4096).collect::<Vec<_>>()).unwrap(),
    );
    let mut timings = [Vec::new(), Vec::new()];
    for _ in 0..7 {
        for (index, mode) in [
            sanim_lang::Preparation::Demand,
            sanim_lang::Preparation::Full,
        ]
        .into_iter()
        .enumerate()
        {
            let mut engine = Engine::default();
            engine.preparation = mode;
            let start = Instant::now();
            let prepared = engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap();
            timings[index].push(start.elapsed().as_micros());
            if index == 0 {
                assert!(!prepared.dependencies.contains_key("items.json"));
            }
        }
    }
    for values in &mut timings {
        values.sort_unstable();
    }
    println!(
        "unused 4096-row pure map: Demand median={}us Full median={}us",
        timings[0][3], timings[1][3]
    );
    assert!(timings[0][3] < timings[1][3]);
}

#[test]
fn inline_code_queries_resolve_after_previous_patch_plans() {
    let mut files = sources(
        "let view=CodeView(code=\"old\\n\",lang=\"plain\")\nview.patch(src=\"next.txt\",duration=100ms)\nview.range(regex=\"new\").edit(code=\"final\",duration=100ms)",
    );
    files.insert("next.txt", "new\n");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        prepared.animation.code_views["view"].document.text.as_ref(),
        "final\n"
    );
    assert!(prepared.dependencies.contains_key("next.txt"));
}

#[test]
fn bound_code_ranges_remain_immutable_snapshot_queries() {
    let files = sources(
        "let view=CodeView(code=\"old\",lang=\"plain\")\nlet absent=view.range(regex=\"new\")\nwait(1s)",
    );
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(error.message.contains("matched nothing"), "{error}");
}

#[test]
fn late_code_query_failures_identify_the_actual_selector_action() {
    let mut files = sources(
        "let view=CodeView(code=\"old\",lang=\"plain\")\nview.patch(src=\"next.txt\",duration=100ms)\nview.range(regex=\"absent\").select(duration=100ms)",
    );
    files.insert("next.txt", "new");
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S304");
    assert_eq!(error.source, "main.sanim");
    assert_eq!(error.span.line, 3);
    assert!(error.message.contains("matched nothing"), "{error}");
}
