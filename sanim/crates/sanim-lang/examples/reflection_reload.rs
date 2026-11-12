//! Read-only benchmark of reflection patches and canonical/legacy key resolution.
use sanim_core::{Color, Type, Viewport};
use sanim_lang::{Engine, FileSources};
use std::{collections::BTreeMap, path::Path, time::Instant};
fn main() -> Result<(), String> {
    let input = std::env::args()
        .nth(1)
        .ok_or("expected a .sanim file with a reflected Color")?;
    let path = Path::new(&input);
    let entry = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or("invalid source filename")?;
    let sources = FileSources::new(path.parent().unwrap_or(Path::new(".")))?;
    let mut engine = Engine::new(Viewport {
        width: 1080,
        height: 1920,
        ..Default::default()
    });
    let started = Instant::now();
    let cold = engine
        .prepare_json(entry, &sources, &BTreeMap::new())
        .map_err(|e| e.to_string())?;
    let cold_us = started.elapsed().as_micros();
    let input = cold
        .reflected
        .iter()
        .find(|input| input.name == "before_color")
        .or_else(|| {
            cold.reflected
                .iter()
                .find(|input| input.ty.as_ref() == &Type::Color)
        })
        .ok_or("no reflected Color input")?;
    let schema = sanim_core::reflection::Schema::new(
        &cold.source,
        cold.reflected.iter().map(|input| input.declaration.clone()),
    )
    .map_err(|e| e.to_string())?;
    let value = serde_json::json!("blue3");
    let started = Instant::now();
    let patched = engine
        .prepare_json(
            entry,
            &sources,
            &BTreeMap::from([(input.key.clone(), value.clone())]),
        )
        .map_err(|e| e.to_string())?;
    let patch_us = started.elapsed().as_micros();
    let reflected = patched
        .reflected
        .iter()
        .find(|v| v.key == input.key)
        .expect("patched input metadata");
    assert_eq!(
        reflected.value,
        sanim_core::Value::Color(Color::parse("blue3")?)
    );
    let started = Instant::now();
    let legacy = engine
        .prepare_json(
            entry,
            &sources,
            &BTreeMap::from([(input.name.clone(), value)]),
        )
        .map_err(|e| e.to_string())?;
    let legacy_us = started.elapsed().as_micros();
    assert_eq!(legacy.fingerprint, patched.fingerprint);
    assert_eq!(legacy.animation, patched.animation);
    println!(
        "patch_cache_key_us={} patch_native_us={} patch_validation_us={} reused_expressions={} rebuilt_expressions={}",
        patched.statistics.cache_key_micros,
        patched.statistics.native_micros,
        patched.statistics.value_validation_micros,
        patched.statistics.reused_expressions,
        patched.statistics.evaluated_expressions
    );
    let started = Instant::now();
    for _ in 0..100_000 {
        std::hint::black_box(
            schema
                .declaration(std::hint::black_box(&input.key))
                .map_err(|e| e.to_string())?,
        );
    }
    println!(
        "cold_us={cold_us} color_patch_us={patch_us} color_patch_operations={} color_patch_queries={} color_patch_decodes={} legacy_equivalent_us={legacy_us} legacy_operations={} lookups=100000 lookup_total_us={} key={} label={}",
        patched.statistics.operations,
        patched.statistics.executed_queries,
        patched.statistics.decoded_assets,
        legacy.statistics.operations,
        started.elapsed().as_micros(),
        input.key,
        input.label
    );
    Ok(())
}
