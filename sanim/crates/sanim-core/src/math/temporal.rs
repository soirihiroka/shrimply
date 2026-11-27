//! Exact temporal layout uses the same alignment choices as spatial flex layout.
use crate::{Fraction, Justify, math};
mod overlap;

pub fn alignment(
    justify: Justify,
    available: Fraction,
    occupied: Fraction,
    count: usize,
) -> Result<(Fraction, Fraction), String> {
    let zero = Fraction::from(0);
    if count == 0 || justify == Justify::Start {
        return Ok((zero, zero));
    }
    let free = math::arithmetic("-", available, occupied)?.max(zero);
    let divide = |n: usize| math::arithmetic("/", free, Fraction::from(n as u64));
    Ok(match justify {
        Justify::Start => (zero, zero),
        Justify::End => (free, zero),
        Justify::Center => (divide(2)?, zero),
        Justify::SpaceBetween if count > 1 => (zero, divide(count - 1)?),
        Justify::SpaceAround => (
            math::arithmetic("/", divide(count)?, 2.into())?,
            divide(count)?,
        ),
        Justify::SpaceEvenly => (divide(count + 1)?, divide(count + 1)?),
        _ => (zero, zero),
    })
}

/// A host viewport is a finite crop, not a finite program scheduling operation.
pub fn viewport(value: Fraction) -> Result<Fraction, String> {
    if !math::finite(&value) || value < Fraction::from(0) {
        Err("temporal viewport must be finite and nonnegative".into())
    } else {
        Ok(value)
    }
}

pub fn nonnegative(name: &str, value: Fraction) -> Result<Fraction, String> {
    if !math::finite(&value) || value < 0.into() {
        return Err(format!(
            "Animation {name} must be a finite nonnegative duration"
        ));
    }
    math::timeline_end(0.into(), value)
}

/// Host placement may exceed two hours without lengthening an individual finite animation.
pub fn end(start: Fraction, span: Fraction) -> Result<Fraction, String> {
    viewport(start)?;
    viewport(span)?;
    math::arithmetic("+", start, span)
}
pub fn writer_end(start: Fraction, duration: Fraction) -> Result<Fraction, String> {
    nonnegative("writer duration", duration)?;
    end(start, duration)
}

/// A compiled writer keeps its natural interpolation span while its active window is cropped.
#[derive(Clone, Copy, Debug, Default)]
pub struct Window {
    pub cycle: Option<crate::math::cycle::Cycle>,
    pub clip: Option<(Fraction, Fraction)>,
}
#[derive(Clone, Copy, Debug)]
pub struct Interval {
    pub start: Fraction,
    pub end: Fraction,
    pub interpolation: Option<(Fraction, Fraction)>,
    pub cycle: Option<crate::math::cycle::Cycle>,
}
impl Window {
    pub fn interval(self, start: Fraction, duration: Fraction) -> Result<Option<Interval>, String> {
        let natural_end = end(start, duration)?;
        let (active_start, active_end) = if let Some((a, b)) = self.clip {
            if b < a {
                return Err("temporal viewport ends before it starts".into());
            }
            viewport(a)?;
            viewport(b)?;
            if start > b
                || duration > Fraction::from(0) && start >= b
                || natural_end < a
                || duration > Fraction::from(0) && b == a
            {
                return Ok(None);
            }
            (start.max(a), natural_end.min(b))
        } else {
            (start, natural_end)
        };
        if duration > Fraction::from(0) && active_end <= active_start {
            return Ok(None);
        }
        let cycle = self.cycle;
        if duration == Fraction::from(0) && cycle.is_some_and(|clock| start == clock.end()) {
            return Err("an instantaneous write at the end of a repeat cycle has no stable initial phase; move the reset to the beginning of the cycle".into());
        }
        if cycle.is_some_and(|clock| active_start < clock.start() || active_end > clock.end()) {
            return Err(
                "animation action must finish within its finite repeat cycle after clipping".into(),
            );
        }
        Ok(Some(Interval {
            start: active_start,
            end: active_end,
            interpolation: ((active_start, active_end) != (start, natural_end))
                .then_some((start, natural_end)),
            cycle,
        }))
    }
}

pub use shrimply_math_core::{SmpteTimecode, format_smpte_timecode, smpte_timecode};

pub fn frame_index(seconds: Fraction, rate: Fraction) -> Result<i64, String> {
    shrimply_math_core::frame_index(shrimply_math_core::Time { seconds }, rate)
        .ok_or_else(|| "frame index requires finite time and a positive rate in range".into())
}
