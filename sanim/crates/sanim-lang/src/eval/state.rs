//! Restorable execution prefixes. Payloads and timeline nodes are persistent/COW values.
use super::*;
use std::sync::Arc;

#[derive(Clone)]
pub struct ExecutionState {
    scope: Env,
    reactive: reactive::Store,
    pub animation: Animation,
    pub cursor: Fraction,
    pub time_map: TimeMap,
    pub element_sources: imbl::OrdMap<String, Arc<crate::ElementSource>>,
    pub statements: usize,
    reflected: imbl::Vector<ReflectedInput>,
    assertions: imbl::Vector<Assertion>,
    checks: imbl::Vector<Verification>,
    used_overrides: imbl::OrdSet<String>,
    functions: imbl::OrdMap<String, (String, usize)>,
    inputs: imbl::OrdMap<String, (Option<Value>, usize)>,
    resources: imbl::OrdMap<String, (String, usize)>,
    declarations: imbl::OrdSet<String>,
    retained_bytes: usize,
    retained_items: usize,
    retained_frames: imbl::OrdSet<String>,
    retained_fonts: sanim_core::value_budget::Budget,
    logical_operations: u64,
    overhead: usize,
    payload: usize,
    age: u64,
}

impl ExecutionState {
    /// Snapshot value without executing code: dirty or unread signals have no cached value.
    pub fn binding(&self, name: &str) -> Option<&Value> {
        match self.scope.get(name)? {
            Binding::Value(Value::Signal(signal)) => {
                self.reactive
                    .cached(signal.id, self.cursor, &self.animation.reactive)
            }
            Binding::Value(value) => Some(value),
            Binding::Reactive(id) | Binding::Deferred(id) => {
                self.reactive
                    .cached(*id, self.cursor, &self.animation.reactive)
            }
            Binding::Module(_) => None,
        }
    }
}

