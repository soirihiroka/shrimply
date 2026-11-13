//! Measure an editor view with an unused detail dataset and the same view enabled.
use sanim_lang::{Engine, MemorySources, Preparation};
use std::{collections::BTreeMap, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sources = MemorySources::default();
    let values = (0..20_000)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    sources.insert("samples.json", format!("[{values}]"));
    for visible in [false, true] {
        sources.insert("main.sanim", format!("let samples=read<List<Number>>(\"samples.json\")\nlet show_details=reflect({visible})\nif show_details {{let label=Text(str(len(samples)));add(label)}}\nwait(1s)"));
        for preparation in [Preparation::Full, Preparation::Demand] {
            let mut engine = Engine::new(Default::default());
            engine.preparation = preparation;
            let started = Instant::now();
            let prepared = engine.prepare("main.sanim", &sources, &BTreeMap::new())?;
            println!(
                "visible={visible} mode={preparation:?} elapsed_us={} operations={} decodes={} deferred={} forced={} layers={}",
                started.elapsed().as_micros(),
                prepared.statistics.operations,
                prepared.statistics.decoded_assets,
                prepared.statistics.deferred_values,
                prepared.statistics.forced_values,
                prepared.animation.layers.len()
            );
        }
    }
    Ok(())
}
