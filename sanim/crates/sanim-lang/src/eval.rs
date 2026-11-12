mod duration;
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
type Env = imbl::OrdMap<String, Value>;

#[derive(serde::Serialize)]
enum CacheDependency<'a> {
    Value(&'a Value),
    Font(String),
}

#[derive(Clone)]
struct Function {
    module: String,
    name: String,
    parameters: Vec<(String, TypeExpr, Option<Expr>)>,
    body: Vec<Statement>,
    fingerprint: String,
    span: Span,
}

pub(crate) fn evaluate(
    loaded: &[Loaded],
    snapshot: &mut Snapshot<'_>,
    overrides: &BTreeMap<String, Value>,
    (limits, viewport, duration_only, progress): (&Limits, &Viewport, bool, &Reporter),
    (values, assets, states): (
        &mut crate::cache::ValueCache,
        &mut crate::cache::ValueCache,
        &mut state::Store,
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
    let mut runtime = Runtime {
        loaded,
        snapshot,
        overrides,
        limits,
        values,
        assets,
        font_keys: crate::cache::FontCache::new(limits.max_cache_bytes, limits.max_cache_entries),
        statistics,
        progress,
        completed_statements: 0,
        total_statements,
        globals: imbl::OrdMap::new(),
        functions: BTreeMap::new(),
        constructors: BTreeMap::new(),
        reflected: imbl::Vector::new(),
        used_overrides: imbl::OrdSet::new(),
        animation: Animation::for_viewport(viewport),
        cursor: Fraction::from(0),
        assertions: imbl::Vector::new(),
        checks: imbl::Vector::new(),
        module: String::new(),
        identity: vec![],
        depth: 0,
        root_active: false,
        duration_only,
        result_needed: false,
        duration_needed: duration::duration_needs(loaded),
        type_bindings: BTreeMap::new(),
        hotspots: imbl::OrdMap::new(),
        performance: Vec::new(),
        retained_bytes: limits.data.max_bytes,
        retained_items: limits.data.max_values,
        logical_operations: 0,
        used_functions: imbl::OrdMap::new(),
        used_resources: imbl::OrdMap::new(),
        input_versions: imbl::OrdMap::new(),
        time_map: TimeMap::default(),
        root_statement: 0,
    };
    if duration_only {
        for module in loaded {
            for (name, ty) in &module.checked.reflected {
                let key = if loaded.last().is_some_and(|last| last.id == module.id) {
                    name.clone()
                } else {
                    format!("{}::{name}", module.id)
                };
                if let Some(value) = overrides.get(&key) {
                    validate_value(ty, value).map_err(|e| {
                        runtime.error(
                            "S302",
                            format!("reflected input '{key}': {e}"),
                            Span::default(),
                        )
                    })?;
                    runtime.used_overrides.insert(key);
                }
            }
        }
    }
    for module in loaded {
        runtime.module = module.id.clone();
        runtime.root_active = loaded.last().is_some_and(|v| v.id == module.id);
        runtime.report_evaluation(None);
        let mut env = Env::new();
        for (alias, id) in &module.imports {
            env.insert(
                alias.clone(),
                Value::Record(runtime.globals.get(id).cloned().ok_or_else(|| {
                    runtime.error("S301", "imported module has no values", Span::default())
                })?),
            );
        }
        for (name, ty) in &module.checked.schemas {
            if name.contains('.') {
                continue;
            }
            match ty {
                Type::Record(_) => {
                    let key = format!("{}::{name}", module.id);
                    runtime.constructors.insert(key.clone(), ty.clone());
                    env.insert(name.clone(), Value::Function(key));
                }
                Type::Enum { variants, .. } => {
                    let mut fields = Env::new();
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
                            runtime.constructors.insert(key.clone(), ty.clone());
                            fields.insert(tag.clone(), Value::Function(key));
                        }
                    }
                    env.insert(name.clone(), Value::Record(fields));
                }
                _ => {}
            }
        }
        for statement in &module.ast.statements {
            if let StatementKind::Function {
                name,
                parameters,
                body,
                ..
            } = &statement.kind
            {
                let key = format!("{}::{name}", module.id);
                runtime.functions.insert(
                    key.clone(),
                    Arc::new(Function {
                        module: module.id.clone(),
                        name: name.clone(),
                        parameters: parameters.clone(),
                        body: body.clone(),
                        span: statement.span,
                        fingerprint: digest(
                            &serde_json::to_vec(statement).expect("serializable function"),
                        ),
                    }),
                );
                env.insert(name.clone(), Value::Function(key));
            }
        }
        runtime.globals.insert(module.id.clone(), env.clone());
        let restored = if runtime.root_active && !duration_only {
            let restored = states.begin(&mut runtime, &mut env, module);
            if restored == 0 {
                states.save(&runtime, &env, 0);
            }
            restored
        } else {
            0
        };
        for (index, statement) in module
            .ast
            .statements
            .iter()
            .filter(|statement| state::executable(statement))
            .enumerate()
        {
            if index < restored {
                continue;
            }
            runtime.root_statement = index;
            runtime.report_evaluation(Some(statement.span));
            let start = runtime.cursor;
            runtime.block(std::slice::from_ref(statement), &mut env)?;
            runtime.completed_statements += 1;
            runtime.report_evaluation(Some(statement.span));
            runtime.globals.insert(module.id.clone(), env.clone());
            if runtime.root_active && !duration_only {
                runtime.map_time(start, statement.span, TimeMapKind::Statement)?;
                states.save(&runtime, &env, index + 1);
            }
        }
    }
    for key in overrides.keys() {
        if !runtime.used_overrides.contains(key) {
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
    let checks = runtime.assertions.len() as u64;
    runtime
        .progress
        .emit(Phase::Verifying, Some(0), Some(checks), None);
    for (index, assertion) in runtime.assertions.clone().into_iter().enumerate() {
        let verification =
            verify::evaluate(&runtime.animation, &assertion.predicate, assertion.time);
        if assertion.required && verification.evidence != verify::Evidence::Proved {
            return Err(Diagnostic::new(
                "S350",
                format!(
                    "visual assertion {:?}: {}{}",
                    verification.evidence,
                    verification.message,
                    assertion
                        .message
                        .map(|v| format!(". {v}"))
                        .unwrap_or_default()
                ),
                &assertion.module,
                assertion.span,
            ));
        }
        runtime.checks.push_back(verification);
        runtime.progress.emit(
            Phase::Verifying,
            Some(index as u64 + 1),
            Some(checks),
            Some(&assertion.module),
        );
    }
    if !duration_only {
        runtime
            .time_map
            .index(
                runtime.animation.rate,
                runtime.animation.duration,
                limits.max_time_map_bytes,
            )
            .map_err(|message| runtime.error("S303", message, Span::default()))?;
        runtime.statistics.checkpoint_count = states.len();
        runtime.statistics.checkpoint_bytes = states.bytes;
    }
    let root = loaded
        .last()
        .ok_or_else(|| runtime.error("S301", "no root module", Span::default()))?;
    let bindings = runtime.globals.remove(&root.id).unwrap_or_default();
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
            duration_only,
        ))
        .map_err(|e| runtime.error("S301", e.to_string(), Span::default()))?,
    );
    let mut performance:Vec<_>=runtime.hotspots.into_iter().filter(|(_,count)|*count>=limits.warn_operations).map(|((module,start,line,column),operations)|PerformanceWarning{
        code:"P001".into(),message:format!("this statement performed {operations} preparation operations (warning budget {}). Hoist invariant work out of loops, reduce collection sizes, or split independent elements.",limits.warn_operations),source:module,span:Span{start,end:start,line,column},operations,
    }).collect();
    performance.extend(runtime.performance);
    Ok(Prepared {
        animation: runtime.animation,
        time_map: runtime.time_map,
        bindings,
        reflected: runtime.reflected,
        types,
        symbols: root.checked.exports.clone(),
        scheduling_helpers: root.checked.effects.iter().cloned().collect(),
        durations: loaded
            .iter()
            .map(|module| (module.id.clone(), module.checked.durations.clone()))
            .collect(),
        checks: runtime.checks,
        contracts: runtime.assertions,
        dependencies: BTreeMap::new(),
        fingerprint,
        statistics: Statistics::default(),
        performance,
    })
}

