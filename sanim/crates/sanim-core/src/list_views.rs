//! Persistent list snapshots and indexed transitions; sampling never replays list edits.
mod fingerprint;
mod serialization;
use crate::{Animation, Easing, Element, Fraction, Geometry, List, Type, Value, math};
use imbl::{OrdMap, OrdSet, Vector};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

fn same_snapshot(a: &List, b: &List) -> bool {
    a.element_type == b.element_type && a.shares_values(b)
}
fn flow_ids(items: &List) -> Result<OrdSet<String>, String> {
    let mut ids = OrdSet::new();
    for element in elements(items)? {
        ids.insert(element.id.clone());
        crate::scene::collect_flow_positions(element, &mut ids);
    }
    Ok(ids)
}

const MAX_LIST_EVENTS: usize =
    crate::reactive::MAX_REACTIVE_BYTES / std::mem::size_of::<Transition>();
const MAX_LIST_ELEMENTS: usize = 100_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "serialization::ListViewsData")]
pub struct ListViews {
    channels: OrdMap<u64, Channel>,
    views: OrdMap<String, u64>,
    /// Each stable identity has one immutable definition, shared across snapshots.
    catalog: OrdMap<String, Element>,
    owners: OrdMap<String, u64>,
    events: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Channel {
    initial: Snapshot,
    transitions: Vector<Transition>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Snapshot {
    items: List,
    ids: OrdSet<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Transition {
    start: Fraction,
    end: Fraction,
    from: Snapshot,
    to: Snapshot,
    easing: Easing,
    /// The same operation supplies proof checking and compact visual keys.
    operation: Option<crate::ListEdit>,
}
pub(crate) struct Selection<'a> {
    pub from: &'a List,
    pub to: &'a List,
    pub progress: Fraction,
    pub appearance: Fraction,
    pub active: bool,
}
pub(crate) fn elements(items: &List) -> Result<impl Iterator<Item = &Element>, String> {
    if items.element_type.as_ref() != &Type::Element {
        return Err(format!(
            "live Row/Column children require List<Element>, found List<{}>",
            items.element_type
        ));
    }
    if items
        .values
        .iter()
        .any(|item| !matches!(item, Value::Element(_)))
    {
        return Err("live list contains a non-element value".into());
    }
    Ok(items.values.iter().map(|value| match value {
        Value::Element(element) => element.as_ref(),
        _ => unreachable!("validated element list"),
    }))
}
impl Channel {
    fn transition(&self, time: Fraction) -> Option<&Transition> {
        let count = self
            .transitions
            .binary_search_by(|event| {
                if event.start <= time {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            })
            .unwrap_err();
        count.checked_sub(1).map(|index| &self.transitions[index])
    }
    fn endpoints(&self, time: Fraction) -> (&List, &List, Option<&Transition>) {
        match self.transition(time) {
            Some(event) if time < event.end => (&event.from.items, &event.to.items, Some(event)),
            Some(event) => (&event.to.items, &event.to.items, None),
            None => (&self.initial.items, &self.initial.items, None),
        }
    }
    fn at(&self, time: Fraction) -> Result<Selection<'_>, String> {
        let (from, to, event) = self.endpoints(time);
        if let Some(event) = event {
            let time = math::progress(time, event.start, event.end)?;
            let (progress, appearance) = math::eased_progress(event.easing, time)?;
            Ok(Selection {
                from,
                to,
                progress,
                appearance,
                active: true,
            })
        } else {
            Ok(Selection {
                from,
                to,
                progress: 1.into(),
                appearance: 1.into(),
                active: false,
            })
        }
    }
}
impl ListViews {
    fn snapshot_ids(&self, items: &List) -> Result<OrdSet<String>, String> {
        fn visit(element: &Element, views: &OrdMap<String, u64>, ids: &mut OrdSet<String>) {
            ids.insert(element.id.clone());
            if !views.contains_key(&element.id)
                && let Some(children) = element.geometry.children()
            {
                for child in children {
                    visit(child, views, ids);
                }
            }
        }
        let mut ids = OrdSet::new();
        for element in elements(items)? {
            visit(element, &self.views, &mut ids);
        }
        Ok(ids)
    }
    /// Reachability is prepared once from static roots and catalog ownership.
    /// Future children may themselves contain live views; their clocks must not
    /// disappear merely because they were absent from the initial scene tree.
    pub(crate) fn reachable_views<'a>(
        &'a self,
        roots: impl Iterator<Item = &'a Element>,
    ) -> BTreeSet<&'a str> {
        let mut owned = std::collections::BTreeMap::<u64, Vec<&Element>>::new();
        for (id, element) in &self.catalog {
            if let Some(cell) = self.owners.get(id) {
                owned.entry(*cell).or_default().push(element);
            }
        }
        let mut pending: Vec<_> = roots.collect();
        let mut seen = BTreeSet::new();
        let mut views = BTreeSet::new();
        while let Some(element) = pending.pop() {
            if !seen.insert(element.id.as_str()) {
                continue;
            }
            if let Some(cell) = self.views.get(&element.id) {
                views.insert(element.id.as_str());
                if let Some(children) = owned.get(cell) {
                    pending.extend(children.iter().copied());
                }
            }
            if let Some(children) = element.geometry.children() {
                pending.extend(children.iter());
            }
        }
        views
    }
    pub fn has_cell(&self, cell: u64) -> bool {
        self.channels.contains_key(&cell)
    }
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.catalog.values()
    }
    pub(crate) fn find(&self, id: &str) -> Option<&Element> {
        self.catalog.get(id)
    }
    pub(crate) fn views_in(&self, element: &Element, cell: u64) -> bool {
        self.views
            .iter()
            .any(|(view, owner)| *owner == cell && element.find(view).is_some())
    }
    pub(crate) fn owner(&self, id: &str) -> Option<u64> {
        self.owners.get(id).copied()
    }
    pub(crate) fn view_cell(&self, id: &str) -> Option<u64> {
        self.views.get(id).copied()
    }
    pub(crate) fn selection(
        &self,
        view: &str,
        time: Fraction,
    ) -> Result<Option<Selection<'_>>, String> {
        self.views
            .get(view)
            .map(|cell| {
                self.channels
                    .get(cell)
                    .ok_or_else(|| "list view references a missing channel".to_string())?
                    .at(time)
            })
            .transpose()
    }
    pub fn sample(&self, cell: u64, time: Fraction) -> Result<Option<List>, String> {
        if !math::finite(&time) || time < 0.into() {
            return Err("list sample time must be finite and nonnegative".into());
        }
        Ok(self.channels.get(&cell).map(|channel| {
            // Logical sampling is a step lookup: no easing or visual interpolation work.
            match channel.transition(time) {
                Some(event) if time < event.end => event.from.items.clone(),
                Some(event) => event.to.items.clone(),
                None => channel.initial.items.clone(),
            }
        }))
    }
    pub(crate) fn visual_children(
        &self,
        view: &str,
        time: Fraction,
    ) -> Result<Option<Vector<Element>>, String> {
        let Some(cell) = self.views.get(view) else {
            return Ok(None);
        };
        let (from, to, event) = self
            .channels
            .get(cell)
            .ok_or("list view references a missing channel")?
            .endpoints(time);
        let mut children: Vector<_> = elements(to)?.cloned().collect();
        if event.is_none() {
            return Ok(Some(children));
        }
        let target: BTreeSet<_> = elements(to)?.map(|element| element.id.as_str()).collect();
        children.extend(
            elements(from)?
                .filter(|element| !target.contains(element.id.as_str()))
                .cloned(),
        );
        Ok(Some(children))
    }
    pub(crate) fn events<'a>(
        &'a self,
        visible: impl Fn(&str) -> bool + 'a,
    ) -> impl Iterator<Item = (Fraction, Fraction, bool)> + 'a {
        let cells: BTreeSet<_> = self
            .views
            .iter()
            .filter(|(view, _)| visible(view))
            .map(|(_, cell)| *cell)
            .collect();
        self.channels
            .iter()
            .filter(move |(cell, _)| cells.contains(cell))
            .flat_map(|(_, channel)| {
                channel
                    .transitions
                    .iter()
                    .map(|event| (event.start, event.end, event.from != event.to))
            })
    }
    pub(crate) fn changed_from(&self, next: &Self) -> Option<Fraction> {
        if self.views != next.views {
            return Some(0.into());
        }
        let mut first: Option<Fraction> = None;
        for cell in self.views.values().collect::<BTreeSet<_>>() {
            let (Some(old), Some(new)) = (self.channels.get(cell), next.channels.get(cell)) else {
                return Some(0.into());
            };
            if old.initial != new.initial {
                return Some(0.into());
            }
            if old.transitions == new.transitions {
                continue;
            }
            let common = old
                .transitions
                .iter()
                .zip(new.transitions.iter())
                .take_while(|(a, b)| a == b)
                .count();
            if let Some(start) = old
                .transitions
                .get(common)
                .into_iter()
                .chain(new.transitions.get(common))
                .map(|event| event.start)
                .min()
            {
                first = Some(first.map_or(start, |previous| previous.min(start)));
            }
        }
        first
    }
    fn validate(&self, cell: u64, items: &List) -> Result<OrdMap<String, Element>, String> {
        fn visit(
            cell: u64,
            element: &Element,
            state: &ListViews,
            additions: &mut OrdMap<String, Element>,
        ) -> Result<(), String> {
            if let Some(owner) = state.owners.get(&element.id)
                && *owner != cell
            {
                return Err(format!(
                    "list element '{}' is already owned by another list; construct a separate element for each view",
                    element.id
                ));
            }
            if let Some(existing) = state.catalog.get(&element.id)
                && existing != element
            {
                return Err(format!(
                    "list element '{}' changed its definition; reuse the original element to move it, or give the replacement a new stable identity",
                    element.id
                ));
            }
            if additions.len() >= MAX_LIST_ELEMENTS {
                return Err("retained list element limit exceeded".into());
            }
            if additions
                .insert(element.id.clone(), element.clone())
                .is_some()
            {
                return Err(format!("duplicate list element identity '{}'", element.id));
            }
            if !state.views.contains_key(&element.id)
                && let Some(children) = element.geometry.children()
            {
                for child in children {
                    visit(cell, child, state, additions)?
                }
            }
            Ok(())
        }
        let mut additions = OrdMap::new();
        for element in elements(items)? {
            visit(cell, element, self, &mut additions)?
        }
        if self.catalog.len().saturating_add(
            additions
                .keys()
                .filter(|id| !self.catalog.contains_key(*id))
                .count(),
        ) > MAX_LIST_ELEMENTS
        {
            return Err("retained list element limit exceeded".into());
        }
        if additions.keys().any(|id| self.views.contains_key(id)) {
            let mut candidate = self.clone();
            candidate.retain(cell, additions.clone());
            candidate.validate_view_graph(&self.views.values().copied().collect())?;
        }
        Ok(additions)
    }
    fn retain(&mut self, cell: u64, additions: OrdMap<String, Element>) {
        for (id, element) in additions {
            self.owners.insert(id.clone(), cell);
            self.catalog.insert(id, element);
        }
    }
}
impl Animation {
    pub fn set_list(&mut self, cell: u64, initial: List) -> Result<(), String> {
        if self.list_views.channels.contains_key(&cell) {
            return Err(format!("list cell {cell} is already registered"));
        }
        if self.list_views.channels.len() >= MAX_LIST_ELEMENTS {
            return Err("retained list channel limit exceeded".into());
        }
        self.list_views.channels.insert(
            cell,
            Channel {
                initial: Snapshot {
                    items: initial,
                    ids: OrdSet::new(),
                },
                transitions: Vector::new(),
            },
        );
        Ok(())
    }
    pub fn bind_list(&mut self, cell: u64, view: &Element, initial: List) -> Result<(), String> {
        if !(view.layout.direction.is_some()
            && matches!(view.geometry.as_ref(), Geometry::Box { .. }))
        {
            return Err("live list children require a Row or Column".into());
        }
        if self.list_views.views.contains_key(&view.id) {
            return Err(format!("list view '{}' is already bound", view.id));
        }
        if self.list_views.views.values().any(|owner| *owner == cell) {
            return Err("a mutable element list currently supports one live view; use a separate list for each view".into());
        }
        let mut additions = self.list_views.validate(cell, &initial)?;
        if let Some(channel) = self.list_views.channels.get(&cell) {
            additions.extend(self.list_views.validate(cell, &channel.initial.items)?);
            for event in &channel.transitions {
                additions.extend(self.list_views.validate(cell, &event.to.items)?);
            }
        }
        for (id, element) in &additions {
            if self.find(id).is_some_and(|existing| existing != element)
                || self.find(id).is_some()
                    && self.list_views.owner(id) != Some(cell)
                    && self
                        .find(&view.id)
                        .is_none_or(|root| root.find(id).is_none())
            {
                return Err(format!("duplicate element identity '{id}'"));
            }
        }
        let mut flow = flow_ids(&initial)?;
        if let Some(channel) = self.list_views.channels.get(&cell) {
            flow.extend(flow_ids(&channel.initial.items)?);
            for event in &channel.transitions {
                flow.extend(flow_ids(&event.to.items)?);
            }
        }
        for id in &flow {
            if self
                .reactive
                .binding(id, crate::Property::Position)
                .is_some()
            {
                return Err(format!(
                    "flow layout controls '{id}.position'; animate its containing Row or Column instead"
                ));
            }
        }
        if !self.list_views.channels.contains_key(&cell) {
            self.set_list(cell, initial)?;
        }
        let mut channel = self
            .list_views
            .channels
            .get(&cell)
            .expect("registered list channel")
            .clone();
        channel.initial.ids = self.list_views.snapshot_ids(&channel.initial.items)?;
        for event in &mut channel.transitions {
            event.from.ids = self.list_views.snapshot_ids(&event.from.items)?;
            event.to.ids = self.list_views.snapshot_ids(&event.to.items)?;
        }
        self.list_views.channels.insert(cell, channel);
        for id in additions.keys() {
            self.element_index.entry(id.clone()).or_default();
        }
        self.list_views.retain(cell, additions);
        self.flow_controlled.extend(flow);
        self.list_views.views.insert(view.id.clone(), cell);
        Ok(())
    }
    pub fn write_list(
        &mut self,
        cell: u64,
        start: Fraction,
        end: Fraction,
        items: List,
        easing: Easing,
    ) -> Result<(), String> {
        self.write_list_transition(cell, start, end, items, easing, None)
    }

