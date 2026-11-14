//! Infer timeline effects over the canonical AST, preserving lexical scopes and control flow.
mod formula;
use crate::{
    Limits,
    ast::*,
    checker::{Checked, Environment},
    prelude,
};
use sanim_core::duration::DurationBudget;
use sanim_core::{
    Diagnostic, DurationExpr, DurationSymbol, Fraction, NodeDuration, Number, Span,
    TimelineEffect as Effect, Type, Unit, Value, math,
};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub(crate) struct Analysis {
    pub nodes: Vec<NodeDuration>,
    pub clauses: BTreeMap<usize, Effect>,
    pub needed: BTreeSet<String>,
    pub value_dependencies: BTreeMap<String, BTreeSet<String>>,
}
pub(crate) fn analyze(
    source: &str,
    module: &Module,
    env: &Environment,
    _imports: &BTreeMap<String, Arc<Checked>>,
    native_methods: &BTreeMap<(usize, usize), crate::NativeMethod>,
    budget: &Cell<usize>,
    limits: &Limits,
) -> Result<Analysis, Diagnostic> {
    let mut functions = BTreeMap::new();
    let mut captures = BTreeMap::new();
    let mut constants = BTreeMap::new();

    for statement in &module.statements {
        match &statement.kind {
            StatementKind::Bind {
                kind, name, value, ..
            } => {
                if *kind == BindingKind::Let
                    && let Some(value) = constant(value, &constants)
                {
                    constants.insert(name.clone(), value);
                } else {
                    constants.remove(name);
                }
            }
            StatementKind::Function {
                name,
                parameters,
                body,
                duration,
                ..
            } => {
                functions
                    .entry(name.clone())
                    .or_insert_with(Vec::new)
                    .push(statement);
                let mut names = BTreeSet::new();
                for statement in body {
                    statement.visit_expressions(&mut |expr| {
                        if let ExprKind::Name(name) = &expr.kind {
                            names.insert(name.clone());
                        }
                    });
                }
                if let Some(duration) = duration {
                    value_references(duration, &mut names);
                }
                for declaration in parameters {
                    let parameter = &declaration.name;
                    let default = &declaration.default;
                    names.remove(parameter);
                    if let Some(expr) = default {
                        value_references(expr, &mut names);
                    }
                }
                captures.insert(
                    statement.span.start,
                    Captures {
                        constants: constants
                            .iter()
                            .filter(|(name, _)| names.contains(*name))
                            .map(|(name, value)| (name.clone(), value.clone()))
                            .collect(),
                        names,
                    },
                );
            }
            _ => {}
        }
    }
    let mut analyzer = Analyzer {
        source,
        native_methods,
        env,
        functions,
        captures,

        budget,
        duration_budget: DurationBudget::default(),
        limits,
        done: BTreeMap::new(),
        clauses: BTreeMap::new(),
        symbolic: BTreeMap::new(),
        active: BTreeSet::new(),
        nodes: BTreeMap::new(),
        constants: BTreeMap::new(),
        needed: BTreeSet::new(),
        bindings: BTreeMap::new(),
    };
    analyzer.collect_bindings(&module.statements, None);
    for name in analyzer.functions.keys().cloned().collect::<Vec<_>>() {
        analyzer.function(&name)?;
    }
    analyzer.block(&module.statements)?;
    loop {
        let before = analyzer.needed.len();
        for name in analyzer.needed.clone() {
            if let Some(dependencies) = analyzer.bindings.get(&name) {
                analyzer.needed.extend(dependencies.iter().cloned());
            }
        }
        if before == analyzer.needed.len() {
            break;
        }
    }
    Ok(Analysis {
        nodes: analyzer.nodes.into_values().collect(),
        clauses: analyzer.clauses,
        needed: analyzer.needed,
        value_dependencies: analyzer.bindings,
    })
}
#[derive(Clone, Default)]
struct Captures {
    constants: BTreeMap<String, Value>,
    names: BTreeSet<String>,
}
impl Captures {
    fn symbols(&self, source: &str) -> BTreeMap<DurationSymbol, DurationExpr> {
        let mut symbols = BTreeMap::new();
        for name in &self.names {
            let qualified = format!("{source}::{name}");
            if let Some(value) = DurationExpr::parameter(qualified.as_str()) {
                symbols.insert(DurationSymbol::Parameter(name.as_str().into()), value);
            }
            if let Some(value) = DurationExpr::length(qualified.as_str()) {
                symbols.insert(DurationSymbol::Length(name.as_str().into()), value);
            }
        }
        symbols
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Flow {
    continuing: Option<Effect>,
    returned: Option<Effect>,
}
fn merge(a: Option<Effect>, b: Option<Effect>) -> Result<Option<Effect>, String> {
    match (a, b) {
        (Some(a), Some(b)) => Effect::alternatives([a, b]).map(Some),
        (a, b) => Ok(a.or(b)),
    }
}
impl Flow {
    fn normal(effect: Effect) -> Self {
        Self {
            continuing: Some(effect),
            returned: None,
        }
    }
    fn total(&self) -> Result<Effect, String> {
        Ok(merge(self.continuing.clone(), self.returned.clone())?.unwrap_or_default())
    }
    fn prepend(self, effect: Effect) -> Result<Self, String> {
        Ok(Self {
            continuing: self
                .continuing
                .map(|v| effect.clone().then(v))
                .transpose()?,
            returned: self.returned.map(|v| effect.clone().then(v)).transpose()?,
        })
    }
}
struct Analyzer<'a> {
    native_methods: &'a BTreeMap<(usize, usize), crate::NativeMethod>,
    source: &'a str,
    env: &'a Environment,
    functions: BTreeMap<String, Vec<&'a Statement>>,
    captures: BTreeMap<usize, Captures>,
    budget: &'a Cell<usize>,
    duration_budget: DurationBudget,
    limits: &'a Limits,
    done: BTreeMap<String, Effect>,
    clauses: BTreeMap<usize, Effect>,
    symbolic: BTreeMap<DurationSymbol, DurationExpr>,
    active: BTreeSet<String>,
    nodes: BTreeMap<(usize, usize), NodeDuration>,
    constants: BTreeMap<String, Value>,
    needed: BTreeSet<String>,
    bindings: BTreeMap<String, BTreeSet<String>>,
}
impl Analyzer<'_> {
    fn fail(&self, message: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic::new("S304", message, self.source, span)
    }
    fn sum(&self, a: Effect, b: Effect, span: Span) -> Result<Effect, Diagnostic> {
        a.then(b).map_err(|e| self.fail(e, span))
    }
    fn charge_duration(&mut self, effect: &Effect, span: Span) -> Result<(), Diagnostic> {
        let mut remaining = self.budget.get();
        self.duration_budget
            .charge(effect, &mut remaining)
            .map_err(|message| Diagnostic::new("S105", message, self.source, span))?;
        self.budget.set(remaining);
        Ok(())
    }
    fn record(&mut self, span: Span, effect: Effect) -> Result<Effect, Diagnostic> {
        self.charge_duration(&effect, span)?;
        if !self.nodes.contains_key(&(span.start, span.end)) {
            self.budget.set(
                self.budget
                    .get()
                    .checked_sub(std::mem::size_of::<NodeDuration>())
                    .ok_or_else(|| {
                        Diagnostic::new(
                            "S105",
                            "semantic metadata budget exceeded",
                            self.source,
                            span,
                        )
                    })?,
            );
        }
        self.nodes.insert(
            (span.start, span.end),
            NodeDuration {
                span,
                effect: effect.clone(),
            },
        );
        Ok(effect)
    }
    fn edge(&mut self, name: &str, expr: &Expr) {
        let mut names = BTreeSet::new();
        value_references(expr, &mut names);
        self.bindings.entry(name.into()).or_default().extend(names);
    }
    fn collect_bindings(&mut self, body: &[Statement], function: Option<&str>) {
        for statement in body {
            match &statement.kind {
                StatementKind::Bind { name, value, .. } | StatementKind::Assign { name, value } => {
                    self.edge(name, value)
                }
                StatementKind::Animate {
                    name,
                    value,
                    duration,
                    ..
                } => {
                    self.edge(name, value);
                    self.edge(name, duration);
                }
                StatementKind::Background { body } => self.collect_bindings(body, function),
                StatementKind::Function {
                    name,
                    body,
                    parameters,
                    ..
                } => {
                    for declaration in parameters {
                        let parameter = &declaration.name;
                        let default = &declaration.default;
                        if let Some(default) = default {
                            self.edge(parameter, default);
                        }
                    }
                    self.collect_bindings(body, Some(name));
                }
                StatementKind::Return(value) => {
                    if let Some(function) = function {
                        self.edge(function, value);
                    }
                }
                StatementKind::If { yes, no, .. } => {
                    self.collect_bindings(yes, function);
                    self.collect_bindings(no, function);
                }
                StatementKind::For {
                    name,
                    iterable,
                    body,
                } => {
                    self.edge(name, iterable);
                    self.collect_bindings(body, function);
                }
                StatementKind::Repeat { body, .. } => self.collect_bindings(body, function),
                StatementKind::Match { value, cases } => {
                    for case in cases {
                        for binding in &case.bindings {
                            self.edge(binding, value);
                        }
                        self.collect_bindings(&case.body, function);
                    }
                }
                _ => {}
            }
        }
    }
    fn need(&mut self, expr: &Expr) {
        value_references(expr, &mut self.needed);
    }
    fn function(&mut self, name: &str) -> Result<Effect, Diagnostic> {
        if let Some(effect) = self.done.get(name) {
            return Ok(effect.clone());
        }
        if self.active.contains(name) {
            return Ok(Effect::Unknown);
        }
        if self.active.len() >= self.limits.max_call_depth {
            return Err(self.fail(
                "function duration analysis exceeds bounded call depth",
                Span::default(),
            ));
        }
        self.active.insert(name.into());
        let mut alternatives = Vec::new();
        for (clause, statement) in self.functions[name].clone().into_iter().enumerate() {
            let StatementKind::Function {
                parameters,
                body,
                duration,
                ..
            } = &statement.kind
            else {
                unreachable!("function index")
            };
            let captured = self
                .captures
                .get(&statement.span.start)
                .cloned()
                .unwrap_or_default();
            let previous_symbolic =
                std::mem::replace(&mut self.symbolic, captured.symbols(self.source));
            let previous = std::mem::replace(&mut self.constants, captured.constants);
            for (parameter_index, parameter) in parameters.iter().enumerate() {
                if let Some(value) = DurationExpr::parameter(parameter.name.as_str()) {
                    self.symbolic.insert(
                        DurationSymbol::Parameter(parameter.name.as_str().into()),
                        value,
                    );
                }
                let known_length = self
                    .env
                    .get(name)
                    .and_then(|ty| ty.signatures().get(clause))
                    .and_then(|signature| signature.parameters.get(parameter_index))
                    .and_then(|parameter| {
                        if parameter.kind == BindingKind::Let
                            && let Type::List(_, sanim_core::ListLength::Exact(length)) =
                                &parameter.ty
                        {
                            Some(*length)
                        } else {
                            None
                        }
                    });
                let length = known_length
                    .and_then(|length| DurationExpr::constant(Fraction::from(length)))
                    .or_else(|| DurationExpr::length(parameter.name.as_str()));
                if let Some(value) = length {
                    self.symbolic.insert(
                        DurationSymbol::Length(parameter.name.as_str().into()),
                        value,
                    );
                }
            }
            let mut effect = self.block(body)?;
            if let Some(annotation) = duration {
                self.need(annotation);
                let declared =
                    self.call_duration(Some(annotation), Default::default(), annotation.span)?;
                if let (Effect::Fixed(actual), Effect::Fixed(declared)) = (&effect, &declared)
                    && actual != declared
                {
                    return Err(self.fail(format!("declared duration {declared}s differs from the function body's {actual}s"),annotation.span));
                }
                effect = declared;
            }
            let mut optional_defaults = Effect::default();
            for parameter in parameters {
                if let Some(default) = &parameter.default {
                    let default_effect = self.expression(default)?;
                    optional_defaults = optional_defaults
                        .then(
                            Effect::alternatives([Effect::default(), default_effect])
                                .map_err(|e| self.fail(e, default.span))?,
                        )
                        .map_err(|e| self.fail(e, default.span))?;
                }
            }
            effect = optional_defaults
                .then(effect)
                .map_err(|e| self.fail(e, statement.span))?;
            self.constants = previous;
            self.symbolic = previous_symbolic;
            self.charge_duration(&effect, statement.span)?;
            self.clauses.insert(statement.span.start, effect.clone());
            alternatives.push(effect);
        }
        self.active.remove(name);
        let effect =
            Effect::alternatives(alternatives).map_err(|e| self.fail(e, Span::default()))?;
        self.charge_duration(&effect, Span::default())?;
        self.done.insert(name.into(), effect.clone());
        Ok(effect)
    }
    fn block(&mut self, body: &[Statement]) -> Result<Effect, Diagnostic> {
        self.block_flow(body)?
            .total()
            .map_err(|e| self.fail(e, Span::default()))
    }
    fn block_flow(&mut self, body: &[Statement]) -> Result<Flow, Diagnostic> {
        let previous = self.constants.clone();
        let previous_symbolic = self.symbolic.clone();
        let mut flow = Flow::normal(Effect::default());
        for statement in body {
            let next = self.statement_flow(statement)?;
            if let Some(prefix) = flow.continuing.take() {
                let returned = next
                    .returned
                    .map(|effect| prefix.clone().then(effect))
                    .transpose()
                    .map_err(|e| self.fail(e, statement.span))?;
                flow.returned =
                    merge(flow.returned, returned).map_err(|e| self.fail(e, statement.span))?;
                flow.continuing = next
                    .continuing
                    .map(|effect| prefix.then(effect))
                    .transpose()
                    .map_err(|e| self.fail(e, statement.span))?;
            }
            if let StatementKind::Bind {
                kind, name, value, ..
            } = &statement.kind
            {
                let scalar = formula::scalar(value, &self.constants, &self.symbolic);
                let length = formula::length(value, &self.constants, &self.symbolic);
                let key = DurationSymbol::Parameter(name.as_str().into());
                let length_key = DurationSymbol::Length(name.as_str().into());
                self.symbolic.remove(&key);
                self.symbolic.remove(&length_key);
                if *kind == BindingKind::Let {
                    let identity = format!("{}:{}::{name}", self.source, statement.span.start);
                    if let Some(value) =
                        scalar.or_else(|| DurationExpr::parameter(identity.as_str()))
                    {
                        self.symbolic.insert(key, value);
                    }
                    if let Some(value) = length.or_else(|| DurationExpr::length(identity.as_str()))
                    {
                        self.symbolic.insert(length_key, value);
                    }
                }
                if *kind == BindingKind::Let
                    && let Some(value) = constant(value, &self.constants)
                {
                    self.constants.insert(name.clone(), value);
                } else {
                    self.constants.remove(name);
                }
            }
            if let StatementKind::Assign { name, .. } = &statement.kind {
                self.constants.remove(name);
                self.symbolic
                    .remove(&DurationSymbol::Parameter(name.as_str().into()));
                self.symbolic
                    .remove(&DurationSymbol::Length(name.as_str().into()));
            }
        }
        self.constants = previous;
        self.symbolic = previous_symbolic;
        if let (Some(first), Some(last)) = (body.first(), body.last()) {
            self.record(
                Span {
                    end: last.span.end,
                    ..first.span
                },
                flow.total().map_err(|e| self.fail(e, first.span))?,
            )?;
        }
        Ok(flow)
    }
    fn statement_flow(&mut self, statement: &Statement) -> Result<Flow, Diagnostic> {
        let flow = match &statement.kind {
            StatementKind::Return(value) => Flow {
                continuing: None,
                returned: Some(self.expression(value)?),
            },
            StatementKind::If { condition, yes, no } => {
                let input = self.expression(condition)?;
                let yes = self.block_flow(yes)?;
                let no = self.block_flow(no)?;
                if yes != no {
                    self.need(condition);
                }
                let selected = match constant(condition, &self.constants) {
                    Some(Value::Bool(true)) => yes,
                    Some(Value::Bool(false)) => no,
                    _ => Flow {
                        continuing: merge(yes.continuing, no.continuing)
                            .map_err(|e| self.fail(e, statement.span))?,
                        returned: merge(yes.returned, no.returned)
                            .map_err(|e| self.fail(e, statement.span))?,
                    },
                };
                selected
                    .prepend(input)
                    .map_err(|e| self.fail(e, statement.span))?
            }
            StatementKind::Match { value, cases } => {
                let input = self.expression(value)?;
                let mut selected = Flow {
                    continuing: None,
                    returned: None,
                };
                for case in cases {
                    let flow = self.block_flow(&case.body)?;
                    selected.continuing = merge(selected.continuing, flow.continuing)
                        .map_err(|e| self.fail(e, statement.span))?;
                    selected.returned = merge(selected.returned, flow.returned)
                        .map_err(|e| self.fail(e, statement.span))?;
                }
                self.need(value);
                selected
                    .prepend(input)
                    .map_err(|e| self.fail(e, statement.span))?
            }
            _ => {
                let effect = self.statement(statement)?;
                if has_return(std::slice::from_ref(statement)) {
                    Flow {
                        continuing: Some(effect.clone()),
                        returned: Some(effect),
                    }
                } else {
                    Flow::normal(effect)
                }
            }
        };
        self.record(
            statement.span,
            flow.total().map_err(|e| self.fail(e, statement.span))?,
        )?;
        Ok(flow)
    }
    fn statement(&mut self, statement: &Statement) -> Result<Effect, Diagnostic> {
        let effect = match &statement.kind {
            StatementKind::Bind {
                kind: BindingKind::Sig,
                value,
                ..
            } => {
                self.expression(value)?;
                Effect::default()
            }
            StatementKind::Bind { value, .. }
            | StatementKind::Assign { value, .. }
            | StatementKind::Expr(value)
            | StatementKind::Return(value) => self.expression(value)?,
            StatementKind::Background { body } => {
                self.block(body)?;
                Effect::Unknown
            }
            StatementKind::Animate {
                name,
                value,
                duration,
                easing,
            } => {
                self.needed.insert(name.clone());
                let mut input = self.expression(value)?;
                let duration_effect = self.expression(duration)?;
                input = self.sum(input, duration_effect, statement.span)?;
                if let Some(easing) = easing {
                    let easing_effect = self.expression(easing)?;
                    input = self.sum(input, easing_effect, statement.span)?;
                }
                let timing = if let Some(value) =
                    constant(duration, &self.constants).and_then(|v| v.number().ok())
                {
                    if value.value <= Fraction::from(0) {
                        return Err(self.fail("animate duration must be positive", duration.span));
                    }
                    Effect::Fixed(value.value)
                } else {
                    self.need(duration);
                    formula::scalar(duration, &self.constants, &self.symbolic)
                        .map_or(Effect::Unknown, Effect::from_expr)
                };
                self.sum(input, timing, statement.span)?
            }
            StatementKind::Repeat {
                count,
                period,
                body,
            } => {
                let count_formula = formula::scalar(count, &self.constants, &self.symbolic);
                if count_formula
                    .as_ref()
                    .and_then(DurationExpr::value)
                    .is_none()
                    || has_return(body)
                {
                    self.need(count);
                }
                let input = self.expression(count)?;
                let period_effect = period
                    .as_ref()
                    .map(|value| self.expression(value))
                    .transpose()?
                    .unwrap_or_default();
                if let Some(period) = period {
                    self.need(period);
                }
                let body_effect = self.block(body)?;
                if let Some(value) = constant(count, &self.constants) {
                    let count = value
                        .number()
                        .and_then(Number::usize)
                        .map_err(|e| self.fail(format!("repeat/timer count: {e}"), count.span))?;
                    if count > self.limits.max_iterations {
                        return Err(Diagnostic::new(
                            "S303",
                            format!(
                                "repeat count {count} exceeds limit {}",
                                self.limits.max_iterations
                            ),
                            self.source,
                            statement.span,
                        ));
                    }
                }
                let repeated = if has_return(body) {
                    Effect::Unknown
                } else if let Some(count) = formula::scalar(count, &self.constants, &self.symbolic)
                {
                    let step = if let Some(period) = period {
                        match formula::scalar(period, &self.constants, &self.symbolic) {
                            Some(value) => {
                                if let Some(period) = value.value() {
                                    if period <= Fraction::from(0) {
                                        return Err(self.fail(
                                            "timer period must be positive",
                                            statement.span,
                                        ));
                                    }
                                    if let Effect::Fixed(body) = &body_effect
                                        && *body > period
                                    {
                                        return Err(self.fail(
                                            "timer body exceeds its period",
                                            statement.span,
                                        ));
                                    }
                                }
                                if !matches!(body_effect, Effect::Fixed(_)) {
                                    Effect::Unknown
                                } else {
                                    Effect::from_expr(value)
                                }
                            }
                            None => Effect::Unknown,
                        }
                    } else {
                        body_effect
                    };
                    step.repeated(count)
                } else {
                    Effect::Unknown
                };
                let evaluated = self.sum(input, period_effect, statement.span)?;
                self.sum(evaluated, repeated, statement.span)?
            }
            StatementKind::For { iterable, body, .. } => {
                let length = formula::length(iterable, &self.constants, &self.symbolic);
                if length.as_ref().and_then(DurationExpr::value).is_none() || has_return(body) {
                    self.need(iterable);
                }
                let input = self.expression(iterable)?;
                let body_effect = self.block(body)?;
                let repeated = if has_return(body) {
                    Effect::Unknown
                } else {
                    length.map_or(Effect::Unknown, |count| body_effect.repeated(count))
                };
                self.sum(input, repeated, statement.span)?
            }
            StatementKind::If { .. } | StatementKind::Match { .. } => {
                unreachable!("control flow is handled by statement_flow")
            }
            StatementKind::Function { .. }
            | StatementKind::Alias { .. }
            | StatementKind::Enum { .. }
            | StatementKind::Import { .. } => Effect::default(),
        };
        self.record(statement.span, effect)
    }
    fn call_dependencies(&mut self, callee: &Expr, args: &[(Option<String>, Expr)]) {
        let Some(ty) = lookup(callee, self.env) else {
            return;
        };
        let Some(signature) = ty.signatures().first() else {
            return;
        };
        let parameters = signature
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                (
                    parameter.name.clone(),
                    ty.signatures()
                        .iter()
                        .any(|signature| signature.duration_parameters.contains(&index)),
                )
            })
            .collect::<Vec<_>>();
        let imported = matches!(callee.kind, ExprKind::Member(..));
        let mut positional = 0;
        for (label, value) in args {
            let parameter = if let Some(label) = label {
                label.clone()
            } else {
                let Some((name, _)) = parameters.get(positional) else {
                    continue;
                };
                positional += 1;
                name.clone()
            };
            self.edge(&parameter, value);
            if imported
                && parameters
                    .iter()
                    .any(|(name, needed)| name == &parameter && *needed)
            {
                self.need(value);
            }
        }
    }
    fn call_duration(
        &mut self,
        duration: Option<&Expr>,
        rule: crate::timing::Timing,
        span: Span,
    ) -> Result<Effect, Diagnostic> {
        let Some(expr) = duration else {
            return Ok(Effect::Fixed(rule.default));
        };
        if let Some(value) = constant(expr, &self.constants).and_then(|value| value.number().ok()) {
            if value.unit != Unit::Duration {
                return Err(self.fail("timeline duration requires Duration units", expr.span));
            }
            return rule
                .validate(value.value)
                .map(Effect::Fixed)
                .map_err(|e| self.fail(e, span));
        }
        self.need(expr);
        Ok(formula::scalar(expr, &self.constants, &self.symbolic)
            .map_or(Effect::Unknown, Effect::from_expr))
    }
    fn envelope(
        &mut self,
        intrinsic: Effect,
        duration: Option<&Expr>,
        span: Span,
    ) -> Result<Effect, Diagnostic> {
        let Some(duration) = duration else {
            return Ok(intrinsic);
        };
        let requested = self.call_duration(Some(duration), Default::default(), span)?;
        match (intrinsic, requested) {
            (Effect::Fixed(body), Effect::Fixed(total)) if body > total => Err(self.fail(
                format!("function body takes {body}s, exceeding requested duration {total}s"),
                span,
            )),
            (Effect::Fixed(_), requested) => Ok(requested),
            _ => Ok(Effect::Unknown),
        }
    }
    fn call_effect(
        &mut self,
        callee: &Expr,
        args: &[(Option<String>, Expr)],
        span: Span,
    ) -> Result<Effect, Diagnostic> {
        let local = if let ExprKind::Name(name) = &callee.kind {
            self.functions.contains_key(name).then_some(name.clone())
        } else {
            None
        };
        if let Some(name) = &local {
            self.function(name)?;
        }
        let Some(ty) = lookup(callee, self.env) else {
            return Ok(Effect::Unknown);
        };
        let mut choices = Vec::new();
        for (index, signature) in ty.signatures().iter().enumerate() {
            let declaration = local
                .as_ref()
                .and_then(|name| self.functions.get(name))
                .and_then(|clauses| clauses.get(index))
                .copied();
            let defaults = declaration.and_then(|statement| {
                if let StatementKind::Function { parameters, .. } = &statement.kind {
                    Some(parameters)
                } else {
                    None
                }
            });
            let mut substitutions: BTreeMap<DurationSymbol, DurationExpr> = BTreeMap::new();
            let mut default_prefix = Effect::default();
            let mut positional = 0;
            let mut selected = true;
            for (parameter_index, parameter) in signature.parameters.iter().enumerate() {
                let explicit = args
                    .iter()
                    .find(|(name, _)| name.as_deref() == Some(parameter.name.as_str()))
                    .map(|(_, value)| value)
                    .or_else(|| {
                        let value = args
                            .iter()
                            .filter(|(name, _)| name.is_none())
                            .nth(positional)
                            .map(|(_, value)| value);
                        positional += 1;
                        value
                    });
                let value = explicit.or_else(|| {
                    defaults
                        .and_then(|parameters| parameters.get(parameter_index))
                        .and_then(|parameter| parameter.default.as_ref())
                });
                if let Some(value) = value {
                    if explicit.is_none() {
                        let default_effect = self
                            .nodes
                            .get(&(value.span.start, value.span.end))
                            .map_or(Effect::Unknown, |node| node.effect.clone());
                        let default_effect = default_effect
                            .substitute(&mut |symbol| substitutions.get(symbol).cloned())
                            .map_err(|e| self.fail(e, value.span))?;
                        default_prefix = default_prefix
                            .then(default_effect)
                            .map_err(|e| self.fail(e, value.span))?;
                    }
                    let captured =
                        declaration.and_then(|statement| self.captures.get(&statement.span.start));
                    let input_constants = if explicit.is_some() {
                        &self.constants
                    } else {
                        captured.map_or(&self.constants, |captures| &captures.constants)
                    };
                    let input_symbols = if explicit.is_some() {
                        std::borrow::Cow::Borrowed(&self.symbolic)
                    } else {
                        let mut symbols = captured
                            .map_or_else(BTreeMap::new, |captures| captures.symbols(self.source));
                        symbols.extend(
                            substitutions
                                .iter()
                                .map(|(key, value)| (key.clone(), value.clone())),
                        );
                        std::borrow::Cow::Owned(symbols)
                    };
                    if parameter.ty.is_concrete()
                        && let Some(value) = constant(value, input_constants)
                        && sanim_core::validate_value(&parameter.ty, &value).is_err()
                    {
                        selected = false;
                        break;
                    }
                    if let Some(value) = formula::scalar(value, input_constants, &input_symbols)
                        .and_then(|value| {
                            value.substitute(&mut |symbol| substitutions.get(symbol).cloned())
                        })
                    {
                        substitutions.insert(
                            DurationSymbol::Parameter(parameter.name.as_str().into()),
                            value,
                        );
                    }
                    if let Some(value) = formula::length(value, input_constants, &input_symbols) {
                        substitutions.insert(
                            DurationSymbol::Length(parameter.name.as_str().into()),
                            value,
                        );
                    }
                }
            }
            if !selected {
                continue;
            }
            let intrinsic = if let Some(statement) = declaration {
                let StatementKind::Function { body, duration, .. } = &statement.kind else {
                    unreachable!("function declaration")
                };
                if let Some(duration) = duration {
                    let captured = self
                        .captures
                        .get(&statement.span.start)
                        .cloned()
                        .unwrap_or_default();
                    let mut symbols = captured.symbols(self.source);
                    for parameter in &signature.parameters {
                        if let Some(value) = DurationExpr::parameter(parameter.name.as_str()) {
                            symbols.insert(
                                DurationSymbol::Parameter(parameter.name.as_str().into()),
                                value,
                            );
                        }
                        if let Some(value) = DurationExpr::length(parameter.name.as_str()) {
                            symbols.insert(
                                DurationSymbol::Length(parameter.name.as_str().into()),
                                value,
                            );
                        }
                    }
                    formula::scalar(duration, &captured.constants, &symbols).map_or_else(
                        || {
                            if let ExprKind::Name(name) = &duration.kind {
                                DurationExpr::parameter(name.as_str())
                                    .map_or(Effect::Unknown, Effect::from_expr)
                            } else {
                                Effect::Unknown
                            }
                        },
                        Effect::from_expr,
                    )
                } else if let (Some(first), Some(last)) = (body.first(), body.last()) {
                    self.nodes
                        .get(&(first.span.start, last.span.end))
                        .map_or(Effect::Unknown, |node| node.effect.clone())
                } else {
                    Effect::default()
                }
            } else {
                signature.duration.clone()
            };
            let called = intrinsic
                .substitute(&mut |symbol| substitutions.get(symbol).cloned())
                .map_err(|e| self.fail(e, span))?;
            let called = if signature
                .parameters
                .iter()
                .any(|parameter| parameter.name == "duration")
            {
                called
            } else {
                self.envelope(called, crate::timing::envelope(args), span)?
            };
            choices.push(
                default_prefix
                    .then(called)
                    .map_err(|e| self.fail(e, span))?,
            );
        }
        if choices.is_empty() {
            Ok(Effect::Unknown)
        } else {
            Effect::alternatives(choices).map_err(|e| self.fail(e, span))
        }
    }
    fn list_effect(
        &mut self,
        call: &crate::timing::BuiltinCall<'_>,
        span: Span,
    ) -> Result<Effect, Diagnostic> {
        use crate::list_functions::CallbackCount;
        let definition = crate::list_functions::definition(call.builtin.name)
            .expect("registered List operation");
        let intrinsic = if let Some(callback) = definition.callback {
            if let Some(callee) = call.argument(callback.index) {
                let mut effect = if let ExprKind::Name(name) = &callee.kind
                    && self.functions.contains_key(name)
                {
                    self.function(name)?
                } else {
                    lookup(callee, self.env).map_or(Effect::Unknown, |ty| {
                        Effect::alternatives(
                            ty.signatures()
                                .iter()
                                .map(|signature| signature.duration.clone()),
                        )
                        .unwrap_or(Effect::Unknown)
                    })
                };
                let callback_type = lookup(callee, self.env);
                let stateful = callback_type.is_some_and(|ty| {
                    ty.signatures()
                        .iter()
                        .any(|signature| signature.schedules || signature.mutates)
                });
                let mut item_dependent = false;
                effect
                    .substitute(&mut |symbol| {
                        let (DurationSymbol::Parameter(name) | DurationSymbol::Length(name)) =
                            symbol;
                        item_dependent |= callback_type.is_some_and(|ty| {
                            ty.signatures().iter().any(|signature| {
                                signature
                                    .parameters
                                    .iter()
                                    .any(|parameter| parameter.name == name.as_ref())
                            })
                        });
                        None
                    })
                    .map_err(|e| self.fail(e, span))?;
                if item_dependent
                    || callback_type
                        .is_some_and(|ty| ty.signatures().iter().any(|signature| signature.mutates))
                        && !matches!(effect, Effect::Fixed(_))
                {
                    effect = Effect::Unknown;
                }
                if effect.is_zero() && !stateful {
                    return if call.duration_argument().is_some() {
                        self.envelope(effect, call.duration_argument(), span)
                    } else {
                        Ok(effect)
                    };
                }
                self.need(callee);
                if let Some(items) = call.argument(0) {
                    self.need(items);
                    if let Some(mut count) = formula::length(items, &self.constants, &self.symbolic)
                    {
                        if let Some(value) = count.value() {
                            let count_value = Number {
                                value,
                                unit: Unit::Number,
                            }
                            .usize()
                            .map_err(|e| self.fail(e, span))?;
                            let visits = match callback.count {
                                CallbackCount::All => Some(count_value),
                                CallbackCount::WithoutFirst => Some(count_value.saturating_sub(1)),
                                CallbackCount::EarlyExit => None,
                            };
                            if visits.is_some_and(|visits| visits > self.limits.max_iterations) {
                                return Err(self.fail(format!("List operation has {count_value} items; iteration limit is {}",self.limits.max_iterations),span));
                            }
                        }
                        effect = match callback.count {
                            CallbackCount::All => effect.repeated(count),
                            CallbackCount::WithoutFirst => {
                                if let Some(value) = count.value() {
                                    count = DurationExpr::constant(
                                        (value - Fraction::from(1)).max(Fraction::from(0)),
                                    )
                                    .expect("bounded count");
                                    effect.repeated(count)
                                } else {
                                    Effect::Unknown
                                }
                            }
                            CallbackCount::EarlyExit => {
                                if effect.is_zero() {
                                    effect
                                } else if let Some(count) = count
                                    .value()
                                    .and_then(|value| {
                                        Number {
                                            value,
                                            unit: Unit::Number,
                                        }
                                        .usize()
                                        .ok()
                                    })
                                    .filter(|count| {
                                        *count <= sanim_core::duration::MAX_DURATION_ALTERNATIVES
                                    })
                                {
                                    if count == 0 {
                                        Effect::default()
                                    } else {
                                        Effect::alternatives((1..=count).map(|count| {
                                            effect.clone().repeated(
                                                DurationExpr::constant(Fraction::from(count))
                                                    .expect("finite count"),
                                            )
                                        }))
                                        .map_err(|e| self.fail(e, span))?
                                    }
                                } else {
                                    Effect::Unknown
                                }
                            }
                        };
                    } else {
                        effect = Effect::Unknown;
                    }
                }
                effect
            } else {
                Effect::default()
            }
        } else {
            Effect::default()
        };
        if call.duration_argument().is_some() {
            self.envelope(intrinsic, call.duration_argument(), span)
        } else {
            Ok(intrinsic)
        }
    }
    fn expression(&mut self, expr: &Expr) -> Result<Effect, Diagnostic> {
        let effect = match &expr.kind {
            ExprKind::Call { callee, args, .. } => {
                let mut effect = self.expression(callee)?;
                for (_, value) in args {
                    let next = self.expression(value)?;
                    effect = self.sum(effect, next, expr.span)?;
                }
                if let Some(call) = crate::timing::builtin_call(callee, args, self.native_methods) {
                    let timing = if crate::list_functions::definition(call.builtin.name).is_some() {
                        self.list_effect(&call, expr.span)?
                    } else {
                        self.call_duration(
                            call.duration_argument(),
                            call.builtin.timing,
                            expr.span,
                        )?
                    };
                    effect = self.sum(effect, timing, expr.span)?;
                } else {
                    let called = self.call_effect(callee, args, expr.span)?;
                    if !matches!(called, Effect::Fixed(_))
                        || lookup(callee, self.env).is_some_and(|ty| {
                            ty.signatures().iter().any(|signature| signature.mutates)
                        })
                    {
                        self.call_dependencies(callee, args);
                    }
                    effect = self.sum(effect, called, expr.span)?;
                }
                effect
            }
            ExprKind::Reflect { value, .. } => {
                let effect = self.expression(value)?;
                if !effect.is_zero() {
                    return Err(Diagnostic::new(
                        "S113",
                        "a reflected default must have zero timeline duration; schedule animations outside reflect(...) instead",
                        self.source,
                        expr.span,
                    ));
                }
                effect
            }
            ExprKind::Unary(_, value) | ExprKind::Member(value, _) => self.expression(value)?,
            ExprKind::Binary(op, a, b) => {
                let left = self.expression(a)?;
                let right = self.expression(b)?;
                if matches!(op.as_str(), "and" | "or") && !right.is_zero() {
                    match constant(a, &self.constants) {
                        Some(Value::Bool(v)) if op == "and" && !v || op == "or" && v => left,
                        Some(Value::Bool(_)) => self.sum(left, right, expr.span)?,
                        _ => {
                            self.need(a);
                            Effect::Unknown
                        }
                    }
                } else {
                    self.sum(left, right, expr.span)?
                }
            }
            ExprKind::Index(a, b) => {
                let a = self.expression(a)?;
                let b = self.expression(b)?;
                self.sum(a, b, expr.span)?
            }
            ExprKind::Template(parts) => {
                let mut out = Effect::default();
                for part in parts {
                    if let sanim_core::TemplatePart::Hole { value, format, .. } = part {
                        let next = self.expression(value)?;
                        out = self.sum(out, next, expr.span)?;
                        if let Some(format) = format {
                            let next = self.expression(format)?;
                            out = self.sum(out, next, expr.span)?;
                        }
                    }
                }
                out
            }
            ExprKind::List(values) => {
                let mut out = Effect::default();
                for value in values {
                    let next = self.expression(value)?;
                    out = self.sum(out, next, expr.span)?;
                }
                out
            }
            ExprKind::Record(values) => {
                let mut out = Effect::default();
                for (_, value) in values {
                    let next = self.expression(value)?;
                    out = self.sum(out, next, expr.span)?;
                }
                out
            }
            ExprKind::Dict(values) => {
                let mut out = Effect::default();
                for (key, value) in values {
                    let a = self.expression(key)?;
                    let b = self.expression(value)?;
                    let next = self.sum(a, b, expr.span)?;
                    out = self.sum(out, next, expr.span)?;
                }
                out
            }
            _ => Effect::default(),
        };
        self.record(expr.span, effect)
    }
}
pub(crate) fn lookup<'a>(expr: &Expr, env: &'a Environment) -> Option<&'a Type> {
    match &expr.kind {
        ExprKind::Name(name) => env.get(name),
        ExprKind::Member(base, name) => {
            if let Type::Record(fields) = lookup(base, env)? {
                fields.get(name)
            } else {
                None
            }
        }
        _ => None,
    }
}
pub(crate) fn constant(expr: &Expr, values: &BTreeMap<String, Value>) -> Option<Value> {
    match &expr.kind {
        ExprKind::Number(text) => Number::parse(text).ok().map(Value::Number),
        ExprKind::Bool(v) => Some(Value::Bool(*v)),
        ExprKind::Name(name) => values
            .get(name)
            .or_else(|| prelude::values().get(name.as_str()))
            .cloned(),
        ExprKind::Unary(op, value) => math::operations::unary(op, constant(value, values)?).ok(),
        ExprKind::Binary(op, a, b) => {
            let left = constant(a, values)?;
            if let Value::Bool(value) = left
                && (op == "and" && !value || op == "or" && value)
            {
                return Some(Value::Bool(value));
            }
            math::operations::binary(op, left, constant(b, values)?).ok()
        }
        _ => None,
    }
}
pub(crate) fn has_return(body: &[Statement]) -> bool {
    body.iter().any(|statement| match &statement.kind {
        StatementKind::Return(_) => true,
        StatementKind::If { yes, no, .. } => has_return(yes) || has_return(no),
        StatementKind::For { body, .. } | StatementKind::Repeat { body, .. } => has_return(body),
        StatementKind::Match { cases, .. } => cases.iter().any(|case| has_return(&case.body)),
        _ => false,
    })
}

