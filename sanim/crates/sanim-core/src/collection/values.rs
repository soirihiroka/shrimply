//! One immutable collection version; mutations fork only the affected vector path.
use crate::Value;
use imbl::Vector;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    io::{BufWriter, Write},
    ops::{Deref, DerefMut, RangeBounds},
    sync::{Arc, OnceLock},
};

#[derive(Debug, Default)]
struct Content {
    values: Vector<Value>,
    fingerprint: OnceLock<String>,
    budget: OnceLock<crate::value_budget::Cost>,
}
impl Clone for Content {
    fn clone(&self) -> Self {
        Self {
            values: self.values.clone(),
            ..Self::default()
        }
    }
}

/// Lists and arrays share this storage, including metadata for unchanged roots.
#[derive(Clone, Debug, Default)]
pub struct Values(Arc<Content>);
impl Values {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.values.ptr_eq(&other.0.values)
    }
    /// Content identity is computed once per immutable version, without a second buffer.
    pub fn fingerprint(&self) -> &str {
        self.0.fingerprint.get_or_init(|| {
            let mut hash = shrimply_resource_pipeline::fingerprint::Writer::default();
            {
                let mut writer = BufWriter::new(&mut hash);
                serde_json::to_writer(&mut writer, &self.0.values)
                    .expect("collection values serialize");
                writer
                    .flush()
                    .expect("in-memory content fingerprint flushes");
            }
            hash.finish()
        })
    }
    /// Reserve lazy metadata before publishing a version, even if not yet forced.
    pub(crate) fn metadata_bytes(&self) -> usize {
        std::mem::size_of::<Content>()
            + 2 * std::mem::size_of::<usize>()
            + shrimply_resource_pipeline::fingerprint::HEX_DIGEST_BYTES
    }
    pub(crate) fn budget_cost(&self, depth: usize) -> Option<crate::value_budget::Cost> {
        self.0
            .budget
            .get()
            .copied()
            .filter(|cost| cost.depth <= depth)
    }
    pub(crate) fn remember_budget_cost(&self, cost: crate::value_budget::Cost) {
        let _ = self.0.budget.set(cost);
    }
    pub fn append(&mut self, other: Self) {
        self.deref_mut().append(other.into_vector());
    }
    pub fn slice<R: RangeBounds<usize>>(&mut self, range: R) -> Self {
        self.deref_mut().slice(range).into()
    }
    pub fn split_off(&mut self, index: usize) -> Self {
        self.deref_mut().split_off(index).into()
    }
    fn into_vector(self) -> Vector<Value> {
        Arc::unwrap_or_clone(self.0).values
    }
}
impl Deref for Values {
    type Target = Vector<Value>;
    fn deref(&self) -> &Self::Target {
        &self.0.values
    }
}
impl DerefMut for Values {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let content = Arc::make_mut(&mut self.0);
        content.fingerprint.take();
        content.budget.take();
        &mut content.values
    }
}
impl PartialEq for Values {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || self.0.values == other.0.values
    }
}
impl From<Vector<Value>> for Values {
    fn from(values: Vector<Value>) -> Self {
        Self(Arc::new(Content {
            values,
            ..Content::default()
        }))
    }
}
impl From<Vec<Value>> for Values {
    fn from(values: Vec<Value>) -> Self {
        Vector::from(values).into()
    }
}
impl FromIterator<Value> for Values {
    fn from_iter<T: IntoIterator<Item = Value>>(values: T) -> Self {
        values.into_iter().collect::<Vector<_>>().into()
    }
}
impl IntoIterator for Values {
    type Item = Value;
    type IntoIter = <Vector<Value> as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.into_vector().into_iter()
    }
}
impl<'a> IntoIterator for &'a Values {
    type Item = &'a Value;
    type IntoIter = <&'a Vector<Value> as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.0.values.iter()
    }
}
impl<'a> IntoIterator for &'a mut Values {
    type Item = &'a mut Value;
    type IntoIter = <&'a mut Vector<Value> as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.deref_mut().into_iter()
    }
}
impl Serialize for Values {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.values.serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Values {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vector::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lazy_metadata_does_not_retain_old_collection_versions() {
        let original: Values = vec![Value::Bool(true)].into();
        original.fingerprint();
        let old = Arc::downgrade(&original.0);
        let mut changed = original.clone();
        changed.push_back(Value::Bool(false));
        changed.fingerprint();
        drop(original);
        assert!(
            old.upgrade().is_none(),
            "no metadata registry or collector retains obsolete roots"
        );
        assert_eq!(changed.len(), 2);
    }

