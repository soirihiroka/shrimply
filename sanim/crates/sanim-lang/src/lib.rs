//! Sanim: a bounded, typed animation scripting language, independent of any editor.
mod analysis;
pub(crate) mod animation_plan;
mod animations;
pub mod ast;
mod cache;
mod charts;
mod checker;
mod code_view;
mod colors;
pub mod dataframe;
mod eval;
#[cfg(all(test, feature = "native-text"))]
mod font_tests;
pub mod formatter;
mod lint;
mod list_functions;
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
mod textures;
mod timing;
mod type_algebra;
pub mod utilities;
mod voxel;
pub mod watch;

pub use animation_plan::presentation::{PageStop, Presentation};
pub use checker::{Checked as CheckedModule, NativeMethod};
pub use eval::state::ExecutionState;
pub use lint::{AnalyzedModule, LintDiagnostic, LintReport, LintSeverity};
pub use sanim_core as core;
use sanim_core::{Animation, Diagnostic, ReflectedInput, Span, Type, Value, digest};
use serde::Serialize;
use source::Snapshot;
pub use source::{ElementSource, SourceLocation};
pub use source::{FileSources, MemorySources, Sources};
use std::{collections::BTreeMap, sync::Arc, time::Instant};
pub use watch::{Reload, Session};

fn layout_diagnostic(
    error: sanim_core::plan::LayoutError,
    plan: &sanim_core::plan::Animation,
    source: &str,
) -> Diagnostic {
    use sanim_core::plan::LayoutError;
    let code = match &error {
        LayoutError::MissingParent => "S138",
        LayoutError::CyclicParent => "S139",
        LayoutError::Invalid(_) | LayoutError::InvalidAt { .. } => "S304",
    };
    let location = if let LayoutError::InvalidAt { location, .. } = &error {
        Some(location)
    } else {
        plan.origin.as_ref().map(|origin| &origin.location)
    };
    let (source, span) = location.map_or((source, Span::default()), |location| {
        (location.source.as_ref(), location.span)
    });
    Diagnostic::new(code, error.to_string(), source, span)
}

