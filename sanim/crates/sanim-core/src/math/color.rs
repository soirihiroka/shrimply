//! Shared float color operations; conversion matrices and ramps live in math-color.
pub use shrimply_math_color::perceptual::*;
mod operations;
mod spaces;
pub use operations::Op;
pub use spaces::Space;

pub fn from_rgba(color: [f32; 4]) -> Result<crate::Color, String> {
    validate_rgba(color)?;
    let mut channels = [crate::Fraction::from(0); 4];
    for (out, channel) in channels.iter_mut().zip(color) {
        *out = super::functions::approximate(f64::from(channel))?;
    }
    Ok(crate::Color(channels))
}
