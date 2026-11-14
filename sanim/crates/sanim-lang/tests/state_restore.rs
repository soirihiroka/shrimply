use sanim_core::{Fraction, Span, Value};
use sanim_lang::{Engine, MemorySources};
use std::{collections::BTreeMap, sync::Arc};

fn prepare(engine: &mut Engine, files: &MemorySources) -> sanim_lang::Prepared {
    engine
        .prepare("main.sanim", files, &BTreeMap::new())
        .unwrap()
}

fn semantic_output(prepared: &sanim_lang::Prepared) -> serde_json::Value {
    serde_json::to_value((
        &prepared.animation,
        &prepared.bindings,
        &prepared.reflected,
        &prepared.checks,
        &prepared.contracts,
    ))
    .unwrap()
}

#[test]
fn suffix_edit_restores_bindings_tracks_cursor_and_skips_prefix_execution() {
    let mut files = MemorySources::default();
    let prefix = "let c=Circle(id=\"dot\")\nrepeat 100 { let unused=1+2 }\nFadeIn(c, duration=1s, ease=\"linear\")\n";
    files.insert("main.sanim", format!("{prefix}wait(1s)\n"));
    let mut engine = Engine::default();
    let old = prepare(&mut engine, &files);
    let boundary = engine.execution_state(3).unwrap();
    assert_eq!(boundary.cursor, Fraction::from(1));
    files.insert("main.sanim", format!("{prefix}wait(2s)\n"));
    let edited = prepare(&mut engine, &files);
    assert_eq!(edited.statistics.restored_statements, 3);
    assert!(edited.statistics.operations < old.statistics.operations / 4);
    assert_eq!(edited.animation.duration, Fraction::from(3));
    assert_eq!(edited.animation.tracks, old.animation.tracks);
    assert!(Arc::ptr_eq(&boundary, &engine.execution_state(3).unwrap()));
    let fresh = prepare(&mut Engine::default(), &files);
    assert_eq!(edited.fingerprint, fresh.fingerprint);
    assert_eq!(semantic_output(&edited), semantic_output(&fresh));
    assert_eq!(edited.bindings, fresh.bindings);
}

#[test]
fn data_changes_resume_before_the_first_resource_read() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "add(Circle(id=\"early\"))\nwait(1s)\nlet delay=read<Duration>(\"delay.json\")\nwait(delay)\n");
    files.insert("delay.json", "\"1s\"");
    let mut engine = Engine::default();
    prepare(&mut engine, &files);
    let before_read = engine.execution_state(2).unwrap();
    files.insert("delay.json", "\"2s\"");
    let edited = prepare(&mut engine, &files);
    assert_eq!(edited.statistics.restored_statements, 2);
    assert_eq!(edited.statistics.decoded_assets, 1);
    assert_eq!(edited.animation.duration, Fraction::from(3));
    assert!(Arc::ptr_eq(
        &before_read,
        &engine.execution_state(2).unwrap()
    ));
    assert_eq!(
        semantic_output(&edited),
        semantic_output(&prepare(&mut Engine::default(), &files))
    );
}

#[test]
fn changed_hoisted_helper_and_default_resume_before_their_first_call() {
    for (old_helper, new_helper) in [
        (
            "fn tail():Duration { return 1s }",
            "fn tail():Duration { return 2s }",
        ),
        (
            "fn tail(value:Duration=1s):Duration { return value }",
            "fn tail(value:Duration=2s):Duration { return value }",
        ),
    ] {
        let mut files = MemorySources::default();
        files.insert(
            "main.sanim",
            format!("wait(1s)\n{old_helper}\nwait(tail())\n"),
        );
        let mut engine = Engine::default();
        prepare(&mut engine, &files);
        files.insert(
            "main.sanim",
            format!("wait(1s)\n{new_helper}\nwait(tail())\n"),
        );
        let edited = prepare(&mut engine, &files);
        assert_eq!(edited.statistics.restored_statements, 1);
        assert_eq!(edited.animation.duration, Fraction::from(3));
        assert_eq!(
            semantic_output(&edited),
            semantic_output(&prepare(&mut Engine::default(), &files))
        );
    }
}