#[derive(Default)]
pub(crate) struct Store {
    context: String,
    prefixes: Vec<[u8; 32]>,
    states: Vec<Option<Arc<ExecutionState>>>,
    active: BTreeSet<usize>,
    order: BTreeSet<(u64, usize)>,
    age: u64,
    declarations: imbl::OrdSet<String>,
    overhead: usize,
    fixed_bytes: usize,
    pub bytes: usize,
}
impl Store {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn get(&self, statement: usize) -> Option<Arc<ExecutionState>> {
        self.states.get(statement)?.clone()
    }
    pub fn len(&self) -> usize {
        self.active.len()
    }
    pub(super) fn begin(
        &mut self,
        runtime: &mut Runtime<'_, '_>,
        env: &mut Env,
        root: &Loaded,
    ) -> usize {
        let index_bytes = root.ast.statements.len().saturating_add(1).saturating_mul(
            std::mem::size_of::<[u8; 32]>() + std::mem::size_of::<Option<Arc<ExecutionState>>>(),
        );
        if runtime.limits.max_cache_entries == 0 || index_bytes > runtime.limits.max_cache_bytes {
            self.clear();
            return 0;
        }
        let context = digest(
            &serde_json::to_vec(&(
                SEMANTICS_VERSION,
                &root.id,
                &root.imports,
                &root.checked.schemas,
                runtime
                    .loaded
                    .iter()
                    .filter(|module| module.id != root.id)
                    .map(|module| {
                        (
                            &module.id,
                            &module.imports,
                            &module.checked.schemas,
                            module
                                .ast
                                .statements
                                .iter()
                                .filter(|statement| executable(statement))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>(),
                runtime
                    .globals
                    .iter()
                    .filter(|(id, _)| *id != &root.id)
                    .collect::<Vec<_>>(),
                &runtime.reflected,
                &runtime.reactive,
                runtime.limits,
                runtime.preparation,
                runtime.animation.dpi,
                runtime.animation.width,
                runtime.animation.height,
                runtime.animation.rate,
            ))
            .expect("serializable state context"),
        );
        if self.context != context {
            self.clear();
            self.context = context;
        }
        self.declarations = env.keys().cloned().collect();
        let mut hash = digest_bytes(self.context.as_bytes());
        let prefixes: Vec<_> = root
            .ast
            .statements
            .iter()
            .filter(|statement| executable(statement))
            .map(|statement| {
                hash = digest_bytes(
                    &serde_json::to_vec(&(&hash, statement)).expect("serializable statement"),
                );
                hash
            })
            .collect();
        let length = prefixes.len().min(self.prefixes.len());
        let (mut left, mut right) = (0, length);
        while left < right {
            let middle = left + (right - left) / 2;
            if prefixes[middle] == self.prefixes[middle] {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        let common = left;
        self.prefixes = prefixes;
        // Changed suffixes are discarded, including states from failed attempts. Valid prefixes survive.
        let stale: Vec<_> = self.active.range((common + 1)..).copied().collect();
        for index in stale {
            self.remove(index);
        }
        self.states.resize_with(self.prefixes.len() + 1, || None);
        self.fixed_bytes = self.prefixes.capacity() * std::mem::size_of::<[u8; 32]>()
            + self.states.capacity() * std::mem::size_of::<Option<Arc<ExecutionState>>>();
        if self.fixed_bytes > runtime.limits.max_cache_bytes {
            self.clear();
            return 0;
        }
        self.recount();
        while self.bytes > runtime.limits.max_cache_bytes {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.remove(oldest);
        }
        // Rejected candidates must not add obsolete/missing files to the new execution's dependencies.
        let mut probes = source::Snapshot {
            provider: runtime.snapshot.provider,
            reads: BTreeMap::new(),
            hashes: Default::default(),
        };
        let mut candidate = self.active.range(..=common).next_back().copied();
        while let Some(index) = candidate {
            let saved = self.get(index).expect("active state checkpoint");
            let mut invalid_at = None;
            for (key, (version, first_use)) in &saved.functions {
                if runtime
                    .functions
                    .get(key)
                    .is_none_or(|function| &function.fingerprint != version)
                {
                    invalid_at = Some(
                        invalid_at.map_or(*first_use, |previous: usize| previous.min(*first_use)),
                    );
                }
            }
            for (key, (value, first_use)) in &saved.inputs {
                if runtime.overrides.get(key) != value.as_ref() {
                    invalid_at = Some(
                        invalid_at.map_or(*first_use, |previous: usize| previous.min(*first_use)),
                    );
                }
            }
            for (path, (version, first_use)) in &saved.resources {
                if invalid_at.is_some_and(|cutoff| *first_use >= cutoff) {
                    continue;
                }
                let current = if runtime.snapshot.reads.contains_key(path) {
                    runtime.snapshot.fingerprint(path)
                } else {
                    let _ = probes.read(path);
                    probes.fingerprint(path)
                };
                if current != *version {
                    invalid_at = Some(
                        invalid_at.map_or(*first_use, |previous: usize| previous.min(*first_use)),
                    );
                }
            }
            if let Some(first_use) = invalid_at {
                candidate = self
                    .active
                    .range(..=first_use)
                    .next_back()
                    .copied()
                    .filter(|&next| next < index);
                continue;
            }
            for (path, (version, _)) in &saved.resources {
                if !runtime.snapshot.reads.contains_key(path) {
                    runtime
                        .snapshot
                        .reads
                        .insert(path.clone(), probes.reads[path].clone());
                    runtime
                        .snapshot
                        .hashes
                        .borrow_mut()
                        .insert(path.clone(), version.clone());
                }
            }
            let declarations = env.clone();
            *env = saved.scope.clone();
            runtime.reactive = saved.reactive.clone();
            for name in &saved.declarations {
                env.remove(name);
            }
            env.extend(declarations);
            runtime.animation = saved.animation.clone();
            runtime.cursor = saved.cursor;
            runtime.time_map = saved.time_map.clone();
            runtime.element_sources = saved.element_sources.clone();
            runtime.reflected = saved.reflected.clone();
            runtime.assertions = saved.assertions.clone();
            runtime.checks = saved.checks.clone();
            runtime.used_overrides = saved.used_overrides.clone();
            runtime.used_functions = saved.functions.clone();
            runtime.retained_bytes = saved.retained_bytes;
            runtime.retained_items = saved.retained_items;
            runtime.retained_frames = saved.retained_frames.clone();
            runtime.retained_fonts = saved.retained_fonts.clone();
            runtime.logical_operations = saved.logical_operations;
            runtime.completed_statements += index as u64;
            runtime.statistics.restored_statements = index as u64;
            runtime.statistics.restored_until = saved.cursor;
            runtime.progress.emit(
                Phase::Restoring,
                Some(index as u64),
                Some(runtime.total_statements),
                Some(&root.id),
            );
            runtime.globals.insert(root.id.clone(), env.clone());
            runtime.used_resources = saved.resources.clone();
            runtime.input_versions = saved.inputs.clone();
            let stale: Vec<_> = self.active.range((index + 1)..).copied().collect();
            for stale in stale {
                self.remove(stale);
            }
            return index;
        }
        self.clear_states();
        0
    }
    pub(super) fn save(&mut self, runtime: &Runtime<'_, '_>, env: &Env, statements: usize) {
        if self.states.is_empty() {
            return;
        }
        let payload = runtime.limits.data.max_bytes - runtime.retained_bytes;
        // Charge an upper estimate of each root's metadata, plus shared retained payload once.
        // This deliberately overcounts shared tree nodes rather than silently growing without a bound.
        let overhead = std::mem::size_of::<ExecutionState>()
            + runtime.reactive.overhead()
            + runtime.animation.reactive.metadata_bytes()
            + env.len() * (std::mem::size_of::<Value>() + std::mem::size_of::<String>() * 2)
            + runtime.animation.element_count()
                * (std::mem::size_of::<String>() + std::mem::size_of::<Vec<usize>>())
            + runtime.animation.layers.len() * std::mem::size_of::<Layer>()
            + runtime
                .animation
                .code_views
                .iter()
                .map(|(id, state)| {
                    id.len()
                        + std::mem::size_of::<String>()
                        + std::mem::size_of::<Arc<sanim_core::code::view::State>>()
                        + state.metadata_bytes()
                })
                .sum::<usize>()
            + runtime.time_map.entries.len() * std::mem::size_of::<TimeMapEntry>()
            + runtime.element_sources.len()
                * (std::mem::size_of::<String>()
                    + std::mem::size_of::<Arc<crate::ElementSource>>())
            + runtime.used_functions.len() * 128
            + runtime.assertions.len() * std::mem::size_of::<Assertion>()
            + runtime.checks.len() * std::mem::size_of::<Verification>()
            + runtime.reflected.len() * std::mem::size_of::<ReflectedInput>();
        let limit = runtime.limits.max_cache_bytes;
        if runtime.limits.max_cache_entries == 0
            || payload
                .saturating_add(overhead)
                .saturating_add(self.fixed_bytes)
                > limit
        {
            return;
        }
        if self.active.contains(&statements) {
            self.remove(statements);
        }
        while self.active.len() >= runtime.limits.max_cache_entries
            || payload
                .saturating_add(self.overhead)
                .saturating_add(overhead)
                .saturating_add(self.fixed_bytes)
                > limit
        {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.remove(oldest);
        }
        self.age = self.age.saturating_add(1);
        let saved = Arc::new(ExecutionState {
            scope: env.clone(),
            reactive: runtime.reactive.clone(),
            animation: runtime.animation.clone(),
            cursor: runtime.cursor,
            time_map: runtime.time_map.clone(),
            element_sources: runtime.element_sources.clone(),
            statements,
            reflected: runtime.reflected.clone(),
            assertions: runtime.assertions.clone(),
            checks: runtime.checks.clone(),
            used_overrides: runtime.used_overrides.clone(),
            functions: runtime.used_functions.clone(),
            inputs: runtime.input_versions.clone(),
            resources: runtime.used_resources.clone(),
            declarations: self.declarations.clone(),
            retained_bytes: runtime.retained_bytes,
            retained_items: runtime.retained_items,
            retained_frames: runtime.retained_frames.clone(),
            retained_fonts: runtime.retained_fonts.clone(),
            logical_operations: runtime.logical_operations,
            overhead,
            payload,
            age: self.age,
        });
        self.overhead += overhead;
        self.bytes = self.fixed_bytes + payload + self.overhead;
        self.states[statements] = Some(saved);
        self.active.insert(statements);
        self.order.insert((self.age, statements));
    }
    fn remove(&mut self, index: usize) {
        if let Some(saved) = self.states.get_mut(index).and_then(Option::take) {
            self.overhead -= saved.overhead;
            self.order.remove(&(saved.age, index));
        }
        self.active.remove(&index);
        self.recount();
    }
    fn recount(&mut self) {
        self.bytes = self.fixed_bytes
            + self
                .active
                .last()
                .and_then(|&index| self.get(index))
                .map_or(0, |saved| saved.payload)
            + self.overhead;
    }
    fn clear_states(&mut self) {
        self.states.fill(None);
        self.active.clear();
        self.order.clear();
        self.overhead = 0;
        self.bytes = self.fixed_bytes;
    }
}

pub(super) fn executable(statement: &Statement) -> bool {
    !matches!(
        statement.kind,
        StatementKind::Function { .. }
            | StatementKind::Alias { .. }
            | StatementKind::Enum { .. }
            | StatementKind::Import { .. }
    )
}

#[cfg(test)]
mod tests {
    use crate::{Engine, MemorySources};
    use std::collections::BTreeMap;
    #[test]
    fn disabled_or_tiny_cache_retains_no_prefix_or_slot_allocations() {
        let mut files = MemorySources::default();
        files.insert("main.sanim", "wait(1s)\n".repeat(100));
        for (bytes, entries) in [(0, 8192), (128, 8192), (64 * 1024 * 1024, 0)] {
            let mut engine = Engine::default();
            engine.limits.max_cache_bytes = bytes;
            engine.limits.max_cache_entries = entries;
            let prepared = engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap();
            assert_eq!(prepared.statistics.checkpoint_bytes, 0);
            assert_eq!(engine.states.prefixes.capacity(), 0);
            assert_eq!(engine.states.states.capacity(), 0);
        }
    }
}
