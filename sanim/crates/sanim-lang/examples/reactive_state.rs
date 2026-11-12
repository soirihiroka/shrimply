//! Reproducible headless reactive-state workloads: make state-reactive-bench.
use sanim_core::Fraction;
use sanim_lang::{Engine, MemorySources, Prepared};
use std::{collections::BTreeMap, time::Instant};
fn prepare(engine: &mut Engine, files: &MemorySources) -> Prepared {
    engine
        .prepare("main.sanim", files, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
}
fn value(prepared: &Prepared, name: &str) -> Fraction {
    prepared.bindings[name].number().unwrap().value
}
fn report(label: &str, prepared: &Prepared, micros: u128) {
    let stats = &prepared.statistics;
    println!(
        "{label}: {micros}us, ops={}, signal eval/hit/invalid={}/{}/{}, writes/noop={}/{}, restored={}, checkpoints={}B, key/native/validation={}/{}/{}us, expressions eval/reuse={}/{}",
        stats.operations,
        stats.signal_evaluations,
        stats.signal_cache_hits,
        stats.signal_invalidations,
        stats.mutable_writes,
        stats.unchanged_writes,
        stats.restored_statements,
        stats.checkpoint_bytes,
        stats.cache_key_micros,
        stats.native_micros,
        stats.value_validation_micros,
        stats.evaluated_expressions,
        stats.reused_expressions
    );
}
fn main() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","mut source=1\nmut unrelated=0\nsig twice=source*2\nsig derived=twice+1\nlet first=derived\nrepeat 1000 {source=source;unrelated=unrelated+1}\nlet last=derived\n");
    let started = Instant::now();
    let result = prepare(&mut Engine::default(), &files);
    report(
        "lazy/no-op/unrelated",
        &result,
        started.elapsed().as_micros(),
    );
    assert_eq!(value(&result, "last"), Fraction::from(3));
    assert_eq!(result.statistics.signal_evaluations, 2);
    assert_eq!(result.statistics.signal_invalidations, 0);
    let prefix =
        "mut source=1\nsig derived=source*2\nrepeat 10000 {source=source+1}\nlet before=derived\n";
    files.insert("main.sanim", format!("{prefix}let result=derived+1\n"));
    let mut engine = Engine::default();
    let started = Instant::now();
    let cold = prepare(&mut engine, &files);
    report("prefix cold", &cold, started.elapsed().as_micros());
    files.insert("main.sanim", format!("{prefix}let result=derived+2\n"));
    let started = Instant::now();
    let hot = prepare(&mut engine, &files);
    report("prefix hot", &hot, started.elapsed().as_micros());
    assert_eq!(hot.statistics.restored_statements, 4);
    assert_eq!(value(&hot, "result"), Fraction::from(20004));
    for count in [1000, 10000] {
        let terms = (1..=10)
            .map(|factor| format!("input*{factor}"))
            .collect::<Vec<_>>()
            .join("+");
        files.insert("main.sanim",format!("fn calculate(input:Number):Number {{sig local={terms};sig derived=local+1;return derived}}\nmut total=0\nrepeat {count} {{total=total+calculate(2)}}\n"));
        let started = Instant::now();
        let result = prepare(&mut Engine::default(), &files);
        report(
            &format!("helper-local sig x{count}"),
            &result,
            started.elapsed().as_micros(),
        );
        assert_eq!(value(&result, "total"), Fraction::from(count * 111));
        assert_eq!(
            result.statistics.signal_evaluations,
            u64::try_from(count * 2).unwrap()
        );
        std::hint::black_box(&result);
    }
}
