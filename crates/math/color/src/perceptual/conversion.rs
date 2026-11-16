use super::ACHROMATIC_EPSILON;
use crate::{Color, Oklab};

pub fn rgba_to_oklab(color: [f32; 4]) -> Oklab {
    let color = Color::<f32>::from(color).to_linear();
    // The locked Oklab library owns the conversion matrices; no byte quantization.
    oklab::linear_srgb_to_oklab(oklab::Rgb::new(color.r, color.g, color.b))
}

pub fn oklab_to_rgba(color: Oklab, alpha: f32) -> [f32; 4] {
    let color = oklab::oklab_to_linear_srgb(color);
    Color::new(color.r, color.g, color.b, alpha)
        .linear_to_srgb::<false>()
        .to_array()
}

pub fn oklab_to_oklch(color: Oklab) -> [f32; 3] {
    let chroma = color.a.hypot(color.b);
    let hue = if chroma <= ACHROMATIC_EPSILON {
        0.0
    } else {
        color.b.atan2(color.a).to_degrees().rem_euclid(360.0)
    };
    [color.l, chroma, hue]
}

pub fn oklch_to_oklab([lightness, chroma, hue]: [f32; 3]) -> Oklab {
    let (sin, cos) = hue.rem_euclid(360.0).to_radians().sin_cos();
    Oklab {
        l: lightness,
        a: chroma * cos,
        b: chroma * sin,
    }
}

pub fn hsv_to_rgba([hue, saturation, value, alpha]: [f32; 4]) -> [f32; 4] {
    let saturation = saturation.clamp(0.0, 1.0);
    let value = value.clamp(0.0, 1.0);
    let chroma = value * saturation;
    let hue = hue.rem_euclid(360.0) / 60.0;
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
    [red + offset, green + offset, blue + offset, alpha]
}

pub fn rgba_to_hsv([red, green, blue, alpha]: [f32; 4]) -> [f32; 4] {
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

pub fn hsl_to_rgba([hue, saturation, lightness, alpha]: [f32; 4]) -> [f32; 4] {
    let value = lightness + saturation * lightness.min(1.0 - lightness);
    let saturation = if value == 0.0 {
        0.0
    } else {
        2.0 * (1.0 - lightness / value)
    };
    hsv_to_rgba([hue, saturation, value, alpha])
}

pub fn rgba_to_hsl(color: [f32; 4]) -> [f32; 4] {
    let [hue, _, maximum, alpha] = rgba_to_hsv(color);
    let minimum = color[0].min(color[1]).min(color[2]);
    let chroma = maximum - minimum;
    let lightness = (maximum + minimum) / 2.0;
    let saturation = if chroma <= f32::EPSILON {
        0.0
    } else {
        chroma / (1.0 - (2.0 * lightness - 1.0).abs())
    };
    [hue, saturation, lightness, alpha]
}
