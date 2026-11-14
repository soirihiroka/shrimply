//! Read-only, structurally shared signal arguments and escaping return values.
use super::*;
use reactive::{Cell, Node as Stored, Signal};

impl Runtime<'_, '_> {
    pub(super) fn charge_signal_metadata(
        &mut self,
        name: &str,
        expression: &Expr,
        scope: &Env,
        ty: &Type,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let mut remaining = self.retained_bytes;
        sanim_core::type_budget::charge(ty, &mut remaining)
            .map_err(|message| self.error("S303", message, span))?;
        for ty in self.type_bindings.values() {
            sanim_core::type_budget::charge(ty, &mut remaining)
                .map_err(|message| self.error("S303", message, span))?;
        }
        let metadata = serde_json::to_vec(expression)
            .map_err(|error| self.error("S303", error.to_string(), span))?
            .len()
            .saturating_add(
                scope
                    .len()
                    .saturating_mul(std::mem::size_of::<Binding>() + std::mem::size_of::<String>()),
            )
            .saturating_add(scope.keys().map(String::len).sum::<usize>())
            .saturating_add(
                self.type_bindings
                    .keys()
                    .map(|name| name.len() + std::mem::size_of::<String>())
                    .sum::<usize>(),
            )
            .saturating_add(
                self.identity
                    .iter()
                    .map(|part| part.len() + std::mem::size_of::<String>())
                    .sum::<usize>(),
            )
            .saturating_add(name.len() + self.module.len())
            .saturating_add(self.source_calls.len() * std::mem::size_of::<crate::SourceLocation>())
            .saturating_add(std::mem::size_of::<Signal>());
        self.retained_bytes = remaining.checked_sub(metadata).ok_or_else(|| {
            self.error(
                "S303",
                "signal metadata exceeds the configured retained-data budget",
                span,
            )
        })?;
        self.retained_items = self.retained_items.checked_sub(1).ok_or_else(|| {
            self.error(
                "S303",
                "signal metadata exceeds the configured retained-value budget",
                span,
            )
        })?;
        Ok(())
    }
    pub(super) fn observed_signal(&self, expr: &Expr) -> Option<sanim_core::SignalRef> {
        let (capturing, observations) = self.signal_observations.last()?;
        if *capturing {
            return None;
        }
        observations
            .get(&(self.module.clone(), expr.span.start, expr.span.end))
            .cloned()
    }
    pub(super) fn has_live_observation(&self, expr: &Expr) -> bool {
        let mut live = false;
        expr.visit_while(&mut |expr| {
            if let Some(signal) = self.observed_signal(expr) {
                live |= match self.reactive.nodes.get(&signal.id) {
                    Some(Stored::Signal(signal)) => {
                        signal.temporal
                            || !signal.dependencies.is_empty()
                            || signal.cached.is_none()
                    }
                    _ => true,
                };
                return false;
            }
            !live
        });
        live
    }
    pub(super) fn sample_signal(
        &mut self,
        mut value: Value,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        for _ in 0..self.limits.max_call_depth {
            let Value::Signal(signal) = value else {
                return Ok(value);
            };
            value = self.read_reactive(signal.id, span)?;
        }
        Err(self.error(
            "S140",
            "signal handle dependency depth exceeds the configured call-depth limit",
            span,
        ))
    }
    pub(super) fn expression(&mut self, expr: &Expr, env: &Env) -> Result<Value, Diagnostic> {
        let value = if let Some(signal) = self.observed_signal(expr) {
            Value::Signal(signal)
        } else {
            self.expression_raw(expr, env)?
        };
        let value = self.sample_signal(value, expr.span)?;
        self.contextual_template(value, expr.span)
    }
    pub(super) fn parameter_type(
        &self,
        key: &str,
        name: &str,
        span: Span,
    ) -> Result<Type, Diagnostic> {
        let function = self
            .functions
            .get(key)
            .and_then(|group| group.clauses.first())
            .ok_or_else(|| self.error("S140", "unknown signal helper", span))?;
        let signature = self
            .loaded
            .iter()
            .find(|module| module.id == function.module)
            .and_then(|module| module.checked.exports.get(&function.name));
        let Some(signature) = signature else {
            return Err(self.error("S140", "function is missing its checked signature", span));
        };
        let substitutions = self.call_type_bindings(span);
        let types = signature.signatures().iter().filter_map(|signature| {
            signature
                .parameters
                .iter()
                .find(|parameter| parameter.name == name)
                .map(|parameter| parameter.ty.substitute(&substitutions))
        });
        let ty = Type::union(types).map_err(|error| self.error("S140", error, span))?;
        Ok(if signature.signatures().len() > 1 {
            ty.widen_literals()
        } else {
            ty
        })
    }

