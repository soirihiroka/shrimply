//! Float-precision color conversions, bounded gamut mapping and alpha-aware ramps.
//! Colors are straight sRGBA; only interpolation/compositing premultiplies alpha.
mod conversion;
mod gamut;
mod mix;

pub use conversion::{
    hsl_to_rgba, hsv_to_rgba, oklab_to_oklch, oklab_to_rgba, oklch_to_oklab, rgba_to_hsl,
    rgba_to_hsv, rgba_to_oklab,
};
pub use gamut::{Gamut, in_gamut, map_oklab};
pub use mix::{HueInterpolation, MixOptions, MixSpace, Ramp, mix_rgba};

pub const ACHROMATIC_EPSILON: f32 = 0.000_004;

pub fn validate_rgba(color: [f32; 4]) -> Result<(), String> {
    if color
        .into_iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        Err("color channels must be finite and within 0..=1".into())
    } else {
        Ok(())
    }
}

pub fn relative_luminance(color: [f32; 4]) -> Result<f32, String> {
    validate_rgba(color)?;
    let color = crate::Color::<f32>::from(color).to_linear();
    Ok(0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b)
}

/// Relative contrast is defined after transparent colors are placed on a background.
pub fn contrast(left: [f32; 4], right: [f32; 4]) -> Result<f32, String> {
    if left[3] != 1.0 || right[3] != 1.0 {
        return Err("contrast requires opaque colors; composite transparent colors onto an opaque background first".into());
    }
    let left = relative_luminance(left)?;
    let right = relative_luminance(right)?;
    Ok((left.max(right) + 0.05) / (left.min(right) + 0.05))
}

/// Source-over in linear light, preserving straight RGBA at the public boundary.
pub fn over(source: [f32; 4], background: [f32; 4]) -> Result<[f32; 4], String> {
    validate_rgba(source)?;
    validate_rgba(background)?;
    if source[3] == 0.0 && background[3] == 0.0 {
        return Ok([0.0; 4]);
    }
    if source[3] == 1.0 || background[3] == 0.0 {
        return Ok(source);
    }
    if source[3] == 0.0 {
        return Ok(background);
    }
    let source = crate::Color::<f32>::from(source).to_linear().to_array();
    let background = crate::Color::<f32>::from(background).to_linear().to_array();
    let alpha = source[3] + background[3] * (1.0 - source[3]);
    if alpha == 0.0 {
        return Ok([0.0; 4]);
    }
    Ok(crate::Color::<f32>::from([
        (source[0] * source[3] + background[0] * background[3] * (1.0 - source[3])) / alpha,
        (source[1] * source[3] + background[1] * background[3] * (1.0 - source[3])) / alpha,
        (source[2] * source[3] + background[2] * background[3] * (1.0 - source[3])) / alpha,
        alpha,
    ])
    .linear_to_srgb::<true>()
    .to_array())
}
