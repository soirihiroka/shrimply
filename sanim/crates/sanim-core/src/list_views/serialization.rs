//! Validate wire data once; native edits already carry sealed collection proofs.
use super::*;

#[derive(Deserialize)]
pub(super) struct ListViewsData {
    channels: OrdMap<u64, Channel>,
    views: OrdMap<String, u64>,
    catalog: OrdMap<String, Element>,
    owners: OrdMap<String, u64>,
    events: usize,
}

impl TryFrom<ListViewsData> for ListViews {
    type Error = String;
    fn try_from(data: ListViewsData) -> Result<Self, String> {
        if data.channels.len() > MAX_LIST_ELEMENTS
            || data.catalog.len() > MAX_LIST_ELEMENTS
            || data.events > MAX_LIST_EVENTS
        {
            return Err("retained list metadata exceeds its limits".into());
        }
        let mut bound = BTreeSet::new();
        for cell in data.views.values() {
            if !data.channels.contains_key(cell) || !bound.insert(*cell) {
                return Err("list view references a missing or multiply bound channel".into());
            }
        }
        let mut checked = Self {
            channels: data.channels.clone(),
            views: data.views.clone(),
            ..Default::default()
        };
        let (mut bytes, mut values) = (crate::reactive::MAX_REACTIVE_BYTES, MAX_LIST_EVENTS);
        let mut events = 0usize;
        for (cell, channel) in &data.channels {
            crate::type_budget::charge(&channel.initial.items.element_type, &mut bytes)?;
            if !channel.initial.items.element_type.is_concrete() {
                return Err("list channel requires a complete static element type".into());
            }
            let mut state = &channel.initial;
            let mut previous_end = Fraction::from(0);
            for (event, snapshot) in std::iter::once((None, &channel.initial)).chain(
                channel
                    .transitions
                    .iter()
                    .map(|event| (Some(event), &event.to)),
            ) {
                if snapshot.items.element_type != channel.initial.items.element_type {
                    return Err("list snapshot changes its static element type".into());
                }
                let value = Value::List(snapshot.items.clone());
                crate::value_budget::charge(
                    &value,
                    &mut bytes,
                    &mut values,
                    crate::reactive::MAX_REACTIVE_DEPTH,
                )?;
                crate::validate_value(
                    &Type::List(Box::new(snapshot.items.element_type.as_ref().clone())),
                    &value,
                )?;
                if let Some(event) = event {
                    events = events.checked_add(1).ok_or("list event count overflow")?;
                    if events > MAX_LIST_EVENTS
                        || !math::finite(&event.start)
                        || !math::finite(&event.end)
                        || event.start < previous_end
                        || event.end < event.start
                    {
                        return Err(
                            "list events must be finite, bounded, ordered and nonoverlapping"
                                .into(),
                        );
                    }
                    math::timeline_end(
                        event.start,
                        math::arithmetic("-", event.end, event.start)?,
                    )?;
                    crate::value_budget::charge(
                        &Value::List(event.from.items.clone()),
                        &mut bytes,
                        &mut values,
                        crate::reactive::MAX_REACTIVE_DEPTH,
                    )?;
                    if event.from != *state {
                        return Err(
                            "list event source does not match its preceding snapshot".into()
                        );
                    }
                    if let Some(operation) = &event.operation {
                        let receipt = state.items.edit(operation.clone())?;
                        if receipt.output().element_type != snapshot.items.element_type
                            || receipt.output() != &snapshot.items
                        {
                            return Err(
                                "list event operation does not produce its target snapshot".into(),
                            );
                        }
                    }
                    previous_end = event.end;
                }
                let additions = if bound.contains(cell) {
                    checked.validate(*cell, &snapshot.items)?
                } else {
                    OrdMap::new()
                };
                let ids: OrdSet<_> = additions.keys().cloned().collect();
                if snapshot.ids != ids {
                    return Err("list snapshot identity index does not match its values".into());
                }
                checked.retain(*cell, additions);
                state = snapshot;
            }
        }
        if events != data.events || checked.catalog != data.catalog || checked.owners != data.owners
        {
            return Err(
                "list catalog, ownership or event count does not match its snapshots".into(),
            );
        }
        checked.validate_view_graph(&bound)?;
        checked.events = events;
        Ok(checked)
    }
}

impl ListViews {
    pub(super) fn validate_view_graph(&self, bound: &BTreeSet<u64>) -> Result<(), String> {
        let mut edges = std::collections::BTreeMap::<u64, BTreeSet<u64>>::new();
        let mut degrees: std::collections::BTreeMap<_, usize> =
            bound.iter().map(|cell| (*cell, 0)).collect();
        for (view, target) in &self.views {
            if let Some(owner) = self.owners.get(view)
                && edges.entry(*owner).or_default().insert(*target)
            {
                *degrees.get_mut(target).expect("bound target") += 1;
            }
        }
        let mut depths: std::collections::BTreeMap<_, usize> =
            bound.iter().map(|cell| (*cell, 1)).collect();
        let mut ready: Vec<_> = degrees
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(cell, _)| *cell)
            .collect();
        let mut visited = 0;
        while let Some(cell) = ready.pop() {
            let depth = depths[&cell];
            if depth > crate::reactive::MAX_REACTIVE_DEPTH {
                return Err("nested list view depth limit exceeded".into());
            }
            visited += 1;
            if let Some(targets) = edges.get(&cell) {
                for target in targets {
                    let next_depth = depths.get_mut(target).expect("bound depth");
                    *next_depth = (*next_depth).max(depth + 1);
                    let degree = degrees.get_mut(target).expect("bound edge");
                    *degree -= 1;
                    if *degree == 0 {
                        ready.push(*target);
                    }
                }
            }
        }
        if visited != degrees.len() {
            return Err("cyclic live list views are not supported".into());
        }
        Ok(())
    }
}
