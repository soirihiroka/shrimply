//! One operator contract used by both the type checker and preparation evaluator.
use super::{float_arithmetic as calc, linear, to_f64};
use crate::{Fraction, Number, Type, Unit, Value};

pub fn scalar_unit(ty: &Type) -> Option<Unit> {
    match ty {
        Type::Numeric(unit) => Some(*unit),
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
    if matches!(op, "and" | "or") && *a == Type::Bool && *b == Type::Bool {
        return Ok(Type::Bool);
    }
    if matches!(op, "==" | "!=")
        && (a.accepts(b) || scalar_unit(a).is_some() && scalar_unit(a) == scalar_unit(b))
    {
        return Ok(Type::Bool);
    }
    if matches!(op, "<" | ">" | "<=" | ">=")
        && ((*a == Type::String && a == b)
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
    if op == "+" && *a == Type::String && a == b {
        return Ok(Type::String);
    }
    if op == "+"
        && let (Type::List(x), Type::List(_)) = (a, b)
        && a.accepts(b)
    {
        return Ok(if **x == Type::Unknown {
            b.clone()
        } else {
            a.clone()
        });
    }
    if matches!(op, "+" | "-")
        && let (Type::Vector(n, x), Type::Vector(m, y)) = (a, b)
        && n == m
        && x.is_some()
        && x == y
    {
        return Ok(a.clone());
    }
    if matches!(op, "*" | "/")
        && let Type::Vector(..) = a
        && scalar_unit(b) == Some(Unit::Number)
    {
        return Ok(a.clone());
    }
    if op == "*"
        && let Type::Vector(..) = b
        && scalar_unit(a) == Some(Unit::Number)
    {
        return Ok(b.clone());
    }
    if *a == Type::Matrix4 && *b == Type::Matrix4 && matches!(op, "+" | "-" | "*") {
        return Ok(Type::Matrix4);
    }
    if *a == Type::Matrix4 && op == "*" && matches!(b, Type::Vector(4, Some(Unit::Number))) {
        return Ok(b.clone());
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
    if matches!(op, "not" | "!") && *ty == Type::Bool {
        return Ok(Type::Bool);
    }
    if matches!(op, "+" | "-")
        && (scalar_unit(ty).is_some() || matches!(ty, Type::Vector(..) | Type::Matrix4))
    {
        return Ok(ty.clone());
    }
    Err(format!("operator '{op}' is not defined for {ty}"))
}

pub fn unary(op: &str, value: Value) -> Result<Value, String> {
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
        vector => {
            let (v, unit) = vector.vector()?;
            Value::from_vector(v.iter().map(|n| -*n).collect(), unit)
        }
    }
}

pub fn binary(op: &str, a: Value, b: Value) -> Result<Value, String> {
    let ty = match binary_type(op, &a.ty(), &b.ty()) {
        Ok(ty) => ty,
        // The checker proves union comparability. Different concrete alternatives
        // compare unequal rather than losing that proof during value evaluation.
        Err(_) if matches!(op, "==" | "!=") => return Ok(Value::Bool(op == "!=")),
        Err(message) => return Err(message),
    };
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
            let Type::List(element_type) = ty else {
                unreachable!("checked list concatenation type")
            };
            let mut values = a.values;
            values.append(b.values);
            Ok(Value::List(crate::List::from_values(
                element_type.into(),
                values,
            )))
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
        (Value::Matrix4(matrix), Value::Vec4(v)) => {
            Value::from_vector(linear::transform(&matrix, &v.values)?, Unit::Number)
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
        (a, b) => {
            if let (Ok((x, unit)), Ok((y, _))) = (a.vector(), b.vector()) {
                return Value::from_vector(
                    x.iter()
                        .zip(y)
                        .map(|(a, b)| calc(op, *a, *b))
                        .collect::<Result<_, _>>()?,
                    unit,
                );
            }
            let (vector, scalar) = if a.vector().is_ok() { (a, b) } else { (b, a) };
            let (v, unit) = vector.vector()?;
            let scalar = to_f64(scalar.number()?.value)?;
            Value::from_vector(
                v.iter()
                    .map(|v| calc(op, *v, scalar))
                    .collect::<Result<_, _>>()?,
                unit,
            )
        }
    }
}
