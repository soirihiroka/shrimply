//! Content-driven hot reload with atomic last-good revisions and failure recovery.
use crate::{Engine, Prepared, Sources};
use sanim_core::{Diagnostic, Value, digest};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

struct Elements<'a> {
    by_id: BTreeMap<&'a str, sanim_core::list_views::ReachableElement<'a>>,
    order: Vec<&'a str>,
}
impl<'a> Elements<'a> {
    fn collect(animation: &'a sanim_core::Animation) -> Self {
        let mut result = Self {
            by_id: BTreeMap::new(),
            order: vec![],
        };
        animation.list_views.visit_reachable(
            animation
                .layers
                .iter()
                .map(|layer| (&layer.element, layer.birth)),
            |reachable| {
                result.order.push(reachable.element.id.as_str());
                result
                    .by_id
                    .insert(reachable.element.id.as_str(), reachable);
            },
        );
        result
    }
    fn propagate(&self, dirty: &mut BTreeSet<String>) {
        // Canonical reachability emits parents before children, including future
        // live-list items. A changed item can move its layout parent and siblings.
        for id in self.order.iter().rev() {
            if dirty.contains(*id)
                && let Some(parent) = self.by_id[id].parent
                && parent.layout.direction.is_some()
            {
                dirty.insert(parent.id.clone());
            }
        }
        for id in &self.order {
            if self.by_id[id]
                .parent
                .is_some_and(|parent| dirty.contains(&parent.id))
            {
                dirty.insert((*id).to_owned());
            }
        }
    }
}

pub enum Reload {
    Unchanged,
    Updated {
        revision: u64,
        prepared: Arc<Prepared>,
        changed_elements: Vec<String>,
    },
    Failed {
        diagnostic: Diagnostic,
        last_good_revision: Option<u64>,
    },
}

