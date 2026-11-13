use sanim_core::{Fraction, Value};
use sanim_lang::{Engine, MemorySources, Preparation};
use std::collections::BTreeMap;

fn demand() -> Engine {
    let mut engine = Engine::new(Default::default());
    engine.preparation = Preparation::Demand;
    engine
}

#[test]
fn unused_data_and_signals_are_not_forced_but_full_validation_reports_them() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "let unused=read<Number>(\"absent.json\")\nsig dormant=read<Number>(\"also-absent.json\")\nwait(1s)");
    let prepared = demand()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.statistics.decoded_assets, 0);
    assert_eq!(prepared.statistics.signal_evaluations, 0);
    assert_eq!(prepared.statistics.deferred_values, 1);
    assert_eq!(prepared.deferred_bindings.len(), 2);
    assert!(!prepared.bindings.contains_key("unused"));
    assert_eq!(prepared.dependencies.len(), 1);
    let error = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert!(error.message.contains("absent.json"), "{error}");
}

#[test]
fn demand_publication_does_not_expand_unused_import_namespaces() {
    let mut sources = MemorySources::default();
    sources.insert("shared.sanim", "let value=read<Number>(\"absent.json\")");
    sources.insert(
        "left.sanim",
        "import \"shared.sanim\" as shared\nlet left_value=1",
    );
    sources.insert(
        "right.sanim",
        "import \"shared.sanim\" as shared\nlet right_value=2",
    );
    sources.insert(
        "main.sanim",
        "import \"left.sanim\" as left\nimport \"right.sanim\" as right\nwait(1s)",
    );
    let prepared = demand()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert!(!prepared.bindings.contains_key("left"));
    assert!(!prepared.bindings.contains_key("right"));
    assert_eq!(prepared.statistics.decoded_assets, 0);
    assert_eq!(prepared.dependencies.len(), 4);
}

#[test]
fn deferred_lets_capture_mutables_and_signals_at_declaration() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "mut factor=2\nsig multiplier=factor+1\nlet result=decode<Number>(Data(14))*multiplier\nfactor=9\nlet label=Text(str(result))\nadd(label)\nwait(1s)");
    let lazy = demand()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    let strict = Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        lazy.bindings["result"].number().unwrap().value,
        Fraction::from(42)
    );
    assert_eq!(lazy.animation, strict.animation);
    assert_eq!(lazy.statistics.forced_values, 1);
}

#[test]
fn deferred_reads_freeze_paths_instead_of_following_later_assignments() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "mut path=\"first.json\"\nlet delay=read<Duration>(path)\npath=\"second.json\"\nwait(delay)");
    sources.insert("first.json", "\"1s\"");
    sources.insert("second.json", "\"2s\"");
    let prepared = demand()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.animation.duration, Fraction::from(1));
    assert!(prepared.dependencies.contains_key("first.json"));
    assert!(!prepared.dependencies.contains_key("second.json"));
}

#[test]
fn forcing_shared_values_keeps_timeline_and_reflection_order() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "let delay=read<Duration>(\"delay.json\")\nlet shown=reflect(false)\nwait(1s)\nwait(delay)\nwait(delay)\n");
    sources.insert("delay.json", "\"0.5s\"");
    let lazy = demand()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(lazy.animation.duration, Fraction::from(2));
    assert_eq!(lazy.statistics.decoded_assets, 1);
    assert_eq!(lazy.statistics.forced_values, 1);
    assert_eq!(lazy.reflected.len(), 1);
    assert_eq!(lazy.bindings["shown"], Value::Bool(false));
}

#[test]
fn deferred_errors_retain_the_original_source_location_when_demanded() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "let delay=read<Duration>(\"absent.json\")\nwait(1s)\nwait(delay)",
    );
    let error = demand()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.span.line, 1);
    assert_eq!(error.source, "main.sanim");
    assert!(error.message.contains("absent.json"), "{error}");
}

#[test]
fn hot_reload_restores_before_first_demand_and_rechecks_resource_versions() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        "let delay=read<Duration>(\"delay.json\")\nwait(1s)\nwait(delay)\nwait(1s)",
    );
    sources.insert("delay.json", "\"1s\"");
    let mut engine = demand();
    engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    sources.insert("delay.json", "\"2s\"");
    let hot = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    let fresh = demand()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(hot.statistics.restored_statements, 2);
    assert_eq!(hot.statistics.decoded_assets, 1);
    assert_eq!(hot.animation.duration, Fraction::from(4));
    assert_eq!(hot.animation, fresh.animation);
    assert_eq!(hot.fingerprint, fresh.fingerprint);
}

#[test]
fn timed_initializers_and_effectful_helpers_keep_eager_program_order() {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", "mut count=0\nfn effect():Number{count=count+1;wait(0.5s);return count}\nlet unused=effect()\nlet timed=read<Number>(\"value.json\",duration=0.5s)\nwait(1s)");
    sources.insert("value.json", "7");
    let lazy = demand()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap();
    assert_eq!(lazy.animation.duration, Fraction::from(2));
    assert_eq!(
        lazy.bindings["count"].number().unwrap().value,
        Fraction::from(1)
    );
    assert_eq!(
        lazy.bindings["timed"].number().unwrap().value,
        Fraction::from(7)
    );
    assert_eq!(lazy.statistics.deferred_values, 0);
}

#[test]
fn unforced_values_still_obey_the_retained_memory_budget() {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        (0..32)
            .map(|index| format!("let pending_{index}=read<Number>(\"absent_{index}.json\")\n"))
            .collect::<String>(),
    );
    let mut engine = demand();
    engine.limits.data.max_bytes = 2_048;
    let error = engine
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S303");
    assert!(error.message.contains("budget"), "{error}");
    assert!(!error.message.contains("could not read"), "{error}");
}