    pub fn write_list_edit(
        &mut self,
        cell: u64,
        start: Fraction,
        end: Fraction,
        edit: &crate::ListEditResult,
        easing: Easing,
    ) -> Result<(), String> {
        self.write_list_transition(cell, start, end, edit.output().clone(), easing, Some(edit))
    }

    fn write_list_transition(
        &mut self,
        cell: u64,
        start: Fraction,
        end: Fraction,
        items: List,
        easing: Easing,
        edit: Option<&crate::ListEditResult>,
    ) -> Result<(), String> {
        if !math::finite(&start) || !math::finite(&end) || start < 0.into() || end < start {
            return Err("list edit requires finite nonnegative start and duration".into());
        }
        math::timeline_end(start, math::arithmetic("-", end, start)?)?;
        if self.list_views.events >= MAX_LIST_EVENTS {
            return Err("retained list event limit exceeded".into());
        }
        let channel = self
            .list_views
            .channels
            .get(&cell)
            .ok_or("list is not registered as a timed list")?;
        if items.element_type != channel.initial.items.element_type {
            return Err("list edit cannot change its static element type".into());
        }
        let mut transitions = channel.transitions.clone();
        let index = transitions
            .binary_search_by(|event| {
                if event.start <= start {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            })
            .unwrap_err();
        if index > 0 && transitions[index - 1].end > start
            || transitions
                .get(index)
                .is_some_and(|event| event.start < end)
        {
            return Err("overlapping edits of the same list; sequence the edits or use separate mutable lists".into());
        }
        let from = if index > 0 {
            transitions[index - 1].to.clone()
        } else {
            channel.initial.clone()
        };
        let bound = self.list_views.views.values().any(|owner| *owner == cell);
        if let Some(edit) = edit
            && !same_snapshot(&from.items, edit.source())
        {
            return Err("list edit source differs from the state at its timeline start; schedule edits to the same list in chronological order".into());
        }
        let mut ids = from.ids.clone();
        let changed_items;
        let additions = if bound {
            if let Some(edit) = edit {
                if let Some(Value::Element(element)) = edit.removed() {
                    let removed = self.list_views.snapshot_ids(&List::typed(
                        Type::Element,
                        vec![Value::Element(element.clone())],
                    ))?;
                    for id in removed {
                        ids.remove(&id);
                    }
                }
                changed_items =
                    List::typed(Type::Element, edit.added().cloned().into_iter().collect());
                let additions = self.list_views.validate(cell, &changed_items)?;
                for id in additions.keys() {
                    if ids.contains(id) {
                        return Err(format!("duplicate list element identity '{id}'"));
                    }
                    ids.insert(id.clone());
                }
                additions
            } else {
                let additions = self.list_views.validate(cell, &items)?;
                ids = additions.keys().cloned().collect();
                changed_items = items.clone();
                additions
            }
        } else {
            changed_items = List::typed(Type::Element, vec![]);
            OrdMap::new()
        };
        let flow: imbl::OrdSet<String> = if bound {
            flow_ids(&changed_items)?
        } else {
            Default::default()
        };
        for id in &flow {
            if self
                .reactive
                .binding(id, crate::Property::Position)
                .is_some()
            {
                return Err(format!(
                    "flow layout controls '{id}.position'; animate its containing Row or Column instead"
                ));
            }
        }
        for (id, element) in &additions {
            if self.find(id).is_some_and(|existing| existing != element)
                || self.find(id).is_some() && self.list_views.owner(id) != Some(cell)
            {
                return Err(format!("duplicate element identity '{id}'"));
            }
        }
        transitions.insert(
            index,
            Transition {
                start,
                end,
                from,
                to: Snapshot {
                    ids,
                    items: items.clone(),
                },
                easing,
                operation: edit.map(|edit| edit.operation().clone()),
            },
        );
        let next_from = transitions[index].to.clone();
        if let Some(next) = transitions.get_mut(index + 1) {
            if next.operation.is_some() && !same_snapshot(&next.from.items, &items) {
                return Err("a future list edit depends on the previous state; schedule edits to the same list in chronological order".into());
            }
            next.from = next_from;
        }
        for id in additions.keys() {
            self.element_index.entry(id.clone()).or_default();
        }
        self.list_views.retain(cell, additions);
        self.flow_controlled.extend(flow);
        self.list_views
            .channels
            .get_mut(&cell)
            .expect("validated list channel")
            .transitions = transitions;
        self.list_views.events += 1;
        self.duration = self.duration.max(end);
        Ok(())
    }
    pub fn sample_list(&self, cell: u64, time: Fraction) -> Result<Option<List>, String> {
        self.list_views.sample(cell, time)
    }
}
