use sanim_core::Fraction;
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn compare(source: &str, expected: Fraction) -> sanim_lang::DurationReport {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let full = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let fast = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(full.animation.duration, expected);
    assert_eq!(fast.duration, expected);
    assert_eq!(fast.indefinite, full.animation.indefinite);
    assert_eq!(fast.event_index, full.time_map.event_index());
    fast
}
#[test]
fn exact_sequence_and_parallel_sizes_use_one_solver() {
    compare("repeat 3 {wait(1s/3)}", Fraction::from(1));
    compare(
        "return Animation(children=[wait(1s/3),wait(1s/2)])",
        Fraction::new(1u64, 2u64),
    );
    compare(
        "return Animation(children=[wait(1s/3),wait(1s/2)],flow=\"sequence\")",
        Fraction::new(5u64, 6u64),
    );
}
#[test]
fn list_mapping_and_guarded_value_functions_build_measured_plans() {
    compare(
        "fn step(n:Number):Animation{return wait(n*100ms)}\nreturn Animation(children=[1,2,3].map(step),flow=\"sequence\")",
        Fraction::new(3u64, 5u64),
    );
    compare(
        "fn choose(n:Int < 1):Animation{return wait(1s)}\nfn choose(n:Int >= 1):Animation{return wait(2s)}\nchoose(0)\nchoose(2)",
        Fraction::from(3),
    );
}
#[test]
fn indefinite_hold_retains_exact_finite_prefix() {
    let report = compare("wait(100ms)\nwait(forever)", Fraction::new(1u64, 10u64));
    assert!(report.indefinite);
    assert!(
        report
            .event_index
            .finite()
            .contains(&Fraction::new(1u64, 10u64))
    );
}
#[test]
fn full_duration_rejects_finite_overflow_and_preserves_compact_repetition() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "return wait(1s).repeat(7201)");
    let error = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("two-hour") || error.message.contains("overflow"),
        "{error}"
    );
    compare("return wait(1s).repeat(7200)", Fraction::from(7200));
}
