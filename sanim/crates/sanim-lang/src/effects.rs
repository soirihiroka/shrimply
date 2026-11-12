//! Infer timeline effects over the canonical AST, preserving lexical scopes and control flow.
use crate::{
    Limits,
    ast::*,
    checker::{Checked, Environment},
    prelude,
};
use sanim_core::{
    Diagnostic, Fraction, NodeDuration, Number, Span, TimelineEffect as Effect, Type, Unit, Value,
    math,
};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub(crate) struct Analysis {
    pub nodes: Vec<NodeDuration>,
    pub functions: BTreeMap<String, Effect>,
    pub needed: BTreeSet<String>,
    pub value_dependencies: BTreeMap<String, BTreeSet<String>>,
}
pub(crate) fn analyze(
    source: &str,
    module: &Module,
    env: &Environment,
    _imports: &BTreeMap<String, Arc<Checked>>,
    budget: &Cell<usize>,
    limits: &Limits,
) -> Result<Analysis, Diagnostic> {
    let mut functions = BTreeMap::new();
    let mut captures = BTreeMap::new();
    let mut constants = BTreeMap::new();

    for statement in &module.statements {
        match &statement.kind {
            StatementKind::Bind {
                name,
                value,
                reflect,
                ..
            } => {
                if !reflect && let Some(value) = constant(value, &constants) {
                    constants.insert(name.clone(), value);
                } else {
                    constants.remove(name);
                }
            }
            StatementKind::Function {
                name,
                parameters,
                body,
                ..
            } => {
                functions.insert(name.clone(), statement);
                let mut names = BTreeSet::new();
                for statement in body {
                    statement.visit_expressions(&mut |expr| {
                        if let ExprKind::Name(name) = &expr.kind {
                            names.insert(name.clone());
                        }
                    });
                }
                for (parameter, _, default) in parameters {
                    names.remove(parameter);
                    if let Some(expr) = default {
                        value_references(expr, &mut names);
                    }
                }
                captures.insert(
                    name.clone(),
                    constants
                        .iter()
                        .filter(|(name, _)| names.contains(*name))
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect(),
                );
            }
            _ => {}
        }
    }
    let mut analyzer = Analyzer {
        source,
        env,
        functions,
        captures,

        budget,
        limits,
        done: BTreeMap::new(),
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
        functions: analyzer.done,
        needed: analyzer.needed,
        value_dependencies: analyzer.bindings,
    })
}
struct Analyzer<'a> {
    source: &'a str,
    env: &'a Environment,
    functions: BTreeMap<String, &'a Statement>,
    captures: BTreeMap<String, BTreeMap<String, Value>>,
    budget: &'a Cell<usize>,
    limits: &'a Limits,
    done: BTreeMap<String, Effect>,
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
    fn record(&mut self, span: Span, effect: Effect) -> Result<Effect, Diagnostic> {
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
        self.nodes
            .insert((span.start, span.end), NodeDuration { span, effect });
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
                StatementKind::Bind { name, value, .. } => self.edge(name, value),
                StatementKind::Function {
                    name,
                    body,
                    parameters,
                    ..
                } => {
                    for (parameter, _, default) in parameters {
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
            return Ok(*effect);
        }
        if self.active.len() >= self.limits.max_call_depth || !self.active.insert(name.into()) {
            return Err(self.fail(
                "function duration analysis exceeds bounded call depth",
                Span::default(),
            ));
        }
        let StatementKind::Function {
            body, parameters, ..
        } = &self.functions[name].kind
        else {
            unreachable!()
        };
        let previous = std::mem::replace(
            &mut self.constants,
            self.captures.get(name).cloned().unwrap_or_default(),
        );
        let mut effect = self.block(body)?;
        for (_, _, default) in parameters {
            if let Some(default) = default
                && !self.expression(default)?.is_zero()
            {
                effect = Effect::Dependent;
            }
        }
        self.constants = previous;
        self.active.remove(name);
        self.done.insert(name.into(), effect);
        Ok(effect)
    }
    fn block(&mut self, body: &[Statement]) -> Result<Effect, Diagnostic> {
        let previous = self.constants.clone();
        let mut effect = Effect::default();
        let mut may_return = false;
        for statement in body {
            let next = self.statement(statement)?;
            effect = if may_return && !next.is_zero() {
                Effect::Dependent
            } else {
                self.sum(effect, next, statement.span)?
            };
            may_return |= has_return(std::slice::from_ref(statement));
            if let StatementKind::Bind {
                name,
                value,
                reflect,
                ..
            } = &statement.kind
            {
                if !reflect && let Some(value) = constant(value, &self.constants) {
                    self.constants.insert(name.clone(), value);
                } else {
                    self.constants.remove(name);
                }
            }
        }
        self.constants = previous;
        Ok(effect)
    }
    fn statement(&mut self, statement: &Statement) -> Result<Effect, Diagnostic> {
        let effect = match &statement.kind {
            StatementKind::Bind { value, .. }
            | StatementKind::Expr(value)
            | StatementKind::Return(value) => self.expression(value)?,
            StatementKind::If { condition, yes, no } => {
                let input = self.expression(condition)?;
                let yes_effect = self.block(yes)?;
                let no_effect = self.block(no)?;
                let branch = match constant(condition, &self.constants) {
                    Some(Value::Bool(true)) => yes_effect,
                    Some(Value::Bool(false)) => no_effect,
                    _ if yes_effect == no_effect => yes_effect,
                    _ => Effect::Dependent,
                };
                let effect = self.sum(input, branch, statement.span)?;
                if effect == Effect::Dependent || has_return(yes) || has_return(no) {
                    self.need(condition);
                }
                effect
            }
            StatementKind::Repeat {
                count,
                period,
                body,
            } => {
                self.need(count);
                if let Some(period) = period {
                    self.need(period);
                }
                let input = self.expression(count)?;
                let body_effect = self.block(body)?;
                let period_effect = if let Some(period) = period {
                    self.expression(period)?
                } else {
                    Effect::default()
                };
                let count_value = constant(count, &self.constants)
                    .and_then(|v| v.number().ok())
                    .and_then(|v| v.usize().ok());
                let repeated = if let Some(count) = count_value {
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
                    let step = if let Some(period) = period {
                        constant(period, &self.constants)
                            .and_then(|v| v.number().ok())
                            .map(|n| Effect::Fixed(n.value))
                            .unwrap_or(Effect::Dependent)
                    } else {
                        body_effect
                    };
                    if count > 0 && has_return(body)
                        || period.is_some() && body_effect == Effect::Dependent
                    {
                        Effect::Dependent
                    } else if let Effect::Fixed(step) = step {
                        if period.is_some() {
                            if step <= Fraction::from(0) {
                                return Err(
                                    self.fail("timer period must be positive", statement.span)
                                );
                            }
                            if count > 0
                                && let Effect::Fixed(body) = body_effect
                                && body > step
                            {
                                return Err(
                                    self.fail("timer body exceeds its period", statement.span)
                                );
                            }
                        }
                        Effect::Fixed(
                            math::arithmetic("*", step, Fraction::from(count))
                                .map_err(|e| self.fail(e, statement.span))?,
                        )
                    } else {
                        Effect::Dependent
                    }
                } else {
                    Effect::Dependent
                };
                self.sum(
                    self.sum(input, period_effect, statement.span)?,
                    repeated,
                    statement.span,
                )?
            }
            StatementKind::For { iterable, body, .. } => {
                self.need(iterable);
                self.expression(iterable)?;
                self.block(body)?;
                Effect::Dependent
            }
            StatementKind::Match { value, cases } => {
                let input = self.expression(value)?;
                let mut branch = None;
                let mut returns = false;
                for case in cases {
                    let effect = self.block(&case.body)?;
                    returns |= has_return(&case.body);
                    branch = Some(match branch {
                        None => effect,
                        Some(previous) if previous == effect => effect,
                        _ => Effect::Dependent,
                    });
                }
                let effect = self.sum(input, branch.unwrap_or_default(), statement.span)?;
                if effect == Effect::Dependent || returns {
                    self.need(value);
                }
                effect
            }
            StatementKind::Function { .. }
            | StatementKind::Alias { .. }
            | StatementKind::Enum { .. }
            | StatementKind::Import { .. } => Effect::default(),
        };
        self.record(statement.span, effect)
    }
    fn call_dependencies(&mut self, callee: &Expr, args: &[(Option<String>, Expr)]) {
        let Some(Type::Function(signature)) = lookup(callee, self.env) else {
            return;
        };
        let parameters = signature
            .parameters
            .iter()
            .enumerate()
            .map(|(index, (name, _))| {
                (name.clone(), signature.duration_parameters.contains(&index))
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
    fn expression(&mut self, expr: &Expr) -> Result<Effect, Diagnostic> {
        let effect = match &expr.kind {
            ExprKind::Call { callee, args, .. } => {
                let mut effect = self.expression(callee)?;
                for (_, value) in args {
                    let next = self.expression(value)?;
                    effect = self.sum(effect, next, expr.span)?;
                }
                if let ExprKind::Name(name) = &callee.kind {
                    if let Some(builtin) = prelude::builtin(name) {
                        if matches!(
                            builtin.execute,
                            prelude::Execution::Wait(_) | prelude::Execution::Play
                        ) {
                            let wait = matches!(builtin.execute, prelude::Execution::Wait(_));
                            let duration = args
                                .iter()
                                .find(|(name, _)| name.as_deref() == Some("duration"))
                                .map(|(_, v)| v)
                                .or_else(|| {
                                    if wait {
                                        args.first().map(|(_, v)| v)
                                    } else {
                                        None
                                    }
                                });
                            let timing = duration
                                .map(|v| {
                                    constant(v, &self.constants)
                                        .and_then(|v| v.number().ok())
                                        .filter(|n| n.unit == Unit::Duration)
                                        .map(|n| Effect::Fixed(n.value))
                                        .unwrap_or(Effect::Dependent)
                                })
                                .unwrap_or(Effect::Fixed(Fraction::from(1)));
                            if let Effect::Fixed(value) = timing
                                && !wait
                                && value <= Fraction::from(0)
                            {
                                return Err(self.fail("play duration must be positive", expr.span));
                            }
                            if timing == Effect::Dependent
                                && let Some(duration) = duration
                            {
                                self.need(duration);
                            }
                            effect = self.sum(effect, timing, expr.span)?;
                        }
                    } else if self.functions.contains_key(name) {
                        let called = self.function(name)?;
                        effect = self.sum(effect, called, expr.span)?;
                        if called == Effect::Dependent {
                            self.call_dependencies(callee, args);
                        }
                    }
                } else if let Some(Type::Function(signature)) = lookup(callee, self.env) {
                    effect = self.sum(effect, signature.duration, expr.span)?;
                    if signature.duration == Effect::Dependent {
                        self.call_dependencies(callee, args);
                    }
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
                            Effect::Dependent
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
fn lookup<'a>(expr: &Expr, env: &'a Environment) -> Option<&'a Type> {
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
fn constant(expr: &Expr, values: &BTreeMap<String, Value>) -> Option<Value> {
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
