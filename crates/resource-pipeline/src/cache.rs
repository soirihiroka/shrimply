use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
    sync::Arc,
};

struct Entry<R> {
    value: R,
    bytes: usize,
    used: u64,
}

/// Byte- and entry-bounded LRU. Arc is the default; platform-affine owners may
/// use Rc without copying the eviction algorithm or sharing native handles.
pub struct Cache<K, V, R = Arc<V>> {
    entries: HashMap<K, Entry<R>>,
    value_type: std::marker::PhantomData<fn() -> V>,
    ages: BTreeMap<u64, K>,
    bytes: usize,
    clock: u64,
    max_bytes: usize,
    max_entries: usize,
}
impl<K: Eq + Hash + Clone, V, R: Clone> Cache<K, V, R> {
    pub fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            value_type: std::marker::PhantomData,
            ages: BTreeMap::new(),
            bytes: 0,
            clock: 0,
            max_bytes,
            max_entries,
        }
    }
    /// Apply host limits before admitting new values, including failed work.
    pub fn configure(&mut self, max_bytes: usize, max_entries: usize) {
        self.max_bytes = max_bytes;
        self.max_entries = max_entries;
        self.trim(max_bytes, max_entries);
    }
    pub fn usage(&self) -> (usize, usize) {
        (self.bytes, self.entries.len())
    }
    fn trim(&mut self, max_bytes: usize, max_entries: usize) {
        while self.entries.len() > max_entries || self.bytes > max_bytes {
            let (_, oldest) = self.ages.pop_first().expect("cached LRU age");
            self.bytes -= self
                .entries
                .remove(&oldest)
                .expect("cached LRU entry")
                .bytes;
        }
    }
    pub fn get(&mut self, key: &K) -> Option<R> {
        let entry = self.entries.get_mut(key)?;
        self.ages.remove(&entry.used);
        self.clock = self
            .clock
            .checked_add(1)
            .expect("cache access counter overflow");
        entry.used = self.clock;
        self.ages.insert(entry.used, key.clone());
        Some(entry.value.clone())
    }
    pub fn insert(&mut self, key: K, value: R, bytes: usize) {
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes -= previous.bytes;
            self.ages.remove(&previous.used);
        }
        if bytes > self.max_bytes || self.max_entries == 0 {
            return;
        }
        self.trim(self.max_bytes - bytes, self.max_entries - 1);
        self.clock = self
            .clock
            .checked_add(1)
            .expect("cache access counter overflow");
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
    /// Drop exactly one invalidated resource without scanning unrelated entries.
    pub fn remove(&mut self, key: &K) -> Option<R> {
        let entry = self.entries.remove(key)?;
        self.bytes -= entry.bytes;
        self.ages.remove(&entry.used);
        Some(entry.value)
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
        self.ages = self
            .entries
            .iter()
            .map(|(key, entry)| (entry.used, key.clone()))
            .collect();
    }
}

impl<K: Eq + Hash + Clone, V, R: Clone> Default for Cache<K, V, R> {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::Cache;
    use std::rc::Rc;
    #[test]
    fn thread_affine_entries_obey_the_same_byte_and_lru_bounds() {
        let mut cache = Cache::<String, usize, Rc<usize>>::new(16, 3);
        let first = Rc::new(1);
        cache.insert("first".into(), first.clone(), 8);
        cache.insert("second".into(), Rc::new(2), 8);
        assert!(Rc::ptr_eq(&cache.get(&"first".into()).unwrap(), &first));
        cache.insert("third".into(), Rc::new(3), 8);
        assert!(cache.get(&"second".into()).is_none());
        cache.rekey(|key| {
            if key == "first" {
                Some("renamed".into())
            } else {
                None
            }
        });
        assert!(Rc::ptr_eq(&cache.get(&"renamed".into()).unwrap(), &first));
        cache.insert("oversized".into(), Rc::new(4), 17);
        assert!(cache.get(&"oversized".into()).is_none());
        cache.clear();
        assert_eq!(
            *first, 1,
            "eviction releases cache ownership, not caller references"
        );
        assert_eq!(Rc::strong_count(&first), 1);
    }
}
