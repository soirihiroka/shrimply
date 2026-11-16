use num_traits::ConstZero;
use serde::{Deserialize, Serialize};

pub use oklab::Oklab;

mod adw;
mod blend;
mod css;
pub use css::parse_css_color;
#[cfg(feature = "gdk")]
mod gdk;
#[cfg(feature = "gif")]
mod gif;
#[cfg(feature = "skia")]
mod skia;
mod transparent_fill;
pub use blend::LayerBlendMode;
#[cfg(feature = "gif")]
pub use gif::*;
pub use transparent_fill::transparent_fill_mask;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Color<T = f32> {
    pub r: T,
    pub g: T,
    pub b: T,
    pub a: T,
}

#[cfg(feature = "cuda")]
unsafe impl<T: shrimply_gpu_cuda::DeviceCopy> shrimply_gpu_cuda::DeviceCopy for Color<T> {}

pub trait ColorChannel: Copy + ConstZero {
    const OPAQUE: Self;
}

impl ColorChannel for f32 {
    const OPAQUE: Self = 1.0;
}

impl ColorChannel for u8 {
    const OPAQUE: Self = u8::MAX;
}

#[derive(Clone, Copy)]
pub struct ColorCorrectionParams {
    pub exposure: f32,
    pub gamma: f32,
    pub temperature: f32,
    pub tint: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub hue_turns: f32,
    pub saturation: f32,
    pub value: f32,
}

impl<T> Color<T> {
    #[inline]
    pub const fn new(r: T, g: T, b: T, a: T) -> Self {
        Self { r, g, b, a }
    }

    #[inline]
    pub fn map<U>(self, map: impl Fn(T) -> U) -> Color<U> {
        Color::new(map(self.r), map(self.g), map(self.b), map(self.a))
    }

    #[inline]
    pub fn to_array(self) -> [T; 4] {
        [self.r, self.g, self.b, self.a]
    }

    #[inline]
    pub fn to_rgb_array(self) -> [T; 3] {
        [self.r, self.g, self.b]
    }

    #[inline]
    pub fn with_alpha(mut self, alpha: T) -> Self {
        self.a = alpha;
        self
    }

    #[inline]
    pub const fn from_rgba(r: T, g: T, b: T, a: T) -> Self {
        Self::new(r, g, b, a)
    }
}

impl<T: ColorChannel> Color<T> {
    pub const TRANSPARENT: Self = Self::new(T::ZERO, T::ZERO, T::ZERO, T::ZERO);
    pub const BLACK: Self = Self::new(T::ZERO, T::ZERO, T::ZERO, T::OPAQUE);
    pub const WHITE: Self = Self::new(T::OPAQUE, T::OPAQUE, T::OPAQUE, T::OPAQUE);

    #[inline]
    pub const fn from_rgb(r: T, g: T, b: T) -> Self {
        Self::new(r, g, b, T::OPAQUE)
    }
}

impl<T: Copy + num_traits::NumCast> Color<T> {
    pub fn try_cast<U: num_traits::NumCast>(self) -> Option<Color<U>> {
        Some(Color::new(
            num_traits::cast(self.r)?,
            num_traits::cast(self.g)?,
            num_traits::cast(self.b)?,
            num_traits::cast(self.a)?,
        ))
    }
}

impl<T> From<[T; 4]> for Color<T> {
    fn from([r, g, b, a]: [T; 4]) -> Self {
        Self::new(r, g, b, a)
    }
}

impl<T: core::ops::Mul> core::ops::Mul for Color<T> {
    type Output = Color<T::Output>;

    fn mul(self, right: Self) -> Self::Output {
        Color::new(
            self.r * right.r,
            self.g * right.g,
            self.b * right.b,
            self.a * right.a,
        )
    }
}

impl<T: core::ops::MulAssign> core::ops::MulAssign for Color<T> {
    fn mul_assign(&mut self, right: Self) {
        self.r *= right.r;
        self.g *= right.g;
        self.b *= right.b;
        self.a *= right.a;
    }
}

impl<T: ColorChannel> From<[T; 3]> for Color<T> {
    fn from([r, g, b]: [T; 3]) -> Self {
        Self::from_rgb(r, g, b)
    }
}

pub fn deserialize_array<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Color<f32>, D::Error> {
    <[f32; 4]>::deserialize(deserializer).map(Color::from)
}

pub fn serialize_array<S: serde::Serializer>(
    color: &Color<f32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    color.to_array().serialize(serializer)
}

impl From<Color<u8>> for Color<f32> {
    fn from(color: Color<u8>) -> Self {
        Self::from(color.to_srgba())
    }
}

