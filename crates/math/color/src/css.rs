// SPDX-License-Identifier: LGPL-2.0-or-later
//! GTK/Pango-compatible picker color syntax, independent of a native toolkit.
//! Adapted from GTK 4.22.5 gdkrgba.c/gdkhsla.c and Pango 1.58.0 pango-color.c.
//! Copyright and original licensing notices are retained in css/NOTICE and css/LICENSE.
use crate::Color;

mod names;

/// Parse the same color strings as the GTK picker, including legacy wide hex channels.
pub fn parse_css_color(value: &str) -> Option<Color<u8>> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let (digits, channels) = match hex.len() {
            3 | 6 | 9 | 12 => (hex.len() / 3, 3),
            4 | 8 | 16 => (hex.len() / 4, 4),
            _ => return None,
        };
        if !hex.bytes().all(|digit| digit.is_ascii_hexdigit()) {
            return None;
        }
        let mut rgba = [1.0; 4];
        for (channel, value) in rgba
            .iter_mut()
            .zip(hex.as_bytes().chunks(digits))
            .take(channels)
        {
            let mut value = u32::from_str_radix(std::str::from_utf8(value).ok()?, 16).ok()?;
            let mut bits = digits * 4;
            value <<= 16 - bits;
            while bits < 16 {
                value |= value >> bits;
                bits *= 2;
            }
            *channel = value as f32 / u16::MAX as f32;
        }
        return Some(Color::from_srgba(rgba));
    }
    let function = ["rgba", "rgb", "hsla", "hsl"]
        .into_iter()
        .find(|name| value.starts_with(name));
    let Some(function) = function else {
        let name = value
            .bytes()
            .filter(|byte| *byte != b' ')
            .map(|byte| byte.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let name = std::str::from_utf8(&name).ok()?;
        let index = names::NAMES
            .binary_search_by_key(&name, |(name, _)| *name)
            .ok()?;
        return Some(Color::from(names::NAMES[index].1));
    };
    let body = value
        .strip_prefix(function)?
        .trim_start_matches(' ')
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let has_alpha = function.ends_with('a');
    let is_hsl = function.starts_with('h');
    let values = body.split(',').collect::<Vec<_>>();
    if values.len() != if has_alpha { 4 } else { 3 } {
        return None;
    }
    let mut rgba = [1.0_f64; 4];
    for (index, token) in values.into_iter().enumerate() {
        let token = token.trim_end_matches(' ');
        let percent = token.ends_with('%');
        if index == 3 && percent {
            return None;
        }
        let token = if percent {
            token.strip_suffix('%')?.trim_end_matches(' ')
        } else {
            token
        };
        let number = number(token)?;
        let number = if index == 3 {
            number
        } else if percent {
            number / 100.0
        } else if is_hsl && index == 0 {
            number.rem_euclid(360.0) / 360.0
        } else if is_hsl {
            number / 100.0
        } else {
            number / u8::MAX as f64
        };
        rgba[index] = number.clamp(0.0, 1.0);
    }
    let rgba = if is_hsl {
        hsla([
            (rgba[0] * 360.0) as f32,
            rgba[1] as f32,
            rgba[2] as f32,
            rgba[3] as f32,
        ])
    } else {
        rgba.map(|value| value as f32)
    };
    Some(Color::from_srgba(rgba))
}

// g_ascii_strtod accepts decimal exponents and C hexadecimal floats, but rejects ERANGE.
fn number(token: &str) -> Option<f64> {
    let token = token.trim_start_matches([' ', '\t', '\r', '\n', '\x0b', '\x0c']);
    if token.is_empty() || token.ends_with(char::is_whitespace) {
        return None;
    }
    let unsigned = token.strip_prefix(['+', '-']).unwrap_or(token);
    let sign = if token.starts_with('-') { -1.0 } else { 1.0 };
    let nonzero;
    let value = if let Some(hex) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        let (mantissa, exponent) = hex
            .split_once(['p', 'P'])
            .map_or(Some((hex, 0)), |(mantissa, exponent)| {
                Some((mantissa, exponent.parse::<i32>().ok()?))
            })?;
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        if whole.is_empty() && fraction.is_empty() {
            return None;
        }
        nonzero = whole
            .bytes()
            .chain(fraction.bytes())
            .any(|byte| byte != b'0');
        let mut value = 0.0;
        for digit in whole.chars() {
            value = value * 16.0 + digit.to_digit(16)? as f64;
        }
        let mut scale = 1.0 / 16.0;
        for digit in fraction.chars() {
            value += digit.to_digit(16)? as f64 * scale;
            scale /= 16.0;
        }
        if value == 0.0 {
            value * sign
        } else {
            value * 2.0_f64.powi(exponent) * sign
        }
    } else {
        nonzero = unsigned
            .split(['e', 'E'])
            .next()?
            .bytes()
            .any(|byte| matches!(byte, b'1'..=b'9'));
        token.parse::<f64>().ok()?
    };
    (value.is_finite() && !value.is_subnormal() && !(value == 0.0 && nonzero)).then_some(value)
}

// Preserve GDK's single-precision HSL conversion before rounding picker channels to bytes.
fn hsla([hue, saturation, lightness, alpha]: [f32; 4]) -> [f32; 4] {
    if saturation == 0.0 {
        return [lightness, lightness, lightness, alpha];
    }
    let m2 = if lightness <= 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let m1 = 2.0 * lightness - m2;
    let channel = |offset: f32| {
        let hue = (hue + offset).rem_euclid(360.0);
        if hue < 60.0 {
            m1 + (m2 - m1) * hue / 60.0
        } else if hue < 180.0 {
            m2
        } else if hue < 240.0 {
            m1 + (m2 - m1) * (240.0 - hue) / 60.0
        } else {
            m1
        }
    };
    [channel(120.0), channel(0.0), channel(-120.0), alpha]
}
