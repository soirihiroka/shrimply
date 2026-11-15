//! Conservative visual invalidation intervals. Equality proves reuse; uncertainty invalidates.
use crate::{Animation, Fraction, HorizontalTransform, Property, RevealKind, Track, Value, math};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Fraction,
    /// Exclusive end; None means every later sample.
    pub end: Option<Fraction>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimelineDiff {
    pub invalidated: Vec<TimeRange>,
    pub old_duration: Fraction,
    pub new_duration: Fraction,
}
impl TimelineDiff {
    pub fn between(old: &Animation, new: &Animation) -> Self {
        let global = old.background != new.background
            || old.dpi != new.dpi
            || old.width != new.width
            || old.height != new.height
            || old.rate != new.rate;
        fn layers(animation: &Animation) -> BTreeMap<&str, (usize, &crate::Layer)> {
            animation
                .layers
                .iter()
                .enumerate()
                .map(|(index, layer)| (layer.element.id.as_str(), (index, layer)))
                .collect()
        }
        let old_layers = layers(old);
        let new_layers = layers(new);
        let births = |animation: &Animation| {
            let mut result = BTreeMap::new();
            animation.list_views.visit_reachable(
                animation
                    .layers
                    .iter()
                    .map(|layer| (&layer.element, layer.birth)),
                |reachable| {
                    result.insert(reachable.element.id.clone(), reachable.birth);
                },
            );
            result
        };
        let old_births = births(old);
        let new_births = births(new);
        let mut dirty_from = global.then_some(Fraction::from(0));
        if let Some(time) = old.list_views.changed_from(&new.list_views) {
            dirty_from = Some(dirty_from.map_or(time, |previous| previous.min(time)));
        }
        if let Some(time) = old.reactive.changed_from(&new.reactive, |id| {
            old_births
                .get(id)
                .into_iter()
                .chain(new_births.get(id))
                .copied()
                .min()
        }) {
            dirty_from = Some(dirty_from.map_or(time, |previous| previous.min(time)));
        }
        for id in old
            .code_views
            .keys()
            .chain(new.code_views.keys())
            .collect::<BTreeSet<_>>()
        {
            let empty = imbl::OrdMap::new();
            let a = old.code_views.get(id).map_or(&empty, |state| &state.events);
            let b = new.code_views.get(id).map_or(&empty, |state| &state.events);
            for change in a.diff(b) {
                let start = match change {
                    imbl::ordmap::DiffItem::Add(_, event)
                    | imbl::ordmap::DiffItem::Remove(_, event) => event.start,
                    imbl::ordmap::DiffItem::Update {
                        old: (_, a),
                        new: (_, b),
                    } => a.start.min(b.start),
                };
                dirty_from = Some(dirty_from.map_or(start, |previous| previous.min(start)));
            }
        }
        for id in old_layers
            .keys()
            .chain(new_layers.keys())
            .collect::<BTreeSet<_>>()
        {
            let a = old_layers.get(id);
            let b = new_layers.get(id);
            if a != b {
                let birth = a
                    .into_iter()
                    .chain(b)
                    .map(|(_, layer)| layer.birth)
                    .min()
                    .expect("changed layer");
                dirty_from = Some(dirty_from.map_or(birth, |previous| previous.min(birth)));
            }
        }
        let mut changed = Vec::new();
        let mut boundaries = BTreeSet::from([Fraction::from(0)]);
        boundaries.extend(dirty_from);
        for id in old
            .tracks
            .keys()
            .chain(new.tracks.keys())
            .collect::<BTreeSet<_>>()
        {
            let a = old.tracks.get(id);
            let b = new.tracks.get(id);
            for property in a
                .into_iter()
                .chain(b)
                .flat_map(|properties| properties.keys())
                .collect::<BTreeSet<_>>()
            {
                let a = a.and_then(|properties| properties.get(property));
                let b = b.and_then(|properties| properties.get(property));
                if a == b {
                    continue;
                }
                let birth = old_births
                    .get(id)
                    .into_iter()
                    .chain(new_births.get(id))
                    .copied()
                    .min();
                let Some(birth) = birth else {
                    continue;
                };
                for cycle in a
                    .into_iter()
                    .chain(b)
                    .flatten()
                    .filter_map(|track| track.cycle)
                {
                    dirty_from = Some(
                        dirty_from.map_or(cycle.start(), |previous| previous.min(cycle.start())),
                    );
                }
                changed.push((id, *property, birth));
                boundaries.insert(birth);
                boundaries.extend(
                    a.into_iter()
                        .chain(b)
                        .flatten()
                        .flat_map(|track| [track.start, track.end]),
                );
            }
        }
        let boundaries: Vec<_> = boundaries.into_iter().collect();
        let mut invalidated: Vec<TimeRange> = Vec::new();
        for (index, &start) in boundaries.iter().enumerate() {
            let dirty = dirty_from.is_some_and(|from| start >= from)
                || changed.iter().any(|(id, property, birth)| {
                    start >= *birth
                        && !equivalent_track(
                            old,
                            new,
                            id,
                            *property,
                            select_track(old, id, *property, start),
                            select_track(new, id, *property, start),
                            start,
                        )
                });
            if !dirty {
                continue;
            }
            let end = boundaries.get(index + 1).copied();
            if let Some(previous) = invalidated.last_mut()
                && previous.end == Some(start)
            {
                previous.end = end;
            } else {
                invalidated.push(TimeRange { start, end });
            }
        }
        Self {
            invalidated,
            old_duration: old.duration,
            new_duration: new.duration,
        }
    }
    pub fn affects(&self, time: Fraction) -> bool {
        let count = self
            .invalidated
            .partition_point(|range| range.start <= time);
        count > 0 && self.invalidated[count - 1].end.is_none_or(|end| time < end)
    }
}

