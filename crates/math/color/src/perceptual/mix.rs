use super::{ACHROMATIC_EPSILON, Gamut, conversion, gamut::GAMUT_STEPS, map_oklab, validate_rgba};
use crate::{Color, Oklab};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MixSpace {
    #[default]
    Oklab,
    Oklch,
    LinearRgb,
    Srgb,
}
impl MixSpace {
    pub const ALL: [Self; 4] = [Self::Oklab, Self::Oklch, Self::LinearRgb, Self::Srgb];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Oklab => "oklab",
            Self::Oklch => "oklch",
            Self::LinearRgb => "linear-rgb",
            Self::Srgb => "srgb",
        }
    }
}
impl std::str::FromStr for MixSpace {
    type Err = String;
    fn from_str(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|value| value.name() == name)
            .ok_or_else(|| {
                format!(
                    "unknown color space '{name}'; choose {}",
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HueInterpolation {
    #[default]
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}
impl HueInterpolation {
    pub const ALL: [Self; 4] = [
        Self::Shorter,
        Self::Longer,
        Self::Increasing,
        Self::Decreasing,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Shorter => "shorter",
            Self::Longer => "longer",
            Self::Increasing => "increasing",
            Self::Decreasing => "decreasing",
        }
    }
    fn delta(self, start: f32, end: f32) -> f32 {
        let delta = end - start;
        match self {
            Self::Shorter => {
                if delta > 180.0 {
                    delta - 360.0
                } else if delta < -180.0 {
                    delta + 360.0
                } else {
                    delta
                }
            }
            Self::Longer => {
                if delta > 0.0 && delta < 180.0 {
                    delta - 360.0
                } else if delta > -180.0 && delta <= 0.0 {
                    delta + 360.0
                } else {
                    delta
                }
            }
            Self::Increasing => delta.rem_euclid(360.0),
            Self::Decreasing => -(-delta).rem_euclid(360.0),
        }
    }
}
impl std::str::FromStr for HueInterpolation {
    type Err = String;
    fn from_str(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|value| value.name() == name)
            .ok_or_else(|| {
                format!(
                    "unknown hue interpolation '{name}'; choose {}",
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MixOptions {
    pub space: MixSpace,
    pub hue: HueInterpolation,
    pub gamut: Gamut,
}

fn coordinates(color: [f32; 4], space: MixSpace) -> [f32; 3] {
    match space {
        MixSpace::Srgb => [color[0], color[1], color[2]],
        MixSpace::LinearRgb => {
            let color = Color::<f32>::from(color).to_linear();
            [color.r, color.g, color.b]
        }
        MixSpace::Oklab => {
            let lab = conversion::rgba_to_oklab(color);
            [lab.l, lab.a, lab.b]
        }
        MixSpace::Oklch => conversion::oklab_to_oklch(conversion::rgba_to_oklab(color)),
    }
}

fn interpolate(
    left: [f32; 3],
    right: [f32; 3],
    alpha: [f32; 2],
    amount: f64,
    options: MixOptions,
) -> Result<[f32; 4], String> {
    let output_alpha = f64::from(alpha[0]) * (1.0 - amount) + f64::from(alpha[1]) * amount;
    if output_alpha == 0.0 {
        return Ok([0.0; 4]);
    }
    let mut channels = std::array::from_fn::<_, 3, _>(|at| {
        ((f64::from(left[at]) * f64::from(alpha[0]) * (1.0 - amount)
            + f64::from(right[at]) * f64::from(alpha[1]) * amount)
            / output_alpha) as f32
    });
    if options.space == MixSpace::Oklch {
        let mut start = left[2];
        let mut end = right[2];
        if left[1] <= ACHROMATIC_EPSILON || alpha[0] == 0.0 {
            start = end;
        }
        if right[1] <= ACHROMATIC_EPSILON || alpha[1] == 0.0 {
            end = start;
        }
        // Cylindrical hue is carried through powerless endpoints, never premultiplied.
        channels[2] = (f64::from(start) + f64::from(options.hue.delta(start, end)) * amount)
            .rem_euclid(360.0) as f32;
    }
    match options.space {
        MixSpace::Oklab => map_oklab(
            Oklab {
                l: channels[0],
                a: channels[1],
                b: channels[2],
            },
            output_alpha as f32,
            options.gamut,
        ),
        MixSpace::Oklch => map_oklab(
            conversion::oklch_to_oklab(channels),
            output_alpha as f32,
            options.gamut,
        ),
        MixSpace::Srgb => Ok([channels[0], channels[1], channels[2], output_alpha as f32]),
        MixSpace::LinearRgb => {
            Ok(
                Color::<f32>::from([channels[0], channels[1], channels[2], output_alpha as f32])
                    .linear_to_srgb::<true>()
                    .to_array(),
            )
        }
    }
}

pub fn mix_rgba(
    left: [f32; 4],
    right: [f32; 4],
    amount: f64,
    options: MixOptions,
) -> Result<[f32; 4], String> {
    validate_rgba(left)?;
    validate_rgba(right)?;
    if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
        return Err("color mix amount must be finite within 0..=1".into());
    }
    if amount == 0.0 {
        return Ok(left);
    }
    if amount == 1.0 {
        return Ok(right);
    }
    interpolate(
        coordinates(left, options.space),
        coordinates(right, options.space),
        [left[3], right[3]],
        amount,
        options,
    )
}

/// A prepared palette converts stops once, never once per generated texel.
pub struct Ramp {
    colors: Vec<[f32; 4]>,
    coordinates: Vec<[f32; 3]>,
    options: MixOptions,
}
impl Ramp {
    pub const MAX_SAMPLE_WORK: usize = GAMUT_STEPS + 2;

    /// Bound storage before allocating exactly `count` input stops and conversions.
    pub fn allocation_bytes(count: usize) -> Result<usize, String> {
        count
            .checked_mul(std::mem::size_of::<[f32; 4]>() + std::mem::size_of::<[f32; 3]>())
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>()))
            .ok_or_else(|| "color ramp storage exceeds the host address range".into())
    }

    pub fn new(colors: Vec<[f32; 4]>, options: MixOptions) -> Result<Self, String> {
        if colors.is_empty() {
            return Err("color ramp needs at least one Color".into());
        }
        for color in &colors {
            validate_rgba(*color)?;
        }
        let coordinates = colors
            .iter()
            .map(|color| coordinates(*color, options.space))
            .collect();
        Ok(Self {
            colors,
            coordinates,
            options,
        })
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.colors.capacity() * std::mem::size_of::<[f32; 4]>()
            + self.coordinates.capacity() * std::mem::size_of::<[f32; 3]>()
    }
    pub fn sample(&self, position: f64) -> Result<[f32; 4], String> {
        if !position.is_finite() {
            return Err("color ramp position must be finite".into());
        }
        let at = position.clamp(0.0, 1.0) * (self.colors.len() - 1) as f64;
        let left = at.floor() as usize;
        let right = (left + 1).min(self.colors.len() - 1);
        let amount = at - left as f64;
        if amount == 0.0 {
            return Ok(self.colors[left]);
        }
        interpolate(
            self.coordinates[left],
            self.coordinates[right],
            [self.colors[left][3], self.colors[right][3]],
            amount,
            self.options,
        )
    }
}
