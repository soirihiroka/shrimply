//! Fixed-size, finite f64 linear algebra. Matrices are row-major.
use super::float_arithmetic as calc;

pub type Matrix4 = [[f64; 4]; 4];

/// Conservative local rectangle after the same stretch/scale/rotation/translation order as drawing.
pub fn transformed_box(
    bounds: [f64; 4],
    position: [f64; 2],
    rotation_degrees: f64,
    scale: f64,
    stretch: [f64; 2],
) -> Result<[f64; 4], String> {
    let (sin, cos) = rotation_degrees.to_radians().sin_cos();
    let mut output = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for x in [bounds[0], bounds[2]] {
        for y in [bounds[1], bounds[3]] {
            let x = calc(
                "*",
                calc(
                    "+",
                    stretch[1],
                    calc("*", calc("-", x, stretch[1])?, stretch[0])?,
                )?,
                scale,
            )?;
            let y = calc("*", y, scale)?;
            let point = [
                calc(
                    "+",
                    position[0],
                    calc("-", calc("*", x, cos)?, calc("*", y, sin)?)?,
                )?,
                calc(
                    "+",
                    position[1],
                    calc("+", calc("*", x, sin)?, calc("*", y, cos)?)?,
                )?,
            ];
            for axis in 0..2 {
                output[axis] = output[axis].min(point[axis]);
                output[axis + 2] = output[axis + 2].max(point[axis]);
            }
        }
    }
    Ok(output)
}

pub fn identity() -> Matrix4 {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    out
}

pub fn dot(a: &[f64], b: &[f64]) -> Result<f64, String> {
    if a.len() != b.len() {
        return Err("dot product dimensions must match".into());
    }
    a.iter()
        .zip(b)
        .try_fold(0.0, |sum, (a, b)| calc("+", sum, calc("*", *a, *b)?))
}

pub fn multiply(a: &Matrix4, b: &Matrix4) -> Result<Matrix4, String> {
    let bt = transpose(b);
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = dot(&a[i], &bt[j])?;
        }
    }
    Ok(out)
}

pub fn transform(matrix: &Matrix4, vector: &[f64]) -> Result<Vec<f64>, String> {
    if vector.len() != 4 {
        return Err(
            "Matrix4 multiplication requires a numeric List of length 4; choose the homogeneous w component explicitly"
                .into(),
        );
    }
    matrix.iter().map(|row| dot(row, vector)).collect()
}

pub fn transpose(matrix: &Matrix4) -> Matrix4 {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = matrix[j][i];
        }
    }
    out
}

pub fn inverse(matrix: &Matrix4) -> Result<Matrix4, String> {
    let mut left = *matrix;
    let mut right = identity();
    for column in 0..4 {
        let pivot = (column..4)
            .max_by(|a, b| left[*a][column].abs().total_cmp(&left[*b][column].abs()))
            .filter(|row| left[*row][column] != 0.0)
            .ok_or("matrix is singular and has no inverse")?;
        left.swap(column, pivot);
        right.swap(column, pivot);
        let divisor = left[column][column];
        for j in 0..4 {
            left[column][j] = calc("/", left[column][j], divisor)?;
            right[column][j] = calc("/", right[column][j], divisor)?;
        }
        for row in 0..4 {
            if row == column {
                continue;
            }
            let factor = left[row][column];
            for j in 0..4 {
                left[row][j] = calc("-", left[row][j], calc("*", factor, left[column][j])?)?;
                right[row][j] = calc("-", right[row][j], calc("*", factor, right[column][j])?)?;
            }
        }
    }
    Ok(right)
}

