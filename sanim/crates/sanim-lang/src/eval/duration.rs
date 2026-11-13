//! Selective timeline evaluation reuses the value evaluator and canonical AST.
use super::*;
#[derive(Default)]
pub(super) struct CallBoundary {
    pub module: String,
    pub span: Span,
    pub start: Fraction,
    pub end: Fraction,
    pub easing: Easing,
}
impl Runtime<'_, '_> {
    pub(super) fn observe_call_horizon(&mut self, end: Fraction) {
        if let Some(horizon) = self.call_horizons.last_mut() {
            *horizon = (*horizon).max(end);
        }
    }
    pub(super) fn timeline_effect(&self, span: Span) -> TimelineEffect {
        let Some(module) = self.loaded.iter().find(|m| m.id == self.module) else {
            return TimelineEffect::Dependent;
        };
        let nodes = &module.checked.durations;
        nodes
            .binary_search_by_key(&(span.start, span.end), |n| (n.span.start, n.span.end))
            .ok()
            .map_or(TimelineEffect::Dependent, |index| nodes[index].effect)
    }
    pub(super) fn duration_binding_needed(&self, name: &str) -> bool {
        self.duration_needed
            .get(&self.module)
            .is_some_and(|needed| needed.contains(name))
    }
    pub(super) fn advance_duration(
        &mut self,
        duration: Fraction,
        span: Span,
    ) -> Result<(), Diagnostic> {
        self.work(span)?;
        if !self.root_active && duration != Fraction::from(0) {
            return Err(self.error(
                "S115",
                "imported modules cannot schedule animation while being imported",
                span,
            ));
        }
        self.cursor =
            math::timeline_end(self.cursor, duration).map_err(|e| self.error("S304", e, span))?;
        self.animation.duration = self.animation.duration.max(self.cursor);
        self.observe_call_horizon(self.cursor);
        Ok(())
    }
    pub(super) fn finish_call_duration(
        &mut self,
        start: Fraction,
        requested: Option<Fraction>,
        horizon: Fraction,
        span: Span,
    ) -> Result<(), Diagnostic> {
        self.last_call.module.clone_from(&self.module);
        self.last_call.span = span;
        self.last_call.start = start;
        self.last_call.end = self.cursor;
        self.last_call.easing = Easing::default();
        if requested == Some(Fraction::from(0)) && self.cursor == start && horizon <= start {
            return Ok(());
        }
        if let Some(requested) = requested {
            let end =
                math::timeline_end(start, requested).map_err(|e| self.error("S304", e, span))?;
            if self.cursor.max(horizon) > end {
                return Err(self.error(
                    "S304",
                    format!(
                        "call body takes {}s, exceeding requested timeline duration {requested}s",
                        math::arithmetic("-", self.cursor.max(horizon), start)
                            .map_err(|e| self.error("S304", e, span))?
                    ),
                    span,
                ));
            }
            if self.cursor < end {
                let remaining = math::arithmetic("-", end, self.cursor)
                    .map_err(|e| self.error("S304", e, span))?;
                self.advance_duration(remaining, span)?;
            }
        }
        if self.cursor.max(horizon) > start {
            self.map_time_until(start, self.cursor.max(horizon), span, TimeMapKind::Schedule)?;
        }
        self.last_call.end = self.cursor;
        Ok(())
    }
    pub(super) fn duration_effects(&mut self, expr: &Expr, env: &Env) -> Result<(), Diagnostic> {
        self.work(expr.span)?;
        if !self.expression_mutates(expr)
            && let TimelineEffect::Fixed(duration) = self.timeline_effect(expr.span)
        {
            return self.advance_duration(duration, expr.span);
        }
        match &expr.kind {
            ExprKind::Call { callee, args, .. } => {
                if let Some(call) = crate::timing::builtin_call(callee, args) {
                    if let Some(receiver) = call.receiver {
                        self.duration_effects(receiver, env)?;
                    }
                    let duration_expr = call.duration_argument();
                    let mut requested = None;
                    for (_, value) in args {
                        if duration_expr.is_some_and(|expr| std::ptr::eq(expr, value)) {
                            requested = Some(self.expression(value, env)?);
                        } else {
                            self.duration_effects(value, env)?;
                        }
                    }
                    let duration = call
                        .builtin
                        .timing
                        .value(requested.as_ref())
                        .map_err(|e| self.error("S304", e, expr.span))?;
                    self.advance_duration(duration, expr.span)?;
                    return Ok(());
                }
                let callee_value = self.expression(callee, env)?;
                let Value::Function(key) = callee_value else {
                    return Err(self.error(
                        "S111",
                        "duration analysis requires a direct function call",
                        callee.span,
                    ));
                };
                let Some(function) = self.functions.get(&key).cloned() else {
                    if let Some(ty) = self.constructors.get(&key) {
                        let duration = if !matches!(ty,Type::Record(fields) if fields.contains_key("duration"))
                        {
                            crate::timing::envelope(args)
                        } else {
                            None
                        };
                        let mut requested = None;
                        for (_, value) in args {
                            if duration.is_some_and(|expr| std::ptr::eq(expr, value)) {
                                requested = Some(self.expression(value, env)?);
                            } else {
                                self.duration_effects(value, env)?;
                            }
                        }
                        let total = crate::timing::Timing::default()
                            .value(requested.as_ref())
                            .map_err(|e| self.error("S304", e, expr.span))?;
                        return self.advance_duration(total, expr.span);
                    }
                    return Err(self.error("S305", format!("unknown function '{key}'"), expr.span));
                };
                let mutates = self.function_mutates(&function);
                let needed = self
                    .duration_needed
                    .get(&function.module)
                    .expect("function duration scope");
                let parameters = function
                    .parameters
                    .iter()
                    .map(|(name, _, _)| (name.clone(), mutates || needed.contains(name)))
                    .collect::<Vec<_>>();
                let mut evaluated = vec![];
                let mut provided = BTreeSet::new();
                let mut positional = 0;
                for (label, value) in args {
                    let parameter = if let Some(label) = label {
                        label.clone()
                    } else {
                        let name = parameters
                            .get(positional)
                            .ok_or_else(|| {
                                self.error("S124", "too many function arguments", expr.span)
                            })?
                            .0
                            .clone();
                        positional += 1;
                        name
                    };
                    provided.insert(parameter.clone());
                    if matches!(parameter.as_str(), "duration" | "ease")
                        && !parameters.iter().any(|(name, _)| name == &parameter)
                        || parameters
                            .iter()
                            .any(|(name, needed)| name == &parameter && *needed)
                    {
                        evaluated.push((Some(parameter), self.expression(value, env)?));
                    } else {
                        self.duration_effects(value, env)?;
                    }
                }
                self.invoke(&key, &evaluated, expr.span, false, Some(&provided))?;
            }
            ExprKind::Unary(_, value)
            | ExprKind::Member(value, _)
            | ExprKind::Reflect { value, .. } => self.duration_effects(value, env)?,
            ExprKind::Binary(op, left, right)
                if matches!(op.as_str(), "and" | "or")
                    && (!self.timeline_effect(right.span).is_zero()
                        || self.expression_mutates(right)) =>
            {
                let value = self
                    .expression(left, env)?
                    .boolean()
                    .map_err(|e| self.error("S305", e, left.span))?;
                if op == "and" && value || op == "or" && !value {
                    self.duration_effects(right, env)?;
                }
            }
            ExprKind::Binary(_, left, right) | ExprKind::Index(left, right) => {
                self.duration_effects(left, env)?;
                self.duration_effects(right, env)?;
            }
            ExprKind::List(values) => {
                for value in values {
                    self.duration_effects(value, env)?;
                }
            }
            ExprKind::Record(values) => {
                for (_, value) in values {
                    self.duration_effects(value, env)?;
                }
            }
            ExprKind::Dict(values) => {
                for (key, value) in values {
                    self.duration_effects(key, env)?;
                    self.duration_effects(value, env)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn fixed_block_duration(&self, body: &[Statement]) -> Option<Fraction> {
        if body
            .iter()
            .any(|statement| self.statement_stateful(statement))
            || crate::effects::has_return(body)
        {
            return None;
        }
        body.iter().try_fold(Fraction::from(0), |sum, s| {
            match self.timeline_effect(s.span) {
                TimelineEffect::Fixed(value) => math::timeline_end(sum, value).ok(),
                _ => None,
            }
        })
    }
}

/// Propagate demanded exports and helper return dependencies through module aliases.
pub(crate) fn duration_needs(loaded: &[Loaded]) -> BTreeMap<String, BTreeSet<String>> {
    let mut needed = loaded
        .iter()
        .map(|module| (module.id.clone(), module.checked.duration_needed.clone()))
        .collect::<BTreeMap<_, _>>();
    loop {
        let before = needed.values().map(BTreeSet::len).sum::<usize>();
        for module in loaded {
            for name in needed[&module.id].clone() {
                if let Some(dependencies) = module.checked.value_dependencies.get(&name) {
                    needed
                        .get_mut(&module.id)
                        .expect("module duration scope")
                        .extend(dependencies.iter().cloned());
                }
                let (base, member) = name
                    .split_once('.')
                    .map_or((name.as_str(), None), |(base, member)| (base, Some(member)));
                if let Some(target) = module.imports.get(base) {
                    if let Some(member) = member {
                        needed
                            .get_mut(target)
                            .expect("import duration scope")
                            .insert(member.into());
                    } else if let Some(imported) = loaded.iter().find(|m| m.id == *target) {
                        needed
                            .get_mut(target)
                            .expect("import duration scope")
                            .extend(imported.checked.exports.keys().cloned());
                    }
                } else if member.is_some() && module.checked.value_dependencies.contains_key(base) {
                    needed
                        .get_mut(&module.id)
                        .expect("module duration scope")
                        .insert(base.into());
                }
            }
        }
        if needed.values().map(BTreeSet::len).sum::<usize>() == before {
            break;
        }
    }
    needed
}

/// Timing that observes scene values needs the canonical track/layout evaluator.
/// Select that evaluator before execution instead of retrying a partial run.
pub(crate) fn scene_dependency(
    loaded: &[Loaded],
    needed: &BTreeMap<String, BTreeSet<String>>,
) -> Option<(String, Span)> {
    for module in loaded {
        let names = &needed[&module.id];
        let mut found = None;
        for statement in &module.ast.statements {
            statement.visit(&mut |statement| {
                if let StatementKind::Bind { name, .. } = &statement.kind
                    && names.contains(name)
                    && module
                        .checked
                        .binding_types
                        .get(&statement.span.start)
                        .is_some_and(Type::contains_element)
                {
                    found.get_or_insert(statement.span);
                }
            });
        }
        if let Some(span) = found {
            return Some((module.id.clone(), span));
        }
        if names.iter().any(|name| {
            module
                .checked
                .exports
                .get(name)
                .is_some_and(Type::contains_element)
                || prelude::builtin(name).is_some_and(|builtin| builtin.result.contains_element())
        }) {
            return Some((module.id.clone(), Span::default()));
        }
    }
    None
}
