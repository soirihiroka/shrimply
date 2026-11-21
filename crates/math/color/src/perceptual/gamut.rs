use super::{conversion, validate_rgba};
use crate::Oklab;

const GAMUT_EPSILON: f32 = 0.000_002;
pub(super) const GAMUT_STEPS: usize = 24;
// All bounded-lightness sRGB colors fit well inside unit Oklab chroma.
const SEARCH_CHROMA_LIMIT: f32 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Gamut {
    #[default]
    Chroma,
    Clip,
    Error,
}
impl Gamut {
    pub const ALL: [Self; 3] = [Self::Chroma, Self::Clip, Self::Error];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Chroma => "chroma",
            Self::Clip => "clip",
            Self::Error => "error",
        }
    }
}
impl std::str::FromStr for Gamut {
    type Err = String;
    fn from_str(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|value| value.name() == name)
            .ok_or_else(|| {
                format!(
                    "unknown gamut policy '{name}'; choose {}",
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }
}

pub fn in_gamut(color: [f32; 4]) -> bool {
    color[..3]
        .iter()
        .all(|value| value.is_finite() && (-GAMUT_EPSILON..=1.0 + GAMUT_EPSILON).contains(value))
}

/// Map into display sRGB. Chroma mapping preserves Oklab lightness and hue.
/// This bounded radial search is deliberately not presented as CSS's local-MINDE mapper.
pub fn map_oklab(color: Oklab, alpha: f32, policy: Gamut) -> Result<[f32; 4], String> {
    if [color.l, color.a, color.b, alpha]
        .into_iter()
        .any(|value| !value.is_finite())
        || !(0.0..=1.0).contains(&color.l)
        || !(0.0..=1.0).contains(&alpha)
    {
        return Err(
            "Oklab lightness and alpha must be finite within 0..=1; a and b must be finite".into(),
        );
    }
    let mut rgb = conversion::oklab_to_rgba(color, alpha);
    if !in_gamut(rgb) {
        match policy {
            Gamut::Error => return Err("color is outside display sRGB gamut; reduce chroma or choose gamut=\"chroma\" or \"clip\"".into()),
            Gamut::Clip => {
                if rgb.into_iter().any(|value| !value.is_finite()) {
                    return Err("color conversion exceeds finite RGB range; reduce chroma or use gamut=\"chroma\"".into());
                }
            }
            Gamut::Chroma => {
                if color.l == 0.0 { return Ok([0.0, 0.0, 0.0, alpha]); }
                if color.l == 1.0 { return Ok([1.0, 1.0, 1.0, alpha]); }
                let [lightness, chroma, hue] = conversion::oklab_to_oklch(color);
                let mut low = 0.0;
                let mut high = chroma.min(SEARCH_CHROMA_LIMIT);
                for _ in 0..GAMUT_STEPS {
                    let middle = (low + high) / 2.0;
                    let candidate = conversion::oklab_to_rgba(conversion::oklch_to_oklab([lightness, middle, hue]), alpha);
                    if in_gamut(candidate) { low = middle; } else { high = middle; }
                }
                rgb = conversion::oklab_to_rgba(conversion::oklch_to_oklab([lightness, low, hue]), alpha);
            }
        }
    }
    rgb = rgb.map(|value| value.clamp(0.0, 1.0));
    validate_rgba(rgb)?;
    Ok(rgb)
}
