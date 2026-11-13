use sanim_core::{Fraction, Unit};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn compound_and_decimal_duration_literals_schedule_exactly() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let hold=4m1s\nwait(hold)\nwait(0.5s)\nwait(250ms)\n",
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        prepared.bindings["hold"].number().unwrap().unit,
        Unit::Duration
    );
    assert_eq!(prepared.animation.duration, Fraction::new(967u64, 4u64));
    let duration = Engine::default()
        .duration_json("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(duration.duration, prepared.animation.duration);
}

#[test]
fn compound_duration_overflow_has_a_timeline_diagnostic() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "wait(120m1ms)\n");
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("duration") || error.message.contains("2 hours"),
        "{error}"
    );
}
