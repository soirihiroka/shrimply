//! Compact exact event clocks: finite boundaries are indexed once, repeats stay periodic.
use crate::{Fraction, fraction_floor_i64};
use num_traits::{CheckedAdd, CheckedDiv, CheckedMul, CheckedSub};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Group {
    #[serde(serialize_with = "crate::serialize_fraction")]
    pub start: Fraction,
    #[serde(serialize_with = "crate::serialize_fraction")]
    pub end: Fraction,
    #[serde(serialize_with = "serialize_optional_fraction")]
    pub until: Option<Fraction>,
    /// Absolute positions within the first period, not expanded repetitions.
    #[serde(serialize_with = "serialize_fractions")]
    pub boundaries: Arc<[Fraction]>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Index {
    #[serde(serialize_with = "serialize_fractions")]
    finite: Arc<[Fraction]>,
    #[serde(serialize_with = "serialize_slice")]
    groups: Arc<[Group]>,
}

/// Incremental admission merges equal clocks and phases before allocating immutable arrays.
pub struct Builder {
    finite: BTreeSet<Fraction>,
    groups: BTreeMap<(Fraction, Fraction, Option<Fraction>), BTreeSet<Fraction>>,
    bytes: usize,
    max_bytes: usize,
}
impl Builder {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            finite: BTreeSet::new(),
            groups: BTreeMap::new(),
            bytes: std::mem::size_of::<Index>(),
            max_bytes,
        }
    }
    pub fn boundaries(&mut self, values: impl IntoIterator<Item = Fraction>) -> Result<(), String> {
        for value in values {
            if !valid(value) {
                return Err("event boundaries must be finite and nonnegative".into());
            }
            if self.finite.insert(value) {
                admit(
                    &mut self.bytes,
                    self.max_bytes,
                    std::mem::size_of::<Fraction>(),
                )?;
            }
        }
        Ok(())
    }
    pub fn clock(
        &mut self,
        start: Fraction,
        end: Fraction,
        until: Option<Fraction>,
        values: impl IntoIterator<Item = Fraction>,
    ) -> Result<(), String> {
        if !valid(start)
            || !valid(end)
            || end <= start
            || until.is_some_and(|until| !valid(until) || until < start)
        {
            return Err("periodic event clock must have a positive finite period and valid clipping interval".into());
        }
        let key = (start, end, until);
        if !self.groups.contains_key(&key) {
            admit(
                &mut self.bytes,
                self.max_bytes,
                std::mem::size_of::<Group>(),
            )?;
        }
        let boundaries = self.groups.entry(key).or_default();
        for value in values {
            if !valid(value) || value < start || value > end {
                return Err("periodic event boundaries must lie within their first period".into());
            }
            if boundaries.insert(value) {
                admit(
                    &mut self.bytes,
                    self.max_bytes,
                    std::mem::size_of::<Fraction>(),
                )?;
            }
        }
        Ok(())
    }
    pub fn finish(mut self) -> Result<Index, String> {
        admit(&mut self.bytes, self.max_bytes, 0)?;
        Ok(Index {
            finite: self.finite.into_iter().collect(),
            groups: self
                .groups
                .into_iter()
                .filter_map(|((start, end, until), boundaries)| {
                    (!boundaries.is_empty()).then(|| Group {
                        start,
                        end,
                        until,
                        boundaries: boundaries.into_iter().collect(),
                    })
                })
                .collect(),
        })
    }
}
fn admit(bytes: &mut usize, max_bytes: usize, additional: usize) -> Result<(), String> {
    *bytes = bytes
        .checked_add(additional)
        .ok_or("event clock index size overflow")?;
    if *bytes > max_bytes {
        return Err(format!(
            "event clock index needs {bytes} bytes; limit {max_bytes}"
        ));
    }
    Ok(())
}
fn valid(value: Fraction) -> bool {
    matches!(value, Fraction::Rational(..)) && value >= Fraction::from(0)
}

fn serialize_slice<S: serde::Serializer, T: Serialize>(
    values: &Arc<[T]>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    values.as_ref().serialize(serializer)
}
struct FractionValue<'a>(&'a Fraction);
impl Serialize for FractionValue<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        crate::serialize_fraction(self.0, serializer)
    }
}
fn serialize_fractions<S: serde::Serializer>(
    values: &Arc<[Fraction]>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut sequence = serializer.serialize_seq(Some(values.len()))?;
    for value in values.iter() {
        sequence.serialize_element(&FractionValue(value))?;
    }
    sequence.end()
}
fn serialize_optional_fraction<S: serde::Serializer>(
    value: &Option<Fraction>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.as_ref().map(FractionValue).serialize(serializer)
}

