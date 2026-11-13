//! Content-driven hot reload with atomic last-good revisions and failure recovery.
use crate::{Engine, Prepared, Sources};
use sanim_core::{Diagnostic, Value, digest};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

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
    /// Publish the exact planned duration before constructing the new animation scene.
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
        probe: Option<&mut dyn FnMut(sanim_core::Fraction)>,
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
        let mut changed=self.observed.is_empty()
            || self.font_environment_changed()
            || overrides!=self.previous_overrides;
        if !changed {
            // Source/input changes can remove an old dependency entirely. Probe
            // the entry and local resources first, stopping as soon as a change
            // is known; never fetch an obsolete remote asset before preparing
            // the new program that no longer references it.
            let paths=std::iter::once(self.entry.as_str())
                .chain(self.observed.keys().map(String::as_str).filter(|path|*path!=self.entry && !crate::source::is_remote(path)))
                .chain(self.observed.keys().map(String::as_str).filter(|path|crate::source::is_remote(path)));
            changed=paths.into_iter().any(|path| {
                let current=match sources.read_snapshot(path) {
                    Ok((_,fingerprint))=>fingerprint,
                    Err(error)=>format!("error:{error}"),
                };
                self.observed.get(path)!=Some(&current)
            });
        }
        if !changed {return Reload::Unchanged;}
        self.previous_overrides = overrides;
        let mut planning_performance = vec![];
        let mut planning_micros = 0;
        if let Some(publish) = probe {
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
                let mut ids = BTreeSet::new();
                for layer in &prepared.animation.layers {
                    let mut nested = vec![];
                    layer.element.ids(&mut nested);
                    ids.extend(nested);
                }
                if let Some(old) = &self.current {
                    for layer in &old.animation.layers {
                        let mut nested = vec![];
                        layer.element.ids(&mut nested);
                        ids.extend(nested);
                    }
                }
                // Birth times and paint order are visual dependencies too, even
                // when an element's definition and property tracks are unchanged.
                let layer_state = |animation: &sanim_core::Animation| {
                    let mut states = BTreeMap::new();
                    for (index, layer) in animation.layers.iter().enumerate() {
                        let mut nested = vec![];
                        layer.element.ids(&mut nested);
                        for id in nested {
                            states.insert(id, (index, layer.birth));
                        }
                    }
                    states
                };
                let old_layers = self.current.as_ref().map(|old| layer_state(&old.animation));
                let new_layers = layer_state(&prepared.animation);
                let global_change = self.current.as_ref().is_none_or(|old| {
                    old.animation.dpi != prepared.animation.dpi
                        || old.animation.rate != prepared.animation.rate
                        || old.animation.width != prepared.animation.width
                        || old.animation.height != prepared.animation.height
                        || old.animation.background != prepared.animation.background
                });
                let mut changed_elements: BTreeSet<String> = ids
                    .iter()
                    .filter(|&id| {
                        let old = self.current.as_ref().and_then(|v| v.animation.find(id));
                        let new = prepared.animation.find(id);
                        let old_tracks = self
                            .current
                            .as_ref()
                            .and_then(|v| v.animation.tracks.get(id));
                        global_change
                            || old != new
                            || old_tracks != prepared.animation.tracks.get(id)
                            || old_layers.as_ref().and_then(|layers| layers.get(id))
                                != new_layers.get(id)
                    })
                    .cloned()
                    .collect();
                fn expand(element: &sanim_core::Element, dirty: &mut BTreeSet<String>) -> bool {
                    let mut dependent = false;
                    if let Some(children) = element.geometry.children() {
                        for child in children {
                            dependent |= expand(child, dirty);
                        }
                        if element.layout.direction.is_some() && dependent {
                            dirty.insert(element.id.clone());
                        }
                    }
                    if dirty.contains(&element.id) {
                        let mut ids = vec![];
                        element.ids(&mut ids);
                        dirty.extend(ids);
                    }
                    dependent || dirty.contains(&element.id)
                }
                for layer in &prepared.animation.layers {
                    expand(&layer.element, &mut changed_elements);
                }
                if let Some(old) = &self.current {
                    for layer in &old.animation.layers {
                        expand(&layer.element, &mut changed_elements);
                    }
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
