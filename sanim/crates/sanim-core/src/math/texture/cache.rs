//! Pure raster memoization uses the shared byte/entry-bounded resource cache.
use super::*;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicU64, Ordering},
};
const MAX_ENTRIES: usize = 1024;
const ENTRY_BYTES: usize = 512;
type Cache = crate::cache::Cache<String, sanim_voxel::Texture>;
static HITS: AtomicU64 = AtomicU64::new(0);
static RASTERIZATIONS: AtomicU64 = AtomicU64::new(0);
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::new(MAX_BYTES, MAX_ENTRIES)))
}
pub(super) fn key(operation: Op, args: &Arguments) -> Result<String, String> {
    let value = Value::Record(args.clone());
    let mut writer = shrimply_resource_pipeline::fingerprint::Writer::default();
    serde_json::to_writer(
        &mut writer,
        &(operation, crate::value_key::ValueKey(&value)),
    )
    .map_err(|error| format!("texture cache key: {error}"))?;
    Ok(writer.finish())
}
pub(super) fn get(key: &String) -> Option<Arc<sanim_voxel::Texture>> {
    let value = cache()
        .lock()
        .expect("texture resource cache poisoned")
        .get(key);
    if value.is_some() {
        HITS.fetch_add(1, Ordering::Relaxed);
    }
    value
}
pub(super) fn publish(
    key: String,
    texture: Arc<sanim_voxel::Texture>,
) -> Arc<sanim_voxel::Texture> {
    RASTERIZATIONS.fetch_add(1, Ordering::Relaxed);
    let mut cache = cache().lock().expect("texture resource cache poisoned");
    if let Some(existing) = cache.get(&key) {
        return existing;
    }
    let bytes = texture
        .pixels()
        .len()
        .saturating_add(std::mem::size_of_val(texture.as_ref()))
        .saturating_add(ENTRY_BYTES);
    cache.insert(key, texture.clone(), bytes);
    texture
}
#[derive(Clone, Copy, Debug)]
pub struct CacheStats {
    pub bytes: usize,
    pub entries: usize,
    pub hits: u64,
    pub rasterizations: u64,
}
pub fn cache_stats() -> CacheStats {
    let (bytes, entries) = cache()
        .lock()
        .expect("texture resource cache poisoned")
        .usage();
    CacheStats {
        bytes,
        entries,
        hits: HITS.load(Ordering::Relaxed),
        rasterizations: RASTERIZATIONS.load(Ordering::Relaxed),
    }
}
#[cfg(test)]
pub(super) fn clear() {
    cache()
        .lock()
        .expect("texture resource cache poisoned")
        .clear();
}
