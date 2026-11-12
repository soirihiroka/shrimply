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
