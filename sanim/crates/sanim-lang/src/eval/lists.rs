//! Retained collection state shares the same checkpoint and timeline as scalar cells.
use super::*;
use reactive::{Id, Node as Stored};

impl Runtime<'_, '_> {
    /// Call arguments complete before the returned value begins its transition.
    /// The shared call evaluator supplies this boundary in full and duration-only modes.
    pub(super) fn assignment_timing(&self, expression: &Expr) -> (Fraction, Easing) {
        if !matches!(expression.kind, ExprKind::Call { .. }) {
            return (self.cursor, Easing::default());
        }
        let call = &self.last_call;
        if call.module == self.module && call.span == expression.span && call.end == self.cursor {
            (call.start, call.easing)
        } else {
            (self.cursor, Easing::default())
        }
    }

    pub(super) fn proven_list_edit_result(
        &self,
        expression: &Expr,
        value: &Value,
    ) -> Option<&crate::lists::EditDelta> {
        let receipt = self.list_edit.as_ref()?;
        if receipt.module != self.module || receipt.span != expression.span {
            return None;
        }
        let Value::List(value) = value else {
            return None;
        };
        let output = receipt.delta.edit.output();
        if output.element_type != value.element_type || !output.shares_values(value) {
            return None;
        }
        Some(&receipt.delta)
    }

    /// Trust only a receipt emitted by this exact native call on the existing,
    /// already validated mutable list. Helpers and compound expressions retain
    /// the ordinary full-value validation path.
    pub(super) fn take_list_edit(
        &mut self,
        name: &str,
        expression: &Expr,
        before: &Value,
        after: &Value,
        expected: &Type,
    ) -> Option<crate::lists::EditDelta> {
        let receipt = self.list_edit.take()?;
        let ExprKind::Call { callee, args, .. } = &expression.kind else {
            return None;
        };
        let ExprKind::Name(function) = &callee.kind else {
            return None;
        };
        if prelude::builtin(function).is_none()
            || receipt.module != self.module
            || receipt.span != expression.span
        {
            return None;
        }
        let source = args
            .iter()
            .find(|(label, _)| label.as_deref() == Some("items"))
            .or_else(|| args.iter().find(|(label, _)| label.is_none()))?;
        if !matches!(&source.1.kind,ExprKind::Name(source) if source==name) {
            return None;
        }
        let (Value::List(before), Value::List(after), Type::List(inner, _)) =
            (before, after, expected)
        else {
            return None;
        };
        let delta = receipt.delta;
        if [before, after, delta.edit.source(), delta.edit.output()]
            .iter()
            .any(|list| list.element_type.as_ref() != inner.as_ref())
        {
            return None;
        }
        if !before.shares_values(delta.edit.source()) || !after.shares_values(delta.edit.output()) {
            return None;
        }
        Some(delta)
    }

    pub(super) fn retain_list_edit(
        &mut self,
        delta: &crate::lists::EditDelta,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if let Some(value) = delta.edit.added() {
            self.retain_binding_value(value, span)?;
        }
        self.retained_bytes = self.retained_bytes.checked_sub(delta.cow_bytes).ok_or_else(|| {
            self.error("S303", "list edit copy-on-write byte budget exceeded; reduce retained updates or list size", span)
        })?;
        Ok(())
    }

    pub(super) fn ensure_list_cell(&mut self, id: Id, span: Span) -> Result<(), Diagnostic> {
        if !self.animation.list_views.has_cell(id) {
            let Some(Stored::Cell(cell)) = self.reactive.nodes.get(&id) else {
                return Err(self.error("S140", "a timed list must be a mutable binding", span));
            };
            let Value::List(items) = &cell.value else {
                return Err(self.error("S140", "a timed list requires List<T>", span));
            };
            self.animation
                .set_list(id, items.clone())
                .map_err(|error| self.error("S140", error, span))?;
        }
        self.reactive.retain(id);
        Ok(())
    }

    pub(super) fn bind_list_children(
        &mut self,
        element: &Element,
        expression: &Expr,
        env: &Env,
    ) -> Result<bool, Diagnostic> {
        let ExprKind::Name(name) = &expression.kind else {
            return Ok(false);
        };
        let Some(Binding::Reactive(id)) = env.get(name) else {
            return Ok(false);
        };
        let id = *id;
        self.refresh_cell(id, expression.span)?;
        let Some(Stored::Cell(cell)) = self.reactive.nodes.get(&id) else {
            return Err(self.error("S140", "children must refer directly to a mut List<Element>; derived child topology is not supported", expression.span));
        };
        let Value::List(items) = &cell.value else {
            return Ok(false);
        };
        let items = items.clone();
        self.animation
            .bind_list(id, element, items)
            .map_err(|error| self.error("S140", error, expression.span))?;
        self.reactive.retain(id);
        Ok(true)
    }

    pub(super) fn write_list_assignment(
        &mut self,
        id: Id,
        value: &Value,
        start: Fraction,
        edit: Option<&sanim_core::ListEditResult>,
        easing: Easing,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let Value::List(items) = value else {
            return Err(self.error("S140", "a timed list assignment requires List<T>", span));
        };
        self.ensure_list_cell(id, span)?;
        let result = if let Some(edit) = edit {
            self.animation
                .write_list_edit(id, start, self.cursor, edit, easing)
        } else {
            self.animation
                .write_list(id, start, self.cursor, items.clone(), easing)
        };
        result.map_err(|error| self.error("S140", error, span))
    }
}