#[test]
fn changed_retained_helper_body_default_and_transitive_calls_match_fresh_graphs() {
    for (old_helper, new_helper, boundary) in [
        (
            "fn double(value:Length):Length{return value*2}",
            "fn double(value:Length):Length{return value*3}",
            3,
        ),
        (
            "fn double(value:Length,scale:Number=2):Length{return value*scale}",
            "fn double(value:Length,scale:Number=3):Length{return value*scale}",
            3,
        ),
        (
            "fn factor():Number{return 2}\nfn double(value:Length):Length{return value*factor()}",
            "fn factor():Number{return 3}\nfn double(value:Length):Length{return value*factor()}",
            3,
        ),
        (
            "fn double(sig value:Length):sig Length{return value*2}",
            "fn double(sig value:Length):sig Length{return value*3}",
            2,
        ),
    ] {
        let program = |helper| {
            format!(
                "wait(1s)\n{helper}\nmut size=10pt\nsig result=double(size)\nlet marker=Circle(radius=result)\nadd(marker)\nanimate(size,to=20pt,duration=1s,ease=\"linear\")\n"
            )
        };
        let mut files = MemorySources::default();
        files.insert("main.sanim", program(old_helper));
        let mut engine = Engine::default();
        let old = prepare(&mut engine, &files);
        let before_use = engine.execution_state(boundary).unwrap();
        files.insert("main.sanim", program(new_helper));
        let edited = prepare(&mut engine, &files);
        assert_eq!(
            edited.statistics.restored_statements, boundary as u64,
            "{old_helper}"
        );
        assert!(Arc::ptr_eq(
            &before_use,
            &engine.execution_state(boundary).unwrap()
        ));
        let fresh = prepare(&mut Engine::default(), &files);
        for time in [
            Fraction::from(0),
            Fraction::new(3u64, 2u64),
            Fraction::from(2),
        ] {
            assert_eq!(
                edited.animation.sample(time).unwrap(),
                fresh.animation.sample(time).unwrap(),
                "{old_helper} at {time}"
            );
        }
        assert_eq!(
            edited.animation.reactive, fresh.animation.reactive,
            "{old_helper}"
        );
        assert_ne!(
            old.animation.sample(Fraction::new(3u64, 2u64)).unwrap(),
            edited.animation.sample(Fraction::new(3u64, 2u64)).unwrap(),
            "{old_helper}"
        );
    }
}

#[test]
fn a_short_circuited_helper_lowered_for_later_playback_invalidates_its_checkpoint() {
    for (old_helper, new_helper) in [
        (
            "fn gate(value:Number):Bool{return value>2}",
            "fn gate(value:Number):Bool{return value>4}",
        ),
        (
            "fn gate(value:Number,limit:Number=2):Bool{return value>limit}",
            "fn gate(value:Number,limit:Number=4):Bool{return value>limit}",
        ),
        (
            "fn threshold(value:Number):Bool{return value>2}\nfn gate(value:Number):Bool{return threshold(value)}",
            "fn threshold(value:Number):Bool{return value>4}\nfn gate(value:Number):Bool{return threshold(value)}",
        ),
    ] {
        let program = |helper| {
            format!(
                "wait(1s)\n{helper}\nmut active=false\nmut input=3\nsig flag=active and gate(input)\nlet view=CodeView(code=r\"x=1\",lang=\"python\",line_numbers=flag)\nadd(view)\nwait(1s)\nactive=true\nwait(1s)\n"
            )
        };
        let mut files = MemorySources::default();
        files.insert("main.sanim", program(old_helper));
        let mut engine = Engine::default();
        let old = prepare(&mut engine, &files);
        let before_use = engine.execution_state(4).unwrap();
        files.insert("main.sanim", program(new_helper));
        let edited = prepare(&mut engine, &files);
        let fresh = prepare(&mut Engine::default(), &files);
        assert_eq!(
            edited.animation.reactive, fresh.animation.reactive,
            "{old_helper}"
        );
        assert!(Arc::ptr_eq(
            &before_use,
            &engine.execution_state(4).unwrap()
        ));
        assert_eq!(edited.statistics.restored_statements, 4, "{old_helper}");
        for time in [
            Fraction::from(1),
            Fraction::new(5u64, 2u64),
            Fraction::from(3),
        ] {
            assert_eq!(
                edited.animation.sample(time).unwrap(),
                fresh.animation.sample(time).unwrap(),
                "{old_helper} at {time}"
            );
        }
        assert_ne!(
            old.animation.reactive, edited.animation.reactive,
            "{old_helper}"
        );
    }
}

