use std::{collections::{BTreeMap, HashMap}, hash::Hash, sync::Arc};

struct Entry<V> {
    value: Arc<V>,
    bytes: usize,
    used: u64,
}

/// Byte- and entry-bounded LRU. Eviction releases only the cache's Arc.
pub(crate) struct Cache<K, V> {
    entries: HashMap<K, Entry<V>>,
    ages: BTreeMap<u64, K>,
    bytes: usize,
    clock: u64,
    max_bytes: usize,
    max_entries: usize,
}
impl<K: Eq + Hash + Clone, V> Cache<K, V> {
    pub fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            ages: BTreeMap::new(),
            bytes: 0,
            clock: 0,
            max_bytes,
            max_entries,
        }
    }
    pub fn get(&mut self, key: &K) -> Option<Arc<V>> {
        let entry = self.entries.get_mut(key)?;
        self.ages.remove(&entry.used);
        self.clock = self.clock.checked_add(1).expect("render cache access counter overflow");
        entry.used = self.clock;
        self.ages.insert(entry.used, key.clone());
        Some(entry.value.clone())
    }
    pub fn insert(&mut self, key: K, value: Arc<V>, bytes: usize) {
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes -= previous.bytes;
            self.ages.remove(&previous.used);
        }
        if bytes > self.max_bytes || self.max_entries == 0 {
            return;
        }
        while self.entries.len() >= self.max_entries || self.bytes > self.max_bytes - bytes {
            let Some((_, oldest)) = self.ages.pop_first() else {
                break;
            };
            self.bytes -= self
                .entries
                .remove(&oldest)
                .expect("cached LRU entry")
                .bytes;
        }
        self.clock = self.clock.checked_add(1).expect("render cache access counter overflow");
        self.ages.insert(self.clock, key.clone());
        self.entries.insert(
            key,
            Entry {
                value,
                bytes,
                used: self.clock,
            },
        );
        self.bytes += bytes;
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.ages.clear();
        self.bytes = 0;
    }
    pub fn rekey(&mut self, mut update: impl FnMut(K) -> Option<K>) {
        let entries = std::mem::take(&mut self.entries);
        self.bytes = 0;
        for (key, entry) in entries {
            if let Some(key) = update(key)
                && self
                    .entries
                    .get(&key)
                    .is_none_or(|previous| previous.used < entry.used)
            {
                self.entries.insert(key, entry);
            }
        }
        self.bytes = self.entries.values().map(|entry| entry.bytes).sum();
        self.ages = self.entries.iter().map(|(key, entry)| (entry.used, key.clone())).collect();
    }
}
