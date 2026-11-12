//! Static identities for expression inputs, independent of their runtime values.
use crate::ast::*;
use sanim_core::{Diagnostic, Span, Type};
use std::collections::BTreeMap;

pub(crate) fn identities(
    source: &str,
    module: &Module,
) -> Result<BTreeMap<usize, String>, Diagnostic> {
    let mut identities = BTreeMap::new();
    for statement in &module.statements {
        match &statement.kind {
            StatementKind::Bind {
                kind: BindingKind::Sig,
                value,
                ..
            } => {
                reject(
                    source,
                    value,
                    "reflect(...) cannot be declared inside a lazy sig; bind the editable input with let/mut first, then derive the sig from it",
                )?;
            }
            StatementKind::Bind { name, value, .. } => {
                collect(source, value, name, &mut identities, false)?
            }
            StatementKind::Assign { value, .. } => reject(
                source,
                value,
                "reflection inputs are declared once with let/mut; assignments cannot create reflect(...) inputs",
            )?,
            StatementKind::Animate {
                value,
                duration,
                easing,
                ..
            } => {
                for expr in std::iter::once(value)
                    .chain(std::iter::once(duration))
                    .chain(easing)
                {
                    reject(source, expr, PLACEMENT)?;
                }
            }
            StatementKind::Background { body } => reject_block(source, body)?,
            StatementKind::Expr(value) => {
                let name = semantic_name(value).unwrap_or_else(|| "input".into());
                collect(source, value, &name, &mut identities, true)?;
            }
            StatementKind::If { condition, yes, no } => {
                collect(source, condition, "condition", &mut identities, false)?;
                reject_block(source, yes)?;
                reject_block(source, no)?;
            }
            StatementKind::For { iterable, body, .. } => {
                collect(source, iterable, "iterable", &mut identities, false)?;
                reject_block(source, body)?;
            }
            StatementKind::Repeat {
                count,
                period,
                body,
            } => {
                collect(source, count, "repeat.count", &mut identities, false)?;
                if let Some(period) = period {
                    collect(source, period, "repeat.period", &mut identities, false)?;
                }
                reject_block(source, body)?;
            }
            StatementKind::Match { value, cases } => {
                collect(source, value, "match", &mut identities, false)?;
                for case in cases {
                    reject_block(source, &case.body)?;
                }
            }
            StatementKind::Function {
                parameters, body, ..
            } => {
                for (_, _, default) in parameters {
                    if let Some(default) = default {
                        reject(source, default, PLACEMENT)?;
                    }
                }
                reject_block(source, body)?;
            }
            StatementKind::Return(value) => reject(source, value, PLACEMENT)?,
            _ => {}
        }
    }
    Ok(identities)
}
const PLACEMENT: &str = "reflect(...) must execute exactly once at module scope; move this input above the function, loop, or branch and refer to its binding inside the body";
fn failure(source: &str, message: &str, span: Span) -> Diagnostic {
    Diagnostic::new("S113", message, source, span)
}
fn reject(source: &str, expr: &Expr, message: &str) -> Result<(), Diagnostic> {
    let mut invalid = None;
    expr.visit_while(&mut |node| {
        if matches!(node.kind, ExprKind::Reflect { .. }) {
            invalid.get_or_insert(node.span);
        }
        invalid.is_none()
    });
    invalid.map_or(Ok(()), |span| Err(failure(source, message, span)))
}
fn reject_block(source: &str, body: &[Statement]) -> Result<(), Diagnostic> {
    for statement in body {
        if let StatementKind::Function {
            parameters, body, ..
        } = &statement.kind
        {
            for (_, _, default) in parameters {
                if let Some(default) = default {
                    reject(source, default, PLACEMENT)?;
                }
            }
            reject_block(source, body)?;
        } else {
            let mut invalid = None;
            statement.visit_expressions(&mut |expr| {
                if invalid.is_none() && matches!(expr.kind, ExprKind::Reflect { .. }) {
                    invalid = Some(expr.span);
                }
            });
            if let Some(span) = invalid {
                return Err(failure(source, PLACEMENT, span));
            }
        }
    }
    Ok(())
}
fn semantic_name(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Call { callee, .. } => callee.qualified_name(),
        ExprKind::Reflect { value, .. } => semantic_name(value),
        _ => expr.qualified_name(),
    }
}
fn collect(
    source: &str,
    expr: &Expr,
    name: &str,
    identities: &mut BTreeMap<usize, String>,
    anonymous: bool,
) -> Result<(), Diagnostic> {
    // Count once per top expression; binary-search span ranges instead of rescanning subtrees.
    let mut positions = Vec::new();
    expr.visit(&mut |node| {
        if matches!(node.kind, ExprKind::Reflect { .. }) {
            positions.push(node.span.start);
        }
    });
    if positions.is_empty() {
        return Ok(());
    }
    positions.sort_unstable();
    descend(source, expr, name, &positions, identities, anonymous)
}
fn descend(
    source: &str,
    expr: &Expr,
    name: &str,
    positions: &[usize],
    identities: &mut BTreeMap<usize, String>,
    anonymous: bool,
) -> Result<(), Diagnostic> {
    let count = positions.partition_point(|start| *start < expr.span.end)
        - positions.partition_point(|start| *start < expr.span.start);
    if count == 0 {
        return Ok(());
    }
    match &expr.kind {
        ExprKind::Reflect { value, .. } => {
            reject(
                source,
                value,
                "reflect defaults cannot contain another reflect(...); bind the inner input first, then use that binding in this default",
            )?;
            identities.insert(expr.span.start, name.into());
        }
        ExprKind::Unary(_, value) | ExprKind::Member(value, _) => {
            descend(source, value, name, positions, identities, anonymous)?
        }
        ExprKind::Binary(op, left, right) => {
            if matches!(op.as_str(), "and" | "or") {
                reject(
                    source,
                    right,
                    "reflect(...) cannot appear on the right side of 'and' or 'or', because short-circuit evaluation may skip it; bind the input before this expression",
                )?;
            }
            for (side, value) in [("left", left), ("right", right)] {
                let path = if count > 1 {
                    format!("{name}.{side}")
                } else {
                    name.into()
                };
                descend(source, value, &path, positions, identities, anonymous)?;
            }
        }
        ExprKind::Index(base, index) => {
            let base_name = if count > 1 {
                format!("{name}.value")
            } else {
                name.into()
            };
            let index_name = if count > 1 {
                format!("{name}.index")
            } else {
                name.into()
            };
            descend(source, base, &base_name, positions, identities, anonymous)?;
            descend(source, index, &index_name, positions, identities, anonymous)?;
        }
        ExprKind::Call { callee, args, .. } => {
            reject(
                source,
                callee,
                "reflect(...) cannot be a function call target; reflect an argument value instead",
            )?;
            let call_name = if anonymous {
                callee.qualified_name().unwrap_or_else(|| name.into())
            } else {
                name.into()
            };
            for (index, (argument, value)) in args.iter().enumerate() {
                let part = argument.clone().unwrap_or_else(|| index.to_string());
                descend(
                    source,
                    value,
                    &format!("{call_name}.{part}"),
                    positions,
                    identities,
                    anonymous,
                )?;
            }
        }
        ExprKind::List(values) => {
            for (index, value) in values.iter().enumerate() {
                descend(
                    source,
                    value,
                    &format!("{name}.{index}"),
                    positions,
                    identities,
                    anonymous,
                )?;
            }
        }
        ExprKind::Record(fields) => {
            for (field, value) in fields {
                descend(
                    source,
                    value,
                    &format!("{name}.{field}"),
                    positions,
                    identities,
                    anonymous,
                )?;
            }
        }
        ExprKind::Dict(entries) => {
            for (index, (key, value)) in entries.iter().enumerate() {
                descend(
                    source,
                    key,
                    &format!("{name}.key{index}"),
                    positions,
                    identities,
                    anonymous,
                )?;
                let part = if let ExprKind::String(key) = &key.kind {
                    key.clone()
                } else {
                    index.to_string()
                };
                descend(
                    source,
                    value,
                    &format!("{name}.{part}"),
                    positions,
                    identities,
                    anonymous,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Generic constraints describe admissible types, not a concrete editor schema.
pub(crate) fn concrete_schema(ty: &Type) -> bool {
    match ty {
        Type::Unknown | Type::Variable(_) | Type::BoundedVariable(..) | Type::Vector(_, None) => {
            false
        }
        Type::List(inner) | Type::Option(inner) | Type::Array(inner) => concrete_schema(inner),
        Type::Record(fields) => fields.values().all(concrete_schema),
        Type::Union(values) => values.iter().all(concrete_schema),
        Type::Enum { variants, .. } => variants.values().flatten().all(concrete_schema),
        _ => true,
    }
}