/// Exact sample equivalence during holds. Every moving interval retains rational time.
#[derive(Clone, Debug)]
pub struct SampleTimes {
    boundaries: Vec<Fraction>,
    moving: Vec<TimeRange>,
}
#[derive(Default)]
struct ClockEvents {
    boundaries: BTreeSet<Fraction>,
    moving: Vec<TimeRange>,
}
impl ClockEvents {
    fn event(&mut self, start: Fraction, end: Fraction, changed: bool, birth: Fraction) {
        if end < birth {
            return;
        }
        let start = start.max(birth);
        let end = end.max(birth);
        self.boundaries.extend([start, end]);
        if changed && start < end {
            self.moving.push(TimeRange {
                start,
                end: Some(end),
            });
        }
    }
    fn cycle(&mut self, cycle: Option<crate::math::cycle::Cycle>) {
        if let Some(cycle) = cycle {
            self.moving.push(TimeRange {
                start: cycle.start(),
                end: cycle.until(),
            });
            self.boundaries.extend(cycle.until());
        }
    }
    fn finish(mut self) -> SampleTimes {
        self.boundaries.insert(Fraction::from(0));
        self.moving.sort_by_key(|range| range.start);
        let mut moving: Vec<TimeRange> = Vec::new();
        for range in self.moving {
            if let Some(previous) = moving.last_mut()
                && previous.end.is_none_or(|end| end >= range.start)
            {
                previous.end = previous.end.zip(range.end).map(|(a, b)| a.max(b));
            } else {
                moving.push(range);
            }
        }
        SampleTimes {
            boundaries: self.boundaries.into_iter().collect(),
            moving,
        }
    }
}
/// One event collector serves both whole-scene and mounted-layer clocks. Future
/// live-list descendants participate before they enter an initial scene snapshot.
fn clocks(animation: &Animation, layered: bool) -> Vec<SampleTimes> {
    let count = if layered { animation.layers.len() } else { 1 };
    let mut events: Vec<ClockEvents> = (0..count).map(|_| ClockEvents::default()).collect();
    let mut nodes = vec![Vec::new(); count];
    let mut elements = BTreeMap::new();
    let mut views = BTreeMap::new();
    animation.list_views.visit_reachable(
        animation
            .layers
            .iter()
            .map(|layer| (&layer.element, layer.birth)),
        |reachable| {
            let root = if layered { reachable.root } else { 0 };
            let id = reachable.element.id.as_str();
            events[root].boundaries.insert(reachable.birth);
            elements.insert(id, (root, reachable.birth));
            if let Some(properties) = animation.reactive.properties(id) {
                nodes[root].extend(properties.values().copied());
            }
            if let Some(cell) = animation.list_views.view_cell(id) {
                views.insert(cell, (root, reachable.birth));
            }
        },
    );
    for (id, properties) in &animation.tracks {
        if let Some(&(root, birth)) = elements.get(id.as_str()) {
            for track in properties.values().flatten() {
                events[root].event(track.start, track.end, track.from != track.to, birth);
                if track.from != track.to {
                    events[root].cycle(track.cycle);
                }
            }
        }
    }
    // The canonical graph walker visits shared ancestors once per layer instead
    // of rewalking the DAG independently for every bound property.
    for (root, nodes) in nodes.into_iter().enumerate() {
        if nodes.is_empty() {
            continue;
        }
        let cells = animation
            .reactive
            .dependency_cells(nodes)
            .expect("retained bindings reference a validated graph");
        let birth = if layered {
            animation.layers[root].birth
        } else {
            Fraction::from(0)
        };
        for id in cells {
            if let Some(cell) = animation.reactive.cells.get(&id) {
                for track in &cell.tracks {
                    events[root].event(track.start, track.end, track.from != track.to, birth);
                    if track.from != track.to {
                        events[root].cycle(track.cycle);
                    }
                }
            }
        }
    }
    for (cell, start, end, changed, cycle) in
        animation.list_views.events(|id| elements.contains_key(id))
    {
        if let Some(&(root, birth)) = views.get(&cell) {
            events[root].event(start, end, changed, birth);
            if changed {
                events[root].cycle(cycle);
            }
        }
    }
    for (id, state) in &animation.code_views {
        if let Some(&(root, birth)) = elements.get(id.as_str()) {
            for event in state.events.values() {
                events[root].event(event.start, event.end, event.from != event.to, birth);
                if event.from != event.to {
                    events[root].cycle(event.cycle);
                }
            }
        }
    }
    for (index, layer) in animation.layers.iter().enumerate() {
        events[if layered { index } else { 0 }]
            .cycle(layer.cycle.filter(|cycle| layer.birth > cycle.start()));
    }
    events.into_iter().map(ClockEvents::finish).collect()
}
impl SampleTimes {
    pub fn new(animation: &Animation) -> Self {
        clocks(animation, false)
            .pop()
            .expect("whole-scene sample clock")
    }
    /// Clocks match immutable mounted-layer indices and exclude unrelated layers.
    /// Build once per prepared scene; sampling performs only indexed lookups.
    pub fn for_layers(animation: &Animation) -> Vec<Self> {
        clocks(animation, true)
    }
    /// A hold's canonical time is reusable. None means some relevant state is moving.
    pub fn constant_time(&self, time: Fraction) -> Result<Option<Fraction>, String> {
        if !math::finite(&time) || time < Fraction::from(0) {
            return Err("sample time must be finite and nonnegative".into());
        }
        let count = self.moving.partition_point(|range| range.start <= time);
        if count > 0 && self.moving[count - 1].end.is_none_or(|end| time < end) {
            return Ok(None);
        }
        let count = self
            .boundaries
            .partition_point(|&boundary| boundary <= time);
        Ok(Some(self.boundaries[count - 1]))
    }
    pub fn canonical(&self, time: Fraction) -> Result<Fraction, String> {
        Ok(self.constant_time(time)?.unwrap_or(time))
    }
}

fn select_track<'a>(
    animation: &'a Animation,
    id: &str,
    property: Property,
    time: Fraction,
) -> Option<&'a Track> {
    let tracks = animation.tracks.get(id)?.get(&property)?;
    Track::at(tracks, time).ok().flatten()
}

fn equivalent_track(
    old: &Animation,
    new: &Animation,
    id: &str,
    property: Property,
    a: Option<&Track>,
    b: Option<&Track>,
    time: Fraction,
) -> bool {
    let moving = |track: Option<&Track>| {
        track.is_some_and(|track| time < track.end && track.from != track.to)
    };
    if moving(a) || moving(b) {
        return a == b;
    }
    let constant = |animation: &Animation,
                    track: Option<&Track>|
     -> Option<(Value, RevealKind, HorizontalTransform)> {
        let element = animation.find(id)?;
        Some(match track {
            Some(track) => (
                if time >= track.end {
                    track.to.clone()
                } else {
                    track.from.clone()
                },
                track.reveal.unwrap_or(element.reveal.kind),
                track.horizontal.unwrap_or(element.horizontal),
            ),
            None => (
                element.property(property).ok()?,
                element.reveal.kind,
                element.horizontal,
            ),
        })
    };
    constant(old, a) == constant(new, b)
}
