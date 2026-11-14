//! Conservative call-by-need for expensive, zero-time immutable data values.
//! Timeline operations, reflected declarations, and element identity stay ordered.
use super::*;
use reactive::{Node, Signal};

impl Runtime<'_, '_> {
    pub(super) fn should_defer(&self, expression: &Expr, env: &Env, span: Span) -> bool {
        if self.preparation != crate::Preparation::Demand
            || self.duration_only
            || self.expression_mutates(expression)
            || self.expression_schedules(expression)
            || self
                .binding_type(span)
                .is_ok_and(|ty| ty.contains_element() || ty.contains_function())
        {
            return false;
        }
        let Some(module) = self.loaded.iter().find(|module| module.id == self.module) else {
            return false;
        };
        let mut pure = true;
        let mut expensive = false;
        expression.visit(&mut |node| match &node.kind {
            ExprKind::Reflect { .. } => pure = false,
            ExprKind::Name(name) => {
                if matches!(env.get(name), Some(Binding::Module(_))) {
                    pure = false;
                }
                // Geometry getters observe scene tracks, including zero-time
                // writes inserted later at the same timestamp. Keep those
                // observations in source order rather than sampling them late.
                if let Some(Binding::Value(value)) = env.get(name) {
                    pure &= !value.ty().contains_element();
                }
                if let Some(Binding::Reactive(id)) = env.get(name) {
                    pure &= self.reactive.nodes.get(id).is_none_or(|node| match node {
                        Node::Cell(cell) => !cell.ty.contains_element(),
                        Node::Signal(signal) => !signal.ty.contains_element(),
                    });
                }
                expensive |= matches!(env.get(name), Some(Binding::Deferred(_)));
            }
            ExprKind::Call { callee, args, .. } => {
                let Some(call) =
                    crate::timing::builtin_call(callee, args, &module.checked.native_methods)
                else {
                    pure = false;
                    return;
                };
                let builtin = call.builtin;

                // Like direct user calls, callbacks can read module globals not
                // captured by the deferred expression's lexical snapshot.
                pure &= crate::list_functions::callback_argument(&call).is_none()
                    && !call.schedules()
                    && !builtin.result.contains_element()
                    && !matches!(builtin.execute, prelude::Execution::Assertion);
                expensive |= self
                    .checked_native_method(callee.span)
                    .is_some_and(|method| method.materializes)
                    || matches!(
                        builtin.execute,
                        prelude::Execution::Read
                            | prelude::Execution::Decode
                            | prelude::Execution::Query
                    );
            }
            _ => {}
        });
        pure && expensive
    }

    pub(super) fn defer_binding(
        &mut self,
        name: &str,
        expression: &Expr,
        env: &Env,
        span: Span,
    ) -> Result<Binding, Diagnostic> {
        if self.reactive.nodes.len() >= self.limits.data.max_values {
            return Err(self.error(
                "S303",
                "deferred values exceed the configured live-value limit",
                span,
            ));
        }
        let mut names = BTreeSet::new();
        dependencies(expression, &mut names);
        let mut scope = Env::new();
        for name in names {
            if let Some(binding) = env.get(&name) {
                let binding = if matches!(binding, Binding::Reactive(_)) {
                    // Freeze the value at declaration. In particular, a later
                    // assignment must never change an immutable let's meaning.
                    Binding::Value(self.binding_value(binding, span)?)
                } else {
                    binding.clone()
                };
                scope.insert(name, binding);
            }
        }
        let ty = self.binding_type(span)?;
        self.charge_signal_metadata(name, expression, &scope, &ty, span)?;
        let id = self.reactive.next;
        self.reactive.next = id
            .checked_add(1)
            .ok_or_else(|| self.error("S303", "deferred value identity counter overflow", span))?;
        self.reactive.nodes.insert(
            id,
            Node::Signal(Signal {
                frozen_at: Some(self.cursor),
                expression: Arc::new(expression.clone()),
                scope,
                module: self.module.clone(),
                name: name.into(),
                span,
                ty: Arc::new(ty),
                types: self.type_bindings.clone(),
                identity: self.identity.clone(),
                source_calls: self.source_calls.clone(),
                cached: None,
                dependencies: Default::default(),
                dirty: true,
                temporal: false,
                sampled_at: None,
            }),
        );
        self.reactive.touch();
        self.statistics.deferred_values += 1;
        Ok(Binding::Deferred(id))
    }

    /// Inspection of an editor revision never forces unused data or signals.
    pub(super) fn available_bindings(&self, env: &Env) -> Values {
        env.iter()
            .filter_map(|(name, binding)| {
                let value = match binding {
                    Binding::Value(Value::Signal(signal)) => self
                        .reactive
                        .cached(signal.id, self.cursor, &self.animation.reactive)
                        .cloned(),
                    Binding::Value(value) => Some(value.clone()),
                    Binding::Reactive(id) | Binding::Deferred(id) => self
                        .reactive
                        .cached(*id, self.cursor, &self.animation.reactive)
                        .cloned(),
                    Binding::Module(_) => None,
                }?;
                Some((name.clone(), value))
            })
            .collect()
    }

    pub(super) fn deferred_bindings(&self, env: &Env) -> Vec<crate::DeferredBinding> {
        env.iter()
            .filter_map(|(name, binding)| {
                let id = match binding {
                    Binding::Reactive(id) | Binding::Deferred(id) => id,
                    Binding::Value(Value::Signal(signal)) => &signal.id,
                    _ => return None,
                };
                if self
                    .reactive
                    .cached(*id, self.cursor, &self.animation.reactive)
                    .is_some()
                {
                    return None;
                }
                let node = self.reactive.nodes.get(id)?;
                let (ty, module, span) = match node {
                    Node::Signal(signal) => (&signal.ty, &signal.module, signal.span),
                    Node::Cell(cell) => (&cell.ty, &cell.module, cell.span),
                };
                Some(crate::DeferredBinding {
                    name: name.clone(),
                    ty: ty.as_ref().clone(),
                    source: module.clone(),
                    span,
                })
            })
            .collect()
    }
}
