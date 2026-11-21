//! Dimensionless cubic Bézier inversion; clocks and value arithmetic remain rational.
use crate::{Fraction, math};
const SOLVE_STEPS: usize = 48;
fn cubic(a: f64, b: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
}
pub fn parameter(
    progress: Fraction,
    outgoing: Fraction,
    incoming: Fraction,
) -> Result<Fraction, String> {
    if progress <= 0.into() {
        return Ok(0.into());
    }
    if progress >= 1.into() {
        return Ok(1.into());
    }
    if outgoing == Fraction::new(1u64, 3u64)
        && incoming == Fraction::new(2u64, 3u64)
        && math::finite(&progress)
    {
        // Default equally spaced time handles make x(t)=t exactly.
        return Ok(progress);
    }
    let p = math::to_f64(progress)?;
    let a = math::to_f64(outgoing)?;
    let b = math::to_f64(incoming)?;
    if !(0.0..=1.0).contains(&a) || !(a..=1.0).contains(&b) {
        return Err("Bézier time handles must stay ordered inside their keyframe interval".into());
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..SOLVE_STEPS {
        let mid = (lo + hi) * 0.5;
        if cubic(a, b, mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    math::functions::approximate((lo + hi) * 0.5)
}
pub fn bezier(values: [Fraction; 4], t: Fraction) -> Result<Fraction, String> {
    let a = math::visual_lerp(values[0], values[1], t)?;
    let b = math::visual_lerp(values[1], values[2], t)?;
    let c = math::visual_lerp(values[2], values[3], t)?;
    math::visual_lerp(math::visual_lerp(a, b, t)?, math::visual_lerp(b, c, t)?, t)
}
/// Catmull finite-difference slope, limited at extrema for monotone auto handles.
pub fn tangent(samples: [(Fraction, Fraction); 3], clamped: bool) -> Result<Fraction, String> {
    let [(ta, a), (tb, b), (tc, c)] = samples;
    let calc = super::arithmetic;
    let before = calc("/", calc("-", b, a)?, calc("-", tb, ta)?)?;
    let after = calc("/", calc("-", c, b)?, calc("-", tc, tb)?)?;
    let central = calc("/", calc("-", c, a)?, calc("-", tc, ta)?)?;
    if !clamped {
        return Ok(central);
    }
    if before == 0.into() || after == 0.into() || (before < 0.into()) != (after < 0.into()) {
        return Ok(0.into());
    }
    let abs = |v: Fraction| if v < 0.into() { -v } else { v };
    let limit = calc("*", 3.into(), abs(before).min(abs(after)))?;
    Ok(if central < 0.into() {
        -abs(central).min(limit)
    } else {
        abs(central).min(limit)
    })
}
