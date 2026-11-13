//! Measure the same requested-frame path used by preview, without raster readback.
use crate::{Args, render_scene};
use sanim_lang::Prepared;
use sanim_render::Renderer;
use serde_json::{Value, json};

pub(crate) const MAX_SAMPLES: u32 = 10_000;

pub(crate) fn run(prepared: &Prepared, args: &Args) -> Result<Value, String> {
    let mut renderer = Renderer::default();
    let started = std::time::Instant::now();
    let (scene, options) = render_scene(prepared, args, &mut renderer)?;
    let scene_micros = started.elapsed().as_micros();
    let frames = sanim_core::math::frame_count(scene.animation().duration, scene.animation().rate)?;
    let samples = u64::from(args.samples.unwrap_or(30)).min(frames.max(1));
    let times = (0..samples)
        .map(|index| {
            let frame = if samples == 1 {
                0
            } else {
                index
                    .checked_mul(frames - 1)
                    .ok_or("profile frame index overflow")?
                    / (samples - 1)
            };
            scene.animation().frame_time(frame)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut passes = Vec::new();
    for name in ["cold", "warm"] {
        let before = (
            renderer.states_computed,
            renderer.state_cache_hits,
            renderer.cache_hits,
            renderer.geometry_cache_hits,
            renderer.geometries_prepared,
            renderer.geometry_key_hits,
        );
        let started = std::time::Instant::now();
        let mut latencies = Vec::new();
        for time in &times {
            let frame_started = std::time::Instant::now();
            renderer.record(&scene, *time, &options)?;
            latencies.push(frame_started.elapsed().as_micros());
        }
        let elapsed = started.elapsed().as_micros();
        latencies.sort_unstable();
        passes.push(json!({"name":name,"samples":samples,"elapsed_micros":elapsed,
            "median_micros":latencies[latencies.len()/2],"p95_micros":latencies[(latencies.len()-1)*95/100],
            "max_micros":latencies.last(),"states_computed":renderer.states_computed-before.0,
            "state_cache_hits":renderer.state_cache_hits-before.1,"recording_cache_hits":renderer.cache_hits-before.2,
            "geometry_cache_hits":renderer.geometry_cache_hits-before.3,"geometries_prepared":renderer.geometries_prepared-before.4,"geometry_key_hits":renderer.geometry_key_hits-before.5}));
    }
    Ok(
        json!({"evaluation":"demand","scene_prepare_micros":scene_micros,"frame_count":frames,
        "width":options.width,"height":options.height,"passes":passes,
        "note":"Debug/release build and cache budgets affect measurements. No rasterization, PNG encoding, or GPU readback is included."}),
    )
}
