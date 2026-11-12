//! Read-only benchmark: change the final hold in memory, then measure actual prefix restoration.
use sanim_core::Viewport;
use sanim_lang::{Engine, FileSources, Sources};
use std::{collections::BTreeMap, path::Path, time::Instant};

struct Edit<'a> {
    sources: &'a FileSources,
    entry: &'a str,
    text: String,
}
impl Sources for Edit<'_> {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        if path == self.entry {
            Ok(self.text.as_bytes().to_vec())
        } else {
            self.sources.read(path)
        }
    }
}
fn main() -> Result<(), String> {
    let file = std::env::args()
        .nth(1)
        .ok_or("expected a .sanim path with a final wait(3s)")?;
    let path = Path::new(&file);
    let sources = FileSources::new(path.parent().unwrap_or(Path::new(".")))?;
    let entry = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid filename")?;
    let mut engine = Engine::new(Viewport {
        width: 1080,
        height: 1920,
        ..Viewport::default()
    });
    let started = Instant::now();
    let cold = engine
        .prepare(entry, &sources, &BTreeMap::new())
        .map_err(|error| error.to_string())?;
    let cold_ms = started.elapsed().as_millis();
    let original = String::from_utf8(sources.read(entry)?).map_err(|error| error.to_string())?;
    let (prefix, suffix) = original
        .rsplit_once("wait(3s)")
        .ok_or("expected a final wait(3s)")?;
    let edit = Edit {
        sources: &sources,
        entry,
        text: format!("{prefix}wait(4s){suffix}"),
    };
    let started = Instant::now();
    let hot = engine
        .prepare(entry, &edit, &BTreeMap::new())
        .map_err(|error| error.to_string())?;
    println!(
        "cold_ms={cold_ms} hot_ms={} cold_operations={} hot_operations={} restored_statements={} checkpoints={} checkpoint_bytes={} queries={} decodes={} duration={}",
        started.elapsed().as_millis(),
        cold.statistics.operations,
        hot.statistics.operations,
        hot.statistics.restored_statements,
        hot.statistics.checkpoint_count,
        hot.statistics.checkpoint_bytes,
        hot.statistics.executed_queries,
        hot.statistics.decoded_assets,
        hot.animation.duration
    );
    let started = Instant::now();
    let mut found = 0;
    for index in 0..1_000_000 {
        if std::hint::black_box(&hot.time_map)
            .at_frame(index % 150)
            .is_some()
        {
            found += 1;
        }
    }
    println!(
        "frame_lookups=1000000 found={found} elapsed_us={}",
        started.elapsed().as_micros()
    );
    Ok(())
}
