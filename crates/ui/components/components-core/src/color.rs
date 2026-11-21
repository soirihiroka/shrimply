use shrimply_math_color::Color;

pub mod picker;

pub const RECENT_LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsva {
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
    pub alpha: f32,
}

impl Hsva {
    pub fn from_color(color: Color<u8>) -> Self {
        let [hue, saturation, value, alpha] = color.to_hsva();
        Self {
            hue,
            saturation,
            value,
            alpha,
        }
    }

    pub fn color(self) -> Color<u8> {
        Color::from_hsva(self.hue, self.saturation, self.value, self.alpha)
    }

    pub fn update_color(self, color: Color<u8>) -> Self {
        let mut next = Self::from_color(color);
        if next.saturation <= f32::EPSILON || next.value <= f32::EPSILON {
            next.hue = self.hue;
        }
        next
    }
}

pub const PALETTE: [(&str, Color<u8>); 45] = [
    ("Very Light Blue", Color::from_rgb(0x99, 0xc1, 0xf1)),
    ("Light Blue", Color::from_rgb(0x62, 0xa0, 0xea)),
    ("Blue", Color::from_rgb(0x35, 0x84, 0xe4)),
    ("Dark Blue", Color::from_rgb(0x1c, 0x71, 0xd8)),
    ("Very Dark Blue", Color::from_rgb(0x1a, 0x5f, 0xb4)),
    ("Very Light Green", Color::from_rgb(0x8f, 0xf0, 0xa4)),
    ("Light Green", Color::from_rgb(0x57, 0xe3, 0x89)),
    ("Green", Color::from_rgb(0x33, 0xd1, 0x7a)),
    ("Dark Green", Color::from_rgb(0x2e, 0xc2, 0x7e)),
    ("Very Dark Green", Color::from_rgb(0x26, 0xa2, 0x69)),
    ("Very Light Yellow", Color::from_rgb(0xf9, 0xf0, 0x6b)),
    ("Light Yellow", Color::from_rgb(0xf8, 0xe4, 0x5c)),
    ("Yellow", Color::from_rgb(0xf6, 0xd3, 0x2d)),
    ("Dark Yellow", Color::from_rgb(0xf5, 0xc2, 0x11)),
    ("Very Dark Yellow", Color::from_rgb(0xe5, 0xa5, 0x0a)),
    ("Very Light Orange", Color::from_rgb(0xff, 0xbe, 0x6f)),
    ("Light Orange", Color::from_rgb(0xff, 0xa3, 0x48)),
    ("Orange", Color::from_rgb(0xff, 0x78, 0x00)),
    ("Dark Orange", Color::from_rgb(0xe6, 0x61, 0x00)),
    ("Very Dark Orange", Color::from_rgb(0xc6, 0x46, 0x00)),
    ("Very Light Red", Color::from_rgb(0xf6, 0x61, 0x51)),
    ("Light Red", Color::from_rgb(0xed, 0x33, 0x3b)),
    ("Red", Color::from_rgb(0xe0, 0x1b, 0x24)),
    ("Dark Red", Color::from_rgb(0xc0, 0x1c, 0x28)),
    ("Very Dark Red", Color::from_rgb(0xa5, 0x1d, 0x2d)),
    ("Very Light Purple", Color::from_rgb(0xdc, 0x8a, 0xdd)),
    ("Light Purple", Color::from_rgb(0xc0, 0x61, 0xcb)),
    ("Purple", Color::from_rgb(0x91, 0x41, 0xac)),
    ("Dark Purple", Color::from_rgb(0x81, 0x3d, 0x9c)),
    ("Very Dark Purple", Color::from_rgb(0x61, 0x35, 0x83)),
    ("Very Light Brown", Color::from_rgb(0xcd, 0xab, 0x8f)),
    ("Light Brown", Color::from_rgb(0xb5, 0x83, 0x5a)),
    ("Brown", Color::from_rgb(0x98, 0x6a, 0x44)),
    ("Dark Brown", Color::from_rgb(0x86, 0x5e, 0x3c)),
    ("Very Dark Brown", Color::from_rgb(0x63, 0x45, 0x2c)),
    ("White", Color::from_rgb(0xff, 0xff, 0xff)),
    ("Light Gray 1", Color::from_rgb(0xf6, 0xf5, 0xf4)),
    ("Light Gray 2", Color::from_rgb(0xde, 0xdd, 0xda)),
    ("Light Gray 3", Color::from_rgb(0xc0, 0xbf, 0xbc)),
    ("Light Gray 4", Color::from_rgb(0x9a, 0x99, 0x96)),
    ("Dark Gray 1", Color::from_rgb(0x77, 0x76, 0x7b)),
    ("Dark Gray 2", Color::from_rgb(0x5e, 0x5c, 0x64)),
    ("Dark Gray 3", Color::from_rgb(0x3d, 0x38, 0x46)),
    ("Dark Gray 4", Color::from_rgb(0x24, 0x1f, 0x31)),
    ("Black", Color::from_rgb(0x00, 0x00, 0x00)),
];

pub fn color_hex(color: Color<u8>, with_alpha: bool) -> String {
    if with_alpha {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            color.r, color.g, color.b, color.a
        )
    } else {
        format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
    }
}

pub fn parse_hex(value: &str, with_alpha: bool) -> Option<Color<u8>> {
    let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if !value.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    let mut color = shrimply_math_color::parse_css_color(&format!("#{value}"))?;
    if !with_alpha {
        color.a = u8::MAX;
    }
    Some(color)
}

pub fn remember_color(colors: &mut Vec<Color<u8>>, color: Color<u8>) {
    colors.retain(|recent| *recent != color);
    colors.insert(0, color);
    colors.truncate(RECENT_LIMIT);
}
