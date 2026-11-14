use glam::Vec2;
use serde::{Deserialize, Serialize};
use shrimply_math_core::{Fraction, Time};
use shrimply_math_geometry::Size2D;

pub fn clamp_item_end_to_next_start(end: Time, next_start: Option<Time>) -> Time {
    next_start.map_or(end, |next_start| end.min(next_start))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CanvasSize {
    pub width: u32,
    pub height: u32,
}

pub const MIN_CANVAS_DIMENSION: u32 = 1;
pub const MAX_CANVAS_DIMENSION: u32 = 16_384;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioClipTransitionCurve {
    #[default]
    EqualPower,
    Linear,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransitionSide {
    Intro,
    Outro,
}

impl Size2D for CanvasSize {
    fn size_2d(&self) -> Vec2 {
        Vec2::new(self.width.max(1) as f32, self.height.max(1) as f32)
    }
}

#[derive(Clone, Copy)]
pub struct FrameRate {
    pub value: Fraction,
    pub label: &'static str,
}

impl FrameRate {
    const fn new(value: Fraction, label: &'static str) -> Self {
        Self { value, label }
    }
}

pub const COMMON_FRAME_RATES: &[FrameRate] = &[
    FrameRate::new(Fraction::new_raw(1, 1), "1"),
    FrameRate::new(Fraction::new_raw(5, 1), "5"),
    FrameRate::new(Fraction::new_raw(10, 1), "10"),
    FrameRate::new(Fraction::new_raw(15, 1), "15"),
    FrameRate::new(Fraction::new_raw(24_000, 1_001), "23.976"),
    FrameRate::new(Fraction::new_raw(24, 1), "24"),
    FrameRate::new(Fraction::new_raw(25, 1), "25"),
    FrameRate::new(Fraction::new_raw(30_000, 1_001), "29.97"),
    FrameRate::new(Fraction::new_raw(30, 1), "30"),
    FrameRate::new(Fraction::new_raw(48, 1), "48"),
    FrameRate::new(Fraction::new_raw(50, 1), "50"),
    FrameRate::new(Fraction::new_raw(60_000, 1_001), "59.94"),
    FrameRate::new(Fraction::new_raw(60, 1), "60"),
    FrameRate::new(Fraction::new_raw(120, 1), "120"),
];

#[derive(Clone, Copy)]
pub struct ProjectPreset {
    pub label: &'static str,
    pub canvas_size: CanvasSize,
    pub fps: Fraction,
}

impl ProjectPreset {
    const fn new(label: &'static str, width: u32, height: u32, fps: u64) -> Self {
        Self {
            label,
            canvas_size: CanvasSize { width, height },
            fps: Fraction::new_raw(fps, 1),
        }
    }
}

pub const DEFAULT_CANVAS_SIZE: CanvasSize = CanvasSize {
    width: 1920,
    height: 1080,
};
pub const DEFAULT_PROJECT_FPS: Fraction = Fraction::new_raw(30, 1);

pub const PROJECT_PRESETS: &[ProjectPreset] = &[
    ProjectPreset::new("720p 30 FPS", 1280, 720, 30),
    ProjectPreset::new("720p 60 FPS", 1280, 720, 60),
    ProjectPreset::new("1080p 24 FPS", 1920, 1080, 24),
    ProjectPreset::new("1080p 30 FPS", 1920, 1080, 30),
    ProjectPreset::new("1080p 60 FPS", 1920, 1080, 60),
    ProjectPreset::new("1440p 30 FPS", 2560, 1440, 30),
    ProjectPreset::new("1440p 60 FPS", 2560, 1440, 60),
    ProjectPreset::new("4K 30 FPS", 3840, 2160, 30),
    ProjectPreset::new("4K 60 FPS", 3840, 2160, 60),
];

#[cfg(test)]
mod tests {
    use super::clamp_item_end_to_next_start;
    use shrimply_math_core::Time;

    #[test]
    fn item_end_stops_at_next_item() {
        assert_eq!(
            clamp_item_end_to_next_start(Time::from_seconds(12), Some(Time::from_seconds(8))),
            Time::from_seconds(8),
        );
    }

    #[test]
    fn item_end_is_unchanged_when_track_has_room() {
        assert_eq!(
            clamp_item_end_to_next_start(Time::from_seconds(8), Some(Time::from_seconds(12))),
            Time::from_seconds(8),
        );
        assert_eq!(
            clamp_item_end_to_next_start(Time::from_seconds(8), None),
            Time::from_seconds(8),
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum TrackKind {
    Video,
    Comment,
    Caption,
    Audio,
}

impl TrackKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Comment => "Comment",
            Self::Caption => "Caption",
            Self::Audio => "Audio",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ItemKey {
    pub kind: TrackKind,
    pub track_index: usize,
    pub item_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct TrackKey {
    pub kind: TrackKind,
    pub track_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrackGap {
    pub track: TrackKey,
    pub start: Time,
    pub end: Time,
}
