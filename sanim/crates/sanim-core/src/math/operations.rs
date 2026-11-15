//! One operator contract used by both the type checker and preparation evaluator.
use super::{float_arithmetic as calc, linear, to_f64};
use crate::{Fraction, Number, Type, Unit, Value};

pub fn scalar_unit(ty: &Type) -> Option<Unit> {
    match ty {
        Type::Numeric(unit) => Some(*unit),
        Type::RefinedNumeric(domain) => Some(domain.unit),
        Type::Fraction => Some(Unit::Number),
        Type::BoundedVariable(_, bound) => match bound.as_ref() {
            crate::TypeBound::Numeric => Some(Unit::Number),
            crate::TypeBound::Type(ty) => scalar_unit(ty),
            _ => None,
        },
        _ => None,
    }
}

pub fn binary_type(op: &str, a: &Type, b: &Type) -> Result<Type, String> {
    if op == "concat" {
        return a.concatenate(b);
    }
    if let Type::Signal(inner) = a {
        return binary_type(op, inner, b);
    }
    if let Type::Signal(inner) = b {
        return binary_type(op, a, inner);
    }
    if matches!(a, Type::Series(_)) || matches!(b, Type::Series(_)) {
        let a = if let Type::Series(inner) = a {
            inner.as_ref()
        } else {
            a
        };
        let b = if let Type::Series(inner) = b {
            inner.as_ref()
        } else {
            b
        };
        return crate::dataframe::binary_type(op, a, b).map(|ty| Type::Series(Box::new(ty)));
    }
    if matches!(op, "and" | "or") && *a == Type::Bool && *b == Type::Bool {
        return Ok(Type::Bool);
    }
    if matches!(op, "==" | "!=")
        && (a.accepts(b)
            || b.accepts(a)
            || matches!((a,b),(Type::List(x,_),Type::List(y,_)) if **x==Type::Unknown || **y==Type::Unknown || x.accepts(y) || y.accepts(x))
            || Type::String.accepts(a) && Type::String.accepts(b)
            || scalar_unit(a).is_some() && scalar_unit(a) == scalar_unit(b))
    {
        return Ok(Type::Bool);
    }
    if matches!(op, "<" | ">" | "<=" | ">=")
        && ((Type::String.accepts(a) && Type::String.accepts(b))
            || scalar_unit(a).is_some() && scalar_unit(a) == scalar_unit(b))
    {
        return Ok(Type::Bool);
    }
    if matches!(op, "+" | "-" | "*" | "/" | "%" | "**") {
        let variable = if matches!(a, Type::BoundedVariable(..)) {
            Some((a, b))
        } else if matches!(b, Type::BoundedVariable(..)) {
            Some((b, a))
        } else {
            None
        };
        if let Some((variable, other)) = variable
            && scalar_unit(variable) == Some(Unit::Number)
            && scalar_unit(other) == Some(Unit::Number)
        {
            if matches!(variable,Type::BoundedVariable(_,bound) if matches!(bound.as_ref(),crate::TypeBound::Type(ty) if matches!(ty.as_ref(),Type::RefinedNumeric(_))))
            {
                return Ok(Type::Numeric(Unit::Number));
            }
            if other == variable || matches!(other, Type::Numeric(Unit::Number)) {
                return Ok(variable.clone());
            }
            return Err("arithmetic across distinct generic numeric types is ambiguous; use the same type parameter for both operands".into());
        }
    }

    if let (Some(x), Some(y)) = (scalar_unit(a), scalar_unit(b)) {
        let unit = Number {
            value: Fraction::from(1),
            unit: x,
        }
        .binary(
            op,
            Number {
                value: Fraction::from(1),
                unit: y,
            },
        )?
        .unit;
        return Ok(
            if unit == Unit::Number && (*a == Type::Fraction || *b == Type::Fraction) {
                Type::Fraction
            } else {
                Type::Numeric(unit)
            },
        );
    }
    if op == "+" && Type::String.accepts(a) && Type::String.accepts(b) {
        return Ok(Type::String);
    }
    if let (Type::List(x, lx), Type::List(y, ly)) = (a, b)
        && matches!(op, "+" | "-" | "*" | "/" | "%" | "**")
    {
        let length = match (*lx, *ly) {
            (crate::ListLength::Exact(x), crate::ListLength::Exact(y)) if x != y => {
                return Err(format!(
                    "pointwise List arithmetic requires matching lengths, found {x} and {y}; use concat(a,b) to concatenate"
                ));
            }
            (crate::ListLength::Exact(length), _) | (_, crate::ListLength::Exact(length)) => {
                crate::ListLength::Exact(length)
            }
            _ => crate::ListLength::Unknown,
        };
        if scalar_unit(x).is_none() || scalar_unit(y).is_none() {
            return Err("List arithmetic requires numeric elements; concatenate lists explicitly with concat(a,b)".into());
        }
        return Ok(Type::List(Box::new(binary_type(op, x, y)?), length));
    }
    if matches!(op, "*" | "/")
        && let Type::List(inner, length) = a
        && scalar_unit(inner).is_some()
        && scalar_unit(b).is_some()
    {
        return Ok(Type::List(Box::new(binary_type(op, inner, b)?), *length));
    }
    if op == "*"
        && let Type::List(inner, length) = b
        && scalar_unit(a).is_some()
        && scalar_unit(inner).is_some()
    {
        return Ok(Type::List(Box::new(binary_type(op, a, inner)?), *length));
    }
    if *a == Type::Matrix4 && *b == Type::Matrix4 && matches!(op, "+" | "-" | "*") {
        return Ok(Type::Matrix4);
    }
    if *a == Type::Matrix4
        && op == "*"
        && matches!(b, Type::List(inner, length) if scalar_unit(inner)==Some(Unit::Number) && length.allows(4))
    {
        return Ok(Type::List(
            Box::new(Type::Numeric(Unit::Number)),
            crate::ListLength::Exact(4),
        ));
    }
    if *a == Type::Matrix4 && matches!(op, "*" | "/") && scalar_unit(b) == Some(Unit::Number) {
        return Ok(Type::Matrix4);
    }
    if *b == Type::Matrix4 && op == "*" && scalar_unit(a) == Some(Unit::Number) {
        return Ok(Type::Matrix4);
    }
    Err(format!(
        "operator '{op}' requires compatible operands; found {a} and {b}"
    ))
}

