//! Compact execution winners; repeated clocks never expand into individual cycles.
use super::{Fraction, TimeMapEntry, math};
use std::{collections::BTreeMap, sync::Arc};

type Key = (usize, usize);
type Events<T, P = usize> = BTreeMap<P, Vec<(bool, T)>>;
const MISSING: usize = usize::MAX;

#[derive(Clone)]
pub(super) struct Index {
    count: Option<usize>,
    rate: Fraction,
    dense: Arc<[Option<Arc<TimeMapEntry>>]>,
    base: Runs<usize>,
    clocks: Vec<Clock>,
    active: Runs<imbl::OrdSet<usize>>,
}
#[derive(Clone)]
struct Clock {
    cycle: crate::math::cycle::Cycle,
    phases: Runs<usize, Fraction>,
    points: BTreeMap<Fraction, Key>,
}
#[derive(Clone, Default)]
struct Runs<T, P = usize> {
    starts: Vec<P>,
    values: Vec<T>,
}
impl<T, P: Ord> Runs<T, P> {
    fn get(&self, frame: P) -> Option<&T> {
        self.starts
            .partition_point(|start| *start <= frame)
            .checked_sub(1)
            .map(|index| &self.values[index])
    }
}
impl Default for Index {
    fn default() -> Self {
        Self {
            count: Some(0),
            rate: Fraction::from(0),
            dense: Arc::default(),
            base: Runs::default(),
            clocks: Vec::new(),
            active: Runs::default(),
        }
    }
}
impl Index {
    pub(super) fn count(&self) -> Option<usize> {
        self.count
    }
    pub(super) fn at<'a>(
        &'a self,
        frame: usize,
        entries: &'a imbl::Vector<Arc<TimeMapEntry>>,
    ) -> Option<&'a TimeMapEntry> {
        if self.count.is_some_and(|count| frame >= count) {
            return None;
        }
        if !self.dense.is_empty() {
            return self.dense[frame].as_deref();
        }
        let time = math::arithmetic("/", Fraction::from(frame as u64), self.rate).ok()?;
        let mut winner = self
            .base
            .get(frame)
            .copied()
            .filter(|index| *index != MISSING);
        if let Some(active) = self.active.get(frame) {
            for index in active {
                let clock = &self.clocks[*index];
                if time < clock.cycle.start() || clock.cycle.until().is_some_and(|end| time > end) {
                    continue;
                }
                let phase = clock.cycle.time(time).ok()?;
                for candidate in [
                    clock.phases.get(phase).copied(),
                    clock.points.get(&phase).map(|key| key.0),
                ]
                .into_iter()
                .flatten()
                .filter(|index| *index != MISSING)
                {
                    if winner.is_none_or(|previous| {
                        key(&entries[candidate], candidate) > key(&entries[previous], previous)
                    }) {
                        winner = Some(candidate);
                    }
                }
            }
        }
        entries.get(winner?).map(AsRef::as_ref)
    }
    pub(super) fn build(
        entries: &imbl::Vector<Arc<TimeMapEntry>>,
        count: Option<usize>,
        rate: Fraction,
        mut budget: usize,
        event_index: &crate::EventIndex,
    ) -> Result<Self, String> {
        if count == Some(0) {
            return Ok(Self::default());
        }
        let mut base = Events::new();
        let mut groups = vec![
            (
                None,
                Events::<Key, Fraction>::new(),
                BTreeMap::<Fraction, Key>::new()
            );
            event_index.groups().len()
        ];
        for (index, entry) in entries.iter().enumerate() {
            if let Some(cycle) = entry.clock.as_deref() {
                let group = event_index
                    .groups()
                    .binary_search_by_key(&(cycle.start(), cycle.end(), cycle.until()), |group| {
                        (group.start, group.end, group.until)
                    })
                    .map_err(
                        |_| "time map source clock is absent from its canonical event index",
                    )?;
                groups[group].0 = Some(*cycle);
                let end = entry.end.unwrap_or(cycle.end());
                if entry.start == end {
                    groups[group]
                        .2
                        .entry(entry.start)
                        .and_modify(|old| *old = (*old).max(key(entry, index)))
                        .or_insert(key(entry, index));
                } else {
                    groups[group]
                        .1
                        .entry(entry.start)
                        .or_default()
                        .push((true, key(entry, index)));
                    groups[group]
                        .1
                        .entry(end)
                        .or_default()
                        .push((false, key(entry, index)));
                }
            } else {
                add(&mut base, entry, rate, index)?;
            }
        }
        let base = winners(base, count, &mut budget)?;
        let mut clocks = Vec::with_capacity(groups.len());
        let mut active = Events::new();
        for (cycle, events, points) in groups {
            let cycle = cycle.ok_or("time map event clock has no source provenance")?;
            let index = clocks.len();
            let start = frame_at(cycle.start(), rate)?;
            let end = cycle
                .until()
                .map(|end| frame_at(end, rate))
                .transpose()?
                .map(|end| end.saturating_add(1));
            let end = match (end, count) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            if end.is_none_or(|end| start < end) {
                active.entry(start).or_default().push((true, index));
                if let Some(end) = end {
                    active.entry(end).or_default().push((false, index));
                }
            }
            charge(
                &mut budget,
                points.len().saturating_mul(
                    std::mem::size_of::<(Fraction, Key)>() + 4 * std::mem::size_of::<usize>(),
                ),
            )?;
            clocks.push(Clock {
                cycle,
                phases: winners(events, None, &mut budget)?,
                points,
            });
        }
        charge(
            &mut budget,
            clocks
                .capacity()
                .saturating_mul(std::mem::size_of::<Clock>()),
        )?;
        let active = sweep(
            active,
            count,
            &mut budget,
            |set| set.clone(),
            |set| set.len().saturating_mul(std::mem::size_of::<usize>() * 4),
        )?;
        let mut index = Self {
            count,
            rate,
            dense: Arc::default(),
            base,
            clocks,
            active,
        };
        // Small timelines retain direct frame lookup. Large timelines retain only event runs.
        let dense_bytes = count
            .and_then(|count| count.checked_mul(std::mem::size_of::<Option<Arc<TimeMapEntry>>>()));
        if let Some(count) = count
            .filter(|_| index.clocks.is_empty() && dense_bytes.is_some_and(|bytes| bytes <= budget))
        {
            let mut dense = vec![None; count];
            for (run, start) in index.base.starts.iter().copied().enumerate() {
                let end = index.base.starts.get(run + 1).copied().unwrap_or(count);
                let winner = index.base.values[run];
                dense[start..end].fill((winner != MISSING).then(|| entries[winner].clone()));
            }
            index.dense = dense.into();
            index.base = Runs::default();
            index.clocks.clear();
            index.active = Runs::default();
        }
        Ok(index)
    }
}
fn key(entry: &TimeMapEntry, index: usize) -> Key {
    (index, entry.statement)
}
fn frame_at(time: Fraction, rate: Fraction) -> Result<usize, String> {
    usize::try_from(math::frame_count(time, rate)?)
        .map_err(|_| "time map frame exceeds address space".into())
}
fn add(
    events: &mut Events<Key>,
    entry: &TimeMapEntry,
    rate: Fraction,
    index: usize,
) -> Result<(), String> {
    let start = frame_at(entry.start, rate)?;
    let Some(end) = entry.end else {
        events
            .entry(start)
            .or_default()
            .push((true, key(entry, index)));
        return Ok(());
    };
    let mut end_frame = frame_at(end, rate)?;
    if entry.start == end {
        if math::arithmetic("/", Fraction::from(start as u64), rate)? != entry.start {
            return Ok(());
        }
        end_frame = start.checked_add(1).ok_or("time map boundary overflow")?;
    }
    if end_frame > start {
        events
            .entry(start)
            .or_default()
            .push((true, key(entry, index)));
        events
            .entry(end_frame)
            .or_default()
            .push((false, key(entry, index)));
    }
    Ok(())
}
fn winners<P: Ord + Copy>(
    events: Events<Key, P>,
    count: Option<P>,
    budget: &mut usize,
) -> Result<Runs<usize, P>, String> {
    sweep(
        events,
        count,
        budget,
        |set| set.iter().next_back().map_or(MISSING, |key| key.0),
        |_| 0,
    )
}
fn sweep<K: Ord + Copy, V: PartialEq, P: Ord + Copy>(
    events: Events<K, P>,
    count: Option<P>,
    budget: &mut usize,
    value: impl Fn(&imbl::OrdSet<K>) -> V,
    extra: impl Fn(&imbl::OrdSet<K>) -> usize,
) -> Result<Runs<V, P>, String> {
    let mut active = imbl::OrdSet::new();
    let mut runs = Runs {
        starts: Vec::new(),
        values: Vec::new(),
    };
    for (frame, changes) in events {
        if count.is_some_and(|count| frame >= count) {
            break;
        }
        for (_, key) in changes.iter().filter(|(add, _)| !*add) {
            active.remove(key);
        }
        for (_, key) in changes.into_iter().filter(|(add, _)| *add) {
            active.insert(key);
        }
        let current = value(&active);
        if runs.values.last() != Some(&current) {
            charge(
                budget,
                (if runs.values.is_empty() { 4 } else { 2 })
                    * (std::mem::size_of::<P>() + std::mem::size_of::<V>())
                    + extra(&active),
            )?;
            runs.starts.push(frame);
            runs.values.push(current);
        }
    }
    Ok(runs)
}
fn charge(budget: &mut usize, bytes: usize) -> Result<(), String> {
    *budget=budget.checked_sub(bytes).ok_or("time map execution index exceeds its byte budget; reduce concurrent source events or increase the host's time-map limit")?;
    Ok(())
}