pub struct Session {
    pub engine: Engine,
    entry: String,
    overrides: BTreeMap<String, Value>,
    json_overrides: Option<BTreeMap<String, serde_json::Value>>,
    last_error: Option<Diagnostic>,
    observed: BTreeMap<String, String>,
    previous_overrides: String,
    revision: u64,
    current: Option<Arc<Prepared>>,
}
impl Session {
    pub fn new(entry: impl Into<String>) -> Self {
        Self {
            engine: Engine::default(),
            entry: entry.into(),
            overrides: BTreeMap::new(),
            json_overrides: None,
            last_error: None,
            observed: BTreeMap::new(),
            previous_overrides: String::new(),
            revision: 0,
            current: None,
        }
    }
    pub fn current(&self) -> Option<&Arc<Prepared>> {
        self.current.as_ref()
    }
    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        self.json_overrides = None;
        self.overrides.insert(key.into(), value);
    }
    pub fn reset(&mut self, key: &str) {
        self.overrides.remove(key);
    }
    pub fn set_json_inputs(&mut self, values: BTreeMap<String, serde_json::Value>) {
        self.json_overrides = Some(values);
        self.overrides.clear();
    }
    pub fn last_error(&self) -> Option<&Diagnostic> {
        self.last_error.as_ref()
    }
    /// Native font activation matters only to sessions that prepared family text.
    pub fn font_environment_changed(&self) -> bool {
        self.engine
            .font_environment
            .is_some_and(|epoch| Some(epoch) != sanim_data::font::system_font_epoch())
    }
    /// Includes failed reads so creating a missing dependency can recover the session.
    pub fn dependencies(&self) -> impl Iterator<Item = &str> {
        self.observed.keys().map(String::as_str)
    }
    pub fn poll(&mut self, sources: &dyn Sources) -> Reload {
        self.poll_inner(sources, None)
    }
    /// Cold loads publish a duration plan before scene construction. Hot edits
    /// restore execution checkpoints once and publish their exact resulting duration.
    pub fn poll_with_duration(
        &mut self,
        sources: &dyn Sources,
        mut publish: impl FnMut(sanim_core::Fraction),
    ) -> Reload {
        self.poll_inner(sources, Some(&mut publish))
    }
    fn poll_inner(
        &mut self,
        sources: &dyn Sources,
        mut probe: Option<&mut dyn FnMut(sanim_core::Fraction)>,
    ) -> Reload {
        let overrides = digest(
            &serde_json::to_vec(&(
                &self.overrides,
                &self.json_overrides,
                &self.engine.limits,
                &self.engine.viewport,
                self.engine.preparation,
            ))
            .expect("serializable overrides"),
        );
        let mut changed = self.observed.is_empty()
            || self.font_environment_changed()
            || overrides != self.previous_overrides;
        if !changed {
            // Source/input changes can remove an old dependency entirely. Probe
            // the entry and local resources first, stopping as soon as a change
            // is known; never fetch an obsolete remote asset before preparing
            // the new program that no longer references it.
            let paths = std::iter::once(self.entry.as_str())
                .chain(
                    self.observed
                        .keys()
                        .map(String::as_str)
                        .filter(|path| *path != self.entry && !crate::source::is_remote(path)),
                )
                .chain(
                    self.observed
                        .keys()
                        .map(String::as_str)
                        .filter(|path| crate::source::is_remote(path)),
                );
            changed = paths.into_iter().any(|path| {
                let current = match sources.read_snapshot(path) {
                    Ok((_, fingerprint)) => fingerprint,
                    Err(error) => format!("error:{error}"),
                };
                self.observed.get(path) != Some(&current)
            });
        }
        if !changed {
            return Reload::Unchanged;
        }
        self.previous_overrides = overrides;
        let mut planning_performance = vec![];
        let mut planning_micros = 0;
        let cold = self.current.is_none();
        if let Some(publish) = probe.as_mut().filter(|_| cold) {
            let duration = if let Some(json) = &self.json_overrides {
                self.engine.duration_json(&self.entry, sources, json)
            } else {
                self.engine.duration(&self.entry, sources, &self.overrides)
            };
            if let Ok(report) = duration {
                publish(report.duration);
                planning_micros = report.statistics.prepare_micros;
                planning_performance = report
                    .performance
                    .into_iter()
                    .map(|mut warning| {
                        warning.message = format!("duration planning: {}", warning.message);
                        warning
                    })
                    .collect();
            }
        }

        let prepared = if let Some(json) = &self.json_overrides {
            self.engine.prepare_json(&self.entry, sources, json)
        } else {
            self.engine.prepare(&self.entry, sources, &self.overrides)
        }
        .map(|mut prepared| {
            if !cold && let Some(publish) = probe {
                publish(prepared.animation.duration);
            }
            prepared.statistics.prepare_micros = prepared
                .statistics
                .prepare_micros
                .saturating_add(planning_micros);
            prepared.performance.extend(planning_performance);
            prepared
        });
        match prepared {
            Ok(prepared) => {
                self.last_error = None;
                self.observed = prepared.dependencies.clone();
                let prepared = Arc::new(prepared);
                if self
                    .current
                    .as_ref()
                    .is_some_and(|old| old.fingerprint == prepared.fingerprint)
                {
                    self.current = Some(prepared);
                    return Reload::Unchanged;
                }
                // One canonical traversal includes future children of mounted
                // list views while excluding unmounted catalog entries.
                let old_elements = self
                    .current
                    .as_ref()
                    .map(|old| Elements::collect(&old.animation));
                let new_elements = Elements::collect(&prepared.animation);
                let global_change = self.current.as_ref().is_none_or(|old| {
                    old.animation.dpi != prepared.animation.dpi
                        || old.animation.rate != prepared.animation.rate
                        || old.animation.width != prepared.animation.width
                        || old.animation.height != prepared.animation.height
                        || old.animation.background != prepared.animation.background
                });
                let mut changed_elements = BTreeSet::new();
                let mut compare = |id: &str| {
                    let old = old_elements.as_ref().and_then(|old| old.by_id.get(id));
                    let new = new_elements.by_id.get(id);
                    let element_changed = match (old, new) {
                        (Some(old), Some(new)) => {
                            old.element != new.element
                                || old.root != new.root
                                || old.birth != new.birth
                                || old.parent.map(|parent| &parent.id)
                                    != new.parent.map(|parent| &parent.id)
                        }
                        _ => old.is_some() != new.is_some(),
                    };
                    if global_change
                        || element_changed
                        || self
                            .current
                            .as_ref()
                            .and_then(|old| old.animation.tracks.get(id))
                            != prepared.animation.tracks.get(id)
                    {
                        changed_elements.insert(id.to_owned());
                    }
                };
                if let Some(old) = &old_elements {
                    for id in old.by_id.keys() {
                        compare(id);
                    }
                }
                for id in new_elements.by_id.keys().filter(|id| {
                    old_elements
                        .as_ref()
                        .is_none_or(|old| !old.by_id.contains_key(*id))
                }) {
                    compare(id);
                }
                if let Some(old) = &self.current {
                    old.animation.reactive.visit_changed_bindings(
                        &prepared.animation.reactive,
                        |id| {
                            old_elements
                                .as_ref()
                                .and_then(|old| old.by_id.get(id))
                                .into_iter()
                                .chain(new_elements.by_id.get(id))
                                .map(|element| element.birth)
                                .min()
                        },
                        |id, _, _| {
                            changed_elements.insert(id.to_owned());
                        },
                    );
                }
                new_elements.propagate(&mut changed_elements);
                if let Some(old) = &old_elements {
                    old.propagate(&mut changed_elements);
                }
                self.revision += 1;
                self.current = Some(prepared.clone());
                Reload::Updated {
                    revision: self.revision,
                    prepared,
                    changed_elements: changed_elements.into_iter().collect(),
                }
            }
            Err(diagnostic) => {
                self.last_error = Some(diagnostic.clone());
                // Keep old dependencies too: a failed edit may stop before reading them.
                self.observed.extend(self.engine.attempted.clone());
                Reload::Failed {
                    diagnostic,
                    last_good_revision: self.current.as_ref().map(|_| self.revision),
                }
            }
        }
    }
}
