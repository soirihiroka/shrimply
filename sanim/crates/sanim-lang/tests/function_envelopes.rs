use sanim_core::{Fraction, TimeMapKind};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn sources(source: &str) -> MemorySources {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    files
}

#[test]
fn function_envelopes_include_spawned_work_in_both_evaluators() {
    for source in [
        "fn launch():Unit{background{wait(2s)}}\nlaunch(duration=1s)",
        "fn leaf():Unit{background{wait(2s)}}\nfn launch():Unit{leaf()}\nlaunch(duration=1s)",
        "fn launch():Unit{background{wait(2s)}}\nwait(1s)\nlaunch(duration=1s)",
        "mut phase=0\nfn launch():Unit{background{animate(phase,to=1,duration=2s)}}\nlaunch(duration=1s)",
        "fn launch():Unit{background{timer 2 every 1s{wait(250ms)}}}\nlaunch(duration=1s)",
        "fn launch():Unit{background{wait(1ms)}}\nlaunch(duration=0s)",
    ] {
        let files = sources(source);
        for duration_only in [false, true] {
            let error = if duration_only {
                Engine::default()
                    .duration("main.sanim", &files, &BTreeMap::new())
                    .unwrap_err()
            } else {
                Engine::default()
                    .prepare("main.sanim", &files, &BTreeMap::new())
                    .unwrap_err()
            };
            assert_eq!(error.code, "S304", "{source}: {error}");
            assert!(
                error.message.contains("requested timeline duration"),
                "{error}"
            );
        }
    }
}

#[test]
fn call_horizons_exclude_older_jobs_and_include_nested_jobs() {
    for (source, expected) in [
        (
            "background{wait(10s)}\nfn short():Unit{return wait(0s)}\nshort(duration=1s)",
            Fraction::from(10),
        ),
        (
            "fn short():Unit{return wait(0s)}\nfn outer():Unit{background{wait(2s)};short(duration=1s)}\nouter(duration=2s)",
            Fraction::from(2),
        ),
        (
            "fn launch():Unit{background{wait(2s)}}\nlaunch(duration=2s)",
            Fraction::from(2),
        ),
        (
            "fn leaf():Unit{background{wait(2s)}}\nfn launch():Unit{leaf()}\nlaunch()\nwait(500ms)",
            Fraction::from(2),
        ),
    ] {
        let files = sources(source);
        let full = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        let planned = Engine::default()
            .duration("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(full.animation.duration, expected, "{source}");
        assert_eq!(planned.duration, expected, "{source}");
    }
}

#[test]
fn function_time_map_retains_spawned_span_without_consuming_main_cursor() {
    let files = sources("fn launch():Unit{background{wait(2s)}}\nlaunch()\nlet after=1");
    let full = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let function = full
        .time_map
        .entries
        .iter()
        .find(|entry| entry.kind == TimeMapKind::Function)
        .unwrap();
    assert_eq!(function.start, Fraction::from(0));
    assert_eq!(function.end, Fraction::from(2));
    let after = full
        .time_map
        .entries
        .iter()
        .find(|entry| entry.span.line == 3)
        .unwrap();
    assert_eq!(after.start, Fraction::from(0));
    assert_eq!(after.end, Fraction::from(0));
}
