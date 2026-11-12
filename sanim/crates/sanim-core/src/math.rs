use fraction::{Fraction, Sign};
use num_traits::{CheckedAdd, CheckedDiv, CheckedMul, CheckedSub, ToPrimitive, Zero};
use serde::{Deserialize, Serialize};
pub mod functions;
pub mod linear;
pub mod operations;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Unit {
    Number,
    Length,
    Duration,
    Angle,
    Rate,
    Percent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Number {
    pub value: Fraction,
    pub unit: Unit,
}

impl Number {
    pub fn scalar(value: i64) -> Self {
        Self {
            value: Fraction::from(value),
            unit: Unit::Number,
        }
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        let split = text
            .find(|c: char| c.is_ascii_alphabetic() || c == '%')
            .unwrap_or(text.len());
        let (digits, suffix) = text.split_at(split);
        let (unit, scale) = match suffix {
            "" => (Unit::Number, Fraction::from(1)),
            "pt" => (Unit::Length, Fraction::from(1)),
            "px" => (Unit::Length, Fraction::new(3u64, 4u64)),
            "%" => (Unit::Percent, Fraction::new(1u64, 100u64)),
            "s" => (Unit::Duration, Fraction::from(1)),
            "ms" => (Unit::Duration, Fraction::new(1u64, 1000u64)),
            "deg" => (Unit::Angle, Fraction::from(1)),
            "fps" => (Unit::Rate, Fraction::from(1)),
            _ => return Err(format!("unknown numeric unit '{suffix}'")),
        };
        let value = if let Some((numerator, denominator)) = digits.split_once('/') {
            arithmetic("/", decimal(numerator)?, decimal(denominator)?)?
        } else {
            decimal(digits)?
        }
        .checked_mul(&scale)
        .ok_or("numeric overflow")?;
        Ok(Self { value, unit })
    }
    pub fn binary(self, op: &str, rhs: Self) -> Result<Self, String> {
        let unit = match op {
            "+" | "-" | "%" if self.unit == rhs.unit => self.unit,
            "**" if self.unit == Unit::Number && rhs.unit == Unit::Number => Unit::Number,
            "*" if self.unit == Unit::Number => rhs.unit,
            "*" if rhs.unit == Unit::Number => self.unit,
            "/" if rhs.unit == Unit::Number => self.unit,
            "/" if self.unit == rhs.unit => Unit::Number,
            "*" if matches!(
                (self.unit, rhs.unit),
                (Unit::Duration, Unit::Rate) | (Unit::Rate, Unit::Duration)
            ) =>
            {
                Unit::Number
            }
            _ => {
                return Err(format!(
                    "incompatible units {:?} {op} {:?}",
                    self.unit, rhs.unit
                ));
            }
        };
        if matches!(op, "/" | "%") && rhs.value.is_zero() {
            return Err("division by zero".into());
        }
        let value = match op {
            "+" => self.value.checked_add(&rhs.value),
            "-" => self.value.checked_sub(&rhs.value),
            "*" => self.value.checked_mul(&rhs.value),
            "/" => self.value.checked_div(&rhs.value),
            "**" => Some(functions::power(self.value, rhs.value)?),
            "%" => {
                let q = self
                    .value
                    .checked_div(&rhs.value)
                    .ok_or("numeric overflow")?
                    .trunc();
                self.value
                    .checked_sub(&q.checked_mul(&rhs.value).ok_or("numeric overflow")?)
            }
            _ => return Err(format!("unsupported numeric operator {op}")),
        }
        .ok_or("numeric overflow")?;
        if !finite(&value) {
            return Err("non-finite arithmetic result".into());
        }
        Ok(Self { value, unit })
    }
    pub fn usize(self) -> Result<usize, String> {
        if self.unit != Unit::Number
            || self.value < Fraction::zero()
            || self.value.fract() != Fraction::zero()
        {
            return Err("expected a nonnegative integer".into());
        }
        self.value.to_usize().ok_or("integer out of range".into())
    }
}

pub fn decimal(text: &str) -> Result<Fraction, String> {
    let text = text.replace('_', "");
    let (sign, text) = if let Some(v) = text.strip_prefix('-') {
        (Sign::Minus, v)
    } else {
        (Sign::Plus, text.as_str())
    };
    let mut parts = text.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some() || (whole.is_empty() && fraction.is_empty()) {
        return Err("invalid decimal".into());
    }
    let denominator = 10u64
        .checked_pow(fraction.len().try_into().map_err(|_| "decimal too long")?)
        .ok_or("decimal precision overflow")?;
    let digits = format!("{whole}{fraction}")
        .parse::<u64>()
        .map_err(|_| "numeric literal out of range")?;
    Ok(Fraction::new_generic(sign, digits, denominator).ok_or("invalid fraction")?)
}

pub fn ratio(numerator: u64, denominator: u64) -> Fraction {
    Fraction::new(numerator, denominator)
}

/// Checked scalar arithmetic shared by layout, geometry and easing.
pub fn arithmetic(op: &str, a: Fraction, b: Fraction) -> Result<Fraction, String> {
    Number {
        value: a,
        unit: Unit::Number,
    }
    .binary(
        op,
        Number {
            value: b,
            unit: Unit::Number,
        },
    )
    .map(|v| v.value)
}

pub fn lerp(a: Fraction, b: Fraction, t: Fraction) -> Result<Fraction, String> {
    a.checked_add(
        &b.checked_sub(&a)
            .and_then(|d| d.checked_mul(&t))
            .ok_or("interpolation overflow")?,
    )
    .ok_or("interpolation overflow".into())
}

pub fn progress(time: Fraction, start: Fraction, end: Fraction) -> Result<Fraction, String> {
    if time <= start {
        return Ok(Fraction::zero());
    }
    if time >= end {
        return Ok(Fraction::from(1));
    }
    time.checked_sub(&start)
        .and_then(|v| end.checked_sub(&start).and_then(|d| v.checked_div(&d)))
        .ok_or("time arithmetic overflow".into())
}

/// The dependency's is_finite accepts NaN; only rational variants are valid here.
pub fn finite(value: &Fraction) -> bool {
    matches!(value, Fraction::Rational(..))
}

pub fn to_f64(value: Fraction) -> Result<f64, String> {
    value
        .to_f64()
        .filter(|v| v.is_finite())
        .ok_or("number cannot be represented as finite f64".into())
}

pub fn from_f64(value: f64) -> Result<Fraction, String> {
    let rational = Fraction::from(value);
    if !value.is_finite() || !finite(&rational) || to_f64(rational).ok() != Some(value) {
        return Err("floating value cannot be represented as a scalar Fraction".into());
    }
    Ok(rational)
}

/// Floating geometry and linear algebra reject non-finite inputs and results.
pub fn float_arithmetic(op: &str, a: f64, b: f64) -> Result<f64, String> {
    if !a.is_finite() || !b.is_finite() {
        return Err("non-finite vector or matrix component".into());
    }
    if op == "/" && b == 0.0 {
        return Err("division by zero".into());
    }
    let value = match op {
        "+" => a + b,
        "-" => a - b,
        "*" => a * b,
        "/" => a / b,
        _ => return Err(format!("unsupported floating operator '{op}'")),
    };
    if !value.is_finite() {
        return Err("floating arithmetic overflow".into());
    }
    Ok(value)
}

pub fn float_lerp(a: f64, b: f64, t: Fraction) -> Result<f64, String> {
    let t = to_f64(t)?;
    if !(0.0..=1.0).contains(&t) {
        return Err("interpolation progress must be between zero and one".into());
    }
    if !a.is_finite() || !b.is_finite() {
        return Err("interpolation endpoints must be finite".into());
    }
    if a == b || t == 0.0 {
        return Ok(a);
    }
    if t == 1.0 {
        return Ok(b);
    }
    // Keep rounded interpolation inside the endpoint envelope used by proofs.
    let value = float_arithmetic(
        "+",
        float_arithmetic("*", a, 1.0 - t)?,
        float_arithmetic("*", b, t)?,
    )?;
    Ok(value.clamp(a.min(b), a.max(b)))
}

/// Exact number of frame samples in the half-open animation interval [0, duration).
pub fn frame_count(duration: Fraction, rate: Fraction) -> Result<u64, String> {
    if !finite(&duration)
        || !finite(&rate)
        || duration < Fraction::zero()
        || rate <= Fraction::zero()
    {
        return Err("video duration must be nonnegative and frame rate positive and finite".into());
    }
    let frames = duration
        .checked_mul(&rate)
        .ok_or("video frame count overflow")?;
    let Fraction::Rational(_, ratio) = frames else {
        return Err("video frame count overflow".into());
    };
    let count = ratio.numer() / ratio.denom();
    count
        .checked_add(u64::from(ratio.numer() % ratio.denom() != 0))
        .ok_or("video frame count overflow".into())
}

/// Every prepared animation has a finite, shared hard maximum of two hours.
pub const MAX_TIMELINE_SECONDS: u64 = 2 * 60 * 60;
pub fn timeline_end(start: Fraction, duration: Fraction) -> Result<Fraction, String> {
    if !finite(&start)
        || !finite(&duration)
        || start < Fraction::zero()
        || duration < Fraction::zero()
    {
        return Err("timeline time and duration must be finite and nonnegative".into());
    }
    let end = start
        .checked_add(&duration)
        .ok_or("timeline duration overflow")?;
    if end > Fraction::from(MAX_TIMELINE_SECONDS) {
        return Err(format!(
            "duration overflow: resolved end {end}s exceeds the two-hour limit ({MAX_TIMELINE_SECONDS}s)"
        ));
    }
    Ok(end)
}
