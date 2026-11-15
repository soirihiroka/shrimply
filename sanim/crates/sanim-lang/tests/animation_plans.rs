use sanim_core::{Fraction, Property, Type, Value};
use sanim_lang::{Engine, MemorySources, Prepared};
fn run(source: &str, available: Option<Fraction>) -> Prepared {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    let mut engine = Engine::default();
    engine.available_duration = available;
    let result = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap();
    let report = engine
        .duration("main.sanim", &sources, &Default::default())
        .unwrap();
    assert_eq!(report.duration, result.animation.duration);
    result
}
#[test]
fn constructing_plans_is_pure_and_only_returned_root_is_lowered() {
    let result = run(
        "let unused=Box()\nlet ignored=unused.fade_in(duration=10s)\nfn introduction():Animation{return wait(2s)}\nreturn introduction()",
        None,
    );
    assert_eq!(result.animation.duration, Fraction::from(2));
    assert!(result.animation.layers.is_empty());
    assert_eq!(
        result.symbols["introduction"].signatures()[0]
            .result
            .as_ref(),
        &Type::Animation
    );
}
#[test]
fn nested_wrappers_target_one_visual_and_schedule_concurrently() {
    let result = run(
        r#"let label=Box(width=10pt,height=10pt)
return <Animation target={label}><FadeIn duration={2s}><MoveTo to={[100pt,0pt]} duration={2s}/></FadeIn></Animation>"#,
        None,
    );
    assert_eq!(result.animation.layers.len(), 1);
    assert_eq!(result.animation.tracks["label"].len(), 2);
    assert_eq!(result.animation.duration, Fraction::from(2));
    assert_eq!(
        result.animation.tracks["label"][&Property::Opacity][0].start,
        Fraction::from(0)
    );
    assert_eq!(
        result.animation.tracks["label"][&Property::Position][0].start,
        Fraction::from(0)
    );
}
#[test]
fn viewport_resize_reuses_plan_and_places_sequence_without_source_evaluation() {
    let result = run(
        r#"let item=Box()
return <Animation target={item} flow="sequence" justify="between"><FadeIn duration={1s}/><FadeOut duration={1s}/></Animation>"#,
        Some(10.into()),
    );
    assert_eq!(
        result.animation.tracks["item"][&Property::Opacity][1].start,
        Fraction::from(9)
    );
    let resized = result
        .relayout(&sanim_core::Viewport::default(), Some(4.into()))
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&result.plan, &resized.plan));
    assert_eq!(resized.statistics.operations, 0);
    assert_eq!(
        resized.animation.tracks["item"][&Property::Opacity][1].start,
        Fraction::from(3)
    );
}
#[test]
fn leaf_matching_parent_interpolates_across_resolved_size() {
    let result = run(
        r#"let item=Box()
return <Animation target={item}><FadeIn duration="match_parent" ease="linear"/></Animation>"#,
        Some(4.into()),
    );
    let track = &result.animation.tracks["item"][&Property::Opacity][0];
    assert_eq!(track.end, Fraction::from(4));
    assert_eq!(
        track.sample(2.into()).unwrap(),
        Value::Number(sanim_core::Number {
            value: Fraction::new(1u64, 2u64),
            unit: sanim_core::Unit::Number
        })
    );
}
#[test]
fn clipping_holds_original_curve_without_rescaling() {
    let result = run(
        "let item=Box()\nreturn item.fade_in(duration=4s,ease=\"linear\")",
        Some(1.into()),
    );
    let track = &result.animation.tracks["item"][&Property::Opacity][0];
    assert_eq!(track.end, Fraction::from(1));
    assert_eq!(
        track
            .sample(Fraction::new(1u64, 2u64))
            .unwrap()
            .number()
            .unwrap()
            .value,
        Fraction::new(1u64, 8u64)
    );
    assert_eq!(
        track.sample(2.into()).unwrap().number().unwrap().value,
        Fraction::new(1u64, 4u64)
    );
}
#[test]
fn explicit_repeat_is_compact_bounded_and_sampleable() {
    let result = run(
        "let item=Box()\nreturn item.fade_in(duration=2s,ease=\"linear\").repeat(forever)",
        None,
    );
    assert!(result.animation.indefinite);
    assert_eq!(result.animation.tracks["item"][&Property::Opacity].len(), 1);
    let track = &result.animation.tracks["item"][&Property::Opacity][0];
    assert_eq!(
        track.sample(3.into()).unwrap().number().unwrap().value,
        Fraction::new(1u64, 2u64)
    );
}
#[test]
fn chronological_lowering_resolves_relative_motion_after_earlier_nested_groups() {
    let result = run(
        r#"let item=Box()
return <Animation>
 <Animation flow="sequence">{wait(2s)}{item.shift([10pt,0pt],duration=1s,ease="linear")}</Animation>
 <Animation offset={1s}>{item.shift([5pt,0pt],duration=1s,ease="linear")}</Animation>
</Animation>"#,
        None,
    );
    let tracks = &result.animation.tracks["item"][&Property::Position];
    assert_eq!(tracks[0].start, Fraction::from(1));
    assert_eq!(tracks[0].to.vec2().unwrap().values[0], 5.0);
    assert_eq!(tracks[1].start, Fraction::from(2));
    assert_eq!(tracks[1].to.vec2().unwrap().values[0], 15.0);
}
#[test]
fn scoped_entering_elements_mount_when_their_first_animation_starts() {
    let result = run(
        r#"let item=Box()
return <Animation target={item} flow="sequence">{wait(1s)}<FadeIn duration={1s}/></Animation>"#,
        None,
    );
    assert_eq!(result.animation.layers[0].birth, Fraction::from(1));
}
#[test]
fn same_cycle_writers_repeat_but_clipped_cycles_allow_later_ordinary_writers() {
    let result = run(
        r#"let item=Box()
let pulse=<Animation target={item} flow="sequence"><FadeIn duration={1s} ease="linear"/><FadeOut duration={1s} ease="linear"/></Animation>
return <Animation flow="sequence"><Animation duration={2s}><Animation absolute>{pulse.repeat(forever)}</Animation></Animation>{item.fade_in(duration=1s,ease="linear")}</Animation>"#,
        None,
    );
    assert!(!result.animation.indefinite);
    let tracks = &result.animation.tracks["item"][&Property::Opacity];
    assert_eq!(tracks.len(), 3);
    assert_eq!(tracks[0].cycle.unwrap().until(), Some(2.into()));
    assert_eq!(tracks[2].start, Fraction::from(2));
    assert_eq!(
        result
            .animation
            .sample_property(
                result.animation.find("item").unwrap(),
                Property::Opacity,
                Fraction::new(5u64, 2u64)
            )
            .unwrap()
            .number()
            .unwrap()
            .value,
        Fraction::new(1u64, 2u64)
    );
}
#[test]
fn local_crops_inside_a_cycle_repeat_without_truncating_the_global_clock() {
    let result = run(
        r#"let item=Box()
let cycle=<Animation flow="sequence"><Animation duration={1s}>{item.fade_in(duration=4s,ease="linear")}</Animation>{wait(1s)}</Animation>
return cycle.repeat(forever)"#,
        None,
    );
    let track = &result.animation.tracks["item"][&Property::Opacity][0];
    assert_eq!(track.cycle.unwrap().until(), None);
    assert_eq!(
        track
            .sample(Fraction::new(5u64, 2u64))
            .unwrap()
            .number()
            .unwrap()
            .value,
        Fraction::new(1u64, 8u64)
    );
}
#[test]
fn absolute_infinite_animation_does_not_make_its_finite_parent_infinite() {
    let result = run(
        r#"let item=Box()
return <Animation>{wait(10s)}<Animation absolute>{item.fade_in(duration=2s).repeat(forever)}</Animation></Animation>"#,
        None,
    );
    assert!(!result.animation.indefinite);
    assert_eq!(result.animation.duration, Fraction::from(10));
    assert_eq!(result.animation.tracks["item"][&Property::Opacity].len(), 1);
}
#[test]
fn two_second_viewport_crops_a_three_second_child_without_retiming() {
    let result = run(
        "let item=Box()\nreturn item.fade_in(duration=3s,ease=\"linear\")",
        Some(2.into()),
    );
    let track = &result.animation.tracks["item"][&Property::Opacity][0];
    assert_eq!(track.end, Fraction::from(2));
    assert_eq!(
        track.sample(1.into()).unwrap().number().unwrap().value,
        Fraction::new(1u64, 3u64)
    );
}
#[test]
#[ignore = "manual temporal lowering and memory benchmark"]
fn viewport_relayout_benchmark() {
    let source = r#"let items=range(1000).map(fn(index:Number):Element{return Box(width=10pt,height=10pt)})
return <Animation target={items}><FadeIn duration="match_parent"/></Animation>"#;
    let prepared = run(source, Some(10.into()));
    let baseline = std::sync::Arc::strong_count(&prepared.plan);
    let mut times = Vec::new();
    for i in 0..100 {
        let start = std::time::Instant::now();
        let result = prepared
            .relayout(
                &sanim_core::Viewport::default(),
                Some(Fraction::from(10 + (i % 2) as u64)),
            )
            .unwrap();
        assert_eq!(result.animation.layers.len(), 1000);
        assert_eq!(result.statistics.operations, 0);
        times.push(start.elapsed().as_micros());
        drop(result);
        assert_eq!(std::sync::Arc::strong_count(&prepared.plan), baseline);
    }
    times.sort();
    println!(
        "pure plan relayout/lower1000 shapes 100edits: median={}us p95={}us; retained root refs stable={baseline}",
        times[50], times[95]
    );
}
#[test]
fn actions_order_by_active_crop_start_and_visuals_never_mount_before_the_crop() {
    let result = run(
        r#"let item=Box()
return <Animation>
 <Animation offset={5s} duration={1s}><Animation offset={-5s}>{item.shift([10pt,0pt],duration=10s,ease="linear")}</Animation></Animation>
 <Animation offset={4s}>{item.shift([20pt,0pt],duration=1s,ease="linear")}</Animation>
</Animation>"#,
        Some(7.into()),
    );
    let tracks = &result.animation.tracks["item"][&Property::Position];
    assert_eq!(tracks[0].start, Fraction::from(4));
    assert_eq!(tracks[1].start, Fraction::from(5));
    assert_eq!(result.animation.layers[0].birth, Fraction::from(4));
    assert_eq!(tracks[1].interpolation, Some((0.into(), 10.into())));
    let result = run(
        r#"let item=Box()
return <Animation offset={5s} duration={1s}><Animation offset={-5s}>{item.fade_in(duration=10s)}</Animation></Animation>"#,
        Some(7.into()),
    );
    assert_eq!(result.animation.layers[0].birth, Fraction::from(5));
}
#[test]
fn extendable_sources_keep_future_work_while_explicit_fixed_parents_crop_it() {
    let source = "let item=Box()\nreturn Animation(flow=\"sequence\",children=[wait(2s),item.fade_in(duration=1s).repeat(forever)])";
    let result = run(source, Some(1.into()));
    assert_eq!(result.animation.duration, Fraction::from(1));
    assert!(
        result
            .animation
            .sample(Fraction::new(1u64, 2u64))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        result
            .animation
            .sample(Fraction::new(5u64, 2u64))
            .unwrap()
            .len(),
        1
    );
    let fixed = run(
        &source.replace("Animation(flow=", "Animation(duration=1s,flow="),
        Some(1.into()),
    );
    assert!(fixed.animation.layers.is_empty());
}

