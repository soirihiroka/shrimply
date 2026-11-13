//! Sanim: a bounded, typed animation scripting language, independent of any editor.
mod animations;
pub mod ast;
mod cache;
mod charts;
mod checker;
mod code_view;
pub mod dataframe;
mod utilities;
mod effects;
mod eval;
#[cfg(test)]
mod font_tests;
pub mod formatter;
mod lint;
mod lists;
mod media;
mod native;
mod native_types;
pub mod parser;
pub mod prelude;
pub mod progress;
mod reflection;
mod scrolling;
pub mod source;
mod timing;
mod type_algebra;
pub mod watch;

pub use checker::{Checked as CheckedModule, NativeMethod};
pub use eval::state::ExecutionState;
pub use lint::{AnalyzedModule, LintDiagnostic, LintReport, LintSeverity};
pub use sanim_core as core;
use sanim_core::{Animation, Diagnostic, ReflectedInput, Span, Type, Value, digest};
use serde::Serialize;
use source::Snapshot;
pub use source::{ElementSource, SourceLocation};
pub use source::{FileSources, MemorySources, Sources};
use std::{collections::BTreeMap, sync::Arc};
pub use watch::{Reload, Session};

#[derive(Clone, Debug, Serialize)]
pub struct Limits {
    pub max_source_bytes: usize,
    pub max_parse_steps: usize,
    pub max_modules: usize,
    pub max_type_bytes: usize,
    pub max_time_map_bytes: usize,
    pub max_operations: u64,
    pub max_iterations: usize,
    pub max_elements: usize,
    pub max_call_depth: usize,
    pub max_cache_entries: usize,
    pub max_cache_bytes: usize,
    pub warn_operations: u64,
    pub warn_prepare_micros: u64,
    pub data: sanim_data::Limits,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_source_bytes: 1024 * 1024,
            max_parse_steps: 200_000,
            max_modules: 128,
            max_type_bytes: 8 * 1024 * 1024,
            max_time_map_bytes: 16 * 1024 * 1024,
            max_operations: 1_000_000,
            max_iterations: 100_000,
            max_elements: 10_000,
            max_call_depth: 64,
            max_cache_entries: 8192,
            max_cache_bytes: 64 * 1024 * 1024,
            warn_operations: 10_000,
            warn_prepare_micros: 50_000,
            data: sanim_data::Limits::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Statistics {
    pub deferred_values: u64,
    pub forced_values: u64,
    pub parsed_modules: u64,
    pub reused_modules: u64,
    pub evaluated_expressions: u64,
    pub reused_expressions: u64,
    pub signal_evaluations: u64,
    pub signal_cache_hits: u64,
    pub signal_invalidations: u64,
    pub mutable_writes: u64,
    pub unchanged_writes: u64,
    pub decoded_assets: u64,
    pub reused_assets: u64,
    pub operations: u64,
    pub restored_statements: u64,
    pub restored_until: sanim_core::Fraction,
    pub checkpoint_count: usize,
    pub checkpoint_bytes: usize,
    pub prepare_micros: u64,
    pub executed_queries: u64,
    pub reused_queries: u64,
    pub reused_databases: u64,
    pub sql_instructions: u64,
    pub query_micros: u64,
    pub cache_key_micros: u64,
    pub native_micros: u64,
    pub value_validation_micros: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct PerformanceWarning {
    pub code: String,
    pub message: String,
    pub source: String,
    pub span: Span,
    pub operations: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Prepared {
    /// Normalized animation entry path relative to its source root.
    pub source: String,
    pub animation: Animation,
    pub time_map: sanim_core::TimeMap,
    /// Constructor provenance is editor metadata, independent of visual cache identity.
    pub element_sources: imbl::OrdMap<String, Arc<ElementSource>>,
    /// Immutable source bytes from this preparation, including imported modules.
    #[serde(skip)]
    pub source_texts: BTreeMap<String, Arc<Vec<u8>>>,
    pub bindings: imbl::OrdMap<String, Value>,
    /// Values intentionally left unevaluated by demand preparation.
    pub deferred_bindings: Vec<DeferredBinding>,
    pub reflected: imbl::Vector<ReflectedInput>,
    pub types: BTreeMap<String, Type>,
    pub symbols: BTreeMap<String, Type>,
    pub scheduling_helpers: Vec<String>,
    pub durations: BTreeMap<String, Vec<sanim_core::NodeDuration>>,
    pub checks: imbl::Vector<sanim_core::verify::Verification>,
    pub contracts: imbl::Vector<sanim_core::verify::Contract>,
    pub dependencies: BTreeMap<String, String>,
    pub fingerprint: String,
    pub statistics: Statistics,
    pub performance: Vec<PerformanceWarning>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeferredBinding {
    pub name: String,
    pub ty: Type,
    pub source: String,
    pub span: Span,
}

/// Full preparation validates every published binding. Demand preparation
/// evaluates only values needed to construct and verify the animation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Preparation {
    #[default]
    Full,
    Demand,
}

impl Prepared {
    /// Verify that metadata still describes the current source/import/data bytes.
    /// This performs I/O; hosts should call it from their background API worker.
    pub fn validate_dependencies(&self, sources: &dyn Sources) -> Result<(), Diagnostic> {
        self.validate_dependencies_with(sources, |_, _, _| {})
    }
    fn validate_dependencies_with(
        &self,
        sources: &dyn Sources,
        mut observed: impl FnMut(u64, u64, &str),
    ) -> Result<(), Diagnostic> {
        let total = self.dependencies.len() as u64;
        for (index, (path, expected)) in self.dependencies.iter().enumerate() {
            let current=sources.read_snapshot(path).map(|(_,fingerprint)|fingerprint).map_err(|error|Diagnostic::new("S203",format!("dependency '{path}' changed or disappeared: {error}; refresh the source before using this revision"),&self.source,Span::default()))?;
            if &current != expected {
                return Err(Diagnostic::new(
                    "S203",
                    format!(
                        "dependency '{path}' changed; the prepared revision is stale. Refresh once the edit is complete."
                    ),
                    &self.source,
                    Span::default(),
                ));
            }
            observed(index as u64 + 1, total, path);
        }
        Ok(())
    }
    /// Reflow into an external viewport and revalidate required visual contracts.
    pub fn animation_for_viewport(&self, width: u32, height: u32) -> Result<Animation, Diagnostic> {
        if width == 0 || height == 0 {
            return Err(Diagnostic::new(
                "S350",
                "viewport dimensions must be positive",
                "",
                Span::default(),
            ));
        }
        let mut animation = self.animation.clone();
        animation.width = width;
        animation.height = height;
        for contract in &self.contracts {
            if !contract.required {
                continue;
            }
            let result =
                sanim_core::verify::evaluate(&animation, &contract.predicate, contract.time);
            if result.evidence != sanim_core::verify::Evidence::Proved {
                return Err(Diagnostic::new(
                    "S350",
                    format!(
                        "visual assertion for {width} × {height} viewport: {}",
                        result.message
                    ),
                    &contract.module,
                    contract.span,
                ));
            }
        }
        Ok(animation)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DurationReport {
    pub duration: sanim_core::Fraction,
    pub frames: u64,
    pub viewport: sanim_core::Viewport,
    pub dependencies: BTreeMap<String, String>,
    pub statistics: Statistics,
    pub performance: Vec<PerformanceWarning>,
}
impl DurationReport {
    fn from_prepared(prepared: Prepared) -> Result<Self, Diagnostic> {
        let animation = prepared.animation;
        let frames = sanim_core::math::frame_count(animation.duration, animation.rate)
            .map_err(|e| Diagnostic::new("S304", e, "", Span::default()))?;
        Ok(Self {
            duration: animation.duration,
            frames,
            viewport: sanim_core::Viewport {
                width: animation.width,
                height: animation.height,
                rate: animation.rate,
                dpi: animation.dpi,
            },
            dependencies: prepared.dependencies,
            statistics: prepared.statistics,
            performance: prepared.performance,
        })
    }
}

#[derive(Default)]
pub struct Engine {
    pub limits: Limits,
    pub preparation: Preparation,
    pub viewport: sanim_core::Viewport,
    parsed: BTreeMap<String, Arc<ast::Module>>,
    values: cache::ValueCache,
    assets: cache::ValueCache,
    databases: sanim_data::sqlite::Databases,
    pub(crate) attempted: BTreeMap<String, String>,
    progress_observer: Option<progress::Observer>,
    states: eval::state::Store,
    font_environment: Option<sanim_data::font::FontEpoch>,
}

#[derive(Clone)]
pub(crate) struct Loaded {
    pub id: String,
    pub ast: Arc<ast::Module>,
    pub imports: BTreeMap<String, String>,
    pub checked: Arc<checker::Checked>,
}

impl Engine {
    pub fn new(viewport: sanim_core::Viewport) -> Self {
        Self {
            viewport,
            ..Self::default()
        }
    }
    pub fn set_progress_observer(&mut self, observer: Option<progress::Observer>) {
        self.progress_observer = observer;
    }
    fn with_progress<T>(
        &mut self,
        entry: &str,
        prepare: impl FnOnce(&mut Self, &progress::Reporter) -> Result<T, Diagnostic>,
    ) -> Result<T, Diagnostic> {
        let reporter = progress::Reporter::new(self.progress_observer.clone(), entry);
        let result = prepare(self, &reporter);
        reporter.finish(result.as_ref().err());
        result
    }
    /// O(1) access to a retained immutable execution root. Cloning it preserves shared storage.
    pub fn execution_state(&self, boundary: usize) -> Option<Arc<ExecutionState>> {
        self.states.get(boundary)
    }
    pub fn clear_cache(&mut self) {
        self.parsed.clear();
        self.values.clear();
        self.assets.clear();
        self.databases.clear();
        self.states.clear();
        self.font_environment = None;
    }
    pub fn prepare(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, Value>,
    ) -> Result<Prepared, Diagnostic> {
        self.with_progress(entry, |engine, reporter| {
            engine.prepare_inputs(entry, sources, overrides, None, false, reporter)
        })
    }
    pub fn prepare_json(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, serde_json::Value>,
    ) -> Result<Prepared, Diagnostic> {
        self.with_progress(entry, |engine, reporter| {
            engine.prepare_inputs(
                entry,
                sources,
                &BTreeMap::new(),
                Some(overrides),
                false,
                reporter,
            )
        })
    }
    /// Resolve timeline length without rendering pixels. Data-only timing skips scene
    /// construction; timing that reads scene state uses normal preparation and emits P004.
    pub fn duration_json(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, serde_json::Value>,
    ) -> Result<DurationReport, Diagnostic> {
        self.with_progress(entry, |engine, reporter| {
            engine
                .prepare_inputs(
                    entry,
                    sources,
                    &BTreeMap::new(),
                    Some(overrides),
                    true,
                    reporter,
                )
                .and_then(DurationReport::from_prepared)
        })
    }
    pub fn duration(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, Value>,
    ) -> Result<DurationReport, Diagnostic> {
        self.with_progress(entry, |engine, reporter| {
            engine
                .prepare_inputs(entry, sources, overrides, None, true, reporter)
                .and_then(DurationReport::from_prepared)
        })
    }
    fn prepare_inputs(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, Value>,
        json: Option<&BTreeMap<String, serde_json::Value>>,
        duration_only: bool,
        reporter: &progress::Reporter,
    ) -> Result<Prepared, Diagnostic> {
        let started = reporter.started;
        let observed_sources = progress::ObservedSources {
            provider: sources,
            reporter,
        };
        let sources = &observed_sources;
        self.values
            .configure(self.limits.max_cache_bytes, self.limits.max_cache_entries);
        self.assets
            .configure(self.limits.max_cache_bytes, self.limits.max_cache_entries);
        self.databases
            .configure(self.limits.max_cache_bytes, self.limits.max_cache_entries);
        let mut snapshot = Snapshot {
            provider: sources,
            reads: BTreeMap::new(),
            hashes: Default::default(),
        };
        let mut statistics = Statistics::default();
        self.viewport
            .validate()
            .map_err(|e| Diagnostic::new("S300", e, entry, Span::default()))?;
        let mut loaded = vec![];
        let type_budget = std::cell::Cell::new(self.limits.max_type_bytes);
        let result = (|| {
            let entry = source::resolve("", entry)
                .map_err(|e| Diagnostic::new("S201", e, entry, Span::default()))?;
            self.load(
                &entry,
                &mut snapshot,
                &mut loaded,
                &mut vec![],
                (&mut statistics, &type_budget, reporter),
            )?;
            let duration_needed = duration_only.then(|| eval::duration::duration_needs(&loaded));
            let scene_timing = duration_needed
                .as_ref()
                .and_then(|needed| eval::duration::scene_dependency(&loaded, needed));
            let duration_only = duration_only && scene_timing.is_none();
            let family_fonts = !duration_only
                && loaded.iter().any(|module| {
                    module.ast.statements.iter().any(|statement| {
                        let mut uses_family = false;
                        statement.visit_expressions(&mut |expression| {
                        if let ast::ExprKind::Call { callee, .. } = &expression.kind
                            && matches!(&callee.kind, ast::ExprKind::Name(name) if prelude::builtin(name).is_some_and(prelude::Builtin::uses_font_environment))
                        {
                            uses_family = true;
                        }
                    });
                        uses_family
                    })
                });
            if family_fonts {
                let epoch = sanim_data::font::system_font_epoch();
                if self.font_environment.is_some() && self.font_environment != epoch {
                    self.values.clear();
                    self.states.clear();
                }
                self.font_environment = epoch;
            }
            reporter.emit(
                progress::Phase::Checking,
                Some(loaded.len() as u64),
                Some(loaded.len() as u64),
                Some(&entry),
            );
            let inputs = sanim_core::reflection::Schema::new(
                &entry,
                loaded
                    .iter()
                    .flat_map(|module| module.checked.reflected.values().cloned()),
            )?;
            let converted = if let Some(json) = json {
                inputs.normalize(json, |ty, value| {
                    sanim_data::validate(value, ty, self.limits.data)
                })?
            } else {
                inputs.normalize(overrides, |ty, value| {
                    sanim_core::validate_value(ty, value)?;
                    Ok(value.clone())
                })?
            };
            let mut prepared = eval::evaluate(
                &loaded,
                &mut snapshot,
                &converted,
                self.preparation,
                (
                    &self.limits,
                    &self.viewport,
                    duration_needed.filter(|_| duration_only),
                    reporter,
                ),
                (
                    &mut self.values,
                    &mut self.assets,
                    &mut self.states,
                    &mut self.databases,
                ),
                &mut statistics,
            )?;
            if let Some((source, span)) = scene_timing {
                prepared.performance.push(PerformanceWarning {
                    code: "P004".into(),
                    message: "timeline duration depends on scene state; prepared animation tracks and layout to sample it accurately. Use data-only duration expressions to keep duration discovery independent of scene preparation".into(),
                    source,
                    span,
                    operations: prepared.statistics.operations,
                });
            }
            if family_fonts {
                let after = sanim_data::font::system_font_epoch();
                // Lazy native face loading may change the map serial during
                // shaping. Preserve the starting registry revision so an app
                // font activated mid-prepare forces the next reload to rebuild.
                self.font_environment = after.map(|after| sanim_data::font::FontEpoch {
                    revision: self
                        .font_environment
                        .map_or(after.revision, |before| before.revision),
                    ..after
                });
                prepared.fingerprint = digest(
                    &serde_json::to_vec(&(&prepared.fingerprint, self.font_environment))
                        .expect("native font environment serializes"),
                );
            }
            prepared.dependencies = snapshot.fingerprints();
            prepared.statistics = statistics.clone();
            Ok(prepared)
        })();
        self.attempted = snapshot.fingerprints();
        let mut prepared = result?;
        let total = prepared.dependencies.len() as u64;
        reporter.emit(
            progress::Phase::Finalizing,
            Some(0),
            Some(total),
            Some(entry),
        );
        prepared.validate_dependencies_with(sources, |completed, total, path| {
            reporter.emit(
                progress::Phase::Finalizing,
                Some(completed),
                Some(total),
                Some(path),
            );
        })?;
        prepared.statistics.prepare_micros =
            started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
        if prepared.statistics.prepare_micros > self.limits.warn_prepare_micros {
            prepared.performance.push(PerformanceWarning{code:"P002".into(),message:format!("preparation took {}us (budget {}us); {} expressions evaluated, {} expression cache hits, {} assets decoded; {} statements restored. Inspect the reported source hotspots and cache-key/native/query timings.",prepared.statistics.prepare_micros,self.limits.warn_prepare_micros,prepared.statistics.evaluated_expressions,prepared.statistics.reused_expressions,prepared.statistics.decoded_assets,prepared.statistics.restored_statements),source:entry.into(),span:Span::default(),operations:prepared.statistics.operations});
        }
        if self.parsed.len() > self.limits.max_modules * 2 {
            self.parsed.clear();
        }
        Ok(prepared)
    }
    fn load(
        &mut self,
        id: &str,
        snapshot: &mut Snapshot<'_>,
        loaded: &mut Vec<Loaded>,
        active: &mut Vec<String>,
        (statistics, type_budget, reporter): (
            &mut Statistics,
            &std::cell::Cell<usize>,
            &progress::Reporter,
        ),
    ) -> Result<(), Diagnostic> {
        if loaded.iter().any(|v| v.id == id) {
            return Ok(());
        }
        if active.iter().any(|v| v == id) {
            return Err(Diagnostic::new(
                "S202",
                format!("cyclic module import: {} -> {id}", active.join(" -> ")),
                id,
                Span::default(),
            ));
        }
        if loaded.len() + active.len() >= self.limits.max_modules {
            return Err(Diagnostic::new(
                "S202",
                "module count limit exceeded",
                id,
                Span::default(),
            ));
        }
        reporter.emit(
            progress::Phase::Sources,
            Some(statistics.parsed_modules + statistics.reused_modules),
            None,
            Some(id),
        );
        let bytes = snapshot
            .read(id)
            .map_err(|e| Diagnostic::new("S201", e, id, Span::default()))?;
        if bytes.len() > self.limits.max_source_bytes {
            return Err(Diagnostic::new(
                "S201",
                "source byte limit exceeded",
                id,
                Span::default(),
            ));
        }
        let hash = digest(
            &serde_json::to_vec(&(digest(&bytes), self.limits.max_parse_steps))
                .map_err(|e| Diagnostic::new("S201", e.to_string(), id, Span::default()))?,
        );
        let module = if let Some(module) = self.parsed.get(&hash) {
            statistics.reused_modules += 1;
            module.clone()
        } else {
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| Diagnostic::new("S201", e.to_string(), id, Span::default()))?;
            let module = Arc::new(parser::parse(id, text, self.limits.max_parse_steps)?);
            self.parsed.insert(hash, module.clone());
            statistics.parsed_modules += 1;
            module
        };
        reporter.emit(
            progress::Phase::Sources,
            Some(statistics.parsed_modules + statistics.reused_modules),
            None,
            Some(id),
        );
        active.push(id.into());
        let mut imports = BTreeMap::new();
        let mut checks = BTreeMap::new();
        for statement in &module.statements {
            if let ast::StatementKind::Import { path, alias } = &statement.kind {
                if prelude::protected(alias) {
                    return Err(Diagnostic::new(
                        "S101",
                        format!(
                            "'{alias}' is a protected Sanim name and cannot be used as a module alias"
                        ),
                        id,
                        statement.span,
                    ));
                }
                let mut path_resolved = source::resolve(id, path)
                    .map_err(|e| Diagnostic::new("S201", e, id, statement.span))?;
                if std::path::Path::new(&path_resolved).extension().is_none() {
                    path_resolved.push_str(".sanim");
                }
                if let Some(start) = active.iter().position(|path| path == &path_resolved) {
                    return Err(Diagnostic::new(
                        "S202",
                        format!(
                            "circular import: {} -> {path_resolved}. This import closes the cycle. Sanim requires acyclic module imports so top-level values have a defined preparation order. Move shared types and helpers into a third module imported by both files.",
                            active[start..].join(" -> ")
                        ),
                        id,
                        statement.span,
                    ));
                }
                self.load(
                    &path_resolved,
                    snapshot,
                    loaded,
                    active,
                    (&mut *statistics, type_budget, reporter),
                )?;
                let imported = loaded
                    .iter()
                    .find(|v| v.id == path_resolved)
                    .ok_or_else(|| {
                        Diagnostic::new("S201", "module resolution failed", id, statement.span)
                    })?;
                checks.insert(path.clone(), imported.checked.clone());
                imports.insert(alias.clone(), path_resolved);
            }
        }
        reporter.emit(
            progress::Phase::Checking,
            Some(loaded.len() as u64),
            None,
            Some(id),
        );
        let checked = checker::check(
            id,
            &module,
            &checks,
            active.len() == 1,
            type_budget,
            &self.limits,
        )?;
        active.pop();
        loaded.push(Loaded {
            id: id.into(),
            ast: module,
            imports,
            checked: Arc::new(checked),
        });
        reporter.emit(
            progress::Phase::Checking,
            Some(loaded.len() as u64),
            None,
            Some(id),
        );
        Ok(())
    }
}

mod templates;
