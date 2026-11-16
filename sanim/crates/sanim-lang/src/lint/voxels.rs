//! Conservative authoring advice: syntax-only proofs, never bounds guesses or resource evaluation.
use crate::ast::*;
use sanim_core::{PositionMode, Span, Value};
use std::collections::BTreeMap;

#[derive(Clone)]
enum Binding {
    Value(usize),
    Part(String),
}
type Bindings<'a> = imbl::OrdMap<&'a str, Binding>;

// Only small literal placement arguments qualify. Unknown expressions remain runtime contracts.
fn literal(expression: &Expr, depth: usize) -> Option<String> {
    if depth > 4 {
        return None;
    }
    Some(match &expression.kind {
        ExprKind::Number(_) | ExprKind::Bool(_) => format!(
            "{:?}",
            crate::analysis::constant(expression, &BTreeMap::new())?
        ),
        ExprKind::String(text) if text.len() <= 64 => format!(
            "{:?}",
            crate::analysis::constant(expression, &BTreeMap::new())?
        ),
        ExprKind::List(items) if items.len() <= 3 => format!(
            "{:?}",
            items
                .iter()
                .map(|item| literal(item, depth + 1))
                .collect::<Option<Vec<_>>>()?
        ),
        ExprKind::Unary(operator, value) if operator == "-" => {
            format!("-{}", literal(value, depth + 1)?)
        }
        _ => return None,
    })
}

fn recipe(expression: &Expr, bindings: &Bindings<'_>) -> Option<String> {
    if let ExprKind::Name(name) = &expression.kind {
        return match bindings.get(name.as_str())? {
            Binding::Part(recipe) => Some(recipe.clone()),
            _ => None,
        };
    }
    let ExprKind::Call { callee, args, .. } = &expression.kind else {
        return None;
    };
    if !matches!(&callee.kind,ExprKind::Name(name) if name=="VoxelPart") {
        return None;
    }
    let mut fields = BTreeMap::new();
    let mut volume = None;
    for (name, value) in args {
        let name = name.as_deref()?;
        if name == "volume" {
            let ExprKind::Name(name) = &value.kind else {
                return None;
            };
            let Binding::Value(identity) = bindings.get(name.as_str())? else {
                return None;
            };
            volume = Some(*identity);
        } else {
            fields.insert(name, literal(value, 0)?);
        }
    }
    let absolute = args.iter().any(|(name, value)| match name.as_deref() {
        Some("position_mode") => {
            crate::analysis::constant(value, &BTreeMap::new())
                == Some(Value::String(PositionMode::Absolute.name().into()))
        }
        _ => false,
    });
    if !absolute {
        return None;
    }
    Some(format!("{}:{fields:?}", volume?))
}

fn inspect(expression: &Expr, bindings: &Bindings<'_>, warn: &mut impl FnMut(&str, String, Span)) {
    let ExprKind::Call { callee, args, .. } = &expression.kind else {
        return;
    };
    if !matches!(&callee.kind,ExprKind::Name(name) if name=="VoxelBox") {
        return;
    }
    let Some((_, children)) = args
        .iter()
        .find(|(name, _)| name.as_deref() == Some("children"))
    else {
        return;
    };
    let ExprKind::List(children) = &children.kind else {
        return;
    };
    let mut previous = BTreeMap::new();
    for child in children {
        let Some(key) = recipe(child, bindings) else {
            continue;
        };
        if let Some(first) = previous.get(&key) {
            warn(
                "V101",
                format!(
                    "duplicate absolute VoxelPart recipe at the same placement as line {first}; these parts merge into the same voxel cells, so the later part can overwrite material. Remove the duplicate, change its placement, or use world.assert_disjoint(first, second) to verify unintended intersections"
                ),
                child.span,
            );
        } else {
            previous.insert(key, child.span.line);
        }
    }
}

pub(super) fn advice(statements: &[Statement], warn: &mut impl FnMut(&str, String, Span)) {
    block(statements, Bindings::new(), warn);
}
fn block<'a>(
    statements: &'a [Statement],
    mut bindings: Bindings<'a>,
    warn: &mut impl FnMut(&str, String, Span),
) {
    for statement in statements {
        statement.visit_own_expressions(&mut |expression| inspect(expression, &bindings, warn));
        match &statement.kind {
            StatementKind::Bind {
                kind, name, value, ..
            } => {
                if *kind == BindingKind::Let {
                    let value = recipe(value, &bindings)
                        .map(Binding::Part)
                        .unwrap_or(Binding::Value(value.span.start));
                    bindings.insert(name, value);
                } else {
                    bindings.remove(name.as_str());
                }
            }
            StatementKind::Function {
                parameters, body, ..
            } => {
                let mut local = bindings.clone();
                for parameter in parameters {
                    local.remove(parameter.name.as_str());
                    if parameter.kind == BindingKind::Let {
                        local.insert(
                            parameter.name.as_str(),
                            Binding::Value(parameter.name_span.start),
                        );
                    }
                }
                block(body, local, warn);
            }
            StatementKind::If { yes, no, .. } => {
                block(yes, bindings.clone(), warn);
                block(no, bindings.clone(), warn);
            }
            StatementKind::Repeat { body, .. } => block(body, bindings.clone(), warn),
            StatementKind::For { name, body, .. } => {
                let mut local = bindings.clone();
                local.remove(name.as_str());
                block(body, local, warn);
            }
            StatementKind::Match { cases, .. } => {
                for case in cases {
                    let mut local = bindings.clone();
                    for name in &case.bindings {
                        local.remove(name.as_str());
                    }
                    block(&case.body, local, warn);
                }
            }
            _ => {}
        }
    }
}
