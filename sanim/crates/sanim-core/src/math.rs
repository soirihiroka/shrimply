use fraction::{Fraction, Sign};
use num_traits::{CheckedAdd, CheckedDiv, CheckedMul, CheckedSub, ToPrimitive, Zero};
use serde::{Deserialize, Serialize};
pub mod chart;
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

// Parsing aliases and canonical display spellings share one unit definition.
const UNIT_SPELLINGS: &[(&str, Unit, u64, u64, bool)] = &[
    ("", Unit::Number, 1, 1, true),
    ("pt", Unit::Length, 1, 1, true),
    ("px", Unit::Length, 3, 4, false),
    ("s", Unit::Duration, 1, 1, true),
    ("m", Unit::Duration, 60, 1, false),
    ("ms", Unit::Duration, 1, 1000, false),
    ("deg", Unit::Angle, 1, 1, true),
    ("fps", Unit::Rate, 1, 1, true),
    ("%", Unit::Percent, 1, 100, true),
];

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
        let (negative, unsigned) = text
            .strip_prefix('-')
            .map_or((false, text), |text| (true, text));
        // Fraction spellings are used by serialized values and inspector controls.
        if unsigned.contains('/') {
            return Self::parse_single(text);
        }
        let mut remaining = unsigned;
        let mut result: Option<Self> = None;
        let mut previous_scale = None;
        while !remaining.is_empty() {
            let length = component_len(remaining);
            if length == 0 {
                return Err("expected a number followed by a unit, such as 0.5s or 4m1s".into());
            }
            let component = &remaining[..length];
            let value = Self::parse_single(component)?;
            let suffix =
                component.trim_start_matches(|c: char| c.is_ascii_digit() || c == '_' || c == '.');
            let scale = UNIT_SPELLINGS
                .iter()
                .find(|(name, ..)| *name == suffix)
                .map(|(_, _, numerator, denominator, _)| Fraction::new(*numerator, *denominator))
                .expect("parsed numeric unit");
            if let Some(total) = result {
                if value.unit != Unit::Duration || total.unit != Unit::Duration {
                    return Err("only durations can combine units; write 4m1s, or separate other values with an operator".into());
                }
                if previous_scale.is_some_and(|previous| scale >= previous) {
                    return Err(
                        "compound duration units must occur once in decreasing order: m, s, ms"
                            .into(),
                    );
                }
                result = Some(total.binary("+", value)?);
            } else {
                result = Some(value);
            }
            previous_scale = Some(scale);
            remaining = &remaining[length..];
        }
        let mut result = result.ok_or("empty numeric literal")?;
        if negative {
            result.value = -result.value;
        }
        Ok(result)
    }
    fn parse_single(text: &str) -> Result<Self, String> {
        let split = text
            .find(|c: char| c.is_ascii_alphabetic() || c == '%')
            .unwrap_or(text.len());
        let (digits, suffix) = text.split_at(split);
        let &(_, unit, numerator, denominator, _) = UNIT_SPELLINGS
            .iter()
            .find(|(name, ..)| *name == suffix)
            .ok_or_else(|| format!("unknown numeric unit '{suffix}'"))?;
        let scale = Fraction::new(numerator, denominator);
        let value = if let Some((numerator, denominator)) = digits.split_once('/') {
            arithmetic("/", decimal(numerator)?, decimal(denominator)?)?
        } else {
            decimal(digits)?
        }
        .checked_mul(&scale)
        .ok_or("numeric overflow")?;
        Ok(Self { value, unit })
    }
    /// Exact visible magnitude and suffix, shared by serialization and editor controls.
    pub fn display_parts(self) -> Result<(Fraction, &'static str), String> {
        if !finite(&self.value) {
            return Err("non-finite values cannot be displayed as numbers".into());
        }
        let &(suffix, _, numerator, denominator, _) = UNIT_SPELLINGS
            .iter()
            .find(|(_, unit, _, _, canonical)| *unit == self.unit && *canonical)
            .expect("every Unit has a canonical spelling");
        Ok((
            arithmetic("/", self.value, Fraction::new(numerator, denominator))?,
            suffix,
        ))
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

fn component_len(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = bytes
        .iter()
        .take_while(|c| c.is_ascii_digit() || **c == b'_')
        .count();
    if count == 0 {
        return 0;
    }
    if bytes.get(count) == Some(&b'.') && bytes.get(count + 1).is_some_and(u8::is_ascii_digit) {
        count += 1;
        while bytes
            .get(count)
            .is_some_and(|c| c.is_ascii_digit() || *c == b'_')
        {
            count += 1;
        }
    }
    while bytes.get(count).is_some_and(u8::is_ascii_alphabetic) {
        count += 1;
    }
    if bytes.get(count) == Some(&b'%') && !bytes.get(count + 1).is_some_and(u8::is_ascii_digit) {
        count += 1;
    }
    count
}

/// Extent of one source numeric literal, including adjacent duration components.
pub fn number_literal_len(text: &str) -> usize {
    let mut count = component_len(text);
    while text.as_bytes().get(count).is_some_and(u8::is_ascii_digit) {
        let next = component_len(&text[count..]);
        if next == 0 {
            break;
        }
        count += next;
    }
    count
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
    let bad = || {
        "floating value cannot be represented exactly as a scalar Fraction; its magnitude or binary precision exceeds a 64-bit numerator/denominator".to_string()
    };
    if !value.is_finite() {
        return Err(bad());
    }
    if value == 0.0 {
        return Ok(Fraction::zero());
    }
    let (mut numerator, exponent, sign) = num_traits::Float::integer_decode(value);
    let denominator = if exponent < 0 {
        let power = u32::from(exponent.unsigned_abs());
        let reduction = numerator.trailing_zeros().min(power);
        numerator >>= reduction;
        1u64.checked_shl(power - reduction).ok_or_else(bad)?
    } else {
        numerator = numerator
            .checked_mul(1u64.checked_shl(exponent as u32).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        1
    };
    Fraction::new_generic(
        if sign < 0 { Sign::Minus } else { Sign::Plus },
        numerator,
        denominator,
    )
    .ok_or_else(bad)
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

/// Quintic easing with zero first and second derivatives at both endpoints.
/// This is the `smooth` curve used by the bundled ManimGL renderer.
pub fn smootherstep(p: Fraction) -> Result<Fraction, String> {
    visual_polynomial(p, &[0, 0, 0, 10, -15, 6])
}

/// ManimGL's degree-five Bezier with controls [0, 0, 1.5, 1.5, 1, 1].
pub fn overshoot(p: Fraction) -> Result<Fraction, String> {
    visual_polynomial(p, &[0, 0, 15, -30, 20, -4])
}

/// Keep ordinary frame fractions exact, while supporting arbitrary seek
/// precision without denominator overflow in a visual-only polynomial.
pub fn visual_polynomial(progress: Fraction, coefficients: &[i64]) -> Result<Fraction, String> {
    if !finite(&progress) {
        return Err("easing progress must be finite".into());
    }
    if let Some(value) =
        coefficients
            .iter()
            .rev()
            .try_fold(Fraction::zero(), |value, coefficient| {
                value
                    .checked_mul(&progress)?
                    .checked_add(&Fraction::from(*coefficient))
            })
    {
        return Ok(value);
    }
    let p = to_f64(progress)?;
    let value = coefficients
        .iter()
        .rev()
        .fold(0., |value, coefficient| value * p + *coefficient as f64);
    functions::approximate_to(value, 1_000_000)
}

/// Closed-form unit-mass damped spring, evaluated directly for arbitrary seeks.
/// The final fifth smoothly removes its tiny residual displacement so the
/// bounded timeline ends exactly at rest without truncating a live oscillation.
pub fn spring(progress: Fraction) -> Result<Fraction, String> {
    spring_response(progress, 1.)
}

/// A lightly underdamped alternative for unconstrained spatial motion.
pub fn spring_bouncy(progress: Fraction) -> Result<Fraction, String> {
    spring_response(progress, 0.78)
}

fn spring_response(progress: Fraction, damping_ratio: f64) -> Result<Fraction, String> {
    if !finite(&progress) {
        return Err("spring progress must be finite".into());
    }
    if progress <= Fraction::zero() {
        return Ok(Fraction::zero());
    }
    if progress >= Fraction::from(1) {
        return Ok(Fraction::from(1));
    }
    const ANGULAR_FREQUENCY: f64 = 12.;
    const SETTLE_START: f64 = 0.8;
    let time = to_f64(progress)?;
    let residual = if damping_ratio == 1. {
        (-ANGULAR_FREQUENCY * time).exp() * (1. + ANGULAR_FREQUENCY * time)
    } else {
        let oscillation = (1. - damping_ratio * damping_ratio).sqrt();
        let angle = ANGULAR_FREQUENCY * oscillation * time;
        (-damping_ratio * ANGULAR_FREQUENCY * time).exp()
            * (angle.cos() + damping_ratio / oscillation * angle.sin())
    };
    let window = if time > SETTLE_START {
        let t = (time - SETTLE_START) / (1. - SETTLE_START);
        1. - t * t * t * (10. + t * (-15. + 6. * t))
    } else {
        1.
    };
    functions::approximate_to(1. - residual * window, 1_000_000)
}

/// Visual composition may approximate if multiplying valid rational inputs
/// would overflow their representation. Timeline and ordinary math stay exact.
pub fn visual_product(a: Fraction, b: Fraction) -> Result<Fraction, String> {
    a.checked_mul(&b).filter(finite).map_or_else(
        || functions::approximate_to(float_arithmetic("*", to_f64(a)?, to_f64(b)?)?, 1_000_000),
        Ok,
    )
}

pub fn visual_lerp(a: Fraction, b: Fraction, progress: Fraction) -> Result<Fraction, String> {
    if progress.is_zero() {
        return Ok(a);
    }
    if progress == Fraction::from(1) {
        return Ok(b);
    }
    lerp(a, b, progress).or_else(|_| {
        let value = functions::approximate_to(
            motion_lerp(to_f64(a)?, to_f64(b)?, progress, true)?,
            1_000_000,
        )?;
        Ok(
            if progress >= Fraction::zero() && progress <= Fraction::from(1) {
                value.max(a.min(b)).min(a.max(b))
            } else {
                value
            },
        )
    })
}

/// Interpolate fixed-size vectors without allocating temporary component lists.
pub fn motion_vector_lerp<const N: usize>(
    a: [f64; N],
    b: [f64; N],
    progress: Fraction,
    overshooting: bool,
) -> Result<[f64; N], String> {
    let mut out = a;
    for (index, value) in out.iter_mut().enumerate() {
        *value = motion_lerp(a[index], b[index], progress, overshooting)?;
    }
    Ok(out)
}

pub fn motion_lerp(a: f64, b: f64, progress: Fraction, overshooting: bool) -> Result<f64, String> {
    if !overshooting || progress <= Fraction::from(1) {
        return float_lerp(a, b, progress);
    }
    float_arithmetic(
        "+",
        a,
        float_arithmetic("*", float_arithmetic("-", b, a)?, to_f64(progress)?)?,
    )
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