pub fn unary_type(op: &str, ty: &Type) -> Result<Type, String> {
    if let Type::Signal(inner) = ty {
        return unary_type(op, inner);
    }
    if let Type::Series(inner) = ty {
        return crate::dataframe::unary_type(op, inner).map(|ty| Type::Series(Box::new(ty)));
    }
    if matches!(op, "not" | "!") && *ty == Type::Bool {
        return Ok(Type::Bool);
    }
    if matches!(op, "+" | "-")
        && (scalar_unit(ty).is_some()
            || matches!(ty, Type::Matrix4)
            || matches!(ty, Type::List(inner,_) if scalar_unit(inner).is_some()))
    {
        if matches!(ty,Type::BoundedVariable(_,bound) if matches!(bound.as_ref(),crate::TypeBound::Type(ty) if matches!(ty.as_ref(),Type::RefinedNumeric(_))))
        {
            return Ok(Type::Numeric(Unit::Number));
        }
        return Ok(if let Type::RefinedNumeric(domain) = ty {
            Type::Numeric(domain.unit)
        } else {
            ty.clone()
        });
    }
    Err(format!("operator '{op}' is not defined for {ty}"))
}

fn map_list(
    list: crate::List,
    inner: Box<Type>,
    mut apply: impl FnMut(usize, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let mut values = list.values.clone();
    for (index, original) in list.values.iter().enumerate() {
        let value = apply(index, original)?;
        if value != *original {
            values.set(index, value);
        }
    }
    let inner = if inner.as_ref() == list.element_type.as_ref() {
        list.element_type.clone()
    } else {
        inner.into()
    };
    Ok(Value::List(crate::List::from_values(inner, values)))
}

pub fn unary(op: &str, value: Value) -> Result<Value, String> {
    if let Value::Series(series) = value {
        return series
            .unary(op)
            .map(|series| Value::Series(std::sync::Arc::new(series)));
    }
    unary_type(op, &value.ty())?;
    if op == "+" {
        return Ok(value);
    }
    match value {
        Value::Bool(v) => Ok(Value::Bool(!v)),
        Value::Number(mut n) => {
            n.value = -n.value;
            Ok(Value::Number(n))
        }
        Value::Fraction(v) => Ok(Value::Fraction(-v)),
        Value::Matrix4(mut m) => {
            for row in m.iter_mut() {
                for value in row {
                    *value = -*value;
                }
            }
            Ok(Value::Matrix4(m))
        }
        Value::List(list) => {
            let Type::List(inner, _) = unary_type(op, &Value::List(list.clone()).ty())? else {
                unreachable!("numeric List")
            };
            map_list(list, inner, |_, value| unary(op, value.clone()))
        }
        _ => Err("unsupported unary operand".into()),
    }
}

fn identity_list<'a>(op: &str, a: &'a Value, b: &'a Value) -> Option<&'a Value> {
    let (list, value) = match (a, b) {
        (Value::List(list), Value::Number(number))
            if matches!(op, "*" | "/")
                && number.unit == Unit::Number
                && number.value == Fraction::from(1) =>
        {
            (list, a)
        }
        (Value::Number(number), Value::List(list))
            if op == "*" && number.unit == Unit::Number && number.value == Fraction::from(1) =>
        {
            (list, b)
        }
        _ => return None,
    };
    matches!(
        list.element_type.as_ref(),
        Type::Numeric(_) | Type::Fraction
    )
    .then_some(value)
}
/// The interpreter and performance diagnostics charge work after no-op proofs.
pub fn binary_work(op: &str, a: &Value, b: &Value) -> usize {
    if !matches!(op, "+" | "-" | "*" | "/" | "%" | "**") || identity_list(op, a, b).is_some() {
        return 0;
    }
    match (a, b) {
        (Value::List(a), Value::List(b)) => a.len().max(b.len()),
        (Value::List(values), _) | (_, Value::List(values)) => values.len(),
        _ => 0,
    }
}
pub fn binary(op: &str, a: Value, b: Value) -> Result<Value, String> {
    if matches!(a, Value::Series(_)) || matches!(b, Value::Series(_)) {
        return crate::dataframe::Series::binary(op, a, b)
            .map(|series| Value::Series(std::sync::Arc::new(series)));
    }
    let ty = match binary_type(op, &a.ty(), &b.ty()) {
        Ok(ty) => ty,
        // The checker proves union comparability. Different concrete alternatives
        // compare unequal rather than losing that proof during value evaluation.
        Err(_) if matches!(op, "==" | "!=") => return Ok(Value::Bool(op == "!=")),
        Err(message) => return Err(message),
    };
    if let Some(value) = identity_list(op, &a, &b) {
        return Ok(value.clone());
    }
    if matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=") {
        if let (Ok(x), Ok(y)) = (a.number(), b.number()) {
            let ordering = x.value.cmp(&y.value);
            return Ok(Value::Bool(match op {
                "==" => ordering.is_eq(),
                "!=" => !ordering.is_eq(),
                "<" => ordering.is_lt(),
                ">" => ordering.is_gt(),
                "<=" => ordering.is_le(),
                _ => ordering.is_ge(),
            }));
        }
        if matches!(op, "==" | "!=") {
            return Ok(Value::Bool(if op == "==" { a == b } else { a != b }));
        }
        let ordering = a.string()?.cmp(b.string()?);
        return Ok(Value::Bool(match op {
            "<" => ordering.is_lt(),
            ">" => ordering.is_gt(),
            "<=" => ordering.is_le(),
            _ => ordering.is_ge(),
        }));
    }
    if matches!(op, "and" | "or") {
        return Ok(b);
    }
    if let (Ok(x), Ok(y)) = (a.number(), b.number()) {
        let n = x.binary(op, y)?;
        return Ok(if ty == Type::Fraction {
            Value::Fraction(n.value)
        } else {
            Value::Number(n)
        });
    }
    match (a, b) {
        (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{a}{b}").into())),
        (Value::List(a), Value::List(b)) => {
            if op == "concat" {
                let Type::List(inner, _) = ty else {
                    unreachable!("checked List concatenation")
                };
                let mut values = a.values;
                values.append(b.values);
                return Ok(Value::List(crate::List::from_values(inner.into(), values)));
            }
            if a.len() != b.len() {
                return Err(format!(
                    "pointwise List arithmetic requires matching lengths, found {} and {}",
                    a.len(),
                    b.len()
                ));
            }
            let Type::List(element_type, _) = ty else {
                unreachable!("checked List result")
            };
            map_list(a, element_type, |index, left| {
                binary(op, left.clone(), b[index].clone())
            })
        }
        (Value::Matrix4(a), Value::Matrix4(b)) => {
            if op == "*" {
                return linear::multiply(&a, &b).map(|m| Value::Matrix4(Box::new(m)));
            }
            let mut out = a;
            for (i, row) in out.iter_mut().enumerate() {
                for (j, value) in row.iter_mut().enumerate() {
                    *value = calc(op, *value, b[i][j])?;
                }
            }
            Ok(Value::Matrix4(out))
        }
        (Value::Matrix4(matrix), Value::List(v)) => {
            let (values, unit) = Value::List(v).vector()?;
            if unit != Unit::Number {
                return Err(
                    "Matrix4 multiplication requires a dimensionless numeric List of length 4"
                        .into(),
                );
            }
            Value::from_vector(linear::transform(&matrix, &values)?, Unit::Number)
        }
        (Value::Matrix4(mut matrix), scalar) | (scalar, Value::Matrix4(mut matrix)) => {
            let scalar = to_f64(scalar.number()?.value)?;
            for row in matrix.iter_mut() {
                for value in row {
                    *value = calc(op, *value, scalar)?;
                }
            }
            Ok(Value::Matrix4(matrix))
        }
        (Value::List(list), scalar) => {
            let Type::List(inner, _) = ty else {
                unreachable!("checked List result")
            };
            map_list(list, inner, |_, value| {
                binary(op, value.clone(), scalar.clone())
            })
        }
        (scalar, Value::List(list)) => {
            let Type::List(inner, _) = ty else {
                unreachable!("checked List result")
            };
            map_list(list, inner, |_, value| {
                binary(op, scalar.clone(), value.clone())
            })
        }
        _ => Err("unsupported arithmetic operands".into()),
    }
}