#[test]
fn current_hoisted_declarations_replace_obsolete_saved_bindings() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "wait(1s)\nfn old_helper():Number { return 1 }\nwait(1s)\n",
    );
    let mut engine = Engine::default();
    prepare(&mut engine, &files);
    files.insert(
        "main.sanim",
        "wait(1s)\nfn new_helper():Number { return 2 }\nlet result=new_helper()\nwait(1s)\n",
    );
    let edited = prepare(&mut engine, &files);
    assert_eq!(edited.statistics.restored_statements, 1);
    assert!(!edited.bindings.contains_key("old_helper"));
    assert!(edited.bindings.contains_key("new_helper"));
    assert_eq!(
        edited.bindings["result"].number().unwrap().value,
        Fraction::from(2)
    );
}

#[test]
fn changed_reflection_restores_before_its_binding_and_failure_keeps_valid_prefix() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "wait(1s)\nlet delay=reflect(1s)\nwait(delay)\n",
    );
    let mut engine = Engine::default();
    prepare(&mut engine, &files);
    let edited = engine
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("main.sanim::delay".into(), serde_json::json!("2s"))]),
        )
        .unwrap();
    assert_eq!(edited.statistics.restored_statements, 1);
    assert_eq!(edited.animation.duration, Fraction::from(3));
    files.insert("main.sanim", "wait(1s)\nlet broken=1/0\n");
    assert!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .is_err()
    );
    files.insert("main.sanim", "wait(1s)\nwait(4s)\n");
    let recovered = prepare(&mut engine, &files);
    assert_eq!(recovered.statistics.restored_statements, 1);
    assert_eq!(recovered.animation.duration, Fraction::from(5));
}

#[test]
fn restored_continuous_assertions_are_checked_against_the_new_suffix() {
    let mut files = MemorySources::default();
    let prefix =
        "let c=Circle(radius=2pt,id=\"c\")\nadd(c)\nassert(always(inside_canvas(c)))\nwait(1s)\n";
    files.insert(
        "main.sanim",
        format!("{prefix}MoveTo(c, Vec2(1pt,0pt), duration=1s, ease=\"linear\")\n"),
    );
    let mut engine = Engine::default();
    prepare(&mut engine, &files);
    files.insert(
        "main.sanim",
        format!("{prefix}MoveTo(c, Vec2(9999pt,0pt), duration=1s, ease=\"linear\")\n"),
    );
    let error = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S350");
}