pub(crate) struct Runtime<'a, 's> {
    loaded: &'a [Loaded],
    snapshot: &'a mut Snapshot<'s>,
    overrides: &'a BTreeMap<String, Value>,
    limits: &'a Limits,
    values: &'a mut crate::cache::ValueCache,
    assets: &'a mut crate::cache::ValueCache,
    statistics: &'a mut Statistics,
    progress: &'a Reporter,
    completed_statements: u64,
    total_statements: u64,
    font_keys: crate::cache::FontCache,
    globals: imbl::OrdMap<String, Env>,
    functions: BTreeMap<String, Arc<Function>>,
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
    duration_only: bool,
    result_needed: bool,
    duration_needed: BTreeMap<String, BTreeSet<String>>,
    type_bindings: BTreeMap<String, Type>,
    hotspots: imbl::OrdMap<(String, usize, usize, usize), u64>,
    performance: Vec<PerformanceWarning>,
    retained_bytes: usize,
    retained_items: usize,
    logical_operations: u64,
    used_functions: imbl::OrdMap<String, (String, usize)>,
    used_resources: imbl::OrdMap<String, (String, usize)>,
    input_versions: imbl::OrdMap<String, (Option<Value>, usize)>,
    time_map: TimeMap,
    root_statement: usize,
}

