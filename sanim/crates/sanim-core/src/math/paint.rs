//! Local gradient coordinates are derived from the same path bounds used by Skia.
use super::{float_arithmetic as calc, to_f64};
use crate::Fraction;
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
