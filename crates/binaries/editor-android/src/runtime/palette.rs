//! Native colors are data; shared theme code retains all drawing semantics.
use serde_json::Value;
use shrimply_components_skia::Color;
use shrimply_cross_ui_theme::PlatformPalette;

const COLOR_ROLES: [&str; 10] = [
    "window_bg",
    "window_fg",
    "view_bg",
    "view_fg",
    "alternate_bg",
    "button_bg",
    "button_fg",
    "border",
    "accent_bg",
    "accent_fg",
];

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct NativePalette {
    dark: bool,
    argb: [u32; COLOR_ROLES.len()],
}

impl NativePalette {
    pub fn from_request(request: &Value) -> Result<Self, String> {
        let dark = request["dark"]
            .as_bool()
            .ok_or("Native palette requires a dark-mode flag")?;
        let mut argb = [0; COLOR_ROLES.len()];
        for (target, key) in argb.iter_mut().zip(COLOR_ROLES) {
            *target = request[key]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| format!("Native palette requires an ARGB color for {key}"))?;
        }
        Ok(Self { dark, argb })
    }

    pub fn apply(self) {
        let [
            window_bg,
            window_fg,
            view_bg,
            view_fg,
            alternate_bg,
            button_bg,
            button_fg,
            border,
            accent_bg,
            accent_fg,
        ] = self.argb.map(|argb| {
            let [alpha, red, green, blue] = argb.to_be_bytes();
            Color::<f32>::from(Color::<u8>::new(red, green, blue, alpha))
        });
        shrimply_cross_ui_theme::set_dark(self.dark);
        shrimply_cross_ui_theme::set_platform_palette(Some(PlatformPalette {
            window_bg,
            window_fg,
            view_bg,
            view_fg,
            alternate_bg,
            button_bg,
            button_fg,
            border,
            accent_bg,
            accent_fg,
        }));
    }
}