#[derive(Clone, Debug, Serialize)]
pub struct Limits {
    pub max_source_bytes: usize,
    pub max_parse_steps: usize,
    pub max_modules: usize,
    pub max_type_bytes: usize,
    pub max_time_map_bytes: usize,
    pub max_operations: u64,
    /// Cumulative native algorithm work, independent of interpreted DSL operations.
    pub max_native_work: u64,
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
            max_native_work: sanim_voxel::procedural::MAX_PROCEDURAL_WORK as u64,
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
    pub checked_modules: u64,
    pub reused_checks: u64,
    pub parse_micros: u64,
    pub check_micros: u64,
    pub compiler_cache_micros: u64,
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
    /// Executed interpreter/validation work, excluding native algorithm loops.
    pub interpreted_operations: u64,
    /// Executed native work units, also included in operations for profiling.
    pub native_work: u64,
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

#[derive(Clone, Debug)]
pub struct Prepared {
    /// Normalized animation entry path relative to its source root.
    pub source: String,
    pub animation: Animation,
    pub plan: Arc<sanim_core::plan::Animation>,
    pub temporal_layout: Arc<[sanim_core::plan::Receipt]>,
    plan_base: Arc<Animation>,
    plan_limits: Arc<Limits>,
    plan_fingerprint: String,
    pub time_map: sanim_core::TimeMap,
    /// Constructor provenance is editor metadata, independent of visual cache identity.
    pub element_sources: imbl::OrdMap<String, Arc<ElementSource>>,
    /// Immutable source bytes from this preparation, including imported modules.
    pub source_texts: BTreeMap<String, Arc<Vec<u8>>>,
    pub bindings: imbl::OrdMap<String, Value>,
    /// Values intentionally left unevaluated by demand preparation.
    pub deferred_bindings: Vec<DeferredBinding>,
    pub reflected: imbl::Vector<ReflectedInput>,
    pub types: BTreeMap<String, Type>,
    pub symbols: BTreeMap<String, Type>,
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
    pub fn natural_extent(&self) -> sanim_core::plan::Extent {
        self.temporal_layout
            .first()
            .expect("prepared root has a temporal receipt")
            .natural_extent()
    }
    /// Infinite and parent-sized sources keep their own clock while the host crops a window.
    pub fn source_extent(&self) -> Option<sanim_core::Fraction> {
        bounded_source_extent(&self.temporal_layout, self.animation.duration)
    }
    /// Reuse the immutable built plan; no source read, parsing or language evaluation.
    pub fn relayout(
        &self,
        viewport: &sanim_core::Viewport,
        available: Option<sanim_core::Fraction>,
    ) -> Result<Self, Diagnostic> {
        viewport
            .validate()
            .map_err(|message| Diagnostic::new("S304", message, &self.source, Span::default()))?;
        let mut base = self.plan_base.as_ref().clone();
        base.width = viewport.width;
        base.height = viewport.height;
        base.dpi = viewport.dpi;
        base.rate = viewport.rate;
        let lowered = animation_plan::lower(&self.plan, &base, available, &self.plan_limits)
            .map_err(|error| layout_diagnostic(error, &self.plan, &self.source))?;
        let time_map = layout_time_map(
            &lowered.layout,
            &lowered.animation,
            self.plan_limits.max_time_map_bytes,
            true,
        )
        .map_err(|message| Diagnostic::new("S303", message, &self.source, Span::default()))?;
        self.validate_contracts(&lowered.animation)?;
        let mut result = self.clone();
        result.animation = lowered.animation;
        result.plan_base = Arc::new(base);
        result.temporal_layout = lowered.layout;
        result.time_map = time_map;
        result.fingerprint = digest(
            &serde_json::to_vec(&(&self.plan_fingerprint, viewport, available))
                .expect("layout inputs serialize"),
        );
        result.statistics = Statistics::default();
        Ok(result)
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
        self.validate_contracts(&animation)?;
        Ok(animation)
    }
    fn validate_contracts(&self, animation: &Animation) -> Result<(), Diagnostic> {
        for contract in &self.contracts {
            if !contract.required {
                continue;
            }
            validate_contract(animation, contract)?;
        }
        Ok(())
    }
}

fn validate_contract(
    animation: &Animation,
    contract: &sanim_core::verify::Contract,
) -> Result<sanim_core::verify::Verification, Diagnostic> {
    use sanim_core::verify;
    let verification = verify::evaluate_with(
        animation,
        &contract.predicate,
        contract.time,
        sanim_data::resolve_pending,
    );
    if contract.required && verification.evidence != verify::Evidence::Proved {
        return Err(Diagnostic::new(
            "S350",
            format!(
                "visual assertion {:?} for {} × {} viewport: {}{}",
                verification.evidence,
                animation.width,
                animation.height,
                verification.message,
                contract
                    .message
                    .as_ref()
                    .map(|message| format!(". {message}"))
                    .unwrap_or_default()
            ),
            &contract.module,
            contract.span,
        ));
    }
    Ok(verification)
}

fn bounded_source_extent(
    layout: &[sanim_core::plan::Receipt],
    duration: sanim_core::Fraction,
) -> Option<sanim_core::Fraction> {
    let root = layout
        .first()
        .expect("prepared root has a temporal receipt");
    (root.requested != sanim_core::plan::TimeDimension::MatchParent
        && root.natural_extent() != sanim_core::plan::Extent::Infinite)
        .then_some(duration)
}

fn layout_time_map(
    layout: &[sanim_core::plan::Receipt],
    animation: &Animation,
    budget: usize,
    frames: bool,
) -> Result<sanim_core::TimeMap, String> {
    let mut map = sanim_core::TimeMap::default();
    let source_end = bounded_source_extent(layout, animation.duration);
    for (index, receipt) in layout.iter().enumerate() {
        let Some(origin) = &receipt.origin else {
            continue;
        };
        let (start, end) = if receipt.clip.is_none()
            && (receipt.resolved == sanim_core::plan::Extent::Infinite
                || index == 0 && source_end.is_none())
        {
            (receipt.start, None)
        } else {
            let Some((start, end)) = receipt.active_interval(animation.duration)? else {
                continue;
            };
            (start, Some(end))
        };
        map.push(
            sanim_core::TimeMapEntry {
                source: origin.location.source.to_string(),
                span: origin.location.span,
                start,
                end,
                statement: origin.checkpoint,
                call_path: Default::default(),
                clock: receipt.clock.clone(),
            },
            budget,
        )?;
    }
    map.index(frames.then_some(animation.rate), source_end, budget)?;
    Ok(map)
}

#[derive(Clone, Debug, Serialize)]
pub struct DurationReport {
    pub duration: sanim_core::Fraction,
    pub indefinite: bool,
    pub event_index: std::sync::Arc<sanim_core::EventIndex>,
    pub temporal_layout: Arc<[sanim_core::plan::Receipt]>,
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
            indefinite: animation.indefinite,
            event_index: prepared.time_map.event_index(),
            temporal_layout: prepared.temporal_layout,
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
    /// External temporal viewport; None measures the returned root intrinsically.
    pub available_duration: Option<sanim_core::Fraction>,
    pub viewport: sanim_core::Viewport,
    parsed: sanim_core::cache::Cache<String, ast::Module>,
    checked: sanim_core::cache::Cache<String, cache::checked::Module>,
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
    pub interface_key: String,
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
        self.parsed.configure(
            self.limits.max_cache_bytes,
            self.limits
                .max_cache_entries
                .min(self.limits.max_modules.saturating_mul(2)),
        );
        self.checked.configure(
            self.limits.max_cache_bytes,
            self.limits
                .max_cache_entries
                .min(self.limits.max_modules.saturating_mul(2)),
        );
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
        self.checked.clear();
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
    /// Build the pure plan and resolve its temporal layout without lowering or rendering it.
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
        measure_only: bool,
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
            if loaded
                .last()
                .is_none_or(|module| module.checked.output_type.is_none())
            {
                return Err(Diagnostic::new(
                    "S134",
                    "the entry script must produce an Animation value; leave an Animation expression unassigned or return it explicitly",
                    &entry,
                    loaded
                        .last()
                        .and_then(|module| module.ast.statements.last())
                        .map_or(Span::default(), |statement| statement.span),
                ));
            }
            let family_fonts = !measure_only
                && loaded.iter().any(|module| {
                    module.ast.statements.iter().any(|statement| {
                        let mut uses_family = false;
                        statement.visit_all_expressions(&mut |expression| {
                            if let ast::ExprKind::Call { callee, args, .. } = &expression.kind
                                && crate::timing::builtin_call(
                                    callee,
                                    args,
                                    &module.checked.native_methods,
                                )
                                .is_some_and(|call| call.builtin.uses_font_environment())
                            {
                                uses_family = true;
                            }
                        });
                        uses_family
                    })
                });
            if family_fonts {
                let epoch = sanim_data::font::system_font_epoch();
                if epoch.is_none()
                    || self.font_environment.is_some() && self.font_environment != epoch
                {
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
                if measure_only {
                    Preparation::Demand
                } else {
                    self.preparation
                },
                (
                    &self.limits,
                    &self.viewport,
                    self.available_duration,
                    measure_only,
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
            prepared.plan_fingerprint = prepared.fingerprint.clone();
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
            &serde_json::to_vec(&(snapshot.fingerprint(id), self.limits.max_parse_steps))
                .map_err(|e| Diagnostic::new("S201", e.to_string(), id, Span::default()))?,
        );
        let module = if let Some(module) = self.parsed.get(&hash) {
            statistics.reused_modules += 1;
            module.clone()
        } else {
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| Diagnostic::new("S201", e.to_string(), id, Span::default()))?;
            let started = Instant::now();
            let module = Arc::new(parser::parse(id, text, self.limits.max_parse_steps)?);
            statistics.parse_micros = statistics
                .parse_micros
                .saturating_add(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
            if self.limits.max_cache_entries > 0
                && let Some(retained) = cache::parsed::retained_bytes(
                    &module,
                    bytes.len(),
                    hash.len(),
                    self.limits.max_cache_bytes,
                )
            {
                self.parsed.insert(hash.clone(), module.clone(), retained);
            }
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
        let mut import_keys = BTreeMap::new();
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
                import_keys.insert(path.clone(), imported.interface_key.clone());
                imports.insert(alias.clone(), path_resolved);
            }
        }
        reporter.emit(
            progress::Phase::Checking,
            Some(loaded.len() as u64),
            None,
            Some(id),
        );
        let root = active.len() == 1;
        let check_key = digest(
            &serde_json::to_vec(&(
                sanim_core::SEMANTICS_VERSION,
                id,
                &hash,
                &import_keys,
                root,
                &self.limits,
            ))
            .expect("serializable compiler inputs"),
        );
        let cached = self
            .checked
            .get(&check_key)
            .filter(|entry| entry.type_bytes <= type_budget.get());
        let (checked, interface_key) = if let Some(cached) = cached {
            type_budget.set(type_budget.get() - cached.type_bytes);
            statistics.reused_checks += 1;
            (cached.checked.clone(), cached.interface_key.clone())
        } else {
            let before = type_budget.get();
            let started = Instant::now();
            let checked = Arc::new(checker::check(
                id,
                &module,
                &checks,
                root,
                type_budget,
                &self.limits,
            )?);
            statistics.check_micros = statistics
                .check_micros
                .saturating_add(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
            statistics.checked_modules += 1;
            let cache_started = Instant::now();
            let interface_key = digest(
                &serde_json::to_vec(&(
                    &checked.exports,
                    &checked.dynamic_exports,
                    &checked.schemas,
                ))
                .expect("serializable module interface"),
            );
            if self.limits.max_cache_entries > 0
                && let Some(bytes) = cache::checked::retained_bytes(
                    &checked,
                    &check_key,
                    self.limits.max_cache_bytes,
                )
            {
                self.checked.insert(
                    check_key.clone(),
                    Arc::new(cache::checked::Module {
                        checked: checked.clone(),
                        type_bytes: before - type_budget.get(),
                        interface_key: interface_key.clone(),
                    }),
                    bytes,
                );
            }
            statistics.compiler_cache_micros = statistics.compiler_cache_micros.saturating_add(
                cache_started
                    .elapsed()
                    .as_micros()
                    .try_into()
                    .unwrap_or(u64::MAX),
            );
            (checked, interface_key)
        };
        active.pop();
        loaded.push(Loaded {
            id: id.into(),
            ast: module,
            imports,
            checked,
            interface_key,
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
