//! Local gradient coordinates are derived from the same path bounds used by Skia.
use super::{float_arithmetic as calc, to_f64};
use crate::Fraction;
/// Interpolate bounded native color channels without changing glyph identity.
pub fn color_lerp(
    from: &crate::Color,
    to: &crate::Color,
    progress: Fraction,
) -> Result<crate::Color, String> {
    let mut color = to.clone();
    for (channel, (from, to)) in color.0.iter_mut().zip(from.0.iter().zip(&to.0)) {
        *channel = super::visual_lerp(*from, *to, progress)?;
    }
    Ok(color)
}

/// Remove the old changed span before revealing its replacement. Stable code
/// continues moving throughout, so newly inserted glyphs do not cover a suffix
/// before its interpolated layout has opened the required space.
pub fn edit_opacities(progress: Fraction) -> Result<[Fraction; 2], String> {
    let doubled = super::visual_product(progress, Fraction::from(2))?;
    Ok([
        super::arithmetic("-", Fraction::from(1), doubled)?.max(Fraction::from(0)),
        super::arithmetic("-", doubled, Fraction::from(1))?
            .min(Fraction::from(1))
            .max(Fraction::from(0)),
    ])
}

pub fn gradient_endpoints(bounds: [f64; 4], angle: Fraction) -> Result<[[f64; 2]; 2], String> {
    if bounds.iter().any(|value| !value.is_finite()) {
        return Err("gradient path bounds must be finite".into());
    }
    let angle = to_f64(angle)?.to_radians();
    let (dy, dx) = angle.sin_cos();
    let width = calc("-", bounds[2], bounds[0])?;
    let height = calc("-", bounds[3], bounds[1])?;
    let reach = calc(
        "+",
        calc("*", width, dx.abs())?,
        calc("*", height, dy.abs())?,
    )? / 2.;
    let center = [
        calc("+", bounds[0] / 2., bounds[2] / 2.)?,
        calc("+", bounds[1] / 2., bounds[3] / 2.)?,
    ];
    Ok([
        [
            calc("-", center[0], dx * reach)?,
            calc("-", center[1], dy * reach)?,
        ],
        [
            calc("+", center[0], dx * reach)?,
            calc("+", center[1], dy * reach)?,
        ],
    ])
}

/// Alpha at a local path coordinate, using native linear-gradient clamp semantics.
pub fn alpha_at(paint: &crate::Paint, bounds: [f64; 4], point: [f64; 2]) -> Result<f64, String> {
    match paint {
        crate::Paint::Solid(color) => to_f64(color.0[3]),
        crate::Paint::LinearGradient { angle, stops } => {
            let [start, end] = gradient_endpoints(bounds, *angle)?;
            let delta = [end[0] - start[0], end[1] - start[1]];
            let squared = delta[0] * delta[0] + delta[1] * delta[1];
            let offset = if squared == 0.0 {
                1.0
            } else {
                (((point[0] - start[0]) * delta[0] + (point[1] - start[1]) * delta[1]) / squared)
                    .clamp(0.0, 1.0)
            };
            let right = stops.partition_point(|stop| {
                to_f64(stop.offset).expect("validated gradient offset") <= offset
            });
            if right == 0 {
                return to_f64(stops[0].color.0[3]);
            }
            if right == stops.len() {
                return to_f64(stops[right - 1].color.0[3]);
            }
            let left = &stops[right - 1];
            let right = &stops[right];
            let interval = to_f64(right.offset)? - to_f64(left.offset)?;
            let amount = if interval == 0.0 {
                1.0
            } else {
                (offset - to_f64(left.offset)?) / interval
            };
            let left = to_f64(left.color.0[3])?;
            Ok(left + (to_f64(right.color.0[3])? - left) * amount)
        }
    }
}
