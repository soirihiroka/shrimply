use sanim_core::{Value, value_budget};
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct ValueCache {
    values: BTreeMap<String, Value>,
    bytes: usize,
    max_bytes: usize,
    max_entries: usize,
}
impl ValueCache {
    pub fn configure(&mut self, bytes: usize, entries: usize) {
        if (bytes, entries) != (self.max_bytes, self.max_entries) {
            self.clear();
            self.max_bytes = bytes;
            self.max_entries = entries;
        }
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn clear(&mut self) {
        self.values.clear();
        self.bytes = 0;
    }
    pub fn insert(&mut self, key: String, value: Value) {
        let mut remaining = self.max_bytes;
        let mut items = usize::MAX;
        if self.max_entries == 0
            || value_budget::charge(&value, &mut remaining, &mut items, 256).is_err()
        {
            return;
        }
        let bytes = self.max_bytes - remaining;
        if self.values.len() >= self.max_entries
            || self.bytes.saturating_add(bytes) > self.max_bytes
        {
            self.clear();
        }
        self.bytes += bytes;
        self.values.insert(key, value);
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
    pub fn key(&mut self, font: &std::sync::Arc<Vec<u8>>) -> String {
        let identity = std::sync::Arc::as_ptr(font) as usize;
        if let Some((_, key)) = self.values.get(&identity) {
            return key.clone();
        }
        let key = sanim_core::digest(font);
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
        let original = cache.key(&font);
        Arc::make_mut(&mut font)[0] = 9;
        assert_ne!(original, cache.key(&font));
        assert_eq!(original, cache.key(&Arc::new(vec![1, 2, 3])));
        let mut disabled = FontCache::new(0, 0);
        assert_eq!(original, disabled.key(&Arc::new(vec![1, 2, 3])));
    }
}