impl Index {
    pub fn is_empty(&self) -> bool {
        self.finite.is_empty() && self.groups.is_empty()
    }
    /// Canonical clock groups are merged before admission; equal times have one result.
    pub fn new(
        finite: Vec<Fraction>,
        groups: Vec<Group>,
        max_bytes: usize,
    ) -> Result<Self, String> {
        let mut builder = Builder::new(max_bytes);
        builder.boundaries(finite)?;
        for group in groups {
            builder.clock(
                group.start,
                group.end,
                group.until,
                group.boundaries.iter().copied(),
            )?;
        }
        builder.finish()
    }
    pub fn finite(&self) -> Arc<[Fraction]> {
        self.finite.clone()
    }
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }
    pub fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of_val(self.finite.as_ref())
            + std::mem::size_of_val(self.groups.as_ref())
            + self
                .groups
                .iter()
                .map(|group| std::mem::size_of_val(group.boundaries.as_ref()))
                .sum::<usize>()
    }
    /// O(log finite boundaries + clock groups × log period boundaries), never O(playback cycles).
    pub fn neighbors(&self, time: Fraction) -> [Option<Fraction>; 2] {
        if !matches!(time, Fraction::Rational(..)) {
            return [None, None];
        }
        let mut result = adjacent(&self.finite, time);
        for group in self.groups.iter() {
            for candidate in group.neighbors(time).into_iter().flatten() {
                if candidate <= time {
                    result[0] = Some(result[0].map_or(candidate, |before| before.max(candidate)));
                }
                if candidate >= time {
                    result[1] = Some(result[1].map_or(candidate, |after| after.min(candidate)));
                }
            }
        }
        result
    }
    /// A source crop is a query constraint, not an expanded or rebuilt event list.
    pub fn neighbors_in(
        &self,
        time: Fraction,
        start: Fraction,
        end: Fraction,
    ) -> [Option<Fraction>; 2] {
        if end < start {
            return [None, None];
        }
        let mut result = [None::<Fraction>; 2];
        for candidate in self
            .neighbors(time.max(start).min(end))
            .into_iter()
            .flatten()
        {
            if candidate < start || candidate > end {
                continue;
            }
            if candidate <= time {
                result[0] = Some(result[0].map_or(candidate, |before| before.max(candidate)));
            }
            if candidate >= time {
                result[1] = Some(result[1].map_or(candidate, |after| after.min(candidate)));
            }
        }
        result
    }
}

fn adjacent(boundaries: &[Fraction], time: Fraction) -> [Option<Fraction>; 2] {
    let index = boundaries.partition_point(|boundary| *boundary < time);
    let next = boundaries.get(index).copied();
    [
        if next == Some(time) {
            next
        } else {
            index.checked_sub(1).map(|index| boundaries[index])
        },
        next,
    ]
}

