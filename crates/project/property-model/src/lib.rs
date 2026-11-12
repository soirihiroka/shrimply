pub use shrimply_timeline_value::{ShapeKind, ShapeRoundingStrategy};
pub mod modifier_model;
pub mod timeline_value;

use serde::{Deserialize, Serialize};
pub use shrimply_math_color::{Color, LayerBlendMode};
pub use shrimply_render_core::VideoSampleMethod;
pub use shrimply_timeline_value::{
    FRACTION_ZERO, Time, deserialize_fraction, fraction_as_f64, fraction_as_label,
    fraction_denominator, fraction_from_f64, fraction_from_integer, fraction_is_finite,
    fraction_new, fraction_numerator, serialize_fraction,
};

pub const DEFAULT_TEXT_FONT_FAMILY: &str = "Geomini";

#[derive(Clone, Debug, Serialize, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FontFamily {
    Local { name: String },
    GoogleFonts { name: String },
}

impl FontFamily {
    pub fn name(&self) -> &str {
        match self {
            Self::Local { name } | Self::GoogleFonts { name } => name,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TextFontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FontVariation {
    pub axis: String,
    pub value: f32,
}

#[derive(Clone, Copy, Debug, Default, Hash, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TextHorizontalAlign {
    Left,
    #[default]
    Center,
    Right,
    Fill,
}

#[derive(Clone, Copy, Debug, Default, Hash, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TextDirection {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, Hash, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAlign {
    Top,
    Middle,
    #[default]
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkiaDrawingStrategy {
    Immediate,
    #[default]
    Picture,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureAddressMode {
    #[default]
    Transparent,
    ClampToEdge,
    Repeat,
    MirrorRepeat,
    BlurredMirror,
    Stochastic,
}

shrimply_timeline_value::timeline_step_type!(
    TextFontStyle,
    TextFontStyle::Normal,
    &[
        shrimply_timeline_value::TimelineStepVariant {
            value: TextFontStyle::Normal,
            key: "normal",
            label: "Normal",
            icon: None
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextFontStyle::Italic,
            key: "italic",
            label: "Italic",
            icon: None
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextFontStyle::Oblique,
            key: "oblique",
            label: "Oblique",
            icon: None
        },
    ]
);
shrimply_timeline_value::timeline_step_type!(
    TextHorizontalAlign,
    TextHorizontalAlign::Center,
    &[
        shrimply_timeline_value::TimelineStepVariant {
            value: TextHorizontalAlign::Left,
            key: "left",
            label: "Left",
            icon: Some("text-justify-left-symbolic")
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextHorizontalAlign::Center,
            key: "center",
            label: "Center",
            icon: Some("text-justify-center-symbolic")
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextHorizontalAlign::Right,
            key: "right",
            label: "Right",
            icon: Some("text-justify-right-symbolic")
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextHorizontalAlign::Fill,
            key: "fill",
            label: "Fill",
            icon: Some("text-justify-fill-symbolic")
        },
    ]
);
shrimply_timeline_value::timeline_step_type!(
    VerticalAlign,
    VerticalAlign::Bottom,
    &[
        shrimply_timeline_value::TimelineStepVariant {
            value: VerticalAlign::Top,
            key: "top",
            label: "Top",
            icon: Some("valign-start-symbolic")
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: VerticalAlign::Middle,
            key: "middle",
            label: "Middle",
            icon: Some("valign-center-symbolic")
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: VerticalAlign::Bottom,
            key: "bottom",
            label: "Bottom",
            icon: Some("valign-end-symbolic")
        },
    ]
);
shrimply_timeline_value::timeline_step_type!(
    TextDirection,
    TextDirection::Horizontal,
    &[
        shrimply_timeline_value::TimelineStepVariant {
            value: TextDirection::Horizontal,
            key: "horizontal",
            label: "Horizontal",
            icon: Some("text-direction-horizontal-symbolic")
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextDirection::Vertical,
            key: "vertical",
            label: "Vertical",
            icon: Some("text-direction-vertical-symbolic")
        },
    ]
);
shrimply_timeline_value::timeline_step_type!(
    TextureAddressMode,
    TextureAddressMode::Transparent,
    &[
        shrimply_timeline_value::TimelineStepVariant {
            value: TextureAddressMode::Transparent,
            key: "transparent",
            label: "Transparent",
            icon: None
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextureAddressMode::ClampToEdge,
            key: "clamp",
            label: "Clamp to edge",
            icon: None
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextureAddressMode::Repeat,
            key: "repeat",
            label: "Repeat",
            icon: None
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextureAddressMode::MirrorRepeat,
            key: "mirror",
            label: "Mirror repeat",
            icon: None
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextureAddressMode::BlurredMirror,
            key: "blurred_mirror",
            label: "Blurred mirror",
            icon: None
        },
        shrimply_timeline_value::TimelineStepVariant {
            value: TextureAddressMode::Stochastic,
            key: "stochastic",
            label: "Stochastic",
            icon: None
        },
    ]
);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualEdges {
    pub top: timeline_value::TimelineValue<f32>,
    pub right: timeline_value::TimelineValue<f32>,
    pub bottom: timeline_value::TimelineValue<f32>,
    pub left: timeline_value::TimelineValue<f32>,
}

impl Default for VisualEdges {
    fn default() -> Self {
        Self {
            top: timeline_value::TimelineValue::new_const(0.0),
            right: timeline_value::TimelineValue::new_const(0.0),
            bottom: timeline_value::TimelineValue::new_const(0.0),
            left: timeline_value::TimelineValue::new_const(0.0),
        }
    }
}