    fn reserve_signal(&mut self, node: Stored, span: Span) -> Result<u64, Diagnostic> {
        if self.reactive.nodes.len() >= self.limits.data.max_values {
            return Err(self.error(
                "S303",
                "live signals exceed the configured data-value limit",
                span,
            ));
        }
        let id = self.reactive.next;
        self.reactive.next = id
            .checked_add(1)
            .ok_or_else(|| self.error("S303", "reactive identity counter overflow", span))?;
        self.reactive.nodes.insert(id, node);
        self.reactive.touch();
        Ok(id)
    }
    pub(super) fn insert_cell(
        &mut self,
        name: &str,
        ty: &Type,
        value: Value,
        span: Span,
    ) -> Result<u64, Diagnostic> {
        self.retain_binding_value(&value, span)?;
        self.reserve_signal(
            Stored::Cell(Cell {
                value,
                ty: Arc::new(ty.clone()),
                name: name.into(),
                module: self.module.clone(),
                span,
                sampled_at: None,
                creation_background_depth: self.background_depth,
            }),
            span,
        )
    }
    pub(super) fn capture_signal(
        &mut self,
        expr: &Expr,
        env: &Env,
        ty: &Type,
    ) -> Result<Value, Diagnostic> {
        self.work(expr.span)?;
        if let ExprKind::Name(name) = &expr.kind {
            let id = match env.get(name) {
                Some(Binding::Reactive(id)) => Some(*id),
                Some(Binding::Value(Value::Signal(signal))) => Some(signal.id),
                _ => None,
            };
            if let Some(id) = id {
                let actual = match self.reactive.nodes.get(&id) {
                    Some(Stored::Cell(cell)) => cell.ty.as_ref(),
                    Some(Stored::Signal(signal)) => signal.ty.as_ref(),
                    None => {
                        return Err(self.error(
                            "S140",
                            "signal input has no live runtime state",
                            expr.span,
                        ));
                    }
                };
                if !ty.accepts(actual) {
                    return Err(self.error(
                        "S140",
                        format!("expected signal of {ty}, found {actual}"),
                        expr.span,
                    ));
                }
                return Ok(Value::Signal(sanim_core::SignalRef {
                    id,
                    ty: Arc::new(ty.clone()),
                }));
            }
        }
        if let ExprKind::Call { callee, .. } = &expr.kind {
            let declared = self
                .loaded
                .iter()
                .find(|module| module.id == self.module)
                .and_then(|module| crate::effects::lookup(callee, &module.checked.exports));
            if declared.is_some_and(|ty| {
                ty.signatures()
                    .iter()
                    .any(|signature| matches!(signature.result.as_ref(), Type::Signal(_)))
            }) {
                return self
                    .expression_raw(expr, env)?
                    .with_type(&Type::Signal(Box::new(ty.clone())))
                    .map_err(|message| self.error("S140", message, expr.span));
            }
        }
        if self.expression_mutates(expr) || self.expression_schedules(expr) {
            return Err(self.error("S140", "sig arguments and returns require pure expressions; evaluate effects into a let binding first", expr.span));
        }
        let mut names = BTreeSet::new();
        dependencies(expr, &mut names);
        let scope = env
            .iter()
            .filter(|(name, _)| names.contains(*name))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        self.charge_signal_metadata("returned signal", expr, &scope, ty, expr.span)?;
        let id = self.reserve_signal(
            Stored::Signal(Signal {
                frozen_at: None,
                expression: Arc::new(expr.clone()),
                scope,
                module: self.module.clone(),
                name: "returned signal".into(),
                span: expr.span,
                ty: Arc::new(ty.clone()),
                types: self.type_bindings.clone(),
                identity: self.identity.clone(),
                source_calls: self.source_calls.clone(),
                cached: None,
                dependencies: Default::default(),
                dirty: true,
                temporal: false,
                sampled_at: None,
            }),
            expr.span,
        )?;
        let value = Value::Signal(sanim_core::SignalRef {
            id,
            ty: Arc::new(ty.clone()),
        });
        Ok(value)
    }
}
