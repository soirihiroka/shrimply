//! Common math operations. Transcendental results are explicit finite approximations;
//! integral powers and rounding retain exact scalar arithmetic.
use super::{finite, from_f64, to_f64};
use crate::{Fraction, Number, Unit};
use fraction::Sign;
use num_traits::{CheckedDiv, CheckedMul, Zero};

pub const CONSTANTS: &[(&str, f64)] = &[
    ("PI", std::f64::consts::PI),
    ("TAU", std::f64::consts::TAU),
    ("E", std::f64::consts::E),
    ("SQRT_2", std::f64::consts::SQRT_2),
    ("LN_2", std::f64::consts::LN_2),
    ("LN_10", std::f64::consts::LN_10),
    ("LOG2_E", std::f64::consts::LOG2_E),
    ("LOG10_E", std::f64::consts::LOG10_E),
    ("FRAC_PI_2", std::f64::consts::FRAC_PI_2),
    ("FRAC_PI_4", std::f64::consts::FRAC_PI_4),
    ("FRAC_1_PI", std::f64::consts::FRAC_1_PI),
    ("FRAC_1_SQRT_2", std::f64::consts::FRAC_1_SQRT_2),
];
pub fn constant(name: &str) -> Option<f64> {
    CONSTANTS.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Scalar,
    Trig,
    Numeric,
}
pub struct Function {
    pub name: &'static str,
    pub parameters: &'static [(&'static str, bool)],
    pub domain: Domain,
}
macro_rules! unary {
    ($($name:literal => $domain:ident),* $(,)?) => { &[$(Function { name:$name, parameters:&[("value",true)], domain:Domain::$domain }),*] };
}
pub const UNARY: &[Function] = unary![
    "sin"=>Trig,"cos"=>Trig,"tan"=>Trig,"asin"=>Scalar,"acos"=>Scalar,"atan"=>Scalar,
    "sqrt"=>Scalar,"cbrt"=>Scalar,"exp"=>Scalar,"exp2"=>Scalar,
    "ln"=>Scalar,"log2"=>Scalar,"log10"=>Scalar,
    "abs"=>Numeric,"floor"=>Numeric,"ceil"=>Numeric,"round"=>Numeric,"trunc"=>Numeric,"sign"=>Numeric,
];
pub const MULTI: &[Function] = &[
    Function {
        name: "min",
        parameters: &[("a", true), ("b", true)],
        domain: Domain::Numeric,
    },
    Function {
        name: "max",
        parameters: &[("a", true), ("b", true)],
        domain: Domain::Numeric,
    },
    Function {
        name: "clamp",
        parameters: &[("value", true), ("low", true), ("high", true)],
        domain: Domain::Numeric,
    },
    Function {
        name: "log",
        parameters: &[("value", true), ("base", false)],
        domain: Domain::Scalar,
    },
    Function {
        name: "pow",
        parameters: &[("base", true), ("exponent", true)],
        domain: Domain::Scalar,
    },
    Function {
        name: "atan2",
        parameters: &[("y", true), ("x", true)],
        domain: Domain::Scalar,
    },
    Function {
        name: "hypot",
        parameters: &[("a", true), ("b", true)],
        domain: Domain::Numeric,
    },
];
pub fn functions() -> impl Iterator<Item = &'static Function> {
    UNARY.iter().chain(MULTI)
}
pub fn function(name: &str) -> Option<&'static Function> {
    functions().find(|f| f.name == name)
}

