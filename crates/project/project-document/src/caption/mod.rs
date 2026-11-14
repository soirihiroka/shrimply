use crate::project::{Color, Time};
use serde::{Deserialize, Serialize};
pub use shrimply_property_model::VerticalAlign;
use uuid::Uuid;

pub mod markup;
pub mod ytt;

pub fn clean_text_for_speech(source: &str) -> String {
    markup::parse(source)
        .into_iter()
        .map(|span| span.ruby.map_or(span.text, |ruby| ruby.annotation))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionItem {
    pub id: Uuid,
    pub start: Time,
    pub end: Time,
    pub text: String,
    pub group_id: Option<u64>,
    #[serde(default = "default_true")]
    pub styling_enabled: bool,
    #[serde(default = "default_true")]
    pub layout_enabled: bool,
    pub h_align: HorizontalAlign,
    pub v_align: VerticalAlign,
    pub position_x: u8,
    pub position_y: u8,
    pub text_color: Color<u8>,
    pub background_color: Color<u8>,
    pub edge_color: Color<u8>,
    pub edge_style: CaptionEdgeStyle,
    pub font: CaptionFont,
    pub font_scale: u16,
    pub writing_direction: CaptionWritingDirection,
}

impl CaptionItem {
    pub fn new(start: Time, end: Time, text: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            start,
            end,
            text,
            group_id: None,
            styling_enabled: false,
            layout_enabled: false,
            h_align: HorizontalAlign::Center,
            v_align: VerticalAlign::Bottom,
            position_x: 50,
            position_y: 90,
            text_color: Color::<u8>::WHITE,
            background_color: Color::<u8>::from_rgba(8, 8, 8, 191),
            edge_color: Color::<u8>::from_rgb(34, 34, 34),
            edge_style: CaptionEdgeStyle::None,
            font: CaptionFont::Roboto,
            font_scale: 100,
            writing_direction: CaptionWritingDirection::Horizontal,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HorizontalAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum CaptionEdgeStyle {
    #[default]
    None,
    HardShadow,
    Bevel,
    Glow,
    SoftShadow,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum CaptionFont {
    #[default]
    Roboto,
    MonospaceSerif,
    Serif,
    MonospaceSans,
    Casual,
    Cursive,
    SmallCapitals,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum CaptionWritingDirection {
    #[default]
    Horizontal,
    VerticalRightToLeft,
    VerticalLeftToRight,
    RotatedLeftToRight,
    RotatedRightToLeft,
}
