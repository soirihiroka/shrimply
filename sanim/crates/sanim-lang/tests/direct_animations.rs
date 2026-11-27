use sanim_core::{Fraction, Property, Type, Value};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn files(source: &str) -> MemorySources {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{}\nAnimation()", source));
    files
}
fn prepare(source: &str) -> sanim_lang::Prepared {
    Engine::default()
        .prepare("main.sanim", &files(source), &BTreeMap::new())
        .unwrap()
}

#[test]
fn animation_methods_return_plans_with_a_bounded_default() {
    let prepared = prepare(
        "let marker=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nmarker.fade_in()",
    );
    assert_eq!(prepared.animation.duration, Fraction::new(3u64, 10u64));
    let marker = prepared.animation.find("marker").unwrap();
    assert_eq!(
        prepared
            .animation
            .sample_property(marker, Property::Opacity, Fraction::from(0))
            .unwrap()
            .number()
            .unwrap()
            .value,
        Fraction::from(0)
    );
    assert_eq!(
        prepared
            .animation
            .sample_property(marker, Property::Opacity, Fraction::new(3u64, 10u64))
            .unwrap()
            .number()
            .unwrap()
            .value,
        Fraction::from(1)
    );
    assert!(sanim_lang::prelude::builtin("play").is_none());
    assert!(!sanim_lang::prelude::types().contains_key("Motion"));
    assert_eq!(
        sanim_lang::prelude::builtin("Element.fade_in")
            .unwrap()
            .result,
        Type::Animation
    );
}

#[test]
fn multiple_targets_share_one_call_interval_and_sequential_calls_advance() {
    let source = "let first=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nlet second=Box(width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center);\n[first,second].fade_in(duration=400ms, ease=\"linear\");\n[first,second].fade_out(duration=200ms, ease=\"linear\")";
    let prepared = prepare(source);
    assert_eq!(prepared.animation.duration, Fraction::new(3u64, 5u64));
    for id in ["first", "second"] {
        let element = prepared.animation.find(id).unwrap();
        assert_eq!(
            prepared
                .animation
                .sample_property(element, Property::Opacity, Fraction::new(1u64, 5u64))
                .unwrap()
                .number()
                .unwrap()
                .value,
            Fraction::new(1u64, 2u64)
        );
        assert_eq!(
            prepared
                .animation
                .sample_property(element, Property::Opacity, Fraction::new(3u64, 5u64))
                .unwrap()
                .number()
                .unwrap()
                .value,
            Fraction::from(0)
        );
    }
    assert_eq!(
        Engine::default()
            .duration("main.sanim", &files(source), &BTreeMap::new())
            .unwrap()
            .duration,
        prepared.animation.duration
    );
}

#[test]
fn direct_calls_compose_with_backgrounds_arguments_helpers_and_zero_time() {
    for (source, expected) in [
        (
            "let marker=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\n<Animation absolute flow=\"sequence\">{marker.fade_in(duration=1s)}</Animation>\nlet other=Box(width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nother.fade_in(duration=500ms)\nwait(500ms)",
            Fraction::from(1),
        ),
        (
            "wait(200ms)\nBox(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center).fade_in(duration=400ms)",
            Fraction::new(3u64, 5u64),
        ),
        (
            "fn reveal(element:Element,span:Duration):Animation{return element.fade_in(duration=span)}\nlet marker=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nreveal(marker,400ms)",
            Fraction::new(2u64, 5u64),
        ),
        (
            "let marker=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nmarker.fade_in(duration=0s)\nwait(1s)",
            Fraction::from(1),
        ),
    ] {
        let prepared = prepare(source);
        assert_eq!(prepared.animation.duration, expected, "{source}");
        assert_eq!(
            Engine::default()
                .duration("main.sanim", &files(source), &BTreeMap::new())
                .unwrap()
                .duration,
            expected,
            "{source}"
        );
    }
}

#[test]
fn direct_animation_calls_are_not_skipped_by_value_caches() {
    let prepared = prepare(
        "let marker=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nmarker.move_to([10pt, 0pt], duration=100ms)\nmarker.move_to([10pt, 0pt], duration=100ms)",
    );
    assert_eq!(prepared.animation.duration, Fraction::new(1u64, 5u64));
    assert_eq!(
        prepared.animation.tracks["marker"][&Property::Position].len(),
        2
    );
}

#[test]
fn empty_targets_and_overlapping_property_writers_fail_with_reasons() {
    for (source, reason) in [
        (
            "let targets:List<Element>=[]\ntargets.fade_in()",
            "target list is empty",
        ),
        (
            "let marker=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center);\n[marker,marker].fade_in()",
            "writers",
        ),
        (
            "let marker=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\n<Animation absolute flow=\"sequence\">{marker.move_to([1pt, 0pt], duration=1s)}</Animation>\nmarker.move_to([2pt, 0pt], duration=1s)",
            "overlapping",
        ),
    ] {
        let error = Engine::default()
            .prepare("main.sanim", &files(source), &BTreeMap::new())
            .unwrap_err();
        assert!(error.message.contains(reason), "{error}");
    }
}

#[test]
fn pure_function_results_feed_explicit_cell_animation() {
    let prepared = prepare(
        "fn target(value:Length):Length{return value}\nmut radius=10pt\nlet marker=Box(width=64pt,height=64pt,radius=radius, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nAnimation(children=[marker])\nanimate(radius,to=target(30pt),duration=1s,ease=\"linear\")",
    );
    let marker = prepared.animation.find("marker").unwrap();
    assert_eq!(
        prepared
            .animation
            .sample(Fraction::new(1u64, 4u64))
            .unwrap()
            .into_iter()
            .find(|element| element.id == marker.id)
            .unwrap()
            .property(Property::Radius)
            .unwrap(),
        Value::Number(sanim_core::Number {
            value: Fraction::from(15),
            unit: sanim_core::Unit::Length
        })
    );
}

#[test]
fn fade_in_preserves_declared_opacity_after_fade_out() {
    let prepared = prepare(
        "let marker=Box(opacity=0.35, width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nmarker.fade_in(duration=1s, ease=\"linear\")\nmarker.fade_out(duration=1s, ease=\"linear\")\nmarker.fade_in(duration=1s, ease=\"linear\")",
    );
    let marker = prepared.animation.find("marker").unwrap();
    for (time, expected) in [
        (Fraction::new(1u64, 2u64), Fraction::new(7u64, 40u64)),
        (Fraction::from(1), Fraction::new(7u64, 20u64)),
        (Fraction::from(2), Fraction::from(0)),
        (Fraction::from(3), Fraction::new(7u64, 20u64)),
    ] {
        assert_eq!(
            prepared
                .animation
                .sample_property(marker, Property::Opacity, time)
                .unwrap()
                .number()
                .unwrap()
                .value,
            expected
        );
    }
}
