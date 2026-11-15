//! Shared typed expression lookup and bounded constant evaluation.
use crate::{
    ast::{Expr, ExprKind},
    prelude,
};
use sanim_core::{Number, Type, Value, math};
use std::collections::BTreeMap;

pub(crate) fn lookup<'a>(expr: &Expr, env: &'a impl crate::checker::TypeScope) -> Option<&'a Type> {
    let declared = match &expr.kind {
        ExprKind::Name(name) => env.get_type(name),
        ExprKind::Member(base, name) => {
            if let Type::Record(fields) = lookup(base, env)? {
                fields.get(name)
            } else {
                None
            }
        }
        _ => None,
    };
    declared.or_else(|| env.expression_type(expr.span))
}
pub(crate) fn constant(expr: &Expr, values: &BTreeMap<String, Value>) -> Option<Value> {
    match &expr.kind {
        ExprKind::Number(text) => Number::parse(text).ok().map(Value::Number),
        ExprKind::String(text) => Some(Value::String(text.as_str().into())),
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
