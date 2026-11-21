//! Inline source queries are resolved where their animation is placed, not during construction.
use super::*;

impl Runtime<'_, '_> {
    pub(super) fn staged_code_range(
        &mut self,
        expression: &Expr,
        env: &Env,
    ) -> Result<Option<Value>, Diagnostic> {
        let ExprKind::Call { callee, args, .. } = &expression.kind else {
            return Ok(None);
        };
        let ExprKind::Member(receiver, _) = &callee.kind else {
            return Ok(None);
        };
        let Some(action) = self
            .checked_native_method(callee.span)
            .and_then(|method| prelude::builtin(&method.name))
            .filter(|builtin| {
                builtin.name.starts_with("CodeRange.") && builtin.result == Type::Animation
            })
        else {
            return Ok(None);
        };
        let ExprKind::Call {
            callee: query_callee,
            args: query_args,
            ..
        } = &receiver.kind
        else {
            return Ok(None);
        };
        let Some(query_method) = self.checked_native_method(query_callee.span) else {
            return Ok(None);
        };
        if query_method.name != "CodeView.range" {
            return Ok(None);
        }
        let checked = self
            .loaded
            .iter()
            .find(|module| module.id == self.module)
            .expect("checked module")
            .checked
            .clone();
        let query = crate::timing::builtin_call(query_callee, query_args, &checked.native_methods)
            .expect("checked source query");
        let mut values = Vec::new();
        if let Some(receiver) = query.receiver {
            values.push((None, self.expression(receiver, env)?));
        }
        for (name, value) in query_args {
            values.push((name.clone(), self.expression(value, env)?));
        }
        let names = query
            .builtin
            .parameters
            .iter()
            .map(|parameter| parameter.name.to_owned())
            .collect::<Vec<_>>();
        let mut query = order_values(&values, &names);
        let view = query.remove("view").expect("checked CodeView receiver");
        let values = args
            .iter()
            .map(|(name, value)| Ok((name.clone(), self.expression(value, env)?)))
            .collect::<Result<Vec<_>, Diagnostic>>()?;
        let names = action
            .parameters
            .iter()
            .skip(1)
            .map(|parameter| parameter.name.to_owned())
            .collect::<Vec<_>>();
        let mut arguments = order_values(&values, &names);
        arguments.insert("query".into(), Value::Record(query));
        arguments.insert("view".into(), view);
        self.native(
            action.name,
            arguments
                .into_iter()
                .map(|(name, value)| (Some(name), value))
                .collect(),
            expression.span,
        )
        .map(Some)
    }
}