/// Exact arithmetic over the canonical persistent List representation.
pub fn list_dot(a: &crate::List, b: &crate::List) -> Result<crate::Number, String> {
    if a.len() != b.len() {
        return Err(format!(
            "dot requires equal List lengths, found {} and {}",
            a.len(),
            b.len()
        ));
    }
    a.values
        .iter()
        .zip(&b.values)
        .try_fold(crate::Number::scalar(0), |sum, (a, b)| {
            let a = a.number()?;
            let b = b.number()?;
            if a.unit != crate::Unit::Number || b.unit != crate::Unit::Number {
                return Err("dot requires dimensionless numeric Lists".into());
            }
            sum.binary("+", a.binary("*", b)?)
        })
}
pub fn list_cross(a: &crate::List, b: &crate::List) -> Result<crate::List, String> {
    if a.len() != 3 || b.len() != 3 {
        return Err("cross requires numeric Lists of length 3".into());
    }
    let component = |index: usize| -> Result<crate::Number, String> {
        let j = (index + 1) % 3;
        let k = (index + 2) % 3;
        let a1 = a[j].number()?;
        let b1 = b[k].number()?;
        let a2 = a[k].number()?;
        let b2 = b[j].number()?;
        if [a1, b1, a2, b2]
            .iter()
            .any(|number| number.unit != crate::Unit::Number)
        {
            return Err("cross requires dimensionless numeric Lists".into());
        }
        a1.binary("*", b1)?.binary("-", a2.binary("*", b2)?)
    };
    Ok(crate::List::typed(
        crate::Type::Numeric(crate::Unit::Number),
        (0..3)
            .map(|index| component(index).map(crate::Value::Number))
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

#[derive(Clone, Copy)]
pub enum ListFunction {
    Dot,
    Norm,
    Cross,
    Concat,
}
pub const LIST_FUNCTIONS: &[(&str, ListFunction)] = &[
    ("dot", ListFunction::Dot),
    ("norm", ListFunction::Norm),
    ("cross", ListFunction::Cross),
    ("concat", ListFunction::Concat),
];
pub fn list_function(name: &str) -> Option<ListFunction> {
    LIST_FUNCTIONS
        .iter()
        .find(|(registered, _)| *registered == name)
        .map(|(_, function)| *function)
}
impl ListFunction {
    pub fn arity(self) -> usize {
        if matches!(self, Self::Norm) { 1 } else { 2 }
    }
    pub fn argument_type(self) -> crate::Type {
        crate::Type::List(
            Box::new(if matches!(self, Self::Concat) {
                crate::Type::Unknown
            } else {
                crate::Type::union([
                    crate::Type::Numeric(crate::Unit::Number),
                    crate::Type::Fraction,
                ])
                .expect("dimensionless scalar types")
            }),
            crate::ListLength::Unknown,
        )
    }
    pub fn result_type(self, actuals: &[crate::Type]) -> Result<crate::Type, String> {
        use crate::{ListLength, Type, Unit};
        if actuals.len() != self.arity() {
            return Err(format!(
                "numeric List function requires {} arguments",
                self.arity()
            ));
        }
        if matches!(self, Self::Concat) {
            return actuals[0].concatenate(&actuals[1]);
        }
        let mut length = None;
        for ty in actuals {
            let Type::List(_, current) = ty else {
                return Err("numeric List function requires Lists".into());
            };
            if !self.argument_type().accepts(ty) {
                return Err(
                    "numeric List function requires dimensionless Number or Fraction elements"
                        .into(),
                );
            }
            if let ListLength::Exact(current) = current {
                if length.is_some_and(|previous| previous != *current) {
                    return Err("numeric List function requires matching List lengths".into());
                }
                length = Some(*current);
            }
        }
        if matches!(self, Self::Cross) {
            if length.is_some_and(|length| length != 3) {
                return Err("cross requires Lists of length 3".into());
            }
            Ok(Type::List(
                Box::new(Type::Numeric(Unit::Number)),
                ListLength::Exact(3),
            ))
        } else {
            Ok(Type::Numeric(Unit::Number))
        }
    }
    pub fn evaluate(self, values: &[crate::Value]) -> Result<crate::Value, String> {
        self.result_type(&values.iter().map(crate::Value::ty).collect::<Vec<_>>())?;
        if matches!(self, Self::Concat) {
            return super::operations::binary("concat", values[0].clone(), values[1].clone());
        }
        let crate::Value::List(a) = &values[0] else {
            return Err("numeric List required".into());
        };
        if matches!(self, Self::Norm) {
            return Ok(crate::Value::Number(super::functions::evaluate(
                "sqrt",
                &[list_dot(a, a)?],
            )?));
        }
        let crate::Value::List(b) = &values[1] else {
            return Err("numeric List required".into());
        };
        match self {
            Self::Dot => list_dot(a, b).map(crate::Value::Number),
            Self::Cross => list_cross(a, b).map(crate::Value::List),
            Self::Norm | Self::Concat => unreachable!("handled before numeric binary operations"),
        }
    }
}