#[test]
fn snapshots_share_large_data_geometry_and_unchanged_timeline_nodes() {
    let mut files = MemorySources::default();
    files.insert(
        "large.json",
        format!(
            "[{}]",
            (0..10_000)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
    );
    files.insert("main.sanim", "let large=read<List<Number>>(\"large.json\")\nlet c=Circle(id=\"c\")\nadd(c)\nwait(1s)\nwait(1s)\n");
    let mut engine = Engine::default();
    let prepared = prepare(&mut engine, &files);
    let first = engine.execution_state(3).unwrap();
    let last = engine.execution_state(5).unwrap();
    let (Value::List(a), Value::List(b)) = (
        first.binding("large").unwrap(),
        last.binding("large").unwrap(),
    ) else {
        panic!("list snapshots")
    };
    assert!(a.values.ptr_eq(&b.values));
    assert!(first.animation.layers.ptr_eq(&last.animation.layers));
    assert!(Arc::ptr_eq(
        &first.animation.layers[0].element.geometry,
        &last.animation.layers[0].element.geometry
    ));
    let sampled = prepared.animation.sample(Fraction::from(1)).unwrap();
    assert!(Arc::ptr_eq(
        &sampled[0].geometry,
        &first.animation.layers[0].element.geometry
    ));
    let mut fork = first.as_ref().clone();
    fork.animation.layers[0].element.opacity = Fraction::from(0);
    assert_eq!(first.animation.layers[0].element.opacity, Fraction::from(1));
    assert_eq!(last.animation.layers[0].element.opacity, Fraction::from(1));
}

#[test]
fn direct_frame_and_source_indexes_cover_changed_parameter_bytes_and_loop_calls() {
    let mut files = MemorySources::default();
    let source =
        "fn pause_step():Unit { wait(100ms) }\nwait(1s)\nrepeat 4 { pause_step() }\nwait(1s)\n";
    files.insert("main.sanim", source);
    let prepared = prepare(&mut Engine::default(), &files);
    let entry = prepared.time_map.at_frame(35).unwrap();
    assert_eq!(entry.statement, 1);
    let offset = source.find("100ms").unwrap();
    assert_eq!(
        prepared.time_map.earliest(
            "main.sanim",
            Span {
                start: offset + 2,
                end: offset + 3,
                ..Span::default()
            }
        ),
        Some(Fraction::from(1))
    );
    assert_eq!(prepared.time_map.at_frame(0).unwrap().statement, 0);
    assert_eq!(prepared.time_map.at_frame(42).unwrap().statement, 2);
}

#[test]
fn logical_operation_and_retention_limits_cannot_be_reset_by_restoring() {
    let mut files = MemorySources::default();
    let prefix = "repeat 20 { let value=1+2 }\nwait(1s)\n";
    files.insert("main.sanim", format!("{prefix}wait(1s)\n"));
    let mut engine = Engine::default();
    engine.limits.max_operations = 200;
    prepare(&mut engine, &files);
    files.insert(
        "main.sanim",
        format!("{prefix}repeat 80 {{ let value=1+2 }}\nwait(1s)\n"),
    );
    assert_eq!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S303"
    );
    let mut files = MemorySources::default();
    let numbers = format!("[{}]", vec!["1"; 1000].join(","));
    files.insert("a.json", &numbers);
    files.insert("b.json", &numbers);
    let prefix = "let a=read<List<Number>>(\"a.json\")\nwait(1s)\n";
    files.insert("main.sanim", format!("{prefix}wait(1s)\n"));
    let mut engine = Engine::default();
    engine.limits.data.max_values = 1500;
    prepare(&mut engine, &files);
    files.insert(
        "main.sanim",
        format!("{prefix}let b=read<List<Number>>(\"b.json\")\nwait(1s)\n"),
    );
    assert_eq!(
        engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err()
            .code,
        "S303"
    );
}

