//! Measure calculated states and native Skia command recording without rasterization or encoding.
use sanim_core::{Fraction, Viewport};
use sanim_lang::{Engine, FileSources};
use sanim_render::{Options, Renderer};
use std::{collections::BTreeMap, path::Path, time::Instant};

fn main() -> Result<(), String> {
    let file = std::env::args().nth(1).ok_or("expected a .sanim path")?;
    let path = Path::new(&file);
    let sources = FileSources::new(path.parent().unwrap_or(Path::new(".")))?;
    let entry = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid script filename")?;
    let prepared = Engine::new(Viewport {
        width: 1080,
        height: 1920,
        ..Viewport::default()
    })
    .prepare(entry, &sources, &BTreeMap::new())
    .map_err(|error| error.to_string())?;
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(prepared.animation)?;
    let options = Options::for_animation(scene.animation());
    let times: Vec<_> = (0u64..150)
        .map(|frame| Fraction::new(frame, 30u64))
        .collect();
    for pass in 1..=2 {
        let started = Instant::now();
        let mut micros = Vec::new();
        let computed = renderer.states_computed;
        let hits = renderer.cache_hits;
        for &time in &times {
            let frame_started = Instant::now();
            renderer.record(&scene, time, &options)?;
            micros.push(frame_started.elapsed().as_micros());
        }
        micros.sort_unstable();
        println!(
            "pass={pass} frames={} elapsed_ms={} median_us={} p95_us={} max_us={} states_computed={} recording_hits={}",
            times.len(),
            started.elapsed().as_millis(),
            micros[micros.len() / 2],
            micros[micros.len() * 95 / 100],
            micros.last().unwrap(),
            renderer.states_computed - computed,
            renderer.cache_hits - hits
        );
    }
    Ok(())
}