impl Group {
    fn neighbors(&self, time: Fraction) -> [Option<Fraction>; 2] {
        let query = self
            .until
            .map_or(time, |until| time.min(until))
            .max(self.start);
        let Some(period) = self.end.checked_sub(&self.start) else {
            return [None, None];
        };
        let Some(offset) = query.checked_sub(&self.start) else {
            return [None, None];
        };
        let Some(cycle) = offset.checked_div(&period).and_then(fraction_floor_i64) else {
            return [None, None];
        };
        let Some(base) = period.checked_mul(&Fraction::from(cycle)) else {
            return [None, None];
        };
        let Some(phase) = query.checked_sub(&base) else {
            return [None, None];
        };
        let local = adjacent(&self.boundaries, phase);
        let mut result = [None::<Fraction>; 2];
        for candidate in [
            local[0]
                .and_then(|value| value.checked_add(&base))
                .or_else(|| {
                    if cycle == 0 {
                        return None;
                    }
                    self.boundaries
                        .last()?
                        .checked_add(&base)?
                        .checked_sub(&period)
                }),
            local[1]
                .and_then(|value| value.checked_add(&base))
                .or_else(|| {
                    self.boundaries
                        .first()?
                        .checked_add(&base)?
                        .checked_add(&period)
                }),
        ]
        .into_iter()
        .flatten()
        {
            if candidate < self.start || self.until.is_some_and(|until| candidate > until) {
                continue;
            }
            if candidate <= time {
                result[0] = Some(result[0].map_or(candidate, |before| before.max(candidate)));
            }
            if candidate >= time {
                result[1] = Some(result[1].map_or(candidate, |after| after.min(candidate)));
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BUDGET: usize = 1024 * 1024;
    fn group(until: Option<Fraction>) -> Group {
        Group {
            start: 2.into(),
            end: 3.into(),
            until,
            boundaries: vec![2.into(), Fraction::new(5u64, 2u64), 3.into()].into(),
        }
    }
    #[test]
    fn late_periodic_event_queries_are_exact_and_do_not_expand() {
        let index = Index::new(vec![0.into()], vec![group(None)], BUDGET).unwrap();
        let bytes = index.estimated_bytes();
        let late = Fraction::new(2_000_001u64, 4u64);
        assert_eq!(
            index.neighbors(late),
            [
                Some(500_000.into()),
                Some(Fraction::new(1_000_001u64, 2u64))
            ]
        );
        assert_eq!(index.neighbors(500_000.into()), [Some(500_000.into()); 2]);
        assert_eq!(index.estimated_bytes(), bytes);
        assert_eq!(index.groups().len(), 1);
        assert_eq!(index.groups()[0].boundaries.len(), 3);
    }
    #[test]
    fn coincident_clock_groups_and_finite_events_merge() {
        let index = Index::new(
            vec![0.into(), 0.into(), 2.into()],
            vec![group(None), group(None)],
            BUDGET,
        )
        .unwrap();
        assert_eq!(index.finite().as_ref(), &[Fraction::from(0), 2.into()]);
        assert_eq!(index.groups().len(), 1);
        assert_eq!(index.neighbors(2.into()), [Some(2.into()); 2]);
    }
    #[test]
    fn bounded_periods_and_crops_match_an_expanded_oracle() {
        let mut repeated = group(Some(Fraction::new(39u64, 4u64)));
        repeated.boundaries = vec![2.into(), Fraction::new(5u64, 2u64)].into();
        let index = Index::new(vec![0.into(), 12.into()], vec![repeated], BUDGET).unwrap();
        let mut expanded = vec![Fraction::from(0), 12.into()];
        expanded.extend((0..16u64).map(|index| Fraction::from(2) + Fraction::new(index, 2u64)));
        expanded.sort_unstable();
        for tick in 0..60u64 {
            let at = Fraction::new(tick, 4u64);
            assert_eq!(index.neighbors(at), adjacent(&expanded, at));
            let cropped = expanded
                .iter()
                .copied()
                .filter(|value| *value >= 3.into() && *value <= 8.into())
                .collect::<Vec<_>>();
            assert_eq!(
                index.neighbors_in(at, 3.into(), 8.into()),
                adjacent(&cropped, at)
            );
        }
    }
    #[test]
    fn finite_repeat_three_keeps_each_boundary() {
        let index = Index::new(
            Vec::new(),
            vec![Group {
                start: 0.into(),
                end: 1.into(),
                until: Some(3.into()),
                boundaries: vec![0.into(), 1.into()].into(),
            }],
            BUDGET,
        )
        .unwrap();
        for tick in 0..=3 {
            assert_eq!(
                index.neighbors(Fraction::from(tick)),
                [Some(Fraction::from(tick)); 2]
            );
        }
        assert_eq!(index.neighbors(4.into()), [Some(3.into()), None]);
    }
    #[test]
    fn period_end_events_do_not_create_a_previous_cycle_before_activation() {
        let index = Index::new(
            Vec::new(),
            vec![Group {
                start: 2.into(),
                end: 3.into(),
                until: None,
                boundaries: vec![3.into()].into(),
            }],
            BUDGET,
        )
        .unwrap();
        assert_eq!(index.neighbors(2.into()), [None, Some(3.into())]);
        assert_eq!(index.neighbors(3.into()), [Some(3.into()); 2]);
    }
    #[test]
    fn event_clock_storage_is_shared_and_budgeted() {
        let index = Index::new(vec![0.into()], vec![group(None)], BUDGET).unwrap();
        assert!(Arc::ptr_eq(&index.finite(), &index.clone().finite()));
        assert!(Index::new(vec![0.into()], vec![group(None)], 0).is_err());
        let mut invalid = group(None);
        invalid.end = invalid.start;
        assert!(Index::new(Vec::new(), vec![invalid], BUDGET).is_err());
    }

    #[test]
    #[ignore = "manual performance regression measurement"]
    fn sanim_event_clock_benchmark() {
        let index = Arc::new(Index::new(vec![0.into()], vec![group(None)], BUDGET).unwrap());
        let bytes = index.estimated_bytes();
        let started = std::time::Instant::now();
        for tick in 0..10_000u64 {
            let at = Fraction::from(500_000) + Fraction::new(tick, 17u64);
            std::hint::black_box(index.neighbors(at));
        }
        eprintln!(
            "10k late periodic event queries: {:?}; index bytes={bytes}",
            started.elapsed()
        );
        assert_eq!(index.estimated_bytes(), bytes);
        let weak = Arc::downgrade(&index);
        drop(index);
        assert!(weak.upgrade().is_none());
    }
}