    #[test]
    fn budget_summaries_preserve_exact_depth_and_bounds_across_cow_edits() {
        let original = crate::List::typed(
            crate::Type::Numeric(crate::Unit::Number),
            (0..1024)
                .map(|n| Value::Number(crate::Number::scalar(n)))
                .collect(),
        );
        let measure = |list: &crate::List, depth, bytes_limit, items_limit| {
            let (mut bytes, mut items) = (bytes_limit, items_limit);
            crate::value_budget::charge(&Value::List(list.clone()), &mut bytes, &mut items, depth)
                .map(|()| (bytes_limit - bytes, items_limit - items))
        };
        let cost = measure(&original, 256, usize::MAX, usize::MAX).unwrap();
        assert_eq!(original.values.0.budget.get().unwrap().depth, 2);
        assert_eq!(measure(&original, 2, usize::MAX, usize::MAX).unwrap(), cost);
        assert!(measure(&original, 1, usize::MAX, usize::MAX).is_err());
        assert!(measure(&original, 2, cost.0 - 1, usize::MAX).is_err());
        assert!(measure(&original, 2, usize::MAX, cost.1 - 1).is_err());
        let mut changed = original.clone();
        changed.values.push_back(Value::Bool(true));
        assert!(changed.values.0.budget.get().is_none());
        assert!(original.values.0.budget.get().is_some());
        let next = measure(&changed, 2, usize::MAX, usize::MAX).unwrap();
        assert!(next.0 > cost.0 && next.1 > cost.1);

        let nested = crate::List::typed(
            crate::Type::Dict,
            vec![Value::Dict(
                [("nested".to_string(), serde_json::json!({"x": [1]}))]
                    .into_iter()
                    .collect(),
            )],
        );
        let expected = measure(&nested, 256, usize::MAX, usize::MAX).unwrap();
        let required = nested.values.0.budget.get().unwrap().depth;
        assert_eq!(
            measure(&nested, required, usize::MAX, usize::MAX).unwrap(),
            expected
        );
        assert!(measure(&nested, required - 1, usize::MAX, usize::MAX).is_err());
    }

    #[test]
    fn shared_payload_receipts_never_become_context_dependent_cost_summaries() {
        let list = crate::List::typed(crate::Type::String, vec![Value::String("shared".into())]);
        let mut budget = crate::value_budget::Budget::default();
        let (mut bytes, mut items) = (usize::MAX, usize::MAX);
        crate::value_budget::charge_with_budget(
            &Value::List(list.clone()),
            &mut bytes,
            &mut items,
            64,
            &mut budget,
        )
        .unwrap();
        let first = usize::MAX - bytes;
        assert!(list.values.0.budget.get().is_none());
        let before = bytes;
        crate::value_budget::charge_with_budget(
            &Value::List(list.clone()),
            &mut bytes,
            &mut items,
            64,
            &mut budget,
        )
        .unwrap();
        assert!(
            before - bytes < first,
            "shared text is charged only once in a shared receipt set"
        );
        assert!(
            list.values.0.budget.get().is_none(),
            "an already-seen receipt is still context dependent"
        );
    }

    #[test]
    #[ignore = "manual immutable collection metadata benchmark"]
    fn persistent_values_benchmark() {
        use std::{hint::black_box, time::Instant};
        const ITEMS: usize = 100_000;
        const FRESH_READS: usize = 10;
        const SHARED_READS: usize = 100_000;
        let list = crate::List::typed(
            crate::Type::Numeric(crate::Unit::Number),
            (0..ITEMS)
                .map(|n| Value::Number(crate::Number::scalar(n as i64)))
                .collect(),
        );
        let first = Instant::now();
        let hash = list.values.fingerprint();
        eprintln!("first streamed fingerprint: {:?}", first.elapsed());
        let started = Instant::now();
        for _ in 0..SHARED_READS {
            black_box(black_box(&list.values).fingerprint());
        }
        eprintln!(
            "shared fingerprint: {:?} total ({SHARED_READS} reads over {ITEMS} values)",
            started.elapsed()
        );
        let started = Instant::now();
        for _ in 0..FRESH_READS {
            let mut writer = shrimply_resource_pipeline::fingerprint::Writer::default();
            serde_json::to_writer(&mut writer, &list.values).unwrap();
            assert_eq!(writer.finish(), hash);
        }
        eprintln!(
            "fresh unbuffered streamed fingerprint: {}ns/read",
            started.elapsed().as_nanos() / FRESH_READS as u128
        );
        let started = Instant::now();
        for _ in 0..FRESH_READS {
            let fresh: Values = list.values.0.values.clone().into();
            assert_eq!(fresh.fingerprint(), hash);
        }
        eprintln!(
            "fresh buffered streamed fingerprint: {}ns/read",
            started.elapsed().as_nanos() / FRESH_READS as u128
        );

        let charge = |list: &crate::List| {
            let (mut bytes, mut items) = (usize::MAX, usize::MAX);
            crate::value_budget::charge(&Value::List(list.clone()), &mut bytes, &mut items, 64)
                .unwrap();
            (bytes, items)
        };
        let expected = charge(&list);
        let started = Instant::now();
        for _ in 0..SHARED_READS {
            assert_eq!(black_box(charge(black_box(&list))), expected);
        }
        eprintln!(
            "shared budget summary: {}ns/read",
            started.elapsed().as_nanos() / SHARED_READS as u128
        );
        let started = Instant::now();
        for _ in 0..FRESH_READS {
            let fresh = crate::List::from_values(
                list.element_type.clone(),
                list.values.0.values.clone().into(),
            );
            assert_eq!(black_box(charge(&fresh)), expected);
        }
        eprintln!(
            "fresh budget traversal: {}ns/read",
            started.elapsed().as_nanos() / FRESH_READS as u128
        );
    }
}
