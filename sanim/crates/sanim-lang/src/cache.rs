use sanim_core::{Font, Value, cache::Cache, value_budget};
use std::{collections::BTreeMap, sync::Arc};
pub(crate) mod checked;
pub(crate) mod parsed;

mod size;

#[derive(Default)]
pub(crate) struct ValueCache {
    values: Cache<String, CachedValue>,
    max_bytes: usize,
    max_entries: usize,
}
pub(crate) struct CachedValue {
    pub value: Value,
    pub sources: imbl::OrdMap<String, Arc<crate::ElementSource>>,
}
impl ValueCache {
    pub fn configure(&mut self, bytes: usize, entries: usize) {
        if (bytes, entries) != (self.max_bytes, self.max_entries) {
            self.values = Cache::new(bytes, entries);
            self.max_bytes = bytes;
            self.max_entries = entries;
        }
    }
    pub fn get(&mut self, key: &String) -> Option<Value> {
        self.get_entry(key).map(|entry| entry.value.clone())
    }
    pub fn get_entry(&mut self, key: &String) -> Option<Arc<CachedValue>> {
        self.values.get(key)
    }
    pub fn clear(&mut self) {
        self.values.clear();
    }
    pub fn insert_measured(
        &mut self,
        key: String,
        value: Value,
        bytes: usize,
        sources: imbl::OrdMap<String, Arc<crate::ElementSource>>,
    ) {
        let source_bytes = sources.iter().fold(0usize, |bytes, (id, source)| {
            bytes.saturating_add(
                id.len()
                    + std::mem::size_of::<String>()
                    + std::mem::size_of::<Arc<crate::ElementSource>>()
                    + source.retained_bytes(),
            )
        });
        let bytes = bytes.saturating_add(key.len()).saturating_add(source_bytes);
        self.values
            .insert(key, Arc::new(CachedValue { value, sources }), bytes);
    }
    pub fn insert(&mut self, key: String, value: Value) {
        let mut remaining = self.max_bytes;
        let mut items = usize::MAX;
        if self.max_entries == 0
            || value_budget::charge(&value, &mut remaining, &mut items, 256).is_err()
        {
            return;
        }
        self.insert_measured(key, value, self.max_bytes - remaining, imbl::OrdMap::new());
    }
}

/// Content hashes for shared immutable font assets. Holding each Arc prevents
/// allocation-address reuse from turning a different font into a stale cache hit.
pub(crate) struct FontCache {
    values: BTreeMap<usize, (std::sync::Arc<Vec<u8>>, String)>,
    bytes: usize,
    max_bytes: usize,
    max_entries: usize,
}
impl FontCache {
    pub fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            values: BTreeMap::new(),
            bytes: 0,
            max_bytes,
            max_entries,
        }
    }
    pub fn key(&mut self, font: &Font) -> String {
        let Font::Asset(font) = font else {
            return format!(
                "family:{}:{:?}",
                sanim_core::digest(&serde_json::to_vec(font).expect("font descriptors serialize")),
                sanim_data::font::system_font_epoch()
            );
        };
        let identity = std::sync::Arc::as_ptr(font) as usize;
        if let Some((_, key)) = self.values.get(&identity) {
            return key.clone();
        }
        let key = format!("asset:{}", sanim_core::digest(font));
        if self.max_entries == 0 || font.len() > self.max_bytes {
            return key;
        }
        if self.values.len() >= self.max_entries
            || self.bytes.saturating_add(font.len()) > self.max_bytes
        {
            self.values.clear();
            self.bytes = 0;
        }
        self.bytes += font.len();
        self.values.insert(identity, (font.clone(), key.clone()));
        key
    }
}

#[cfg(test)]
mod tests {
    use super::FontCache;
    use std::sync::Arc;
    #[test]
    fn font_fingerprints_follow_content_through_copy_on_write_and_eviction() {
        let mut cache = FontCache::new(4, 1);
        let mut font = Arc::new(vec![1, 2, 3]);
        let original = cache.key(&sanim_core::Font::Asset(font.clone()));
        Arc::make_mut(&mut font)[0] = 9;
        assert_ne!(original, cache.key(&sanim_core::Font::Asset(font)));
        assert_eq!(
            original,
            cache.key(&sanim_core::Font::Asset(Arc::new(vec![1, 2, 3])))
        );
        let mut disabled = FontCache::new(0, 0);
        assert_eq!(
            original,
            disabled.key(&sanim_core::Font::Asset(Arc::new(vec![1, 2, 3])))
        );
        let sans = sanim_core::Font::family("sans-serif", "sans-serif").unwrap();
        let serif = sanim_core::Font::family("serif", "sans-serif").unwrap();
        assert_ne!(original, cache.key(&sans));
        assert_ne!(cache.key(&sans), cache.key(&serif));
    }
}
