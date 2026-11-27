mod code_queries;
mod dataframe;
mod demand;
mod functions;
mod list_functions;
mod lists;
mod measure;
mod native;
mod plans;
mod provenance;
mod pure;
mod reactive;
mod reactive_animation;
mod resources;
mod signals;
mod templates;
use reactive::Binding;
pub(crate) mod state;
use crate::ast::referenced_names as dependencies;
use crate::{
    Limits, Loaded, PerformanceWarning, Prepared, Statistics,
    ast::*,
    prelude,
    progress::{Phase, Reporter},
    source::{self, Snapshot},
};
use sanim_core::{
    verify::{Contract as Assertion, Verification},
    *,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Instant,
};

// Arithmetic is centralized in sanim-core::math; the evaluator only dispatches operators.
type Values = imbl::OrdMap<String, Value>;
type Env = imbl::OrdMap<String, Binding>;
type ObservedSignals = BTreeMap<(String, usize, usize), sanim_core::SignalRef>;
type TypeBindings = Arc<BTreeMap<String, Type>>;

#[derive(serde::Serialize)]
enum CacheDependency<'a> {
    Value(sanim_core::value_key::ValueKey<'a>),
    Font(String),
}

#[derive(Clone, serde::Serialize)]
struct Function {
    capabilities: sanim_core::FnCapabilities,
    #[serde(skip)]
    references: BTreeSet<String>,
    module: String,
    name: String,
    parameters: Vec<Parameter>,
    body: Vec<Statement>,
    clause: usize,
    span: Span,
}
struct ClosureDefinition {
    function: Arc<Function>,
    fingerprint: String,
}
#[derive(Clone)]
struct FunctionGroup {
    clauses: Vec<Arc<Function>>,
    fingerprint: String,
    captures: Option<Env>,
    types: TypeBindings,
}

