//! Selective timeline evaluation reuses the value evaluator and canonical AST.
use super::*;
impl Runtime<'_, '_> {
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
        Ok(())
    }
    pub(super) fn duration_effects(&mut self, expr: &Expr, env: &Env) -> Result<(), Diagnostic> {
        self.work(expr.span)?;
        if let TimelineEffect::Fixed(duration) = self.timeline_effect(expr.span) {
            return self.advance_duration(duration, expr.span);
        }
        match &expr.kind {
            ExprKind::Call { callee, args, .. } => {
                if let ExprKind::Name(name) = &callee.kind
                    && let Some(builtin) = prelude::builtin(name)
                {
                    if matches!(
                        builtin.execute,
                        prelude::Execution::Play | prelude::Execution::Wait(_)
                    ) {
                        if !self.root_active {
                            return Err(self.error(
                                "S115",
                                "imported modules cannot schedule animation while being imported",
                                expr.span,
                            ));
                        }
                        let wait = matches!(builtin.execute, prelude::Execution::Wait(_));
                        let mut duration = None;
                        for (index, (label, value)) in args.iter().enumerate() {
                            if label.as_deref() == Some("duration")
                                || wait && index == 0 && label.is_none()
                            {
                                duration = Some(
                                    self.expression(value, env)?
                                        .number()
                                        .map_err(|e| self.error("S304", e, value.span))?
                                        .value,
                                );
                            } else {
                                self.duration_effects(value, env)?;
                            }
                        }
                        let duration = duration.unwrap_or(Fraction::from(1));
                        if !wait && duration <= Fraction::from(0) {
                            return Err(self.error(
                                "S304",
                                "play duration must be positive",
                                expr.span,
                            ));
                        }
                        return self.advance_duration(duration, expr.span);
                    }
                    for (_, value) in args {
                        self.duration_effects(value, env)?;
                    }
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
                    if self.constructors.contains_key(&key) {
                        for (_, value) in args {
                            self.duration_effects(value, env)?;
                        }
                        return Ok(());
                    }
                    return Err(self.error("S305", format!("unknown function '{key}'"), expr.span));
                };
                let needed = self
                    .duration_needed
                    .get(&function.module)
                    .expect("function duration scope");
                let parameters = function
                    .parameters
                    .iter()
                    .map(|(name, _, _)| (name.clone(), needed.contains(name)))
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
                    if parameters
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
            ExprKind::Unary(_, value) | ExprKind::Member(value, _) => {
                self.duration_effects(value, env)?
            }
            ExprKind::Binary(op, left, right)
                if matches!(op.as_str(), "and" | "or")
                    && !self.timeline_effect(right.span).is_zero() =>
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
        if crate::effects::has_return(body) {
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
pub(super) fn duration_needs(loaded: &[Loaded]) -> BTreeMap<String, BTreeSet<String>> {
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