pub(super) fn approximate(value: f64) -> Result<Fraction, String> {
    if !value.is_finite() {
        return Err("math result is not finite (domain error or overflow)".into());
    }
    // Transcendental output is approximate already. Bound its denominator so
    // ordinary follow-up operations retain arithmetic headroom (sin(x)*0.25+1).
    // This does not quantize source literals, explicit fractions, or timeline time.
    const MAX_APPROXIMATE_DENOMINATOR: u64 = 1_000_000_000;
    if let Ok(exact) = from_f64(value)
        && let Fraction::Rational(_, ratio) = exact
        && *ratio.denom() <= MAX_APPROXIMATE_DENOMINATOR
    {
        return Ok(exact);
    }
    // Quantize only explicit approximate math results. Time and ordinary scalar
    // arithmetic never use this conversion. Leave headroom for float rounding.
    let denominator = ((u64::MAX as f64 / (value.abs().max(1.) * 2.)).floor() as u64)
        .min(MAX_APPROXIMATE_DENOMINATOR);
    if denominator == 0 {
        return Err("math result exceeds scalar numeric range".into());
    }
    let numerator = (value.abs() * denominator as f64).round() as u64;
    Ok(Fraction::new_generic(
        if value < 0. { Sign::Minus } else { Sign::Plus },
        numerator,
        denominator,
    )
    .ok_or("math result overflow")?)
}
pub fn power(base: Fraction, exponent: Fraction) -> Result<Fraction, String> {
    if !finite(&base) || !finite(&exponent) {
        return Err("power requires finite operands".into());
    }
    if exponent.fract().is_zero() {
        let Fraction::Rational(sign, ratio) = exponent else {
            unreachable!()
        };
        let mut count = *ratio.numer();
        let mut factor = if sign == Sign::Minus {
            if base.is_zero() {
                return Err("zero cannot be raised to a negative exponent".into());
            }
            Fraction::from(1)
                .checked_div(&base)
                .ok_or("power overflow")?
        } else {
            base
        };
        let mut result = Fraction::from(1);
        while count != 0 {
            if count & 1 != 0 {
                result = result.checked_mul(&factor).ok_or("power overflow")?;
            }
            count >>= 1;
            if count != 0 {
                factor = factor.checked_mul(&factor).ok_or("power overflow")?;
            }
        }
        return Ok(result);
    }
    let a = to_f64(base)?;
    let b = to_f64(exponent)?;
    if a < 0. {
        return Err("negative base requires an integral exponent for a real result".into());
    }
    if a == 0. && b < 0. {
        return Err("zero cannot be raised to a negative exponent".into());
    }
    approximate(a.powf(b))
}
pub fn evaluate(name: &str, values: &[Number]) -> Result<Number, String> {
    let first = values.first().ok_or("math function needs an argument")?;
    if name == "pow" {
        return Ok(Number {
            value: power(first.value, values[1].value)?,
            unit: Unit::Number,
        });
    }
    if matches!(
        name,
        "abs" | "min" | "max" | "clamp" | "floor" | "ceil" | "round" | "trunc" | "sign"
    ) {
        let argument = |index: usize| {
            values
                .get(index)
                .map(|number| number.value)
                .ok_or_else(|| format!("{name} is missing numeric argument {index}"))
        };
        let value = match name {
            "abs" => {
                if first.value < Fraction::zero() {
                    -first.value
                } else {
                    first.value
                }
            }
            "min" => first.value.min(argument(1)?),
            "max" => first.value.max(argument(1)?),
            "clamp" => {
                let (low, high) = (argument(1)?, argument(2)?);
                if low > high {
                    return Err("clamp low must not exceed high".into());
                }
                first.value.max(low).min(high)
            }
            "floor" => first.value.floor(),
            "ceil" => first.value.ceil(),
            "round" => first.value.round(),
            "trunc" => first.value.trunc(),
            _ => Fraction::from(if first.value.is_zero() {
                0
            } else if first.value < Fraction::zero() {
                -1
            } else {
                1
            }),
        };
        return Ok(Number {
            value,
            unit: if name == "sign" {
                Unit::Number
            } else {
                first.unit
            },
        });
    }
    let raw = to_f64(first.value)?;
    let x = if first.unit == Unit::Angle && function(name).is_some_and(|f| f.domain == Domain::Trig)
    {
        raw.to_radians()
    } else {
        raw
    };
    let second = || {
        values
            .get(1)
            .map(|n| to_f64(n.value))
            .unwrap_or(Ok(std::f64::consts::E))
    };
    let value = match name {
        "sin" => x.sin(),
        "cos" => x.cos(),
        "tan" => x.tan(),
        "asin" => x.asin(),
        "acos" => x.acos(),
        "atan" => x.atan(),
        "atan2" => x.atan2(second()?),
        "sqrt" => x.sqrt(),
        "cbrt" => x.cbrt(),
        "exp" => x.exp(),
        "exp2" => x.exp2(),
        "ln" => x.ln(),
        "log2" => x.log2(),
        "log10" => x.log10(),
        "log" => {
            let base = second()?;
            if x <= 0. || base <= 0. || base == 1. {
                return Err("log requires value > 0 and base > 0, base != 1".into());
            }
            x.log(base)
        }
        "hypot" => x.hypot(second()?),
        _ => return Err(format!("unknown math function '{name}'")),
    };
    approximate(value)
        .map(|value| Number {
            value,
            unit: if name == "hypot" {
                first.unit
            } else {
                Unit::Number
            },
        })
        .map_err(|e| format!("{name}({x}): {e}"))
}
