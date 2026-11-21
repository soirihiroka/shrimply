//! Bounded rectangular algebra; matrices have no representation beyond nested Lists.
use super::ListFunction;
use crate::{List, ListLength, Number, Type, Unit, Value};

pub const MAX_DIMENSION: usize = 64;

pub fn shape(ty: &Type) -> Option<(ListLength, ListLength)> {
    let Type::List(row, rows) = ty else {
        return None;
    };
    let Type::List(element, columns) = row.as_ref() else {
        return None;
    };
    numeric(element).then_some((*rows, *columns))
}

fn numeric(ty: &Type) -> bool {
    super::super::operations::scalar_unit(ty) == Some(Unit::Number)
        || matches!(ty, Type::Union(options) if !options.is_empty() && options.iter().all(numeric))
}

pub fn type_of(rows: ListLength, columns: ListLength) -> Type {
    Type::List(
        Box::new(Type::List(Box::new(Type::Numeric(Unit::Number)), columns)),
        rows,
    )
}

fn matching(a: ListLength, b: ListLength) -> Result<(), String> {
    if let (ListLength::Exact(a), ListLength::Exact(b)) = (a, b)
        && a != b
    {
        return Err(format!("matrix dimensions do not match: {a} versus {b}"));
    }
    Ok(())
}

fn bounded(length: ListLength) -> Result<(), String> {
    if let ListLength::Exact(length) = length
        && (length == 0 || length > MAX_DIMENSION)
    {
        return Err(format!(
            "matrix dimensions must be between 1 and {MAX_DIMENSION}; found {length}"
        ));
    }
    Ok(())
}

pub fn function_type(function: ListFunction, ty: &Type) -> Result<Type, String> {
    let (rows, columns) = shape(ty)
        .ok_or("matrix operation requires a rectangular nested List of dimensionless numbers")?;
    bounded(rows)?;
    bounded(columns)?;
    if matches!(function, ListFunction::Inverse) {
        matching(rows, columns)?;
        Ok(type_of(rows, columns))
    } else {
        let Type::List(row, _) = ty else {
            unreachable!("validated nested List")
        };
        let Type::List(element, _) = row.as_ref() else {
            unreachable!("validated row")
        };
        Ok(Type::List(
            Box::new(Type::List(element.clone(), rows)),
            columns,
        ))
    }
}

pub fn product_type(a: &Type, b: &Type) -> Option<Result<Type, String>> {
    let (rows, columns) = shape(a)?;
    Some((|| {
        bounded(rows)?;
        bounded(columns)?;
        if let Some((right_rows, right_columns)) = shape(b) {
            bounded(right_rows)?;
            bounded(right_columns)?;
            matching(columns, right_rows)?;
            Ok(type_of(rows, right_columns))
        } else if let Type::List(element, length) = b
            && numeric(element)
        {
            matching(columns, *length)?;
            Ok(Type::List(Box::new(Type::Numeric(Unit::Number)), rows))
        } else {
            Err("matrix multiplication requires a numeric List or rectangular nested List".into())
        }
    })())
}

fn rows(list: &List) -> Result<Vec<&List>, String> {
    bounded(ListLength::Exact(list.len()))?;
    let rows = list
        .iter()
        .map(|value| match value {
            Value::List(row) => Ok(row),
            _ => Err("matrix rows must be numeric Lists".to_owned()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let columns = rows[0].len();
    bounded(ListLength::Exact(columns))?;
    if rows.iter().any(|row| row.len() != columns) {
        return Err("matrix rows must all have the same length".into());
    }
    Ok(rows)
}

fn collect(rows: impl IntoIterator<Item = List>, columns: usize) -> Value {
    let mut rows = rows.into_iter().peekable();
    let element = rows
        .peek()
        .expect("nonempty bounded matrix")
        .element_type
        .as_ref()
        .clone();
    Value::List(List::typed(
        Type::List(Box::new(element), ListLength::Exact(columns)),
        rows.map(Value::List).collect(),
    ))
}

pub fn product(a: &List, b: &List) -> Result<Value, String> {
    let left = rows(a)?;
    if shape(&Value::List(b.clone()).ty()).is_none() {
        if left[0].len() != b.len() {
            return Err("matrix column count must match vector length".into());
        }
        return Ok(Value::List(List::typed(
            Type::Numeric(Unit::Number),
            left.iter()
                .map(|row| super::list_dot(row, b).map(Value::Number))
                .collect::<Result<_, _>>()?,
        )));
    }
    let right = rows(b)?;
    if left[0].len() != right.len() {
        return Err("left matrix column count must match right matrix row count".into());
    }
    let columns = right[0].len();
    let transposed = (0..columns)
        .map(|column| {
            List::typed(
                right[0].element_type.as_ref().clone(),
                right.iter().map(|row| row[column].clone()).collect(),
            )
        })
        .collect::<Vec<_>>();
    let output = left
        .iter()
        .map(|row| {
            Ok(List::typed(
                Type::Numeric(Unit::Number),
                transposed
                    .iter()
                    .map(|column| super::list_dot(row, column).map(Value::Number))
                    .collect::<Result<_, String>>()?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(collect(output, columns))
}

pub fn evaluate(function: ListFunction, list: &List) -> Result<Value, String> {
    let rows = rows(list)?;
    if matches!(function, ListFunction::Transpose) {
        return Ok(collect(
            (0..rows[0].len()).map(|column| {
                List::typed(
                    rows[0].element_type.as_ref().clone(),
                    rows.iter().map(|row| row[column].clone()).collect(),
                )
            }),
            rows.len(),
        ));
    }
    let size = rows.len();
    if rows[0].len() != size {
        return Err("inverse requires a square nested List".into());
    }
    let mut left = rows
        .iter()
        .map(|row| row.iter().map(Value::number).collect::<Result<Vec<_>, _>>())
        .collect::<Result<Vec<_>, _>>()?;
    let mut right = (0..size)
        .map(|row| {
            (0..size)
                .map(|column| Number::scalar(i64::from(row == column)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for column in 0..size {
        let pivot = (column..size)
            .find(|row| left[*row][column].value != crate::Fraction::from(0))
            .ok_or("matrix is singular and has no inverse")?;
        left.swap(column, pivot);
        right.swap(column, pivot);
        let divisor = left[column][column];
        for index in 0..size {
            left[column][index] = left[column][index].binary("/", divisor)?;
            right[column][index] = right[column][index].binary("/", divisor)?;
        }
        for row in 0..size {
            if row == column {
                continue;
            }
            let factor = left[row][column];
            for index in 0..size {
                left[row][index] =
                    left[row][index].binary("-", factor.binary("*", left[column][index])?)?;
                right[row][index] =
                    right[row][index].binary("-", factor.binary("*", right[column][index])?)?;
            }
        }
    }
    Ok(collect(
        right.into_iter().map(|row| {
            List::typed(
                Type::Numeric(Unit::Number),
                row.into_iter().map(Value::Number).collect(),
            )
        }),
        size,
    ))
}

pub fn work(function: ListFunction, list: &List) -> Result<usize, String> {
    let rows = rows(list)?;
    let cells = rows.len() * rows[0].len();
    Ok(if matches!(function, ListFunction::Inverse) {
        cells * rows.len() * 6
    } else {
        cells
    })
}