#[test]
fn rejected_candidates_do_not_retain_dependencies_from_an_obsolete_branch() {
    for reflected in [false, true] {
        let mut files = MemorySources::default();
        let flag = if reflected {
            "let use_extra=reflect(true)"
        } else {
            "let use_extra=read<Bool>(\"flag.json\")"
        };
        files.insert("main.sanim", format!("wait(1s)\n{flag}\nif use_extra {{ let extra=read<Data>(\"gone.json\") }}\nwait(1s)\n"));
        files.insert("flag.json", "true");
        files.insert("gone.json", "[1,2,3]");
        let mut engine = Engine::default();
        prepare(&mut engine, &files);
        files.files.remove("gone.json");
        files.insert("flag.json", "false");
        let edited = if reflected {
            engine
                .prepare_json(
                    "main.sanim",
                    &files,
                    &BTreeMap::from([("main.sanim::use_extra".into(), serde_json::json!(false))]),
                )
                .unwrap()
        } else {
            prepare(&mut engine, &files)
        };
        assert_eq!(edited.statistics.restored_statements, 1);
        assert!(!edited.dependencies.contains_key("gone.json"));
        assert_eq!(edited.animation.duration, Fraction::from(2));
    }
}

#[test]
fn bounded_checkpoint_eviction_keeps_suffix_restoration_available() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "wait(1s)\n".repeat(10));
    let mut engine = Engine::default();
    engine.limits.max_cache_entries = 3;
    let cold = prepare(&mut engine, &files);
    assert_eq!(cold.statistics.checkpoint_count, 3);
    assert!(engine.execution_state(0).is_none());
    let prefix = engine.execution_state(9).unwrap();
    files.insert(
        "main.sanim",
        format!("{}wait(2s)\n", "wait(1s)\n".repeat(9)),
    );
    let hot = prepare(&mut engine, &files);
    assert_eq!(hot.statistics.restored_statements, 9);
    assert_eq!(hot.animation.duration, Fraction::from(11));
    assert!(Arc::ptr_eq(&prefix, &engine.execution_state(9).unwrap()));
    engine.clear_cache();
    assert!(engine.execution_state(9).is_none());
    assert_eq!(prefix.cursor, Fraction::from(9));
}

#[test]
fn changed_imported_reflection_defaults_refresh_even_when_the_override_is_unchanged() {
    let mut files = MemorySources::default();
    files.insert("theme.sanim", "let tint=reflect(Color(\"red\"))\n");
    files.insert("main.sanim", "import \"theme.sanim\" as theme\nwait(1s)\n");
    let inputs = BTreeMap::from([("theme.sanim::tint".into(), serde_json::json!("green"))]);
    let mut engine = Engine::default();
    engine.prepare_json("main.sanim", &files, &inputs).unwrap();
    files.insert("theme.sanim", "let tint=reflect(Color(\"blue\"))\n");
    let edited = engine.prepare_json("main.sanim", &files, &inputs).unwrap();
    assert_eq!(
        edited.reflected[0].default,
        Value::Color(sanim_core::Color::parse("blue").unwrap())
    );
    assert_eq!(
        edited.reflected[0].value,
        Value::Color(sanim_core::Color::parse("green").unwrap())
    );
    assert_eq!(
        semantic_output(&edited),
        semantic_output(
            &Engine::default()
                .prepare_json("main.sanim", &files, &inputs)
                .unwrap()
        )
    );
}

#[test]
fn imported_work_changes_cannot_discard_fresh_operation_or_retention_cost() {
    for retention in [false, true] {
        let mut files = MemorySources::default();
        files.insert("library.sanim", "repeat 1 { let scratch=0 }\n");
        files.insert(
            "main.sanim",
            "import \"library.sanim\" as library\nrepeat 40 { let scratch=0 }\nwait(1s)\n",
        );
        let mut engine = Engine::default();
        if retention {
            engine.limits.data.max_values = 100;
        } else {
            engine.limits.max_operations = 200;
        }
        prepare(&mut engine, &files);
        files.insert("library.sanim", "repeat 80 { let scratch=0 }\n");
        assert_eq!(
            engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap_err()
                .code,
            "S303"
        );
        let mut fresh = Engine::default();
        fresh.limits = engine.limits.clone();
        assert_eq!(
            fresh
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap_err()
                .code,
            "S303"
        );
    }
}