pub(crate) fn evaluate(
    loaded: &[Loaded],
    snapshot: &mut Snapshot<'_>,
    overrides: &BTreeMap<String, Value>,
    preparation: crate::Preparation,
    (limits, viewport, available, measure_only, progress): (
        &Limits,
        &Viewport,
        Option<Fraction>,
        bool,
        &Reporter,
    ),
    (values, assets, states, databases): (
        &mut crate::cache::ValueCache,
        &mut crate::cache::ValueCache,
        &mut state::Store,
        &mut sanim_data::sqlite::Databases,
    ),
    statistics: &mut Statistics,
) -> Result<Prepared, Diagnostic> {
    let total_statements = loaded
        .iter()
        .flat_map(|module| &module.ast.statements)
        .filter(|statement| {
            !matches!(
                statement.kind,
                StatementKind::Function { .. }
                    | StatementKind::Alias { .. }
                    | StatementKind::Enum { .. }
                    | StatementKind::Import { .. }
            )
        })
        .count() as u64;
    let mut runtime = Runtime::new(
        loaded,
        snapshot,
        limits,
        statistics,
        (values, assets, databases),
        (Animation::for_viewport(viewport), progress),
        None,
    );
    runtime.overrides = overrides;
    runtime.preparation = preparation;
    runtime.measure_only = measure_only;
    runtime.total_statements = total_statements;
    for module in loaded {
        runtime.module = module.id.clone();
        runtime.root_active = loaded.last().is_some_and(|v| v.id == module.id);
        runtime.report_evaluation(None);
        let mut env = Env::new();
        for (alias, id) in &module.imports {
            if !runtime.globals.contains_key(id) {
                return Err(runtime.error(
                    "S301",
                    "imported module has no values",
                    Span::default(),
                ));
            }
            env.insert(alias.clone(), Binding::Module(id.clone()));
        }
        runtime.install_bindings(ModuleBindings::new(module, true), &mut env);
        runtime.globals.insert(module.id.clone(), env.clone());
        let restored = if runtime.root_active {
            let restored = states.begin(&mut runtime, &mut env, module);
            if restored == 0 {
                states.save(&runtime, &env, 0);
            }
            restored
        } else {
            0
        };
        runtime
            .animation
            .reactive
            .set_native_byte_limit(runtime.limits.data.max_bytes)
            .map_err(|error| runtime.error("S140", error, Span::default()))?;
        for (index, statement) in module
            .ast
            .statements
            .iter()
            .filter(|statement| state::executable(statement))
            .enumerate()
        {
            if runtime.entry_returned {
                break;
            }
            if index < restored {
                continue;
            }
            runtime.root_statement = index;
            runtime.report_evaluation(Some(statement.span));
            let output = runtime.block(std::slice::from_ref(statement), &mut env)?;
            if runtime.root_active
                && let Some(Value::Animation(plan)) = output
            {
                runtime.collect_plan(plan, statement.span)?;
                runtime.entry_returned = true;
            }
            runtime.completed_statements += 1;
            runtime.report_evaluation(Some(statement.span));
            runtime.globals.insert(module.id.clone(), env.clone());
            if runtime.root_active {
                states.save(&runtime, &env, index + 1);
            }
        }
    }
    for key in overrides.keys() {
        if !measure_only && !runtime.used_overrides.contains(key) {
            return Err(runtime.error(
                "S302",
                format!(
                    "unknown reflected input '{key}'; available inputs: {}",
                    runtime
                        .reflected
                        .iter()
                        .map(|v| v.key.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Span::default(),
            ));
        }
    }
    let root = loaded
        .last()
        .ok_or_else(|| runtime.error("S301", "no root module", Span::default()))?;
    let scope = runtime.globals.get(&root.id).cloned().unwrap_or_default();
    let reactive_revision = runtime.reactive.revision;
    let bindings = if measure_only {
        Values::new()
    } else if preparation == crate::Preparation::Demand {
        runtime.available_bindings(&scope)
    } else {
        runtime.materialize(&scope, Span::default())?
    };
    let deferred_bindings = runtime.deferred_bindings(&scope);
    if !measure_only && runtime.reactive.revision != reactive_revision {
        states.save(
            &runtime,
            &scope,
            root.ast
                .statements
                .iter()
                .filter(|statement| state::executable(statement))
                .count(),
        );
    }
    let plan=match runtime.outputs.len() {
        0=>return Err(runtime.error("S134","the entry script must produce an Animation value; leave an Animation expression unassigned or return it explicitly",Span::default())),
        1=>runtime.outputs[0].clone(),
        _=>{
            let mut plan=sanim_core::plan::Animation::new(sanim_core::plan::Node::Container(runtime.outputs.clone()));
            plan.style.flow=sanim_core::plan::Flow::Sequence;
            Arc::new(plan)
        }
    };
    runtime.retain_binding_value(&Value::Animation(plan.clone()), Span::default())?;
    runtime
        .animation
        .reactive
        .set_native_byte_limit(runtime.limits.data.max_bytes)
        .map_err(|error| runtime.error("S140", error, Span::default()))?;
    let plan_base = Arc::new(runtime.animation.clone());
    runtime
        .progress
        .emit(Phase::Duration, None, None, Some(&runtime.module));
    let temporal_layout = if measure_only {
        let (duration, indefinite, layout) =
            crate::animation_plan::measure(&plan, available, limits)
                .map_err(|error| crate::layout_diagnostic(error, &plan, &runtime.module))?;
        runtime.animation.duration = duration;
        runtime.animation.indefinite = indefinite;
        layout
    } else {
        let mut lower_limits = limits.clone();
        lower_limits.max_native_work = limits
            .max_native_work
            .saturating_sub(runtime.logical_native_work);
        let lowered = crate::animation_plan::lower(&plan, &plan_base, available, &lower_limits)
            .map_err(|error| crate::layout_diagnostic(error, &plan, &runtime.module))?;
        runtime.charge_native_work(
            lowered.native_work,
            plan.origin
                .as_ref()
                .map_or(Span::default(), |origin| origin.location.span),
        )?;
        runtime.animation = lowered.animation;
        lowered.layout
    };
    let placements = temporal_layout.len() as u64;
    runtime.progress.emit(
        Phase::Duration,
        Some(placements),
        Some(placements),
        Some(&runtime.module),
    );
    let time_map = crate::layout_time_map(
        &temporal_layout,
        &runtime.animation,
        limits.max_time_map_bytes,
        !measure_only,
    )
    .map_err(|message| runtime.error("S303", message, Span::default()))?;
    let checks = runtime.assertions.len() as u64;
    runtime
        .progress
        .emit(Phase::Verifying, Some(0), Some(checks), None);
    for (index, assertion) in runtime.assertions.clone().into_iter().enumerate() {
        let verification = crate::validate_contract(&runtime.animation, &assertion)?;
        runtime.checks.push_back(verification);
        runtime.progress.emit(
            Phase::Verifying,
            Some(index as u64 + 1),
            Some(checks),
            Some(&assertion.module),
        );
    }
    runtime.statistics.checkpoint_count = states.len();
    runtime.statistics.checkpoint_bytes = states.bytes;
    let types = root.checked.schemas.clone();
    // Determinism makes complete semantic inputs a revision identity. Do not serialize
    // the scene and bindings again: both can reference the same large payloads.
    let fingerprint = digest(
        &serde_json::to_vec(&(
            SEMANTICS_VERSION,
            &root.id,
            runtime.snapshot.fingerprints(),
            overrides,
            limits,
            viewport,
            available,
            preparation,
        ))
        .map_err(|e| runtime.error("S301", e.to_string(), Span::default()))?,
    );
    let mut performance:Vec<_>=runtime.hotspots.into_iter().filter(|(_,count)|*count>=limits.warn_operations).map(|((module,start,line,column),operations)|PerformanceWarning{
        code:"P001".into(),message:format!("this statement performed {operations} interpreted preparation operations (warning budget {}). Hoist invariant work out of loops, reduce collection sizes, or split independent elements.",limits.warn_operations),source:module,span:Span{start,end:start,line,column},operations,
    }).collect();
    performance.extend(runtime.performance);
    Ok(Prepared {
        source: root.id.clone(),
        animation: runtime.animation,
        plan,
        temporal_layout,
        plan_base,
        plan_limits: Arc::new(limits.clone()),
        time_map,
        element_sources: runtime.element_sources,
        source_texts: loaded
            .iter()
            .filter_map(|module| {
                runtime.snapshot.reads.get(&module.id).and_then(|bytes| {
                    bytes
                        .as_ref()
                        .ok()
                        .map(|bytes| (module.id.clone(), bytes.clone()))
                })
            })
            .collect(),
        bindings,
        deferred_bindings,
        reflected: runtime.reflected,
        types,
        symbols: root.checked.exports.clone(),
        checks: runtime.checks,
        contracts: runtime.assertions,
        dependencies: BTreeMap::new(),
        plan_fingerprint: fingerprint.clone(),
        fingerprint,
        statistics: Statistics::default(),
        performance,
    })
}

pub(crate) struct Runtime<'a, 's> {
    loaded: &'a [Loaded],
    host: Option<crate::expression::Host<'a>>,
    snapshot: &'a mut Snapshot<'s>,
    overrides: &'a BTreeMap<String, Value>,
    limits: &'a Limits,
    preparation: crate::Preparation,
    measure_only: bool,
    values: &'a mut crate::cache::ValueCache,
    assets: &'a mut crate::cache::ValueCache,
    databases: &'a mut sanim_data::sqlite::Databases,
    statistics: &'a mut Statistics,
    progress: &'a Reporter,
    completed_statements: u64,
    total_statements: u64,
    font_keys: crate::cache::FontCache,
    expression_plans: sanim_core::cache::Cache<u64, plans::Plan>,
    globals: imbl::OrdMap<String, Env>,
    reactive: reactive::Store,
    signal_frames: Vec<reactive::Frame>,
    signal_observations: Vec<(bool, ObservedSignals)>,
    // Intrinsic work observed during active calls, excluding their argument effects.
    active_calls: Vec<String>,
    named_constructor: Option<(usize, String)>,
    pub(super) list_edit: Option<crate::lists::EditReceipt>,
    instance_paths: BTreeMap<String, BTreeMap<usize, String>>,
    functions: BTreeMap<String, Arc<FunctionGroup>>,
    closures: imbl::OrdMap<String, Arc<FunctionGroup>>,
    anonymous_definitions: imbl::OrdMap<(String, usize, usize), Arc<ClosureDefinition>>,
    next_closure: u64,
    constructors: BTreeMap<String, Type>,
    reflected: imbl::Vector<ReflectedInput>,
    used_overrides: imbl::OrdSet<String>,
    pub(crate) animation: Animation,
    pub(crate) cursor: Fraction,
    assertions: imbl::Vector<Assertion>,
    checks: imbl::Vector<Verification>,
    module: String,
    identity: Vec<String>,
    depth: usize,
    root_active: bool,
    signal_return: Option<Type>,
    type_bindings: TypeBindings,
    hotspots: imbl::OrdMap<(String, usize, usize, usize), u64>,
    performance: Vec<PerformanceWarning>,
    retained_bytes: usize,
    retained_items: usize,
    retained_frames: imbl::OrdSet<String>,
    retained_resources: sanim_core::value_budget::Budget,
    logical_operations: u64,
    logical_native_work: u64,
    used_functions: imbl::OrdMap<String, (String, usize)>,
    used_resources: imbl::OrdMap<String, (String, usize)>,
    input_versions: imbl::OrdMap<String, (Option<Value>, usize)>,
    element_sources: imbl::OrdMap<String, Arc<crate::ElementSource>>,
    source_calls: imbl::Vector<crate::SourceLocation>,
    source_ids: imbl::OrdMap<String, Arc<str>>,
    root_statement: usize,
    outputs: imbl::Vector<Arc<sanim_core::plan::Animation>>,
    entry_returned: bool,
}

pub(crate) struct ModuleBindings {
    env: Env,
    constructors: BTreeMap<String, Type>,
    functions: BTreeMap<String, Arc<FunctionGroup>>,
}

impl ModuleBindings {
    pub(crate) fn new(module: &Loaded, memoize: bool) -> Self {
        let mut env = Env::new();
        let mut constructors = BTreeMap::new();
        let mut functions = BTreeMap::new();
        for (name, ty) in &module.checked.schemas {
            if name.contains('.') {
                continue;
            }
            match ty {
                Type::Record(_) => {
                    let key = format!("{}::{name}", module.id);
                    constructors.insert(key.clone(), ty.clone());
                    env.insert(name.clone(), Value::Function(key).into());
                }
                Type::Enum { variants, .. } => {
                    let mut fields = Values::new();
                    for (tag, types) in variants {
                        if types.is_empty() {
                            fields.insert(
                                tag.clone(),
                                Value::Variant {
                                    ty: ty.clone().into(),
                                    tag: tag.clone(),
                                    fields: vec![].into(),
                                },
                            );
                        } else {
                            let key = format!("{}::{name}.{tag}", module.id);
                            constructors.insert(key.clone(), ty.clone());
                            fields.insert(tag.clone(), Value::Function(key));
                        }
                    }
                    env.insert(name.clone(), Value::Record(fields).into());
                }
                _ => {}
            }
        }
        for statement in &module.ast.statements {
            if let StatementKind::Function { name, .. } = &statement.kind {
                let key = format!("{}::{name}", module.id);
                let group = functions.entry(key.clone()).or_insert_with(|| {
                    Arc::new(FunctionGroup {
                        clauses: Vec::new(),
                        fingerprint: String::new(),
                        captures: None,
                        types: TypeBindings::default(),
                    })
                });
                let clause = module
                    .checked
                    .function_clauses
                    .get(&statement.span.start)
                    .copied()
                    .expect("checked function clause");
                Arc::make_mut(group)
                    .clauses
                    .push(Arc::new(Function::from_statement(
                        module.id.clone(),
                        statement,
                        clause,
                    )));
                env.insert(name.clone(), Value::Function(key).into());
            }
        }
        if memoize {
            for group in functions.values_mut() {
                let group = Arc::make_mut(group);
                group.fingerprint = digest(
                    &serde_json::to_vec(&group.clauses).expect("serializable function clauses"),
                );
            }
        }

        Self {
            env,
            constructors,
            functions,
        }
    }
}

pub(crate) fn evaluate_expression(
    expression: &crate::expression::CheckedExpression,
    inputs: crate::expression::Bindings,
    call: &crate::expression::HostCall<'_>,
) -> Result<Value, Diagnostic> {
    let files = crate::MemorySources::default();
    let mut snapshot = Snapshot {
        provider: &files,
        reads: BTreeMap::new(),
        hashes: Default::default(),
    };
    let mut values = crate::cache::ValueCache::default();
    let mut assets = crate::cache::ValueCache::default();
    let mut databases = sanim_data::sqlite::Databases::default();
    let mut statistics = Statistics::default();
    let progress = Reporter::new(None, "");
    static EMPTY_ANIMATION: std::sync::LazyLock<Animation> =
        std::sync::LazyLock::new(Animation::default);
    let module = &expression.module;
    let mut runtime = Runtime::new(
        std::slice::from_ref(module),
        &mut snapshot,
        &expression.limits,
        &mut statistics,
        (&mut values, &mut assets, &mut databases),
        (EMPTY_ANIMATION.clone(), &progress),
        Some(crate::expression::Host {
            interface: &expression.interface,
            call,
        }),
    );
    runtime.module.clone_from(&module.id);
    runtime.root_active = true;
    let mut env = expression.bindings.env.clone();
    runtime
        .constructors
        .clone_from(&expression.bindings.constructors);
    runtime.functions.clone_from(&expression.bindings.functions);
    if inputs.len() != expression.interface.inputs.len() {
        return Err(runtime.error(
            "S140",
            "expression input names do not match its interface",
            Span::default(),
        ));
    }
    for (name, value) in inputs {
        let ty = expression.interface.inputs.get(&name).ok_or_else(|| {
            runtime.error(
                "S140",
                format!("unknown expression input '{name}'"),
                Span::default(),
            )
        })?;
        runtime.retain_binding_value(&value, Span::default())?;
        sanim_core::validate_value(ty, &value).map_err(|error| {
            runtime.error("S140", format!("input '{name}': {error}"), Span::default())
        })?;
        env.insert(name, value.into());
    }
    if !runtime.functions.is_empty() {
        runtime.globals.insert(module.id.clone(), env.clone());
    }
    for statement in module
        .ast
        .statements
        .iter()
        .filter(|statement| state::executable(statement))
    {
        if let Some(value) = runtime.block(std::slice::from_ref(statement), &mut env)? {
            runtime.retain_binding_value(&value, statement.span)?;
            return runtime.typed_value(&value, &expression.interface.result, statement.span);
        }
        if !runtime.functions.is_empty() {
            runtime.globals.insert(module.id.clone(), env.clone());
        }
    }
    Err(runtime.error("S110", "expression must return a value", Span::default()))
}

impl<'a, 's> Runtime<'a, 's> {
    fn new(
        loaded: &'a [Loaded],
        snapshot: &'a mut Snapshot<'s>,
        limits: &'a Limits,
        statistics: &'a mut Statistics,
        (values, assets, databases): (
            &'a mut crate::cache::ValueCache,
            &'a mut crate::cache::ValueCache,
            &'a mut sanim_data::sqlite::Databases,
        ),
        (animation, progress): (Animation, &'a Reporter),
        host: Option<crate::expression::Host<'a>>,
    ) -> Self {
        static EMPTY_OVERRIDES: std::sync::OnceLock<BTreeMap<String, Value>> =
            std::sync::OnceLock::new();
        let instance_paths = if host.is_some() {
            BTreeMap::new()
        } else {
            loaded
                .iter()
                .map(|module| (module.id.clone(), reactive::instance_paths(&module.ast)))
                .collect()
        };
        Self {
            loaded,
            host,
            snapshot,
            overrides: EMPTY_OVERRIDES.get_or_init(BTreeMap::new),
            limits,
            values,
            assets,
            databases,
            font_keys: crate::cache::FontCache::new(
                limits.max_cache_bytes,
                limits.max_cache_entries,
            ),
            expression_plans: sanim_core::cache::Cache::new(
                limits.max_cache_bytes,
                limits.max_cache_entries,
            ),
            statistics,
            progress,
            completed_statements: 0,
            total_statements: 0,
            globals: imbl::OrdMap::new(),
            reactive: reactive::Store::default(),
            signal_frames: Vec::new(),
            signal_observations: Vec::new(),
            active_calls: Vec::new(),
            named_constructor: None,
            list_edit: None,
            instance_paths,
            functions: BTreeMap::new(),
            closures: imbl::OrdMap::new(),
            anonymous_definitions: imbl::OrdMap::new(),
            next_closure: 0,
            constructors: BTreeMap::new(),
            reflected: imbl::Vector::new(),
            used_overrides: imbl::OrdSet::new(),
            animation,
            cursor: Fraction::from(0),
            assertions: imbl::Vector::new(),
            checks: imbl::Vector::new(),
            module: String::new(),
            identity: vec![],
            depth: 0,
            root_active: false,
            preparation: crate::Preparation::Full,
            measure_only: false,
            signal_return: None,
            type_bindings: Arc::default(),
            hotspots: imbl::OrdMap::new(),
            performance: Vec::new(),
            retained_bytes: limits.data.max_bytes,
            retained_items: limits.data.max_values,
            retained_frames: imbl::OrdSet::new(),
            retained_resources: sanim_core::value_budget::Budget::default(),
            logical_operations: 0,
            logical_native_work: 0,
            used_functions: imbl::OrdMap::new(),
            used_resources: imbl::OrdMap::new(),
            input_versions: imbl::OrdMap::new(),
            element_sources: imbl::OrdMap::new(),
            source_calls: imbl::Vector::new(),
            source_ids: imbl::OrdMap::new(),
            root_statement: 0,
            outputs: imbl::Vector::new(),
            entry_returned: false,
        }
    }
    fn install_bindings(&mut self, bindings: ModuleBindings, env: &mut Env) {
        self.constructors.extend(bindings.constructors);
        self.functions.extend(bindings.functions);
        env.extend(bindings.env);
    }
    fn builtin(&self, name: &str) -> Option<&prelude::Builtin> {
        match &self.host {
            Some(host) => host.interface.builtin(name),
            None => prelude::builtin(name),
        }
    }

    fn report_evaluation(&self, span: Option<Span>) {
        if !self.progress.enabled() {
            return;
        }
        let resource = span.map_or_else(
            || self.module.clone(),
            |span| format!("{}:{}", self.module, span.line),
        );
        self.progress.emit(
            Phase::Evaluating,
            Some(self.completed_statements),
            Some(self.total_statements),
            Some(&resource),
        );
    }
    fn error(&self, code: &str, message: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic::new(code, message, &self.module, span)
    }
    fn work(&mut self, span: Span) -> Result<(), Diagnostic> {
        self.charge_work(1, span)
    }
    fn charge_work(&mut self, amount: u64, span: Span) -> Result<(), Diagnostic> {
        if amount == 0 {
            return Ok(());
        }
        self.statistics.operations = self.statistics.operations.saturating_add(amount);
        self.statistics.interpreted_operations = self
            .statistics
            .interpreted_operations
            .saturating_add(amount);
        self.logical_operations = self.logical_operations.saturating_add(amount);
        const PROGRESS_WORK_INTERVAL: u64 = 256;
        if self.progress.enabled()
            && self
                .statistics
                .operations
                .is_multiple_of(PROGRESS_WORK_INTERVAL)
        {
            self.report_evaluation(Some(span));
        }
        if self.logical_operations > self.limits.max_operations {
            return Err(self.error("S303",format!("preparation exceeded {} operations. All loops are bounded, but their combined work is too large; reduce counts or move invariant work out of loops.",self.limits.max_operations),span));
        }
        Ok(())
    }
    fn charge_native_work(&mut self, amount: u64, span: Span) -> Result<(), Diagnostic> {
        self.logical_native_work = self
            .logical_native_work
            .checked_add(amount)
            .ok_or_else(|| self.error("S303", "native work counter overflow", span))?;
        self.statistics.native_work = self.statistics.native_work.saturating_add(amount);
        self.statistics.operations = self.statistics.operations.saturating_add(amount);
        if self.logical_native_work > self.limits.max_native_work {
            return Err(self.error("S303", format!("preparation exceeded its native work budget: {} work units, limit {}. Reduce native samples, resolution, or repeated computation; the host can configure max_native_work independently of the DSL operation limit.", self.logical_native_work, self.limits.max_native_work),span));
        }
        Ok(())
    }
    fn collect_plan(
        &mut self,
        plan: Arc<sanim_core::plan::Animation>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        self.retain_binding_value(&Value::Animation(plan.clone()), span)?;
        self.retained_bytes = self
            .retained_bytes
            .checked_sub(std::mem::size_of::<Arc<sanim_core::plan::Animation>>())
            .ok_or_else(|| {
                self.error(
                    "S303",
                    "entry Animation collection exceeds its retained-data byte budget",
                    span,
                )
            })?;
        self.outputs.push_back(plan);
        Ok(())
    }
    fn block(&mut self, body: &[Statement], env: &mut Env) -> Result<Option<Value>, Diagnostic> {
        for statement in body {
            self.work(statement.span)?;
            let before = self.statistics.interpreted_operations;
            match &statement.kind {
                StatementKind::Bind {
                    kind, name, value, ..
                } => {
                    let previous_name = self
                        .named_constructor
                        .replace((value.span.start, name.clone()));
                    let mut binding = if *kind == BindingKind::Let
                        && self.should_defer(value, env, statement.span)
                    {
                        self.defer_binding(name, value, env, statement.span)
                    } else if *kind == BindingKind::Let {
                        let ty = self.binding_type(statement.span)?;
                        let result = if let Type::Signal(inner) = &ty {
                            self.capture_signal(value, env, inner)
                        } else {
                            self.expression(value, env)
                        };
                        result.and_then(|value| {
                            let value = self.typed_value(
                                &value,
                                &self.binding_type(statement.span)?,
                                statement.span,
                            )?;
                            self.retain_binding_value(&value, statement.span)?;
                            Ok(Binding::Value(value))
                        })
                    } else {
                        self.declare_reactive(name, *kind, value, env, statement.span)
                    };
                    if let Ok(Binding::Value(Value::Animation(plan))) = &mut binding {
                        let plan = std::sync::Arc::make_mut(plan);
                        let origin = plan.origin.get_or_insert_with(|| sanim_core::plan::Origin {
                            checkpoint: self.root_statement,
                            location: self.source_location(value.span),
                            binding: None,
                        });
                        origin.binding = Some(name.as_str().into());
                    }
                    self.named_constructor = previous_name;
                    env.insert(name.clone(), binding?);
                }
                StatementKind::Assign { name, value } => {
                    self.assign(name, value, env, statement.span)?
                }
                StatementKind::Expr(expr) => {
                    let value = self.expression(expr, env)?;
                    if self.root_active
                        && self.depth == 0
                        && let Value::Animation(plan) = value
                    {
                        self.collect_plan(plan, expr.span)?;
                    }
                }
                StatementKind::Return(expr) => {
                    let Some(expr) = expr else {
                        return Ok(Some(Value::Unit));
                    };
                    return if let Some(ty) = self.signal_return.clone() {
                        let value = self.capture_signal(expr, env, &ty)?;
                        self.retain_binding_value(&value, expr.span)?;
                        Ok(Some(value))
                    } else {
                        self.expression(expr, env).map(Some)
                    };
                }
                StatementKind::If { condition, yes, no } => {
                    let selected = if self
                        .expression(condition, env)?
                        .boolean()
                        .map_err(|e| self.error("S301", e, condition.span))?
                    {
                        yes
                    } else {
                        no
                    };
                    if let Some(value) = self.scoped_block(selected, &mut env.clone())? {
                        return Ok(Some(value));
                    }
                }
                StatementKind::For {
                    name,
                    iterable,
                    body,
                } => {
                    let collection = self.expression(iterable, env)?;
                    let values = collection
                        .list()
                        .map_err(|e| self.error("S301", e, iterable.span))?;
                    if values.len() > self.limits.max_iterations {
                        return Err(self.error(
                            "S303",
                            format!(
                                "for has {} iterations; limit is {}",
                                values.len(),
                                self.limits.max_iterations
                            ),
                            statement.span,
                        ));
                    }
                    for (index, value) in values.iter().enumerate() {
                        let mut scope = env.clone();
                        scope.insert(name.clone(), value.clone().into());
                        self.identity
                            .push(format!("for{}:{index}", self.instance_site(statement.span)));
                        let result = self.scoped_block(body, &mut scope);
                        self.identity.pop();
                        if let Some(value) = result? {
                            return Ok(Some(value));
                        }
                    }
                }
                StatementKind::Repeat { count, body } => {
                    let count = self
                        .expression(count, env)?
                        .number()
                        .and_then(Number::usize)
                        .map_err(|e| {
                            self.error("S303", format!("repeat count: {e}"), count.span)
                        })?;
                    if count > self.limits.max_iterations {
                        return Err(self.error(
                            "S303",
                            format!(
                                "repeat count {count} exceeds iteration limit {}",
                                self.limits.max_iterations
                            ),
                            statement.span,
                        ));
                    }
                    for index in 0..count {
                        self.identity.push(format!(
                            "repeat{}:{index}",
                            self.instance_site(statement.span)
                        ));
                        let result = self.scoped_block(body, &mut env.clone());
                        self.identity.pop();
                        if let Some(value) = result? {
                            return Ok(Some(value));
                        }
                    }
                }
                StatementKind::Match { value, cases } => {
                    let Value::Variant { tag, fields, .. } = self.expression(value, env)? else {
                        return Err(self.error("S301", "expected enum variant", value.span));
                    };
                    let case = cases
                        .iter()
                        .find(|v| v.variant == tag)
                        .ok_or_else(|| self.error("S301", "missing enum case", value.span))?;
                    let mut scope = env.clone();
                    for (name, value) in case.bindings.iter().zip(fields.iter()) {
                        scope.insert(name.clone(), value.clone().into());
                    }
                    if let Some(value) = self.scoped_block(&case.body, &mut scope)? {
                        return Ok(Some(value));
                    }
                }
                _ => {
                    return Err(self.error(
                        "S301",
                        "definition in executable block",
                        statement.span,
                    ));
                }
            }
            let count = self.statistics.interpreted_operations - before;
            *self
                .hotspots
                .entry((
                    self.module.clone(),
                    statement.span.start,
                    statement.span.line,
                    statement.span.column,
                ))
                .or_default() += count;
        }
        Ok(None)
    }
    fn expression_raw(&mut self, expr: &Expr, env: &Env) -> Result<Value, Diagnostic> {
        self.work(expr.span)?;
        if matches!(expr.kind, ExprKind::Name(_)) {
            self.statistics.evaluated_expressions += 1;
            let value = self.expression_inner(expr, env)?;
            return self.contextual_template(value, expr.span);
        }
        let key = if self.limits.max_cache_entries > 0
            && self.limits.max_cache_bytes > 0
            && !self
                .loaded
                .iter()
                .find(|module| module.id == self.module)
                .and_then(|module| {
                    module
                        .checked
                        .result_types
                        .get(&(expr.span.start, expr.span.end))
                })
                .is_some_and(Type::contains_animation)
        {
            let cache_started = Instant::now();
            let plan = self.expression_plan(expr)?;
            // Resolving reactive reads remains execution work; never hide them behind a value hit.
            let key = if let Some(fingerprint) = &plan.fingerprint
                && !plan
                    .names
                    .iter()
                    .any(|name| env.get(name).is_some_and(|binding| binding.is_dynamic() || matches!(binding,Binding::Value(value) if pure::contains_animation(value))))
            {
                let identity = plan.element_identity.then_some(&self.identity);
                let limits = plan.iteration_limit.then_some(self.limits.max_iterations);
                let mut deps = BTreeMap::new();
                for name in &plan.names {
                    if let Some(Binding::Value(value)) = env.get(name) {
                        let value = match value {
                            Value::Font(font) => CacheDependency::Font(self.font_keys.key(font)),
                            _ => CacheDependency::Value(sanim_core::value_key::ValueKey(value)),
                        };
                        deps.insert(name, value);
                    }
                }
                Some(digest(
                    &serde_json::to_vec(&(
                        SEMANTICS_VERSION,
                        self.measure_only,
                        self.template_context(expr.span),
                        &self.type_bindings,
                        limits,
                        self.limits.data,
                        &self.module,
                        identity,
                        self.named_constructor
                            .as_ref()
                            .filter(|_| identity.is_some()),
                        fingerprint,
                        plan.names
                            .iter()
                            .any(|name| {
                                prelude::builtin(name)
                                    .is_some_and(prelude::Builtin::uses_font_environment)
                            })
                            .then(sanim_data::font::system_font_epoch),
                        deps,
                    ))
                    .map_err(|error| self.error("S301", error.to_string(), expr.span))?,
                ))
            } else {
                None
            };
            self.statistics.cache_key_micros = self.statistics.cache_key_micros.saturating_add(
                cache_started
                    .elapsed()
                    .as_micros()
                    .try_into()
                    .unwrap_or(u64::MAX),
            );
            key
        } else {
            None
        };
        if let Some(key) = &key
            && let Some(entry) = self.values.get_entry(key)
        {
            self.restore_element_sources(&entry, expr.span)?;
            self.statistics.reused_expressions += 1;
            return Ok(entry.value.clone());
        }
        let previous_sources = key.as_ref().map(|_| self.element_sources.clone());
        self.statistics.evaluated_expressions += 1;
        self.depth += 1;
        if self.depth > self.limits.max_call_depth * 4 {
            return Err(self.error("S303", "expression evaluation depth exceeded", expr.span));
        }
        let result = self.expression_inner(expr, env);
        self.depth -= 1;
        let value = self.contextual_template(result?, expr.span)?;
        if let Value::Signal(signal) = &value
            && let Some((true, observed)) = self.signal_observations.last_mut()
        {
            observed.insert(
                (self.module.clone(), expr.span.start, expr.span.end),
                signal.clone(),
            );
        }
        let mut bytes = self.limits.data.max_bytes;
        let mut items = self.limits.data.max_values;
        let validation_started = Instant::now();
        if let Some(delta) = self.proven_list_edit_result(expr, &value) {
            crate::lists::charge_delta(delta, &mut bytes, &mut items)
        } else {
            sanim_core::value_budget::charge(
                &value,
                &mut bytes,
                &mut items,
                self.limits.data.max_depth,
            )
        }
        .map_err(|e| self.error("S303", e, expr.span))?;
        self.statistics.value_validation_micros =
            self.statistics.value_validation_micros.saturating_add(
                validation_started
                    .elapsed()
                    .as_micros()
                    .try_into()
                    .unwrap_or(u64::MAX),
            );
        if let Some(key) = key.filter(|_| !value.ty().contains_signal()) {
            let sources = previous_sources
                .expect("cached expression provenance root")
                .diff(&self.element_sources)
                .filter_map(|change| match change {
                    imbl::ordmap::DiffItem::Add(id, source) => Some((id.clone(), source.clone())),
                    _ => None,
                })
                .collect();
            self.values.insert_measured(
                key,
                value.clone(),
                self.limits.data.max_bytes - bytes,
                sources,
            );
        }
        Ok(value)
    }
    fn expression_inner(&mut self, expr: &Expr, env: &Env) -> Result<Value, Diagnostic> {
        let bad = |e| self.error("S305", e, expr.span);
        match &expr.kind {
            ExprKind::Function(declaration) => self.closure(declaration, env, expr.span),
            ExprKind::Number(v) => Number::parse(v).map(Value::Number).map_err(bad),
            ExprKind::String(v) if self.template_expected(expr.span) => {
                Ok(Value::Template(Arc::new(sanim_core::Template {
                    parts: vec![sanim_core::TemplatePart::Text(v.clone().into())].into(),
                })))
            }
            ExprKind::String(v) => Ok(Value::String(v.clone().into())),
            ExprKind::Template(parts) => self.template(expr, parts, env),
            ExprKind::Bool(v) => Ok(Value::Bool(*v)),
            ExprKind::None => Ok(Value::None),
            ExprKind::Reflect { value, .. } => {
                let default = self.expression(value, env)?;
                let declaration = self
                    .loaded
                    .iter()
                    .find(|module| module.id == self.module)
                    .and_then(|module| module.checked.reflected.get(&expr.span.start))
                    .cloned()
                    .ok_or_else(|| {
                        self.error(
                            "S301",
                            "missing reflected expression declaration",
                            expr.span,
                        )
                    })?;
                let default = default
                    .with_type(&declaration.ty)
                    .map_err(|e| self.error("S140", e, value.span))?;
                let key = &declaration.key;
                let supplied = self.overrides.get(key);
                let current = supplied.unwrap_or(&default).clone();
                if supplied.is_some() {
                    self.used_overrides.insert(key.clone());
                }
                self.input_versions
                    .insert(key.clone(), (supplied.cloned(), self.root_statement));
                // Inline defaults/current values live in reflection metadata even when the
                // enclosing expression only retains a small projection of them.
                for retained in std::iter::once(&default).chain(supplied) {
                    sanim_core::value_budget::charge(
                        retained,
                        &mut self.retained_bytes,
                        &mut self.retained_items,
                        self.limits.data.max_depth,
                    )
                    .map_err(|message| {
                        self.error(
                            "S303",
                            format!("reflected input '{key}' storage budget: {message}"),
                            expr.span,
                        )
                    })?;
                }
                self.reflected.push_back(ReflectedInput {
                    declaration,
                    default,
                    value: current.clone(),
                });
                Ok(current)
            }
            ExprKind::Name(name) if prelude::values().contains_key(name.as_str()) => {
                Ok(prelude::values()[name.as_str()].clone())
            }
            ExprKind::Name(name) => {
                let binding = env.get(name).cloned().ok_or_else(|| {
                    self.error(
                        "S120",
                        format!("name '{name}' is not available in this preparation scope"),
                        expr.span,
                    )
                })?;
                self.binding_value(&binding, expr.span)
            }
            ExprKind::List(items) => Ok(Value::List(
                items
                    .iter()
                    .map(|v| self.expression(v, env))
                    .collect::<Result<_, _>>()?,
            )),
            ExprKind::Record(fields) => Ok(Value::Record(
                fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.expression(v, env)?)))
                    .collect::<Result<_, Diagnostic>>()?,
            )),
            ExprKind::Dict(entries) => {
                let mut out = BTreeMap::new();
                for (key, value) in entries {
                    let key_value = self.expression(key, env)?;
                    let name = key_value
                        .string()
                        .map_err(|e| self.error("S305", e, key.span))?;
                    let value = self.expression(value, env)?;
                    if out
                        .insert(
                            name.to_string(),
                            sanim_data::to_json(&value)
                                .map_err(|e| self.error("S305", e, expr.span))?,
                        )
                        .is_some()
                    {
                        return Err(self.error(
                            "S305",
                            format!("duplicate dictionary key {name:?}"),
                            key.span,
                        ));
                    }
                }
                Ok(Value::Dict(out.into()))
            }
            ExprKind::Unary(op, value) => {
                let value = self.expression(value, env)?;
                math::operations::unary(op, value).map_err(|e| self.error("S305", e, expr.span))
            }
            ExprKind::Binary(op, a, b) => {
                let left = self.expression(a, env)?;
                if op == "and" && matches!(left, Value::Bool(false)) {
                    return Ok(Value::Bool(false));
                }
                if op == "or" && matches!(left, Value::Bool(true)) {
                    return Ok(Value::Bool(true));
                }
                let right = self.expression(b, env)?;
                // Preflight concatenation before it can double a large allocation.
                if op == "+" && matches!(left, Value::String(_) | Value::List(_)) {
                    let mut bytes = self.limits.data.max_bytes;
                    let mut items = self.limits.data.max_values;
                    sanim_core::value_budget::charge(
                        &left,
                        &mut bytes,
                        &mut items,
                        self.limits.data.max_depth,
                    )
                    .and_then(|()| {
                        sanim_core::value_budget::charge(
                            &right,
                            &mut bytes,
                            &mut items,
                            self.limits.data.max_depth,
                        )
                    })
                    .map_err(|e| self.error("S303", e, expr.span))?;
                }
                self.charge_work(
                    math::operations::binary_work(op, &left, &right) as u64,
                    expr.span,
                )?;
                math::operations::binary(op, left, right)
                    .map_err(|e| self.error("S305", e, expr.span))
            }
            ExprKind::Member(base, name) => {
                if let ExprKind::Name(alias) = &base.kind
                    && let Some(Binding::Module(module)) = env.get(alias)
                {
                    let binding = self
                        .globals
                        .get(module)
                        .and_then(|scope| scope.get(name))
                        .cloned()
                        .ok_or_else(|| {
                            self.error(
                                "S120",
                                format!("module '{alias}' has no export '{name}'"),
                                expr.span,
                            )
                        })?;
                    return self.binding_value(&binding, expr.span);
                }
                let value = self.expression(base, env)?;
                match value {
                    Value::DataFrame(frame) if name == "columns" => {
                        Ok(Value::List(sanim_core::List::typed(
                            Type::String,
                            frame
                                .schema
                                .keys()
                                .map(|name| Value::String(name.clone().into()))
                                .collect(),
                        )))
                    }
                    Value::Record(fields) => fields.get(name).cloned().ok_or_else(|| {
                        self.error("S305", format!("missing field '{name}'"), expr.span)
                    }),
                    Value::Fraction(value) => {
                        let magnitude = if name == "numerator" {
                            *value
                                .numer()
                                .ok_or_else(|| self.error("S305", "invalid fraction", expr.span))?
                        } else {
                            *value
                                .denom()
                                .ok_or_else(|| self.error("S305", "invalid fraction", expr.span))?
                        };
                        let mut n = Fraction::from(magnitude);
                        if name == "numerator" && value < Fraction::from(0) {
                            n = -n;
                        }
                        Ok(Value::Number(Number {
                            value: n,
                            unit: Unit::Number,
                        }))
                    }
                    Value::Element(element) => {
                        if name == "id" {
                            return Ok(Value::String(element.id.clone().into()));
                        }
                        let property =
                            reactive_animation::property_name(name).ok_or_else(|| {
                                self.error(
                                    "S305",
                                    format!("unknown element property '{name}'"),
                                    expr.span,
                                )
                            })?;
                        if let Some(node) = self.animation.reactive.binding(&element.id, property) {
                            let cells = self
                                .animation
                                .reactive
                                .cell_dependencies(node)
                                .map_err(|error| self.error("S140", error, expr.span))?;
                            for cell in &cells {
                                self.refresh_cell(*cell, expr.span)?;
                            }
                            if let Some(frame) = self.signal_frames.last_mut() {
                                frame.dependencies.extend(cells);
                            }
                        }
                        if let Some(frame) = self.signal_frames.last_mut() {
                            frame.temporal = true;
                        }
                        self.animation
                            .sample_property(&element, property, self.cursor)
                            .map_err(|error| self.error("S305", error, expr.span))
                    }
                    Value::Array { shape, .. } if name == "shape" => Ok(Value::List(
                        shape
                            .into_iter()
                            .map(|n| {
                                Value::Number(Number {
                                    value: Fraction::from(n as u64),
                                    unit: Unit::Number,
                                })
                            })
                            .collect(),
                    )),
                    _ => Err(self.error("S305", format!("value has no field '{name}'"), expr.span)),
                }
            }
            ExprKind::Index(base, index) => {
                let value = self.expression(base, env)?;
                if let Value::DataFrame(frame) = &value {
                    let index = self.expression(index, env)?;
                    return self.frame_index(frame, index, expr.span);
                }
                if matches!(value, Value::Dict(_) | Value::Data(_)) {
                    let key = self.expression(index, env)?;
                    let result = value.lookup(&key).map_err(|e| {
                        self.error("S308", format!("{}: {e}", access_path(expr)), index.span)
                    })?;
                    return if let Value::Some(value) = result {
                        Ok(value.as_ref().clone())
                    } else {
                        let available = match &value {
                            Value::Dict(entries) => format!(
                                "available keys: {}",
                                entries
                                    .keys()
                                    .take(12)
                                    .map(|k| format!("{k:?}"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            Value::Data(data) if data.is_object() => format!(
                                "available keys: {}",
                                data.as_object()
                                    .expect("object")
                                    .keys()
                                    .take(12)
                                    .map(|k| format!("{k:?}"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            Value::Data(data) if data.is_array() => {
                                format!("array has {} items", data.as_array().expect("array").len())
                            }
                            _ => String::new(),
                        };
                        Err(self.error("S308",format!("{}: missing key or index {}; {available}. Use get(value, key) for an optional lookup.",access_path(expr),sanim_data::to_json(&key).map_err(|e|self.error("S305",e,index.span))?),index.span))
                    };
                }
                let direct = crate::list_functions::constant_count(Some(index), None)
                    .and_then(|index| {
                        self.loaded
                            .iter()
                            .find(|module| module.id == self.module)
                            .and_then(|module| {
                                module
                                    .checked
                                    .result_types
                                    .get(&(base.span.start, base.span.end))
                            })
                            .and_then(|ty| ty.fixed_index(index).ok().flatten())
                    })
                    .is_some();
                let index = self
                    .expression(index, env)?
                    .number()
                    .and_then(Number::usize)
                    .map_err(|e| self.error("S305", e, expr.span))?;
                let item = value
                    .list()
                    .map_err(|error| self.error("S305", error, expr.span))?
                    .get(index)
                    .cloned();
                if direct {
                    item.ok_or_else(|| {
                        self.error(
                            "S305",
                            "fixed List cardinality changed after type checking",
                            expr.span,
                        )
                    })
                } else {
                    Ok(item
                        .map(|value| Value::Some(Arc::new(value)))
                        .unwrap_or(Value::None))
                }
            }
            ExprKind::Call { callee, args, .. } => {
                if let Some(value) = self.measure_animation_call(callee, args, env, expr.span)? {
                    return Ok(value);
                }
                if let Some(value) = self.staged_code_range(expr, env)? {
                    return Ok(value);
                }
                let mut receiver = None;
                let native = match &callee.kind {
                    ExprKind::Name(name) if self.builtin(name).is_some() => Some(name.clone()),
                    ExprKind::Member(base, _)
                        if self.checked_native_method(callee.span).is_some() =>
                    {
                        let name = self
                            .checked_native_method(callee.span)
                            .expect("checked method")
                            .name
                            .clone();
                        receiver = Some(self.expression(base, env)?);
                        Some(name)
                    }
                    ExprKind::Member(base, name)
                        if prelude::builtin(name).is_some_and(|v| v.result == Type::Predicate) =>
                    {
                        let value = self.expression(base, env)?;
                        if matches!(value, Value::Element(_)) {
                            receiver = Some(value);
                            Some(name.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(name) = native {
                    if let Some(builtin) = prelude::builtin(&name).filter(|builtin| {
                        matches!(builtin.execute, prelude::Execution::CellAnimation)
                    }) {
                        return self.cell_animation(builtin, args, env, expr.span);
                    }
                    let mut evaluated = vec![];
                    if let Some(receiver) = receiver {
                        evaluated.push((None, receiver));
                    }
                    let retaining = prelude::builtin(&name)
                        .is_some_and(|builtin| builtin.result == Type::Element);
                    let mut observed = Vec::with_capacity(if retaining { args.len() } else { 0 });
                    for (label, argument) in args {
                        let value = if retaining {
                            self.signal_observations
                                .push((true, ObservedSignals::new()));
                            let result = self.expression(argument, env);
                            observed.push(
                                self.signal_observations
                                    .pop()
                                    .expect("active argument observations")
                                    .1,
                            );
                            result?
                        } else {
                            self.expression(argument, env)?
                        };
                        evaluated.push((label.clone(), value));
                    }
                    self.infer_constructor_id(&name, args, &mut evaluated, expr.span);
                    let value = self.native(&name, evaluated, expr.span)?;
                    if retaining {
                        self.retain_constructor_signals(
                            &name, &value, args, &observed, env, expr.span,
                        )?;
                    }
                    return Ok(value);
                }
                let Value::Function(key) = self.expression(callee, env)? else {
                    return Err(self.error("S305", "expected callable function", expr.span));
                };

                let first_signal = self.reactive.next;
                let mut evaluated = vec![];
                for (index, (label, value)) in args.iter().enumerate() {
                    let parameter = self
                        .function_group(&key)
                        .and_then(|group| group.clauses.first())
                        .and_then(|function| {
                            if let Some(label) = label {
                                function
                                    .parameters
                                    .iter()
                                    .find(|parameter| &parameter.name == label)
                            } else {
                                function.parameters.get(index)
                            }
                        })
                        .cloned();
                    let value = if let Some(parameter) =
                        parameter.filter(|parameter| parameter.kind == BindingKind::Sig)
                    {
                        let ty = self.parameter_type(&key, &parameter.name, expr.span)?;
                        self.capture_signal(value, env, &ty)?
                    } else {
                        self.expression(value, env)?
                    };
                    evaluated.push((label.clone(), value));
                }
                if let Some(ty) = self.constructors.get(&key).cloned() {
                    let value = match ty {
                        Type::Record(fields) => {
                            let ordered = order_values(
                                &evaluated,
                                &fields.keys().cloned().collect::<Vec<_>>(),
                            );
                            let mut out = BTreeMap::new();
                            for (name, ty) in fields {
                                out.insert(
                                    name.clone(),
                                    ordered
                                        .get(&name)
                                        .cloned()
                                        .ok_or_else(|| {
                                            self.error(
                                                "S305",
                                                format!("missing field '{name}'"),
                                                expr.span,
                                            )
                                        })?
                                        .with_type(&ty)
                                        .map_err(|e| self.error("S140", e, expr.span))?,
                                );
                            }
                            Ok(Value::Record(out.into()))
                        }
                        Type::Enum { .. } => {
                            let tag = key
                                .rsplit('.')
                                .next()
                                .ok_or_else(|| {
                                    self.error("S305", "invalid variant constructor", expr.span)
                                })?
                                .to_string();
                            Ok(Value::Variant {
                                ty: ty.clone().into(),
                                tag: tag.clone(),
                                fields: {
                                    let Type::Enum { variants, .. } = &ty else {
                                        unreachable!()
                                    };
                                    let names = (0..variants[&tag].len())
                                        .map(|i| format!("value{i}"))
                                        .collect::<Vec<_>>();
                                    let mut ordered = order_values(&evaluated, &names);
                                    names
                                        .iter()
                                        .zip(&variants[&tag])
                                        .map(|(name, ty)| {
                                            ordered
                                                .remove(name)
                                                .expect("checked enum argument")
                                                .with_type(ty)
                                                .map_err(|e| self.error("S140", e, expr.span))
                                        })
                                        .collect::<Result<Vec<_>, _>>()?
                                        .into()
                                },
                            })
                        }
                        _ => Err(self.error("S305", "type is not constructible", expr.span)),
                    }?;
                    return Ok(value);
                }
                let result = self.invoke(&key, &evaluated, expr.span);
                self.reactive.remove_since(first_signal);
                result
            }
        }
    }
    fn invoke(
        &mut self,
        key: &str,
        evaluated: &[(Option<String>, Value)],
        span: Span,
    ) -> Result<Value, Diagnostic> {
        if self.active_calls.iter().any(|active| active == key) {
            return Err(self.error(
                "S111",
                format!("recursive call to '{key}' is forbidden; use bounded for/repeat"),
                span,
            ));
        }
        if self.depth >= self.limits.max_call_depth {
            return Err(self.error("S303", "bounded function call depth exceeded", span));
        }
        self.depth += 1;
        let group = self
            .function_group(key)
            .cloned()
            .ok_or_else(|| self.error("S305", format!("unknown function '{key}'"), span))?;
        let first = group
            .clauses
            .first()
            .ok_or_else(|| self.error("S140", "function has no checked clauses", span))?;
        let ordered = order_values(
            evaluated,
            &first
                .parameters
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect::<Vec<_>>(),
        );
        let bindings = self.call_type_bindings(span, &group.types);
        let function = self.select_function(&group, &ordered, &bindings, span)?;
        self.record_function(key, &group);
        let call_site = self.instance_site(span);
        let previous_types = std::mem::replace(&mut self.type_bindings, bindings);
        let caller = self.source_location(span);
        let previous = std::mem::replace(&mut self.module, function.module.clone());
        let mut scope = group.captures.clone().unwrap_or_else(|| {
            self.globals
                .get(&function.module)
                .cloned()
                .unwrap_or_default()
        });
        self.identity
            .push(format!("{}@{}", function.name, call_site));
        self.source_calls.push_back(caller);
        for parameter in &function.parameters {
            let name = &parameter.name;
            let default = &parameter.default;
            let value = if let Some(value) = ordered.get(name) {
                value.clone()
            } else if let Some(default) = default {
                if parameter.kind == BindingKind::Sig {
                    let ty = self
                        .function_signature(&function)
                        .and_then(|signature| {
                            signature
                                .parameters
                                .iter()
                                .find(|parameter| parameter.name == *name)
                        })
                        .map(|parameter| parameter.ty.substitute(&self.type_bindings))
                        .ok_or_else(|| self.error("S140", "missing checked sig parameter", span))?;
                    self.capture_signal(default, &scope, &ty)?
                } else {
                    self.expression(default, &scope)?
                }
            } else {
                return Err(self.error("S305", format!("missing argument '{name}'"), span));
            };
            let parameter_type = self
                .function_signature(&function)
                .and_then(|signature| {
                    signature
                        .parameters
                        .iter()
                        .find(|parameter| parameter.name == *name)
                })
                .map(|parameter| parameter.ty.substitute(&self.type_bindings));
            if parameter.kind == BindingKind::Sig {
                let Value::Signal(signal) = value else {
                    return Err(self.error(
                        "S140",
                        "sig parameter requires a captured signal",
                        span,
                    ));
                };
                scope.insert(name.clone(), Binding::Reactive(signal.id));
                continue;
            }
            let value = self.sample_signal(value, span)?;
            let value = if let Some(ty) = &parameter_type {
                self.typed_value(&value, ty, span)?
            } else {
                value
            };
            if parameter.kind == BindingKind::Mut {
                let ty = parameter_type.unwrap_or_else(|| value.ty());
                let id = self.insert_cell(name, &ty, value, span)?;
                scope.insert(name.clone(), Binding::Reactive(id));
            } else {
                scope.insert(name.clone(), value.into());
            }
        }
        let live_return = self.function_signature(&function).and_then(|signature| {
            if let Type::Signal(inner) = signature.result.as_ref() {
                Some(inner.substitute(&self.type_bindings))
            } else {
                None
            }
        });
        let previous_signal_return = std::mem::replace(&mut self.signal_return, live_return);
        let memo_scope = self.pure_result_type(&function).then(|| scope.clone());
        let memo_marker = format!("pure-eligible:{}:{}", group.fingerprint, function.clause);
        let memo_key = if memo_scope.is_some() && self.values.get(&memo_marker).is_some() {
            self.pure_call_key(&function, &group, &scope)?
        } else {
            None
        };
        let memo_hit = memo_key.as_ref().and_then(|key| self.values.get(key));
        let body_operations = self.statistics.operations;
        self.active_calls.push(key.to_owned());
        let result = if let Some(value) = memo_hit {
            self.statistics.reused_expressions += 1;
            Ok(Some(value))
        } else {
            self.scoped_block(&function.body, &mut scope)
        };
        self.active_calls.pop();
        let result = result.and_then(|value| {
            let ty = self
                .function_signature(&function)
                .map(|signature| signature.result.substitute(&self.type_bindings));
            value
                .map(|value| {
                    if let Some(ty) = &ty {
                        self.typed_value(&value, ty, span)
                    } else {
                        Ok(value)
                    }
                })
                .transpose()
        });
        let mut memo_result_nodes = pure::MAX_MEMO_INPUT_NODES;
        if let (Some(entry_scope), Ok(Some(value))) = (memo_scope.as_ref(), result.as_ref())
            && self.statistics.operations.saturating_sub(body_operations)
                >= pure::MIN_MEMO_OPERATIONS
            && pure::supported(value, &mut memo_result_nodes, self.limits.max_cache_bytes)
        {
            let key = if let Some(key) = memo_key {
                Some(key)
            } else {
                self.pure_call_key(&function, &group, entry_scope)?
            };
            if let Some(key) = key {
                self.values.insert(memo_marker, Value::Unit);
                self.values.insert(key, value.clone());
            }
        }
        self.depth -= 1;
        self.signal_return = previous_signal_return;
        self.identity.pop();
        self.source_calls.pop_back();
        self.module = previous;
        self.type_bindings = previous_types;
        let result = result?;
        Ok(result.unwrap_or(Value::Unit))
    }
    fn check_elements(&self, span: Span) -> Result<(), Diagnostic> {
        let count = self.animation.element_count();
        if count > self.limits.max_elements {
            Err(self.error(
                "S303",
                format!(
                    "scene contains {} elements; limit {}",
                    count, self.limits.max_elements
                ),
                span,
            ))
        } else {
            Ok(())
        }
    }
}

fn order_values(args: &[(Option<String>, Value)], parameters: &[String]) -> Values {
    let mut out = Values::new();
    let mut positional = 0;
    for (name, value) in args {
        if let Some(name) = name {
            out.insert(name.clone(), value.clone());
        } else if let Some(name) = parameters.get(positional) {
            out.insert(name.clone(), value.clone());
            positional += 1;
        }
    }
    out
}

fn cacheable(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Call { callee, args, .. } => {
            matches!(&callee.kind,ExprKind::Name(name)if prelude::builtin(name).is_some_and(|v|v.capabilities().is_pure()&&v.result!=Type::Animation&&matches!(v.execute,prelude::Execution::Native(_))))
                && args.iter().all(|(_, v)| cacheable(v))
        }
        // Recording metadata and override dependencies is part of evaluating an input.
        // Cache its pure default, but never skip this boundary or an enclosing expression.
        ExprKind::Function(_)
        | ExprKind::Reflect { .. }
        | ExprKind::Member(_, _)
        | ExprKind::Index(_, _) => false,
        ExprKind::Unary(_, a) => cacheable(a),
        ExprKind::Binary(_, a, b) => cacheable(a) && cacheable(b),
        ExprKind::Template(parts) => parts.iter().all(|part| match part {
            sanim_core::TemplatePart::Text(_) => true,
            sanim_core::TemplatePart::Hole { value, format, .. } => {
                cacheable(value) && format.as_ref().is_none_or(cacheable)
            }
        }),
        ExprKind::List(values) => values.iter().all(cacheable),
        ExprKind::Record(values) => values.iter().all(|(_, v)| cacheable(v)),
        ExprKind::Dict(values) => values.iter().all(|(k, v)| cacheable(k) && cacheable(v)),
        _ => true,
    }
}
fn access_path(expr: &Expr) -> String {
    match &expr.kind {
        ExprKind::Name(name) => name.clone(),
        ExprKind::Index(base, key) => format!(
            "{}[{}]",
            access_path(base),
            match &key.kind {
                ExprKind::String(s) => format!("{s:?}"),
                ExprKind::Number(n) => n.clone(),
                _ => "key".into(),
            }
        ),
        _ => "data".into(),
    }
}
