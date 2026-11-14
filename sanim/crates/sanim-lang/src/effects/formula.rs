//! Abstract numeric values used only by timeline effects; source expressions stay in the AST.
use super::*;
use sanim_core::DurationExpr;

pub(super) fn scalar(
    expr: &Expr,
    constants: &BTreeMap<String, Value>,
    symbolic: &BTreeMap<DurationSymbol, DurationExpr>,
) -> Option<DurationExpr> {
    if let Some(Value::Number(value)) = constant(expr, constants) {
        return DurationExpr::constant(value.value);
    }
    match &expr.kind {
        ExprKind::Name(name) => symbolic
            .get(&DurationSymbol::Parameter(name.as_str().into()))
            .cloned(),
        ExprKind::Reflect { .. } => None,
        ExprKind::Unary(op, value) if op == "-" => DurationExpr::product([
            DurationExpr::constant(Fraction::from(-1))?,
            scalar(value, constants, symbolic)?,
        ]),
        ExprKind::Binary(op, left, right) => {
            let left = scalar(left, constants, symbolic)?;
            let right = scalar(right, constants, symbolic)?;
            match op.as_str() {
                "+" => DurationExpr::sum([left, right]),
                "-" => DurationExpr::sum([
                    left,
                    DurationExpr::product([DurationExpr::constant(Fraction::from(-1))?, right])?,
                ]),
                "*" => DurationExpr::product([left, right]),
                "/" => DurationExpr::product([
                    left,
                    DurationExpr::constant(
                        math::arithmetic("/", Fraction::from(1), right.value()?).ok()?,
                    )?,
                ]),
                _ => None,
            }
        }
        ExprKind::Call { callee, args, .. } if matches!(&callee.kind,ExprKind::Name(name) if name=="len") => {
            length(&args.first()?.1, constants, symbolic)
        }
        _ => None,
    }
}

pub(super) fn length(
    expr: &Expr,
    constants: &BTreeMap<String, Value>,
    symbolic: &BTreeMap<DurationSymbol, DurationExpr>,
) -> Option<DurationExpr> {
    if let Some(value) = constant(expr, constants)
        && let Ok(list) = value.list()
    {
        return DurationExpr::constant(Fraction::from(list.len()));
    }
    match &expr.kind {
        ExprKind::List(values) => DurationExpr::constant(Fraction::from(values.len())),
        ExprKind::Name(name) => symbolic
            .get(&DurationSymbol::Length(name.as_str().into()))
            .cloned(),
        _ => None,
    }
}