#[test]
fn terminal_instantaneous_actions_commit_instead_of_disappearing() {
    for source in [
        "let item=Box()\nreturn item.move_to([10pt,0pt],duration=0s)",
        "let item=Box()\nreturn Animation(flow=\"sequence\",children=[wait(1s),item.move_to([10pt,0pt],duration=0s)])",
    ] {
        let result = run(source, None);
        let track = &result.animation.tracks["item"][&Property::Position][0];
        assert_eq!(
            track.sample(track.end).unwrap().vec2().unwrap().values[0],
            10.0
        );
    }
}
#[test]
fn repeating_centered_growth_rejects_a_rotation_that_holds_into_later_cycles() {
    let source = r#"let item=Box(x_align=Align.Center,y_align=Align.Center)
let pulse=Animation(flow="sequence",children=[item.grow_from_center(duration=1s),wait(1s)])
return <Animation>{pulse.repeat(forever)}<Animation offset={1s}>{item.rotate_to(20deg,duration=500ms)}</Animation></Animation>"#;
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error.message.contains("repeating centered growth"),
        "{error}"
    );
}
#[test]
fn scope_only_mounts_fail_the_element_budget_before_building_a_large_scene() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let a=Box()\nlet b=Box()\nreturn Animation(target=[a,b],children=[wait(1s)])",
    );
    let mut engine = Engine::default();
    engine.limits.max_elements = 1;
    let error = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(error.message.contains("element budget"), "{error}");
}
#[test]
fn repeat_terminal_instantaneous_reset_fails_cleanly_instead_of_changing_initial_state() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let item=Box()\nlet cycle=Animation(flow=\"sequence\",children=[item.fade_in(duration=1s),item.fade_out(duration=0s)])\nreturn cycle.repeat(forever)");
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error.message.contains("move the reset to the beginning"),
        "{error}"
    );
}
#[test]
fn repeated_child_margins_are_real_timeline_pauses() {
    let result = run(
        "let item=Box()\nlet chunk=Animation(target=item,margin=1s,children=[FadeIn(duration=1s)])\nreturn chunk.repeat(2)",
        None,
    );
    assert_eq!(result.animation.duration, Fraction::from(6));
    let tracks = &result.animation.tracks["item"][&Property::Opacity];
    assert_eq!(tracks[0].start, Fraction::from(1));
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].cycle.unwrap().until(), Some(6.into()));
    assert_eq!(
        tracks[0]
            .sample(Fraction::new(9u64, 2u64))
            .unwrap()
            .number()
            .unwrap()
            .value,
        Fraction::new(1u64, 2u64)
    );
}
#[test]
fn finite_immutable_repeats_replay_keyframes_while_builder_loops_accumulate() {
    let result = run(
        "let item=Box()\nreturn item.shift([10pt,0pt],duration=1s,ease=\"linear\").repeat(3)",
        None,
    );
    let tracks = &result.animation.tracks["item"][&Property::Position];
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].cycle.unwrap().until(), Some(3.into()));
    assert_eq!(
        tracks[0]
            .sample(Fraction::new(3u64, 2u64))
            .unwrap()
            .vec2()
            .unwrap()
            .values[0],
        5.0
    );
    assert_eq!(
        tracks[0].sample(3.into()).unwrap().vec2().unwrap().values[0],
        10.0
    );
    let fresh = run(
        "let item=Box()\nrepeat 3 {item.shift([10pt,0pt],duration=1s,ease=\"linear\")}",
        None,
    );
    let tracks = &fresh.animation.tracks["item"][&Property::Position];
    assert_eq!(tracks.len(), 3);
    assert_eq!(tracks[2].to.vec2().unwrap().values[0], 30.0);
}
#[test]
fn finite_signal_repeat_keeps_pulsing_for_every_iteration() {
    let result = run(
        "mut phase=0deg\nsig size=64pt+16pt*sin(phase)\nlet shape=Box(width=size,height=size)\nlet pulse=animate(phase,to=360deg,duration=1s,ease=\"linear\").repeat(3)\nreturn Animation(children=[shape,pulse])",
        None,
    );
    let at = |time| {
        result.animation.sample(time).unwrap()[0]
            .property(Property::Width)
            .unwrap()
    };
    assert_eq!(at(Fraction::new(1u64, 4u64)), at(Fraction::new(5u64, 4u64)));
    assert_eq!(at(Fraction::new(1u64, 4u64)), at(Fraction::new(9u64, 4u64)));
}
#[test]
fn trivial_nested_repeat_counts_share_one_clock() {
    let result = run(
        "let item=Box()\nreturn item.fade_in(duration=1s).repeat(2).repeat(3)",
        None,
    );
    assert_eq!(result.animation.duration, Fraction::from(6));
    let tracks = &result.animation.tracks["item"][&Property::Opacity];
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].cycle.unwrap().until(), Some(6.into()));
}
#[test]
fn structured_nested_clocks_fail_with_an_actionable_constraint() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let item=Box()\nlet inner=item.fade_in(duration=1s).repeat(2)\nlet outer=Animation(flow=\"sequence\",children=[inner,wait(1s)])\nreturn outer.repeat(2)");
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error
            .message
            .contains("nested repeat clocks require flattening"),
        "{error}"
    );
}
#[test]
fn repeats_advanced_before_the_parent_window_fail_instead_of_losing_the_prototype() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let item=Box()\nlet pulse=item.fade_in(duration=1s).repeat(forever)\nreturn Animation(offset=5s,duration=1s,children=[Animation(offset=-5s,children=[pulse])])");
    let mut engine = Engine::default();
    engine.available_duration = Some(7.into());
    let error = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error
            .message
            .contains("repeat begins before its parent time window"),
        "{error}"
    );
}
#[test]
fn a_property_remains_owned_during_the_gaps_of_a_repeating_writer() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let item=Box()\nlet pulse=Animation(flow=\"sequence\",children=[item.fade_in(duration=1s),wait(1s)])\nreturn Animation(children=[pulse.repeat(forever),Animation(offset=1s,children=[item.fade_out(duration=500ms)])])");
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error
            .message
            .contains("owns this property until its cropped end"),
        "{error}"
    );
}
#[test]
fn a_cycle_can_reset_instantaneously_at_its_beginning() {
    let result = run(
        "let item=Box()\nlet cycle=Animation(flow=\"sequence\",children=[item.move_to([0pt,0pt],duration=0s),item.shift([10pt,0pt],duration=1s,ease=\"linear\")])\nreturn cycle.repeat(2)",
        None,
    );
    let item = result.animation.find("item").unwrap();
    assert_eq!(
        result
            .animation
            .sample_property(item, Property::Position, Fraction::new(3u64, 2u64))
            .unwrap()
            .vec2()
            .unwrap()
            .values[0],
        5.0
    );
}
