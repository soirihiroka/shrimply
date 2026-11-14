use shrimply_math_core::{Fraction, fraction_as_f64};

const TARGET_TICK_PIXELS: f64 = 120.0;

pub fn nice_tick_step(seconds_per_pixel: Fraction) -> Fraction {
    // Preserve the desktop ruler's rounding at visual zoom thresholds. Only
    // spacing selection uses drawing coordinates; tick timestamps stay exact.
    let raw = (fraction_as_f64(seconds_per_pixel) * TARGET_TICK_PIXELS).max(0.001);
    let magnitude = 10_f64.powf(raw.log10().floor());
    for multiplier in [1.0, 2.0, 5.0, 10.0] {
        let step = magnitude * multiplier;
        if step >= raw {
            return Fraction::from(step);
        }
    }
    Fraction::from(magnitude * 10.0)
}
