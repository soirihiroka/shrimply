//! Duration-only preparation projects native animation calls onto their temporal
//! arguments. Visual expressions remain deferred; no substitute geometry exists.
use super::*;

impl Runtime<'_, '_> {
    pub(super) fn measure_animation_call(
        &mut self,
        callee: &Expr,
        args: &[(Option<String>, Expr)],
        env: &Env,
        span: Span,
    ) -> Result<Option<Value>, Diagnostic> {
        if !self.measure_only {
            return Ok(None);
        }
        let Some(module) = self.loaded.iter().find(|module| module.id == self.module) else {
            return Ok(None);
        };
        let Some(call) = crate::timing::builtin_call(callee, args, &module.checked.native_methods)
        else {
            return Ok(None);
        };
        let builtin = call.builtin;
        if builtin.result != Type::Animation
            || matches!(builtin.execute, prelude::Execution::CellAnimation)
        {
            return Ok(None);
        }
        let arguments = builtin
            .parameters
            .iter()
            .enumerate()
            .filter_map(|(index, parameter)| {
                call.argument(index)
                    .map(|expression| (parameter.name, parameter.ty.clone(), expression))
            })
            .collect::<Vec<_>>();
        let mut evaluated = Vec::new();
        for (name, ty, expression) in arguments {
            if ty.contains_element()
                && !ty.contains_animation()
                && !self.expression_mutates(expression)
            {
                continue;
            }
            let value = if name == "children" && ty.contains_animation() {
                self.measure_children(expression, env)?
            } else {
                self.expression(expression, env)?
            };
            evaluated.push((Some(name.to_string()), value));
        }
        self.native(builtin.name, evaluated, span).map(Some)
    }
    fn measure_children(&mut self, expression: &Expr, env: &Env) -> Result<Value, Diagnostic> {
        let zero = Value::Animation(Arc::new(sanim_core::plan::Animation::new(
            sanim_core::plan::Node::Hold(Some(0.into())),
        )));
        let ExprKind::List(children) = &expression.kind else {
            if let Some(Type::List(element, ListLength::Exact(count))) =
                self.measure_type(expression, env)
                && element.contains_element()
                && !element.contains_animation()
                && !self.expression_mutates(expression)
            {
                self.measure_slot_budget(count, expression.span)?;
                return Ok(Value::List(sanim_core::List::typed(
                    Type::Animation,
                    vec![zero; count],
                )));
            }
            return self.expression(expression, env);
        };
        self.measure_slot_budget(children.len(), expression.span)?;
        let mut animations = Vec::with_capacity(children.len());
        for child in children {
            let visual = self
                .measure_type(child, env)
                .is_some_and(|ty| ty.contains_element() && !ty.contains_animation());
            if visual && !self.expression_mutates(child) {
                // Preserve the real zero-time temporal slot: sequence gaps and
                // space-between sizing still depend on the number of children.
                animations.push(zero.clone());
            } else {
                animations.push(self.expression(child, env)?);
            }
        }
        Ok(Value::List(sanim_core::List::typed(
            Type::Animation,
            animations,
        )))
    }
    fn measure_slot_budget(&mut self, count: usize, span: Span) -> Result<(), Diagnostic> {
        // Vec construction and persistent-list publication can overlap.
        let bytes = count
            .checked_mul(std::mem::size_of::<Value>())
            .and_then(|bytes| bytes.checked_mul(2))
            .ok_or_else(|| self.error("S303", "temporal child list byte count overflow", span))?;
        if count > self.limits.data.max_values || bytes > self.limits.data.max_bytes {
            return Err(self.error(
                "S303",
                "temporal child list exceeds the host value/byte budget",
                span,
            ));
        }
        self.charge_work(count as u64, span)
    }
    fn measure_type(&self, expression: &Expr, env: &Env) -> Option<Type> {
        if let ExprKind::Name(name) = &expression.kind {
            match env.get(name) {
                Some(Binding::Value(value)) => return Some(value.ty()),
                Some(Binding::Deferred(id) | Binding::Reactive(id)) => {
                    return self.reactive.nodes.get(id).map(|node| match node {
                        reactive::Node::Cell(cell) => cell.ty.as_ref().clone(),
                        reactive::Node::Signal(signal) => signal.ty.as_ref().clone(),
                    });
                }
                _ => {}
            }
        }
        self.loaded
            .iter()
            .find(|module| module.id == self.module)
            .and_then(|module| crate::analysis::lookup(expression, module.checked.as_ref()))
            .map(|ty| ty.substitute(&self.type_bindings))
    }
}