#[cfg(test)]
mod duration_memory_tests {
    use super::*;
    #[test]
    fn published_duration_formulas_obey_the_aggregate_semantic_budget() {
        let limits = Limits::default();
        let published_function_nodes = (0..16)
            .map(|i| format!("fn segment_{i}(n:Number):Unit{{repeat n{{wait(250ms)}}}}\n"))
            .collect::<String>();
        let published_root_nodes =
            format!("let n=reflect(4)\n{}", "repeat n{wait(250ms)}\n".repeat(16));
        let published_clause_roots = (0..16)
            .map(|i| format!("@duration(n*250ms)\nfn segment_{i}(n:Number):Unit{{wait(1s)}}\n"))
            .collect::<String>();
        for text in [
            published_function_nodes,
            published_root_nodes,
            published_clause_roots,
        ] {
            let module = crate::parser::parse("main.sanim", &text, limits.max_parse_steps).unwrap();
            let remaining = Cell::new(1024 * 1024);
            let analysis = analyze(
                "main.sanim",
                &module,
                &Environment::new(),
                &BTreeMap::new(),
                &BTreeMap::new(),
                &remaining,
                &limits,
            )
            .unwrap();
            let inline = analysis.nodes.len() * std::mem::size_of::<NodeDuration>();
            assert!(
                1024 * 1024 - remaining.get() > inline,
                "published formula heaps must add to inline NodeDuration storage"
            );
            let tiny = Cell::new(inline);
            let error = analyze(
                "main.sanim",
                &module,
                &Environment::new(),
                &BTreeMap::new(),
                &BTreeMap::new(),
                &tiny,
                &limits,
            )
            .err()
            .expect("duration heaps exceed the inline-only budget");
            assert_eq!(error.code, "S105");
            assert!(error.message.contains("budget"), "{error:?}");
        }
    }
}
