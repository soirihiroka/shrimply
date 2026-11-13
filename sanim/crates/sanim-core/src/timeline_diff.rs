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
                |element, birth| {
                    result.insert(element.id.clone(), birth);
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

pub(crate) fn events(animation: &Animation) -> BTreeSet<Fraction> {
    let visible = animation
        .list_views
        .reachable_views(animation.layers.iter().map(|layer| &layer.element));
    let cells = animation
        .reactive
        .bound_cells(|id| animation.find(id).is_some());
    std::iter::once(Fraction::from(0))
        .chain(animation.layers.iter().map(|layer| layer.birth))
        .chain(
            animation
                .tracks
                .values()
                .flat_map(|properties| properties.values())
                .flatten()
                .flat_map(|track| [track.start, track.end]),
        )
        .chain(
            cells
                .iter()
                .filter_map(|id| animation.reactive.cells.get(id))
                .flat_map(|cell| &cell.tracks)
                .flat_map(|track| [track.start, track.end]),
        )
        .chain(
            animation
                .list_views
                .events(|id| visible.contains(id))
                .flat_map(|(start, end, _)| [start, end]),
        )
        .chain(code_events(animation).flat_map(|event| [event.start, event.end]))
        .collect()
}

fn code_events(
    animation: &Animation,
) -> impl Iterator<Item = &crate::code::view::PresentationEvent> {
    animation
        .code_views
        .iter()
        .filter(|(id, _)| animation.find(id).is_some())
        .flat_map(|(_, state)| state.events.values())
}

/// Index of exact sample equivalence during holds. Active motions retain exact rational time.
pub struct SampleTimes {
    boundaries: Vec<Fraction>,
    moving: Vec<TimeRange>,
}
impl SampleTimes {
    pub fn new(animation: &Animation) -> Self {
        let visible = animation
            .list_views
            .reachable_views(animation.layers.iter().map(|layer| &layer.element));
        let cells = animation
            .reactive
            .bound_cells(|id| animation.find(id).is_some());
        let mut moving: Vec<_> = animation
            .tracks
            .values()
            .flat_map(|properties| properties.values())
            .flatten()
            .chain(
                cells
                    .iter()
                    .filter_map(|id| animation.reactive.cells.get(id))
                    .flat_map(|cell| &cell.tracks),
            )
            .filter(|track| track.from != track.to)
            .map(|track| TimeRange {
                start: track.start,
                end: Some(track.end),
            })
            .collect();
        moving.extend(
            animation
                .list_views
                .events(|id| visible.contains(id))
                .filter(|(start, end, changed)| *changed && start < end)
                .map(|(start, end, _)| TimeRange {
                    start,
                    end: Some(end),
                }),
        );
        moving.extend(
            code_events(animation)
                .filter(|event| event.start < event.end && event.from != event.to)
                .map(|event| TimeRange {
                    start: event.start,
                    end: Some(event.end),
                }),
        );
        moving.sort_by_key(|range| range.start);
        let mut merged: Vec<TimeRange> = Vec::new();
        for range in moving {
            if let Some(previous) = merged.last_mut()
                && previous.end >= Some(range.start)
            {
                previous.end = previous.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        Self {
            boundaries: events(animation).into_iter().collect(),
            moving: merged,
        }
    }
    pub fn canonical(&self, time: Fraction) -> Result<Fraction, String> {
        if !math::finite(&time) || time < Fraction::from(0) {
            return Err("sample time must be finite and nonnegative".into());
        }
        let count = self.moving.partition_point(|range| range.start <= time);
        if count > 0 && self.moving[count - 1].end.is_some_and(|end| time < end) {
            return Ok(time);
        }
        let count = self
            .boundaries
            .partition_point(|&boundary| boundary <= time);
        Ok(self.boundaries[count - 1])
    }
}

fn select_track<'a>(
    animation: &'a Animation,
    id: &str,
    property: Property,
    time: Fraction,
) -> Option<&'a Track> {
    let tracks = animation.tracks.get(id)?.get(&property)?;
    Track::at(tracks, time)
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