impl Color<f32> {
    /// Decode straight sRGB channels to linear light, preserving alpha and floating-point precision.
    pub fn to_linear(mut self) -> Self {
        let channel = |value: f32| {
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        self.r = channel(self.r);
        self.g = channel(self.g);
        self.b = channel(self.b);
        self
    }

    pub const fn from_rgb8(red: u8, green: u8, blue: u8) -> Self {
        Self::from_rgb8_alpha(red, green, blue, 1.0)
    }

    pub const fn from_rgb8_alpha(red: u8, green: u8, blue: u8, alpha: f32) -> Self {
        Self::new(
            red as f32 / 255.0,
            green as f32 / 255.0,
            blue as f32 / 255.0,
            alpha,
        )
    }

    pub const fn from_rgba8(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self::from_rgb8_alpha(red, green, blue, alpha as f32 / 255.0)
    }

    pub fn is_finite(self) -> bool {
        self.r.is_finite() && self.g.is_finite() && self.b.is_finite() && self.a.is_finite()
    }

    #[inline]
    pub fn alpha_multiply(mut self, alpha: f32) -> Self {
        self.a = (self.a * alpha).clamp(0.0, 1.0);
        self
    }

    #[inline(always)]
    pub fn premultiply(mut self) -> Self {
        self.r *= self.a;
        self.g *= self.a;
        self.b *= self.a;
        self
    }

    #[inline(always)]
    pub fn unpremultiply(mut self) -> Self {
        if self.a <= f32::EPSILON {
            return Self::TRANSPARENT;
        }
        self.r /= self.a;
        self.g /= self.a;
        self.b /= self.a;
        self
    }

    #[inline]
    pub fn is_transparent(self) -> bool {
        self.a <= 0.0
    }

    #[inline]
    pub fn from_rgba_u32(value: u32) -> Self {
        Self::new(
            (value & 0xff) as f32 / 255.0,
            ((value >> 8) & 0xff) as f32 / 255.0,
            ((value >> 16) & 0xff) as f32 / 255.0,
            ((value >> 24) & 0xff) as f32 / 255.0,
        )
    }

    #[inline]
    pub fn to_rgba_u32(self) -> u32 {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        channel(self.r) | (channel(self.g) << 8) | (channel(self.b) << 16) | (channel(self.a) << 24)
    }

    #[inline(always)]
    pub fn to_cmyk(self) -> [f32; 4] {
        let key = 1.0 - self.r.max(self.g).max(self.b);
        let color = 1.0 - key;
        if color == 0.0 {
            [0.0, 0.0, 0.0, key]
        } else {
            [
                (1.0 - self.r - key) / color,
                (1.0 - self.g - key) / color,
                (1.0 - self.b - key) / color,
                key,
            ]
        }
    }

    #[inline(always)]
    pub fn corrected(mut self, params: ColorCorrectionParams) -> Self {
        let exposure = 2.0f32.powf(params.exposure);
        self.r *= exposure;
        self.g *= exposure;
        self.b *= exposure;

        self.r += params.temperature * 0.1 + params.tint * 0.03;
        self.g -= params.tint * 0.08;
        self.b += -params.temperature * 0.1 + params.tint * 0.03;

        let contrast = (1.0 + params.contrast).max(0.0);
        self.r = ((self.r - 0.5) * contrast + 0.5 + params.brightness).clamp(0.0, 1.0);
        self.g = ((self.g - 0.5) * contrast + 0.5 + params.brightness).clamp(0.0, 1.0);
        self.b = ((self.b - 0.5) * contrast + 0.5 + params.brightness).clamp(0.0, 1.0);

        let maximum = self.r.max(self.g).max(self.b);
        let minimum = self.r.min(self.g).min(self.b);
        let delta = maximum - minimum;
        let hue = if delta <= 0.000_001 {
            0.0
        } else if maximum == self.r {
            (self.g - self.b) / delta / 6.0
        } else if maximum == self.g {
            ((self.b - self.r) / delta + 2.0) / 6.0
        } else {
            ((self.r - self.g) / delta + 4.0) / 6.0
        };
        let hue = (hue + params.hue_turns).rem_euclid(1.0);
        let source_saturation = if maximum <= 0.000_001 {
            0.0
        } else {
            delta / maximum
        };
        let saturation = (source_saturation * params.saturation.max(0.0)).clamp(0.0, 1.0);
        let value = (maximum * params.value.max(0.0)).clamp(0.0, 1.0);
        let sector = hue * 6.0;
        let x = 1.0 - ((sector * 0.5).rem_euclid(1.0) * 2.0 - 1.0).abs();
        let color = if sector < 1.0 {
            [1.0, x, 0.0]
        } else if sector < 2.0 {
            [x, 1.0, 0.0]
        } else if sector < 3.0 {
            [0.0, 1.0, x]
        } else if sector < 4.0 {
            [0.0, x, 1.0]
        } else if sector < 5.0 {
            [x, 0.0, 1.0]
        } else {
            [1.0, 0.0, x]
        };
        let offset = value * (1.0 - saturation);
        let scale = value * saturation;
        let inverse_gamma = 1.0 / params.gamma.max(0.01);
        self.r = (color[0] * scale + offset).powf(inverse_gamma);
        self.g = (color[1] * scale + offset).powf(inverse_gamma);
        self.b = (color[2] * scale + offset).powf(inverse_gamma);
        self
    }

    #[inline]
    pub fn lerp(self, other: Self, amount: f32) -> Self {
        Self::new(
            self.r + (other.r - self.r) * amount,
            self.g + (other.g - self.g) * amount,
            self.b + (other.b - self.b) * amount,
            self.a + (other.a - self.a) * amount,
        )
    }

    #[inline]
    pub fn rec709_luma(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    #[inline(always)]
    pub fn aces_tone_mapped<const CLAMP: bool>(mut self) -> Self {
        let map = |value: f32| {
            let value = (value * (2.51 * value + 0.03)) / (value * (2.43 * value + 0.59) + 0.14);
            if CLAMP { value.clamp(0.0, 1.0) } else { value }
        };
        self.r = map(self.r);
        self.g = map(self.g);
        self.b = map(self.b);
        self
    }

    #[inline(always)]
    pub fn linear_to_srgb<const CLAMP: bool>(mut self) -> Self {
        let convert = |value: f32| {
            let value = if value <= 0.003_130_8 {
                value * 12.92
            } else {
                1.055 * value.powf(1.0 / 2.4) - 0.055
            };
            if CLAMP { value.clamp(0.0, 1.0) } else { value }
        };
        self.r = convert(self.r);
        self.g = convert(self.g);
        self.b = convert(self.b);
        self
    }

    #[inline(always)]
    pub fn bt2020_ycbcr_distance(self, other: Self) -> f32 {
        let luma = |color: Self| 0.2627 * color.r + 0.6780 * color.g + 0.0593 * color.b;
        let self_luma = luma(self);
        let other_luma = luma(other);
        let luma = self_luma - other_luma;
        let cb = (self.b - self_luma) * 0.5315 - (other.b - other_luma) * 0.5315;
        let cr = (self.r - self_luma) * 0.6782 - (other.r - other_luma) * 0.6782;
        let alpha = self.a - other.a;
        (2.0 * luma * luma + cb * cb + cr * cr + alpha * alpha).sqrt()
    }

    #[inline(always)]
    pub fn from_bt709_ycbcr(luma: f32, cb: f32, cr: f32, alpha: f32) -> Self {
        let luma = (luma - 0.0625).max(0.0) * 1.164_383_5;
        Self::new(
            (luma + 1.792_741_1 * cr).clamp(0.0, 1.0),
            (luma - 0.213_248_61 * cb - 0.532_909_33 * cr).clamp(0.0, 1.0),
            (luma + 2.112_401_7 * cb).clamp(0.0, 1.0),
            alpha,
        )
    }

    #[inline]
    pub fn to_bt709_ycbcr(self) -> [u8; 3] {
        let byte = |value: f32| (value.clamp(0.0, 255.0) + 0.5) as u8;
        [
            byte(16.0 + 219.0 * self.rec709_luma()),
            byte(128.0 + 224.0 * (-0.114_572 * self.r - 0.385_428 * self.g + 0.5 * self.b)),
            byte(128.0 + 224.0 * (0.5 * self.r - 0.454_153 * self.g - 0.045_847 * self.b)),
        ]
    }
}

impl Color<u8> {
    pub const fn to_rgba_u32(self) -> u32 {
        self.r as u32 | (self.g as u32) << 8 | (self.b as u32) << 16 | (self.a as u32) << 24
    }

    pub const fn from_rgb_u32(value: u32) -> Self {
        Self::from_rgb(
            ((value >> 16) & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            (value & 0xff) as u8,
        )
    }

    pub const fn to_rgb_u32(self) -> u32 {
        (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    pub const fn from_gray(luminance: u8) -> Self {
        Self::from_rgb(luminance, luminance, luminance)
    }

    pub const fn from_gray_alpha(luminance: u8, alpha: u8) -> Self {
        Self::from_rgba(luminance, luminance, luminance, alpha)
    }

    pub fn from_srgb(rgb: [f32; 3]) -> Self {
        Self::from_srgba([rgb[0], rgb[1], rgb[2], 1.0])
    }

    pub fn from_srgba(rgba: [f32; 4]) -> Self {
        let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self::from_rgba(byte(rgba[0]), byte(rgba[1]), byte(rgba[2]), byte(rgba[3]))
    }

    pub fn to_srgba(self) -> [f32; 4] {
        self.to_array().map(|channel| f32::from(channel) / 255.0)
    }

    pub fn to_linear(self) -> Color<f32> {
        Color::<f32>::from(self).to_linear()
    }

    pub fn rec709_luma(self) -> u8 {
        ((54 * u32::from(self.r) + 183 * u32::from(self.g) + 19 * u32::from(self.b) + 128) >> 8)
            as u8
    }

    pub fn alpha_multiply(mut self, alpha: f32) -> Self {
        self.a = (f32::from(self.a) * alpha).round().clamp(0.0, 255.0) as u8;
        self
    }

    pub fn from_hsv(hue_degrees: f32, saturation: f32, value: f32) -> Self {
        Self::from_hsva(hue_degrees, saturation, value, 1.0)
    }

    pub fn from_hsva(hue_degrees: f32, saturation: f32, value: f32, alpha: f32) -> Self {
        let saturation = saturation.clamp(0.0, 1.0);
        let value = value.clamp(0.0, 1.0);
        let chroma = value * saturation;
        let hue = hue_degrees.rem_euclid(360.0) / 60.0;
        let secondary = chroma * (1.0 - (hue.rem_euclid(2.0) - 1.0).abs());
        let (red, green, blue) = match hue as u8 {
            0 => (chroma, secondary, 0.0),
            1 => (secondary, chroma, 0.0),
            2 => (0.0, chroma, secondary),
            3 => (0.0, secondary, chroma),
            4 => (secondary, 0.0, chroma),
            _ => (chroma, 0.0, secondary),
        };
        let offset = value - chroma;
        Self::from_srgba([red + offset, green + offset, blue + offset, alpha])
    }

    pub fn to_hsva(self) -> [f32; 4] {
        let [red, green, blue, alpha] = self.to_srgba();
        let maximum = red.max(green).max(blue);
        let minimum = red.min(green).min(blue);
        let chroma = maximum - minimum;
        let hue = if chroma <= f32::EPSILON {
            0.0
        } else if maximum == red {
            60.0 * ((green - blue) / chroma).rem_euclid(6.0)
        } else if maximum == green {
            60.0 * ((blue - red) / chroma + 2.0)
        } else {
            60.0 * ((red - green) / chroma + 4.0)
        };
        let saturation = if maximum <= f32::EPSILON {
            0.0
        } else {
            chroma / maximum
        };
        [hue, saturation, maximum, alpha]
    }

    pub fn from_oklab(l: f32, a: f32, b: f32) -> Self {
        Self::from_oklaba(l, a, b, 1.0)
    }

    pub fn from_oklaba(l: f32, a: f32, b: f32, alpha: f32) -> Self {
        let rgb = oklab::oklab_to_srgb_f32(Oklab { l, a, b });
        Self::from_srgba([rgb.r, rgb.g, rgb.b, alpha])
    }

    pub fn to_oklab(self) -> Oklab {
        oklab::srgb_to_oklab(oklab::Rgb::new(self.r, self.g, self.b))
    }

    pub fn relative_luminance(self) -> f32 {
        let linear = oklab::oklab_to_linear_srgb(self.to_oklab());
        0.2126 * linear.r + 0.7152 * linear.g + 0.0722 * linear.b
    }

    pub fn oklab_distance(self, other: Self) -> f32 {
        let left = self.to_oklab();
        let right = other.to_oklab();
        ((right.l - left.l).powi(2) + (right.a - left.a).powi(2) + (right.b - left.b).powi(2))
            .sqrt()
    }

    pub fn oklaba_distance(self, other: Self) -> f32 {
        let color_distance = self.oklab_distance(other);
        let alpha_distance = f32::from(other.a) / 255.0 - f32::from(self.a) / 255.0;
        color_distance.hypot(alpha_distance)
    }

    pub fn mix_srgba(self, other: Self, progress: f32) -> Self {
        let progress = progress.clamp(0.0, 1.0);
        let channel = |left: u8, right: u8| {
            (f32::from(left) + f32::from(right) * progress - f32::from(left) * progress)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Self::from_rgba(
            channel(self.r, other.r),
            channel(self.g, other.g),
            channel(self.b, other.b),
            channel(self.a, other.a),
        )
    }

    pub fn mix_oklaba(self, other: Self, progress: f64) -> Self {
        let left = self.to_oklab();
        let right = other.to_oklab();
        let mix = |left: f32, right: f32| {
            num_traits::cast(f64::from(left) + f64::from(right - left) * progress)
                .expect("mixed f32 color channel must remain representable")
        };
        Self::from_oklaba(
            mix(left.l, right.l),
            mix(left.a, right.a),
            mix(left.b, right.b),
            mix(f32::from(self.a) / 255.0, f32::from(other.a) / 255.0),
        )
    }
}

impl From<u32> for Color<u8> {
    fn from(value: u32) -> Self {
        Self::new(
            value as u8,
            (value >> 8) as u8,
            (value >> 16) as u8,
            (value >> 24) as u8,
        )
    }
}
