//! Sanim: a bounded, typed animation scripting language, independent of any editor.
pub mod ast;
mod cache;
mod checker;
mod effects;
mod eval;
mod native;
mod native_types;
pub mod parser;
pub mod prelude;
pub mod source;
pub mod watch;

pub use sanim_core as core;
use sanim_core::{Animation, Diagnostic, ReflectedInput, Span, Type, Value, digest};
use serde::Serialize;
use source::Snapshot;
pub use source::{FileSources, MemorySources, Sources};
use std::{collections::BTreeMap, sync::Arc, time::Instant};
pub use watch::{Reload, Session};

#[derive(Clone, Debug, Serialize)]
pub struct Limits {
    pub max_source_bytes: usize,
    pub max_parse_steps: usize,
    pub max_modules: usize,
    pub max_type_bytes: usize,
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
    pub parsed_modules: u64,
    pub reused_modules: u64,
    pub evaluated_expressions: u64,
    pub reused_expressions: u64,
    pub decoded_assets: u64,
    pub reused_assets: u64,
    pub operations: u64,
    pub prepare_micros: u64,
    pub executed_queries: u64,
    pub reused_queries: u64,
    pub query_micros: u64,
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
    pub animation: Animation,
    pub bindings: BTreeMap<String, Value>,
    pub reflected: Vec<ReflectedInput>,
    pub types: BTreeMap<String, Type>,
    pub symbols: BTreeMap<String, Type>,
    pub scheduling_helpers: Vec<String>,
    pub durations: BTreeMap<String, Vec<sanim_core::NodeDuration>>,
    pub checks: Vec<sanim_core::verify::Verification>,
    pub contracts: Vec<sanim_core::verify::Contract>,
    pub dependencies: BTreeMap<String, String>,
    pub fingerprint: String,
    pub statistics: Statistics,
    pub performance: Vec<PerformanceWarning>,
}

impl Prepared {
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
    pub viewport: sanim_core::Viewport,
    parsed: BTreeMap<String, Arc<ast::Module>>,
    values: cache::ValueCache,
    assets: cache::ValueCache,
    pub(crate) attempted: BTreeMap<String, String>,
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
    pub fn clear_cache(&mut self) {
        self.parsed.clear();
        self.values.clear();
        self.assets.clear();
    }
    pub fn prepare(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, Value>,
    ) -> Result<Prepared, Diagnostic> {
        self.prepare_inputs(entry, sources, overrides, None, false)
    }
    pub fn prepare_json(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, serde_json::Value>,
    ) -> Result<Prepared, Diagnostic> {
        self.prepare_inputs(entry, sources, &BTreeMap::new(), Some(overrides), false)
    }
    /// Resolve timeline length without creating animation layers or rendering pixels.
    pub fn duration_json(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, serde_json::Value>,
    ) -> Result<DurationReport, Diagnostic> {
        self.duration_prepared(entry, sources, &BTreeMap::new(), Some(overrides))
            .and_then(DurationReport::from_prepared)
    }
    pub fn duration(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, Value>,
    ) -> Result<DurationReport, Diagnostic> {
        self.duration_prepared(entry, sources, overrides, None)
            .and_then(DurationReport::from_prepared)
    }
    fn duration_prepared(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, Value>,
        json: Option<&BTreeMap<String, serde_json::Value>>,
    ) -> Result<Prepared, Diagnostic> {
        match self.prepare_inputs(entry, sources, overrides, json, true) {
            Err(error) if error.code == "S390" => {
                self.prepare_inputs(entry, sources, overrides, json, false).map(|mut prepared|{prepared.performance.push(PerformanceWarning{code:"P004".into(),message:"duration depends on sampled animation state; full scene preparation was required".into(),source:error.source,span:error.span,operations:prepared.statistics.operations});prepared})
            }
            result => result,
        }
    }
    fn prepare_inputs(
        &mut self,
        entry: &str,
        sources: &dyn Sources,
        overrides: &BTreeMap<String, Value>,
        json: Option<&BTreeMap<String, serde_json::Value>>,
        duration_only: bool,
    ) -> Result<Prepared, Diagnostic> {
        let started = Instant::now();
        self.values
            .configure(self.limits.max_cache_bytes, self.limits.max_cache_entries);
        self.assets
            .configure(self.limits.max_cache_bytes, self.limits.max_cache_entries);
        let mut snapshot = Snapshot {
            provider: sources,
            reads: BTreeMap::new(),
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
                &mut statistics,
                &type_budget,
            )?;
            let mut converted = BTreeMap::new();
            if let Some(json) = json {
                let mut inputs = BTreeMap::new();
                for module in &loaded {
                    for statement in &module.ast.statements {
                        if let ast::StatementKind::Bind {
                            name,
                            reflect: true,
                            ..
                        } = &statement.kind
                        {
                            let key = if module.id == entry {
                                name.clone()
                            } else {
                                format!("{}::{name}", module.id)
                            };
                            inputs.insert(
                                key,
                                (&module.id, statement.span, &module.checked.exports[name]),
                            );
                        }
                    }
                }
                for (key, value) in json {
                    let (source, span, ty) = inputs.get(key).ok_or_else(|| {
                        Diagnostic::new(
                            "S302",
                            format!("unknown reflected input '{key}'"),
                            &entry,
                            Span::default(),
                        )
                    })?;
                    converted.insert(
                        key.clone(),
                        sanim_data::validate(value, ty, self.limits.data).map_err(|e| {
                            Diagnostic::new(
                                "S302",
                                format!("input '{key}' must be {ty}: {e}"),
                                source,
                                *span,
                            )
                        })?,
                    );
                }
            }
            let mut prepared = eval::evaluate(
                &loaded,
                &mut snapshot,
                if json.is_some() {
                    &converted
                } else {
                    overrides
                },
                (&self.limits, &self.viewport, duration_only),
                &mut self.values,
                &mut self.assets,
                &mut statistics,
            )?;
            prepared.dependencies = snapshot.fingerprints();
            prepared.statistics = statistics.clone();
            Ok(prepared)
        })();
        self.attempted = snapshot.fingerprints();
        let mut prepared = result?;
        for (path, expected) in &prepared.dependencies {
            let current=sources.read(path).map(|bytes|digest(&bytes)).map_err(|e|Diagnostic::new("S203",format!("dependency '{path}' changed or disappeared while preparing: {e}; retry the revision"),entry,Span::default()))?;
            if &current != expected {
                return Err(Diagnostic::new(
                    "S203",
                    format!(
                        "dependency '{path}' changed while preparing; the revision was discarded. Retry once the edit is complete."
                    ),
                    entry,
                    Span::default(),
                ));
            }
        }
        prepared.statistics.prepare_micros =
            started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
        if prepared.statistics.prepare_micros > self.limits.warn_prepare_micros {
            prepared.performance.push(PerformanceWarning{code:"P002".into(),message:format!("preparation took {}us (budget {}us); {} expressions rebuilt, {} reused, {} assets decoded. Inspect source hotspots and move repeated work outside loops.",prepared.statistics.prepare_micros,self.limits.warn_prepare_micros,prepared.statistics.evaluated_expressions,prepared.statistics.reused_expressions,prepared.statistics.decoded_assets),source:entry.into(),span:Span::default(),operations:prepared.statistics.operations});
        }
        if self.values.len() > self.limits.max_cache_entries {
            self.values.clear();
        }
        if self.assets.len() > self.limits.max_cache_entries {
            self.assets.clear();
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
        statistics: &mut Statistics,
        type_budget: &std::cell::Cell<usize>,
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
                    statistics,
                    type_budget,
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
        Ok(())
    }
}
