//! Numeric algebra over canonical persistent Lists. Matrices are row-major.
use super::float_arithmetic as calc;

pub mod matrix;

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
    Transpose,
    Inverse,
}
pub const LIST_FUNCTIONS: &[(&str, ListFunction)] = &[
    ("dot", ListFunction::Dot),
    ("norm", ListFunction::Norm),
    ("cross", ListFunction::Cross),
    ("concat", ListFunction::Concat),
    ("transpose", ListFunction::Transpose),
    ("inverse", ListFunction::Inverse),
];
pub fn list_function(name: &str) -> Option<ListFunction> {
    LIST_FUNCTIONS
        .iter()
        .find(|(registered, _)| *registered == name)
        .map(|(_, function)| *function)
}
impl ListFunction {
    pub fn arity(self) -> usize {
        if matches!(self, Self::Norm | Self::Transpose | Self::Inverse) {
            1
        } else {
            2
        }
    }
    pub fn argument_type(self) -> crate::Type {
        if matches!(self, Self::Transpose | Self::Inverse) {
            let scalar = crate::Type::union([
                crate::Type::Numeric(crate::Unit::Number),
                crate::Type::Fraction,
            ])
            .expect("numeric scalar types");
            return crate::Type::List(
                Box::new(crate::Type::List(
                    Box::new(scalar),
                    crate::ListLength::Unknown,
                )),
                crate::ListLength::Unknown,
            );
        }
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
        if matches!(self, Self::Transpose | Self::Inverse) {
            return matrix::function_type(self, &actuals[0]);
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
        if matches!(self, Self::Transpose | Self::Inverse) {
            return matrix::evaluate(self, a);
        }
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
            Self::Norm | Self::Concat | Self::Transpose | Self::Inverse => {
                unreachable!("handled before numeric binary operations")
            }
        }
    }
}