impl Runtime<'_, '_> {
    fn report_evaluation(&self, span: Option<Span>) {
        if !self.progress.enabled() {
            return;
        }
        let resource = span.map_or_else(
            || self.module.clone(),
            |span| format!("{}:{}", self.module, span.line),
        );
        self.progress.emit(
            if self.duration_only {
                Phase::Duration
            } else {
                Phase::Evaluating
            },
            Some(self.completed_statements),
            Some(self.total_statements),
            Some(&resource),
        );
    }
    fn map_time(
        &mut self,
        start: Fraction,
        span: Span,
        kind: TimeMapKind,
    ) -> Result<(), Diagnostic> {
        if self.duration_only || !self.root_active {
            return Ok(());
        }
        self.time_map
            .push(
                TimeMapEntry {
                    source: self.module.clone(),
                    span,
                    start,
                    end: self.cursor,
                    statement: self.root_statement,
                    kind,
                    call_path: self.identity.iter().cloned().collect(),
                },
                self.limits.max_time_map_bytes,
            )
            .map_err(|message| self.error("S303", message, span))
    }
    fn error(&self, code: &str, message: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic::new(code, message, &self.module, span)
    }
    fn work(&mut self, span: Span) -> Result<(), Diagnostic> {
        self.statistics.operations += 1;
        self.logical_operations += 1;
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
    fn block(&mut self, body: &[Statement], env: &mut Env) -> Result<Option<Value>, Diagnostic> {
        for statement in body {
            if self.duration_only {
                let binding_needed = match &statement.kind {
                    StatementKind::Bind { name, .. } => {
                        self.result_needed || self.duration_binding_needed(name)
                    }
                    _ => false,
                };
                if !binding_needed
                    && !matches!(
                        statement.kind,
                        StatementKind::Repeat { .. } | StatementKind::For { .. }
                    )
                    && !crate::effects::has_return(std::slice::from_ref(statement))
                    && let TimelineEffect::Fixed(duration) = self.timeline_effect(statement.span)
                {
                    self.advance_duration(duration, statement.span)?;
                    continue;
                }
                match &statement.kind {
                    StatementKind::Bind { name, .. }
                        if !self.result_needed && !self.duration_binding_needed(name) =>
                    {
                        if let StatementKind::Bind { value, .. } = &statement.kind {
                            self.duration_effects(value, env)?;
                            continue;
                        }
                    }
                    StatementKind::Expr(value) => {
                        self.duration_effects(value, env)?;
                        continue;
                    }
                    StatementKind::Return(value) if !self.result_needed => {
                        self.duration_effects(value, env)?;
                        return Ok(Some(Value::Unit));
                    }
                    _ => {}
                }
            }

            self.work(statement.span)?;
            let before = self.statistics.operations;
            match &statement.kind {
                StatementKind::Bind {
                    name,
                    value,
                    reflect,
                    ..
                } => {
                    let default = self.expression(value, env)?;
                    let mut value = default.clone();
                    if *reflect {
                        let root = self.loaded.last().is_some_and(|v| v.id == self.module);
                        let key = if root {
                            name.clone()
                        } else {
                            format!("{}::{name}", self.module)
                        };
                        let ty = self
                            .loaded
                            .iter()
                            .find(|v| v.id == self.module)
                            .and_then(|v| v.checked.reflected.get(name))
                            .cloned()
                            .ok_or_else(|| {
                                self.error("S301", "missing reflected type", statement.span)
                            })?;
                        if let Some(supplied) = self.overrides.get(&key) {
                            validate_value(&ty, supplied).map_err(|e| {
                                self.error(
                                    "S302",
                                    format!("reflected input '{key}': {e}"),
                                    statement.span,
                                )
                            })?;
                            value = supplied.clone();
                            self.used_overrides.insert(key.clone());
                        }
                        self.input_versions.insert(
                            key.clone(),
                            (self.overrides.get(&key).cloned(), self.root_statement),
                        );
                        self.reflected.push_back(ReflectedInput {
                            key,
                            name: name.clone(),
                            ty: ty.into(),
                            default,
                            value: value.clone(),
                        });
                    }
                    sanim_core::value_budget::charge(
                        &value,
                        &mut self.retained_bytes,
                        &mut self.retained_items,
                        self.limits.data.max_depth,
                    )
                    .map_err(|e| {
                        self.error(
                            "S303",
                            format!("preparation binding budget: {e}"),
                            statement.span,
                        )
                    })?;
                    env.insert(name.clone(), value);
                }
                StatementKind::Expr(expr) => {
                    self.expression(expr, env)?;
                }
                StatementKind::Return(expr) => return self.expression(expr, env).map(Some),
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
                    if let Some(value) = self.block(selected, &mut env.clone())? {
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
                    if self.duration_only
                        && let Some(duration) = self.fixed_block_duration(body)
                    {
                        let total = math::arithmetic("*", duration, Fraction::from(values.len()))
                            .map_err(|e| self.error("S304", e, statement.span))?;
                        self.advance_duration(total, statement.span)?;
                        continue;
                    }
                    for (index, value) in values.iter().enumerate() {
                        let mut scope = env.clone();
                        scope.insert(name.clone(), value.clone());
                        self.identity
                            .push(format!("for{}:{index}", statement.span.start));
                        let result = self.block(body, &mut scope);
                        self.identity.pop();
                        if let Some(value) = result? {
                            return Ok(Some(value));
                        }
                    }
                }
                StatementKind::Repeat {
                    count,
                    period,
                    body,
                } => {
                    let count = self
                        .expression(count, env)?
                        .number()
                        .and_then(Number::usize)
                        .map_err(|e| {
                            self.error("S303", format!("repeat/timer count: {e}"), count.span)
                        })?;
                    if count > self.limits.max_iterations {
                        return Err(self.error(
                            "S303",
                            format!(
                                "repeat/timer count {count} exceeds iteration limit {}",
                                self.limits.max_iterations
                            ),
                            statement.span,
                        ));
                    }
                    let period = period
                        .as_ref()
                        .map(|p| {
                            self.expression(p, env).and_then(|v| {
                                v.number()
                                    .map(|v| v.value)
                                    .map_err(|e| self.error("S303", e, p.span))
                            })
                        })
                        .transpose()?;
                    if period.is_some_and(|v| v <= Fraction::from(0)) {
                        return Err(self.error(
                            "S303",
                            "timer period must be positive",
                            statement.span,
                        ));
                    }
                    if self.duration_only
                        && let Some(body_duration) = self.fixed_block_duration(body)
                    {
                        if let Some(period) = period
                            && count > 0
                            && body_duration > period
                        {
                            return Err(self.error(
                                "S304",
                                "timer body exceeds its period",
                                statement.span,
                            ));
                        }
                        let duration = math::arithmetic(
                            "*",
                            period.unwrap_or(body_duration),
                            Fraction::from(count),
                        )
                        .map_err(|e| self.error("S304", e, statement.span))?;
                        self.advance_duration(duration, statement.span)?;
                        continue;
                    }
                    let start = self.cursor;
                    for index in 0..count {
                        self.identity
                            .push(format!("repeat{}:{index}", statement.span.start));
                        let result = self.block(body, &mut env.clone());
                        self.identity.pop();
                        if let Some(value) = result? {
                            return Ok(Some(value));
                        }
                        if let Some(period) = period {
                            let next = math::Number {
                                value: period,
                                unit: Unit::Duration,
                            }
                            .binary(
                                "*",
                                Number::scalar((index + 1).try_into().map_err(|_| {
                                    self.error("S303", "timer count overflow", statement.span)
                                })?),
                            )
                            .and_then(|v| {
                                v.binary(
                                    "+",
                                    Number {
                                        value: start,
                                        unit: Unit::Duration,
                                    },
                                )
                            })
                            .map_err(|e| self.error("S303", e, statement.span))?
                            .value;
                            if self.cursor > next {
                                return Err(self.error("S304",format!("timer iteration {} ends at {}s, past its {}s deadline. Shorten play/wait durations or increase the timer period.",index+1,self.cursor,next),statement.span));
                            }
                            math::timeline_end(Fraction::from(0), next)
                                .map_err(|e| self.error("S304", e, statement.span))?;
                            self.cursor = next;
                            self.animation.duration = self.animation.duration.max(next);
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
                        scope.insert(name.clone(), value.clone());
                    }
                    if let Some(value) = self.block(&case.body, &mut scope)? {
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
            let count = self.statistics.operations - before;
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
    fn expression(&mut self, expr: &Expr, env: &Env) -> Result<Value, Diagnostic> {
        self.work(expr.span)?;
        if matches!(expr.kind, ExprKind::Name(_)) {
            self.statistics.evaluated_expressions += 1;
            return self.expression_inner(expr, env);
        }
        let cache_started = Instant::now();
        let key = if cacheable(expr) {
            let mut names = BTreeSet::new();
            dependencies(expr, &mut names);
            let identity = names
                .iter()
                .any(|name| prelude::builtin(name).is_some_and(|b| b.result == Type::Element))
                .then_some(&self.identity);
            let limits = names
                .contains("range")
                .then_some(self.limits.max_iterations);
            let mut deps = BTreeMap::new();
            for name in names {
                if let Some(value) = env.get(&name) {
                    let key = match value {
                        Value::Font(font) => CacheDependency::Font(self.font_keys.key(font)),
                        _ => CacheDependency::Value(value),
                    };
                    deps.insert(name, key);
                }
            }
            Some(digest(
                &serde_json::to_vec(&(
                    SEMANTICS_VERSION,
                    &self.type_bindings,
                    limits,
                    self.limits.data,
                    &self.module,
                    identity,
                    &expr.kind,
                    deps,
                ))
                .map_err(|e| self.error("S301", e.to_string(), expr.span))?,
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
        if let Some(key) = &key
            && let Some(value) = self.values.get(key)
        {
            self.statistics.reused_expressions += 1;
            return Ok(value.clone());
        }
        self.statistics.evaluated_expressions += 1;
        self.depth += 1;
        if self.depth > self.limits.max_call_depth * 4 {
            return Err(self.error("S303", "expression evaluation depth exceeded", expr.span));
        }
        let result = self.expression_inner(expr, env);
        self.depth -= 1;
        let value = result?;
        let mut bytes = self.limits.data.max_bytes;
        let mut items = self.limits.data.max_values;
        let validation_started = Instant::now();
        sanim_core::value_budget::charge(
            &value,
            &mut bytes,
            &mut items,
            self.limits.data.max_depth,
        )
        .map_err(|e| self.error("S303", e, expr.span))?;
        self.statistics.value_validation_micros =
            self.statistics.value_validation_micros.saturating_add(
                validation_started
                    .elapsed()
                    .as_micros()
                    .try_into()
                    .unwrap_or(u64::MAX),
            );
        if let Some(key) = key {
            self.values
                .insert_measured(key, value.clone(), self.limits.data.max_bytes - bytes);
        }
        Ok(value)
    }
    fn expression_inner(&mut self, expr: &Expr, env: &Env) -> Result<Value, Diagnostic> {
        let bad = |e| self.error("S305", e, expr.span);
        match &expr.kind {
            ExprKind::Number(v) => Number::parse(v).map(Value::Number).map_err(bad),
            ExprKind::Color(v) => Color::parse(v).map(Value::Color).map_err(bad),
            ExprKind::String(v) => Ok(Value::String(v.clone().into())),
            ExprKind::Bool(v) => Ok(Value::Bool(*v)),
            ExprKind::None => Ok(Value::None),
            ExprKind::Name(name) if prelude::values().contains_key(name.as_str()) => {
                Ok(prelude::values()[name.as_str()].clone())
            }
            ExprKind::Name(name) => env.get(name).cloned().ok_or_else(|| {
                bad(format!(
                    "name '{name}' is not available in this preparation scope"
                ))
            }),
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
                if op == "and" && !left.boolean().map_err(|e| self.error("S305", e, a.span))? {
                    return Ok(Value::Bool(false));
                }
                if op == "or" && left.boolean().map_err(|e| self.error("S305", e, a.span))? {
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
                math::operations::binary(op, left, right)
                    .map_err(|e| self.error("S305", e, expr.span))
            }
            ExprKind::Member(base, name) => {
                let value = self.expression(base, env)?;
                match value {
                    Value::Record(fields) => fields.get(name).cloned().ok_or_else(|| {
                        self.error("S305", format!("missing field '{name}'"), expr.span)
                    }),
                    vector @ (Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_)) => {
                        let (values, unit) = vector
                            .vector()
                            .map_err(|e| self.error("S305", e, expr.span))?;
                        let index = match name.as_str() {
                            "x" => 0,
                            "y" => 1,
                            "z" => 2,
                            _ => 3,
                        };
                        Ok(Value::Number(Number {
                            value: math::from_f64(*values.get(index).ok_or_else(|| {
                                self.error("S305", "vector has no such component", expr.span)
                            })?)
                            .map_err(|e| self.error("S305", e, expr.span))?,
                            unit,
                        }))
                    }
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
                    Value::Matrix4(matrix) => Ok(Value::List(
                        matrix
                            .into_iter()
                            .map(|row| {
                                Ok(Value::List(
                                    row.into_iter()
                                        .map(|v| {
                                            Ok(Value::Number(Number {
                                                value: math::from_f64(v).map_err(|e| {
                                                    self.error("S305", e, expr.span)
                                                })?,
                                                unit: Unit::Number,
                                            }))
                                        })
                                        .collect::<Result<_, Diagnostic>>()?,
                                ))
                            })
                            .collect::<Result<_, Diagnostic>>()?,
                    )),
                    Value::Element(element) => {
                        if name == "id" {
                            return Ok(Value::String(element.id.clone().into()));
                        }
                        let property = match name.as_str() {
                            "position" => Property::Position,
                            "rotation" => Property::Rotation,
                            "scale" => Property::Scale,
                            "opacity" => Property::Opacity,
                            _ => Property::Fill,
                        };
                        Ok(element.property(property))
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
                let index = self
                    .expression(index, env)?
                    .number()
                    .and_then(Number::usize)
                    .map_err(|e| self.error("S305", e, expr.span))?;
                Ok(value
                    .list()
                    .map_err(|e| self.error("S305", e, expr.span))?
                    .get(index)
                    .cloned()
                    .map(|v| Value::Some(std::sync::Arc::new(v)))
                    .unwrap_or(Value::None))
            }
            ExprKind::Call { callee, args, .. } => {
                let mut receiver = None;
                let native = match &callee.kind {
                    ExprKind::Name(name) if prelude::builtin(name).is_some() => Some(name.clone()),
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
                    let mut evaluated = vec![];
                    if let Some(receiver) = receiver {
                        evaluated.push((None, receiver));
                    }
                    for (name, value) in args {
                        evaluated.push((name.clone(), self.expression(value, env)?));
                    }
                    return self.native(&name, evaluated, expr.span);
                }
                let Value::Function(key) = self.expression(callee, env)? else {
                    return Err(self.error("S305", "expected callable function", expr.span));
                };
                let mut evaluated = vec![];
                for (name, value) in args {
                    evaluated.push((name.clone(), self.expression(value, env)?));
                }
                if let Some(ty) = self.constructors.get(&key).cloned() {
                    return match ty {
                        Type::Record(fields) => {
                            let ordered = order_values(
                                &evaluated,
                                &fields.keys().cloned().collect::<Vec<_>>(),
                            );
                            let mut out = BTreeMap::new();
                            for (name, _) in fields {
                                out.insert(
                                    name.clone(),
                                    ordered.get(&name).cloned().ok_or_else(|| {
                                        self.error(
                                            "S305",
                                            format!("missing field '{name}'"),
                                            expr.span,
                                        )
                                    })?,
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
                                        .map(|name| {
                                            ordered.remove(name).expect("checked enum argument")
                                        })
                                        .collect()
                                },
                            })
                        }
                        _ => Err(self.error("S305", "type is not constructible", expr.span)),
                    };
                }
                self.invoke(&key, &evaluated, expr.span, true, None)
            }
        }
    }
    fn invoke(
        &mut self,
        key: &str,
        evaluated: &[(Option<String>, Value)],
        span: Span,
        need_result: bool,
        provided: Option<&BTreeSet<String>>,
    ) -> Result<Value, Diagnostic> {
        let function = self
            .functions
            .get(key)
            .cloned()
            .ok_or_else(|| self.error("S305", format!("unknown function '{key}'"), span))?;
        self.used_functions
            .entry(key.to_owned())
            .or_insert((function.fingerprint.clone(), self.root_statement));
        let bindings = self
            .loaded
            .iter()
            .find(|m| m.id == self.module)
            .and_then(|m| m.checked.call_types.get(&span.start))
            .map(|types| {
                types
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.substitute(&self.type_bindings)))
                    .collect()
            })
            .unwrap_or_default();
        let previous_types = std::mem::replace(&mut self.type_bindings, bindings);
        let previous = std::mem::replace(&mut self.module, function.module.clone());
        let mut scope = self
            .globals
            .get(&function.module)
            .cloned()
            .unwrap_or_default();
        let ordered = order_values(
            evaluated,
            &function
                .parameters
                .iter()
                .map(|(n, _, _)| n.clone())
                .collect::<Vec<_>>(),
        );
        self.identity
            .push(format!("{}@{}", function.name, span.start));
        for (name, _, default) in &function.parameters {
            if self.duration_only && !need_result && !self.duration_binding_needed(name) {
                if !ordered.contains_key(name)
                    && !provided.is_some_and(|names| names.contains(name))
                    && let Some(default) = default
                {
                    self.duration_effects(default, &scope)?;
                }
                continue;
            }
            let value = if let Some(value) = ordered.get(name) {
                value.clone()
            } else if let Some(default) = default {
                self.expression(default, &scope)?
            } else {
                return Err(self.error("S305", format!("missing argument '{name}'"), span));
            };
            scope.insert(name.clone(), value.clone());
        }
        let previous_result = self.result_needed;
        self.result_needed = need_result;
        let function_start = self.cursor;
        self.depth += 1;
        let result = if self.depth > self.limits.max_call_depth {
            Err(self.error("S303", "bounded function call depth exceeded", span))
        } else {
            self.block(&function.body, &mut scope)
        };
        if result.is_ok() {
            self.map_time(function_start, function.span, TimeMapKind::Function)?;
        }
        self.depth -= 1;
        self.result_needed = previous_result;
        self.identity.pop();
        self.module = previous;
        self.type_bindings = previous_types;
        Ok(result?.unwrap_or(Value::Unit))
    }
    fn native(
        &mut self,
        name: &str,
        arguments: Vec<(Option<String>, Value)>,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let builtin = prelude::builtin(name)
            .ok_or_else(|| self.error("S305", format!("unknown native '{name}'"), span))?;
        if self.duration_only && name == "Shift" {
            return Err(self.error(
                "S390",
                "duration value depends on sampled animation state; full preparation is required",
                span,
            ));
        }
        if builtin.schedule && !self.root_active {
            return Err(self.error("S115","imported modules cannot schedule or assert while being imported; call a helper from the root script instead",span));
        }
        let params = builtin
            .parameters
            .iter()
            .map(|v| v.name.to_string())
            .collect::<Vec<_>>();
        let mut args = if matches!(builtin.execute, prelude::Execution::Play) {
            order_values(
                &arguments
                    .iter()
                    .filter(|(name, _)| name.is_some())
                    .cloned()
                    .collect::<Vec<_>>(),
                &params,
            )
        } else {
            order_values(&arguments, &params)
        };
        if matches!(builtin.execute, prelude::Execution::Read) {
            let path = args
                .get("path")
                .ok_or_else(|| self.error("S305", "missing path", span))?
                .string()
                .map_err(|e| self.error("S305", e, span))?;
            let path =
                source::resolve(&self.module, path).map_err(|e| self.error("S201", e, span))?;
            let bytes = self
                .snapshot
                .read(&path)
                .map_err(|e| self.error("S201", e, span))?;
            let resource_version = self.snapshot.fingerprint(&path);
            self.used_resources
                .entry(path.clone())
                .or_insert((resource_version.clone(), self.root_statement));
            let ty = self
                .loaded
                .iter()
                .find(|v| v.id == self.module)
                .and_then(|v| v.checked.read_types.get(&span.start))
                .ok_or_else(|| self.error("S301", "missing read schema", span))?
                .substitute(&self.type_bindings);
            let key = digest(
                &serde_json::to_vec(&(
                    SEMANTICS_VERSION,
                    &path,
                    &ty,
                    self.limits.data,
                    resource_version,
                ))
                .map_err(|e| self.error("S301", e.to_string(), span))?,
            );
            if let Some(value) = self.assets.get(&key) {
                self.statistics.reused_assets += 1;
                return Ok(value.clone());
            }
            self.progress.emit(Phase::Decoding, None, None, Some(&path));
            let value = sanim_data::decode(&path, &bytes, &ty, self.limits.data).map_err(|e| {
                self.error("S306", format!("reading '{path}' as {ty:?}: {e}"), span)
            })?;
            self.statistics.decoded_assets += 1;
            self.assets.insert(key, value.clone());
            self.report_evaluation(Some(span));
            return Ok(value);
        }
        if matches!(
            builtin.execute,
            prelude::Execution::Decode | prelude::Execution::Query
        ) {
            let ty = self
                .loaded
                .iter()
                .find(|v| v.id == self.module)
                .and_then(|v| v.checked.read_types.get(&span.start))
                .ok_or_else(|| self.error("S301", "missing prepared data schema", span))?
                .substitute(&self.type_bindings);
            return if matches!(builtin.execute, prelude::Execution::Decode) {
                let resource = format!("{}:{}", self.module, span.line);
                self.progress
                    .emit(Phase::Decoding, None, None, Some(&resource));
                let data =
                    sanim_data::to_json(&args["value"]).map_err(|e| self.error("S306", e, span))?;
                let value = sanim_data::validate(&data, &ty, self.limits.data)
                    .map_err(|e| self.error("S306", format!("decode as {ty}: {e}"), span))?;
                self.report_evaluation(Some(span));
                Ok(value)
            } else {
                let key = digest(
                    &serde_json::to_vec(&(
                        "sqlite",
                        SEMANTICS_VERSION,
                        &ty,
                        self.limits.data,
                        &args,
                    ))
                    .map_err(|e| self.error("S301", e.to_string(), span))?,
                );
                if let Some(value) = self.assets.get(&key) {
                    self.statistics.reused_queries += 1;
                    return Ok(value.clone());
                }
                let resource = format!("{}:{}", self.module, span.line);
                self.progress
                    .emit(Phase::Querying, None, None, Some(&resource));
                let started = std::time::Instant::now();
                let value = sanim_data::sqlite::query(
                    args["sql"]
                        .string()
                        .map_err(|e| self.error("S306", e, span))?,
                    &args["tables"],
                    args.get("params"),
                    &ty,
                    self.limits.data,
                )
                .map_err(|e| self.error("S309", e, span))?;
                let elapsed = started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
                self.statistics.query_micros = self.statistics.query_micros.saturating_add(elapsed);
                if elapsed > self.limits.warn_prepare_micros {
                    self.performance.push(PerformanceWarning{code:"P003".into(),message:format!("SQLite query took {elapsed}us (warning budget {}us), returning {} rows. Filter before joining or grouping, reduce input tables, or reuse a prepared result outside loops.",self.limits.warn_prepare_micros,value.list().map_or(0,|rows|rows.len())),source:self.module.clone(),span,operations:0});
                }
                self.statistics.executed_queries += 1;
                self.assets.insert(key, value.clone());
                self.report_evaluation(Some(span));
                Ok(value)
            };
        }
        if matches!(builtin.execute, prelude::Execution::Play) && self.duration_only {
            let duration = args
                .get("duration")
                .map(Value::number)
                .transpose()
                .map_err(|e| self.error("S304", e, span))?
                .map_or(Fraction::from(1), |n| n.value);
            if duration <= Fraction::from(0) {
                return Err(self.error("S304", "play duration must be positive", span));
            }
            self.advance_duration(duration, span)?;
            return Ok(Value::Unit);
        }
        if self.duration_only
            && builtin.schedule
            && !matches!(
                builtin.execute,
                prelude::Execution::Play | prelude::Execution::Wait(_)
            )
        {
            return Ok(Value::Unit);
        }
        if matches!(builtin.execute, prelude::Execution::Play) {
            let motions = arguments
                .iter()
                .filter(|(name, _)| name.is_none())
                .map(|(_, v)| {
                    if let Value::Motion(m) = v {
                        Ok(m.as_ref().clone())
                    } else {
                        Err("play expects Motion".to_string())
                    }
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| self.error("S305", e, span))?;
            let duration = args
                .get("duration")
                .map(Value::number)
                .transpose()
                .map_err(|e| self.error("S305", e, span))?
                .map_or(Fraction::from(1), |v| v.value);
            let ease = Easing::parse(
                args.get("ease")
                    .map(Value::string)
                    .transpose()
                    .map_err(|e| self.error("S305", e, span))?
                    .unwrap_or("linear"),
            )
            .map_err(|e| self.error("S305", e, span))?;
            let schedule_start = self.cursor;
            self.animation
                .schedule(&motions, self.cursor, duration, ease)
                .map_err(|e| self.error("S307", e, span))?;
            self.cursor = Number {
                value: self.cursor,
                unit: Unit::Duration,
            }
            .binary(
                "+",
                Number {
                    value: duration,
                    unit: Unit::Duration,
                },
            )
            .map_err(|e| self.error("S307", e, span))?
            .value;
            self.map_time(schedule_start, span, TimeMapKind::Schedule)?;
            self.check_elements(span)?;
            return Ok(Value::Unit);
        }
        if matches!(name, "assert" | "check") {
            let condition = args
                .remove("condition")
                .ok_or_else(|| self.error("S305", "missing assertion condition", span))?;
            let message = args
                .get("message")
                .map(Value::string)
                .transpose()
                .map_err(|e| self.error("S305", e, span))?
                .map(str::to_string);
            match condition {
                Value::Bool(value) => {
                    let verification = Verification {
                        evidence: if value {
                            verify::Evidence::Proved
                        } else {
                            verify::Evidence::Refuted
                        },
                        message: message.unwrap_or_else(|| "boolean assertion must be true".into()),
                        time: Some(self.cursor),
                    };
                    if !value && name == "assert" {
                        return Err(self.error("S350", verification.message, span));
                    }
                    self.checks.push_back(verification);
                }
                Value::Predicate(predicate) => self.assertions.push_back(Assertion {
                    predicate: predicate.as_ref().clone(),
                    time: self.cursor,
                    required: name == "assert",
                    module: self.module.clone(),
                    span,
                    message,
                }),
                _ => {
                    return Err(self.error(
                        "S305",
                        "assert/check requires a boolean or relationship",
                        span,
                    ));
                }
            }
            return Ok(Value::Unit);
        }
        let (prelude::Execution::Native(evaluate)
        | prelude::Execution::Access(evaluate)
        | prelude::Execution::Wait(evaluate)) = builtin.execute
        else {
            unreachable!("registered host operation was handled above")
        };
        if name == "Text" {
            let resource = format!("{}:{}", self.module, span.line);
            self.progress
                .emit(Phase::Shaping, None, None, Some(&resource));
        }
        let schedule_start = self.cursor;
        let native_started = Instant::now();
        let result = evaluate(
            name,
            &args,
            crate::native::Context {
                module: &self.module,
                identity: &self.identity,
                span,
                animation: &mut self.animation,
                cursor: &mut self.cursor,
                limits: self.limits,
            },
        )
        .map_err(|e| self.error("S305", e, span));
        self.statistics.native_micros = self.statistics.native_micros.saturating_add(
            native_started
                .elapsed()
                .as_micros()
                .try_into()
                .unwrap_or(u64::MAX),
        );
        if result.is_ok() && matches!(builtin.execute, prelude::Execution::Wait(_)) {
            self.map_time(schedule_start, span, TimeMapKind::Schedule)?;
        }
        if name == "Text" && result.is_ok() {
            self.report_evaluation(Some(span));
        }
        result
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

fn order_values(args: &[(Option<String>, Value)], parameters: &[String]) -> Env {
    let mut out = Env::new();
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
            matches!(&callee.kind,ExprKind::Name(name)if prelude::builtin(name).is_some_and(|v|!v.schedule&&v.result!=Type::Motion&&matches!(v.execute,prelude::Execution::Native(_))))
                && args.iter().all(|(_, v)| cacheable(v))
        }
        ExprKind::Member(_, _) | ExprKind::Index(_, _) => false,
        ExprKind::Unary(_, a) => cacheable(a),
        ExprKind::Binary(_, a, b) => cacheable(a) && cacheable(b),
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
pub fn validate_value(ty: &Type, value: &Value) -> Result<(), String> {
    if !ty.accepts(&value.ty()) {
        return Err(format!("expected {ty}, found {}", value.ty()));
    }
    match (ty, value) {
        (Type::Fraction, Value::Fraction(value)) if !math::finite(value) => {
            return Err("fraction must be finite".into());
        }
        (Type::Matrix4, Value::Matrix4(matrix))
            if matrix.iter().flatten().any(|v| !v.is_finite()) =>
        {
            return Err("matrix entries must be finite".into());
        }
        (Type::Numeric(_), Value::Number(n)) if !math::finite(&n.value) => {
            return Err("non-finite numeric input".into());
        }
        (Type::Color, Value::Color(color))
            if color
                .0
                .iter()
                .any(|v| !math::finite(v) || *v < Fraction::from(0) || *v > Fraction::from(1)) =>
        {
            return Err("color channels must be in [0, 1]".into());
        }
        (Type::Vector(..), vector @ (Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_)))
            if vector.vector()?.0.iter().any(|v| !v.is_finite()) =>
        {
            return Err("vector components must be finite".into());
        }
        (Type::Array(_), Value::Array { shape, values, .. })
            if shape.iter().try_fold(1usize, |n, d| n.checked_mul(*d)) != Some(values.len()) =>
        {
            return Err("array shape does not match its value count".into());
        }
        (Type::List(inner) | Type::Table(inner) | Type::Array(inner), values) => {
            for value in values.list()? {
                validate_value(inner, value)?;
            }
        }
        (Type::Option(inner), Value::Some(value)) => validate_value(inner, value)?,
        (Type::Record(fields), Value::Record(values)) => {
            for (name, ty) in fields {
                validate_value(ty, &values[name]).map_err(|e| format!("field '{name}': {e}"))?;
            }
        }
        (Type::Enum { variants, .. }, Value::Variant { tag, fields, .. }) => {
            let types = variants.get(tag).ok_or("unknown enum variant")?;
            if types.len() != fields.len() {
                return Err("variant field count mismatch".into());
            }
            for (ty, value) in types.iter().zip(fields.iter()) {
                validate_value(ty, value)?;
            }
        }
        _ => {}
    }
    Ok(())
}
