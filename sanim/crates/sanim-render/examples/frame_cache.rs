//! Measure calculated states and native Skia command recording without rasterization or encoding.
use sanim_core::{Fraction, Viewport};
use sanim_lang::{Engine, FileSources, Preparation};
use sanim_render::{Options, Renderer};
use std::{collections::BTreeMap, path::Path, time::Instant};

fn main() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let file = arguments.next().ok_or("expected a .sanim path")?;
    let preparation = match arguments.next().as_deref() {
        None => Preparation::Full,
        Some("--demand") => Preparation::Demand,
        Some(other) => return Err(format!("unexpected option '{other}'; expected --demand")),
    };
    if arguments.next().is_some() {
        return Err("too many benchmark arguments".into());
    }
    let path = Path::new(&file);
    let sources = FileSources::new(path.parent().unwrap_or(Path::new(".")))?;
    let entry = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid script filename")?;
    let mut engine = Engine::new(Viewport {
        width: 1080,
        height: 1920,
        ..Viewport::default()
    });
    engine.preparation = preparation;
    let started = Instant::now();
    let prepared = engine
        .prepare(entry, &sources, &BTreeMap::new())
        .map_err(|error| error.to_string())?;
    let prepare_us = started.elapsed().as_micros();
    let mut renderer = Renderer::default();
    let started = Instant::now();
    let scene = renderer.prepare(prepared.animation)?;
    println!(
        "mode={preparation:?} prepare_us={prepare_us} scene_prepare_us={} operations={} decodes={} deferred={} forced={}",
        started.elapsed().as_micros(),
        prepared.statistics.operations,
        prepared.statistics.decoded_assets,
        prepared.statistics.deferred_values,
        prepared.statistics.forced_values
    );
    let options = Options::for_animation(scene.animation());
    let times: Vec<_> = (0u64..150)
        .map(|frame| Fraction::new(frame, 30u64))
        .collect();
    for pass in 1..=2 {
        let started = Instant::now();
        let mut micros = Vec::new();
        let mut new_state = Vec::new();
        let mut new_record = Vec::new();
        let layer_hits = renderer.layer_recording_hits;
        let layer_states = renderer.layer_state_hits;
        let computed = renderer.states_computed;
        let hits = renderer.cache_hits;
        let paths = renderer.geometries_prepared;
        let identity_hits = renderer.geometry_key_hits;
        let geometry_hits = renderer.geometry_cache_hits;
        for &time in &times {
            let frame_started = Instant::now();
            let computed = renderer.states_computed;
            renderer.frame_state(&scene, time, &options)?;
            let state_us = frame_started.elapsed().as_micros();
            let record_started = Instant::now();
            renderer.record(&scene, time, &options)?;
            if renderer.states_computed > computed {
                new_state.push(state_us);
                new_record.push(record_started.elapsed().as_micros());
            }
            micros.push(frame_started.elapsed().as_micros());
        }
        micros.sort_unstable();
        new_state.sort_unstable();
        new_record.sort_unstable();
        println!(
            "mode={preparation:?} pass={pass} frames={} elapsed_ms={} median_us={} p95_us={} max_us={} states_computed={} recording_hits={} path_builds={} identity_hits={} geometry_hits={}",
            times.len(),
            started.elapsed().as_millis(),
            micros[micros.len() / 2],
            micros[micros.len() * 95 / 100],
            micros.last().unwrap(),
            renderer.states_computed - computed,
            renderer.cache_hits - hits,
            renderer.geometries_prepared - paths,
            renderer.geometry_key_hits - identity_hits,
            renderer.geometry_cache_hits - geometry_hits
        );
        println!(
            "new_frames={} state_median_us={} recording_median_us={} reused_element_states={} reused_element_pictures={}",
            new_state.len(),
            new_state.get(new_state.len() / 2).copied().unwrap_or(0),
            new_record.get(new_record.len() / 2).copied().unwrap_or(0),
            renderer.layer_state_hits - layer_states,
            renderer.layer_recording_hits - layer_hits,
        );
    }
    Ok(())
}
