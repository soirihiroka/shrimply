//! End-to-end typed edit and hot-prefix benchmarks: make collection-language-bench.
use sanim_lang::{Engine, MemorySources};
use std::{collections::BTreeMap, time::Instant};

fn main() {
    const EDITS: usize = 200;
    for size in [1_000, 20_000] {
        let prefix =
            format!("mut items=range({size})\nrepeat {EDITS} {{items=append(items,-1)}}\n");
        let mut files = MemorySources::default();
        files.insert("main.sanim", format!("{prefix}let count=len(items)\n"));
        let mut engine = Engine::default();
        let start = Instant::now();
        let first = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(first.bindings["items"].list().unwrap().len(), size + EDITS);
        println!(
            "List<{size}>/{EDITS} append edits: {}us; native={}us value_budget={}us cache_keys={}us; retained_checkpoints={}B",
            start.elapsed().as_micros(),
            first.statistics.native_micros,
            first.statistics.value_validation_micros,
            first.statistics.cache_key_micros,
            first.statistics.checkpoint_bytes
        );
        files.insert("main.sanim", format!("{prefix}let count=len(items)+1\n"));
        let start = Instant::now();
        let hot = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert!(hot.statistics.restored_statements >= 2);
        println!(
            "List<{size}> suffix hot patch: {}us, restored={} statements, ops={}",
            start.elapsed().as_micros(),
            hot.statistics.restored_statements,
            hot.statistics.operations
        );
    }
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "mut items=range(20000)\nrepeat 1000 {items=move_item(items,0,0)}\n",
    );
    let start = Instant::now();
    let no_op = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(no_op.statistics.unchanged_writes, 1000);
    println!(
        "List<20000>/1000 no-op edits: {}us; native={}us value_budget={}us",
        start.elapsed().as_micros(),
        no_op.statistics.native_micros,
        no_op.statistics.value_validation_micros
    );
}
