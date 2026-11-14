use canvas::{Align2, Color, FontId, Rect, TimelinePainter, vec2};
use hashbrown::HashMap;
use icons::*;
use math::item_rect;
use project::*;
pub use shrimply_components_skia::{canvas, cursor};
pub use shrimply_cross_ui_theme as theme;
use shrimply_math_core::Time;
pub use shrimply_project_document as project;
pub mod icons;
pub mod rows;
pub mod track_controls;
pub mod visual_items;
pub use item_decoration::*;
mod item_decoration;

pub mod decoration;
pub mod math;
pub mod ruler;

pub const RULER_HEIGHT: f64 = 44.0;
pub const TRACK_HEIGHT: f64 = 36.0;
pub const ITEM_BORDER_STROKE_WIDTH: f32 = 2.0;
pub const MIN_ITEM_PIXEL_WIDTH: f64 = 1.0;
pub const PLAYHEAD_HANDLE_WIDTH: f64 = 16.0;
pub const PLAYHEAD_HANDLE_HEIGHT: f64 = 8.0;
pub const PLAYHEAD_HANDLE_TOP: f64 = 21.0;
pub const PLAYHEAD_HANDLE_TRIANGLE_HEIGHT: f64 = 5.0;
pub const FRAME_TICK_MIN_WIDTH: f64 = 8.0;
pub const RULER_LABEL_ALPHA: f32 = 0.55;
pub const TRACK_SELECTION_LABEL_ALPHA: f32 = 0.24;
pub const TRACK_LABEL_BUTTON_SIZE: f64 = 26.0;
pub const TRACK_INDEX_COLUMN_WIDTH: f64 = 40.0;
pub const TRACK_LABEL_PADDING_X: f64 = 14.0;
pub const TRACK_LABEL_BUTTON_START_X: f64 = TRACK_INDEX_COLUMN_WIDTH + TRACK_LABEL_PADDING_X;
pub const TRACK_LABEL_BUTTON_GAP: f64 = 6.0;
pub const TRACK_LABEL_BUTTON_STRIDE: f64 = TRACK_LABEL_BUTTON_SIZE + TRACK_LABEL_BUTTON_GAP;
pub const TRACK_LABEL_TOGGLE_X: f64 = TRACK_LABEL_BUTTON_START_X;
pub const TRACK_LABEL_ADD_X: f64 = TRACK_LABEL_BUTTON_START_X + TRACK_LABEL_BUTTON_STRIDE;
pub const TRACK_LABEL_RECORD_X: f64 = TRACK_LABEL_BUTTON_START_X + TRACK_LABEL_BUTTON_STRIDE * 2.0;
pub const LABEL_WIDTH: f64 = TRACK_LABEL_RECORD_X + TRACK_LABEL_BUTTON_SIZE + TRACK_LABEL_PADDING_X;
pub const TIMELINE_PADDING_LEFT: f64 = 2.0;
pub const TIMELINE_X: f64 = LABEL_WIDTH + TIMELINE_PADDING_LEFT;
const LABEL_FONT_SIZE: f32 = 12.0;
const CLIP_FONT_SIZE: f32 = 10.0;
const LABEL_INSET: f64 = 4.0;

#[derive(Clone, Copy)]
pub struct Viewport {
    pub scale: ruler::TimelineScale,
    pub scroll_y: f64,
}

#[derive(Clone, Copy)]
pub struct NaturalEndMarker {
    pub start: Time,
    pub end: Time,
    pub position: Option<Time>,
    pub repeat_interval: Option<Time>,
    pub real_start: Option<Time>,
    pub real_end: Option<Time>,
}

#[derive(Clone, Copy)]
pub struct TimedItemBox {
    pub marker: NaturalEndMarker,
    pub bounds: Rect,
    pub fill: Color,
    pub timeline_x: f64,
    pub view: Viewport,
    pub selected: bool,
    pub selected_border_color: Color,
}

#[derive(Clone, Copy)]
pub enum PreviewTimeMode {
    Move,
    Resize,
}

pub struct TrackLabel<'a> {
    pub y: f64,
    pub width: f64,
    pub index_x: f64,
    pub text: &'a str,
    pub accent: Color,
    pub selected: bool,
    pub theme: &'a shrimply_cross_ui_theme::Theme,
}

pub fn draw_track_label(painter: &TimelinePainter, label: TrackLabel<'_>) {
    let TrackLabel {
        y,
        width,
        index_x,
        text,
        accent,
        selected,
        theme,
    } = label;
    if selected {
        painter.rect_filled(
            rect(0.0, y, width, TRACK_HEIGHT - 1.0),
            0,
            theme.view_fg.alpha_multiply(TRACK_SELECTION_LABEL_ALPHA),
        );
    }
    painter.rect_filled(rect(0.0, y, 3.0, TRACK_HEIGHT - 1.0), 0, accent);
    draw_track_divider(painter, 0.0, y, width, theme.sidebar_shade);
    painter.text(
        vec2(index_x as f32, (y + TRACK_HEIGHT / 2.0) as f32),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(LABEL_FONT_SIZE),
        accent,
    );
}

pub fn draw_clip_label(painter: &TimelinePainter, bounds: Rect, text: &str, color: Color) {
    let max_width = (f64::from(bounds.width()) - LABEL_INSET * 2.0).max(0.0);
    painter.with_clip_rect(bounds).system_text_ellipsized(
        vec2(bounds.min.x + LABEL_INSET as f32, bounds.min.y + 12.0),
        text,
        FontId::proportional(CLIP_FONT_SIZE),
        color,
        max_width as f32,
    );
}

pub fn draw_track_divider(painter: &TimelinePainter, x: f64, y: f64, width: f64, color: Color) {
    painter.rect_filled(rect(x, y + TRACK_HEIGHT - 1.0, width, 1.0), 0, color);
}

pub fn draw_playhead(
    painter: &TimelinePainter,
    x: f64,
    frame_width: f64,
    height: f64,
    color: Color,
) {
    cursor::draw_playhead(
        painter,
        x,
        frame_width,
        height,
        color,
        cursor::PlayheadStyle {
            ruler_height: RULER_HEIGHT,
            frame_y: Some(RULER_HEIGHT - 4.0),
            handle_width: PLAYHEAD_HANDLE_WIDTH,
            handle_height: PLAYHEAD_HANDLE_HEIGHT,
            handle_top: PLAYHEAD_HANDLE_TOP,
            triangle_height: PLAYHEAD_HANDLE_TRIANGLE_HEIGHT,
        },
    );
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect::from_xywh(
        x as f32,
        y as f32,
        width.max(0.0) as f32,
        height.max(0.0) as f32,
    )
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TrackLabelAction {
    Select,
    Toggle,
    Add,
    AudioRecord,
    VideoRecord,
}

pub type TrackButtonId = (TrackKey, TrackLabelAction);

pub struct TrackControlDraw<'a> {
    pub animation_active: &'a mut bool,
    pub buttons: &'a mut HashMap<TrackButtonId, shrimply_components_skia::button::Button>,
    pub active_audio_recording_key: Option<TrackKey>,
    pub active_video_recording_key: Option<TrackKey>,
}
