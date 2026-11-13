use sanim_core::{Fraction, Property, Type, Value};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn files(source: &str) -> MemorySources {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    files
}
fn prepare(source: &str) -> sanim_lang::Prepared {
    Engine::default()
        .prepare("main.sanim", &files(source), &BTreeMap::new())
        .unwrap()
}

#[test]
fn animation_functions_schedule_directly_with_a_bounded_default() {
    let prepared = prepare("let marker=Circle()\nFadeIn(marker)");
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
        sanim_lang::prelude::builtin("FadeIn").unwrap().result,
        Type::Unit
    );
}

#[test]
fn multiple_targets_share_one_call_interval_and_sequential_calls_advance() {
    let source = "let first=Circle()\nlet second=Rect()\nFadeIn([first,second],duration=400ms,ease=\"linear\")\nFadeOut([first,second],duration=200ms,ease=\"linear\")";
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
            "let marker=Circle()\nbackground{FadeIn(marker,duration=1s)}\nlet other=Rect()\nFadeIn(other,duration=500ms)\nwait(500ms)",
            Fraction::from(1),
        ),
        (
            "FadeIn(Circle(duration=200ms),duration=400ms)",
            Fraction::new(3u64, 5u64),
        ),
        (
            "fn reveal(element:Element,span:Duration):Unit{FadeIn(element,duration=span)}\nlet marker=Circle()\nreveal(marker,400ms)",
            Fraction::new(2u64, 5u64),
        ),
        (
            "let marker=Circle()\nFadeIn(marker,duration=0s)\nwait(1s)",
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
        "let marker=Circle()\nMoveTo(marker,Vec2(10pt,0pt),duration=100ms)\nMoveTo(marker,Vec2(10pt,0pt),duration=100ms)",
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
            "let targets:List<Element>=[]\nFadeIn(targets)",
            "target list is empty",
        ),
        ("let marker=Circle()\nFadeIn([marker,marker])", "writers"),
        (
            "let marker=Circle()\nbackground{MoveTo(marker,Vec2(1pt,0pt),duration=1s)}\nMoveTo(marker,Vec2(2pt,0pt),duration=1s)",
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
fn user_function_easing_control_reaches_timed_value_assignment() {
    let prepared = prepare(
        "fn target(value:Length):Length{return value}\nmut radius=10pt\nlet marker=Circle(radius=radius)\nadd(marker)\nradius=target(30pt,duration=1s,ease=\"linear\")",
    );
    let marker = prepared.animation.find("marker").unwrap();
    assert_eq!(
        prepared
            .animation
            .sample_property(marker, Property::Radius, Fraction::new(1u64, 4u64))
            .unwrap(),
        Value::Number(sanim_core::Number {
            value: Fraction::from(15),
            unit: sanim_core::Unit::Length
        })
    );
}
