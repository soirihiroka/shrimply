//! Benchmark real filesystem notifications in an isolated copy, never editing the input.
use sanim_core::Viewport;
use sanim_lang::{Engine, FileSources};
use sanim_runtime::{Runtime, Snapshot};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
struct CopyRoot(PathBuf);
impl Drop for CopyRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove isolated benchmark files");
    }
}
fn ready(runtime: &Runtime, revision: u64) -> Result<Snapshot, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut state = runtime.snapshot();
    loop {
        if !state.preparing {
            if let Some(error) = &state.diagnostic {
                return Err(error.to_string());
            }
            if state.prepared.is_some() && state.revision > revision {
                return Ok(state);
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(
                "runtime benchmark timed out waiting for a native file notification".into(),
            );
        }
        state = runtime.wait_for_change(state.generation, remaining);
    }
}
fn main() -> Result<(), String> {
    let input = std::env::args()
        .nth(1)
        .ok_or("expected .sanim file with final wait(3s)")?;
    let path = Path::new(&input);
    let parent = path.parent().unwrap_or(Path::new("."));
    let entry = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("invalid source filename")?;
    let viewport = Viewport {
        width: 1080,
        height: 1920,
        ..Default::default()
    };
    let sources = FileSources::new(parent)?;
    let dependencies = Engine::new(viewport.clone())
        .prepare(entry, &sources, &BTreeMap::new())
        .map_err(|e| e.to_string())?
        .dependencies;
    let directory = std::env::temp_dir().join(format!("sanim-hot-bench-{}", std::process::id()));
    std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let root = CopyRoot(directory);
    for resource in dependencies.keys() {
        let target = root.0.join(resource);
        std::fs::create_dir_all(target.parent().ok_or("missing dependency directory")?)
            .map_err(|e| e.to_string())?;
        std::fs::copy(parent.join(resource), target).map_err(|e| e.to_string())?;
    }
    let started = Instant::now();
    let runtime = Runtime::with_viewport(root.0.join(entry), viewport, Default::default())?;
    let first = ready(&runtime, 0)?;
    let cold = started.elapsed().as_micros();
    let source = std::fs::read_to_string(root.0.join(entry)).map_err(|e| e.to_string())?;
    let (prefix, suffix) = source
        .rsplit_once("wait(3s)")
        .ok_or("expected final wait(3s)")?;
    let started = Instant::now();
    let replacement = root.0.join("edited.sanim");
    std::fs::write(&replacement, format!("{prefix}wait(4s){suffix}")).map_err(|e| e.to_string())?;
    std::fs::rename(&replacement, root.0.join(entry)).map_err(|e| e.to_string())?;
    let hot = ready(&runtime, first.revision)?;
    let edit = started.elapsed().as_micros();
    std::fs::write(root.0.join("unrelated.log"), "must not reload").map_err(|e| e.to_string())?;
    let idle = runtime.wait_for_change(hot.generation, Duration::from_millis(300));
    if idle.generation != hot.generation || idle.preparing {
        return Err("unrelated filesystem activity woke or reloaded the animation".into());
    }
    let p = hot.prepared.as_ref().expect("ready runtime");
    println!(
        "runtime_cold_us={cold} filesystem_hot_edit_us={edit} restored_statements={} operations={} queries={} decodes={} idle_generation_changes={} duration={}",
        p.statistics.restored_statements,
        p.statistics.operations,
        p.statistics.executed_queries,
        p.statistics.decoded_assets,
        idle.generation - hot.generation,
        p.animation.duration
    );
    Ok(())
}
