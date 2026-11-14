use canvas::{Align2, Color, FontId, Rect, TimelinePainter, vec2};
pub use shrimply_components_skia::{canvas, cursor};
use shrimply_math_core::{Fraction, Time};

pub mod decoration;
pub mod fixture;
mod math;
pub mod ruler;

pub const RULER_HEIGHT: f64 = 44.0;
pub const TRACK_HEIGHT: f64 = 36.0;
pub const ITEM_BORDER_STROKE_WIDTH: f32 = 2.0;
pub const PLAYHEAD_HANDLE_WIDTH: f64 = 16.0;
pub const PLAYHEAD_HANDLE_HEIGHT: f64 = 8.0;
pub const PLAYHEAD_HANDLE_TOP: f64 = 21.0;
pub const PLAYHEAD_HANDLE_TRIANGLE_HEIGHT: f64 = 5.0;
pub const FRAME_TICK_MIN_WIDTH: f64 = 8.0;
pub const RULER_LABEL_ALPHA: f32 = 0.55;
pub const TRACK_SELECTION_LABEL_ALPHA: f32 = 0.24;
pub const LABEL_WIDTH: f64 = 172.0;
const LABEL_FONT_SIZE: f32 = 12.0;
const CLIP_FONT_SIZE: f32 = 10.0;
const LABEL_INSET: f64 = 4.0;

/// Presentation data only. No project, media, or editor runtime is required.
pub struct Clip {
    pub start: Time,
    pub end: Time,
    pub label: String,
    pub selected: bool,
}

pub struct Track {
    pub label: String,
    pub fill: Color,
    pub accent: Color,
    pub selected: bool,
    pub clips: Vec<Clip>,
}

#[derive(Clone, Copy)]
pub struct Viewport {
    pub scale: ruler::TimelineScale,
    pub scroll_y: f64,
}

pub struct Timeline<'a> {
    pub tracks: &'a [Track],
    pub viewport: Viewport,
    pub playhead: Time,
    pub frame_rate: Fraction,
    pub width: f64,
    pub height: f64,
    pub theme: &'a shrimply_cross_ui_theme::Theme,
}

pub fn draw(painter: &TimelinePainter, timeline: Timeline<'_>) {
    let Timeline {
        tracks,
        viewport,
        playhead,
        frame_rate,
        width,
        height,
        theme,
    } = timeline;
    let content_width = (width - LABEL_WIDTH).max(0.0);
    let visible_end = viewport.scale.scroll_seconds.saturating_add(
        Time {
            seconds: viewport.scale.seconds_per_pixel,
        }
        .scaled(Fraction::from(content_width)),
    );
    painter.rect_filled(rect(0.0, 0.0, width, height), 0, theme.view_bg);
    {
        let content = painter.with_clip_rect(rect(LABEL_WIDTH, 0.0, content_width, height));
        ruler::draw(
            &content,
            ruler::RulerDraw {
                scale: viewport.scale,
                timeline_x: LABEL_WIDTH,
                timeline_width: content_width,
                content_height: height,
                frame_step_seconds: Time {
                    seconds: Fraction::from(1) / frame_rate,
                },
                frame_rate,
                hide_zero_label: false,
                style: ruler::RulerStyle {
                    height: RULER_HEIGHT,
                    frame_tick_min_width: FRAME_TICK_MIN_WIDTH,
                    grid_color: theme.sidebar_shade,
                    label_color: theme.view_fg.alpha_multiply(RULER_LABEL_ALPHA),
                },
            },
        );
    }
    {
        let rows = painter.with_clip_rect(rect(
            0.0,
            RULER_HEIGHT,
            width,
            (height - RULER_HEIGHT).max(0.0),
        ));
        for (row, track) in tracks.iter().enumerate() {
            let y = RULER_HEIGHT + row as f64 * TRACK_HEIGHT - viewport.scroll_y;
            if y + TRACK_HEIGHT <= RULER_HEIGHT || y >= height {
                continue;
            }
            draw_track_label(
                &rows,
                TrackLabel {
                    y,
                    width: LABEL_WIDTH,
                    index_x: LABEL_WIDTH / 2.0,
                    text: &track.label,
                    accent: track.accent,
                    selected: track.selected,
                    theme,
                },
            );
            let clips = rows.with_clip_rect(rect(
                LABEL_WIDTH,
                RULER_HEIGHT,
                content_width,
                (height - RULER_HEIGHT).max(0.0),
            ));
            for clip in &track.clips {
                if clip.end <= viewport.scale.scroll_seconds || clip.start >= visible_end {
                    continue;
                }
                let x = viewport.scale.time_to_x(clip.start, LABEL_WIDTH);
                let end = viewport.scale.time_to_x(clip.end, LABEL_WIDTH);
                if end <= LABEL_WIDTH || x >= width {
                    continue;
                }
                let bounds = rect(x, y, (end - x).max(1.0), TRACK_HEIGHT);
                decoration::draw_item_box(
                    &clips,
                    bounds,
                    track.fill,
                    clip.selected,
                    track.accent,
                    theme.sidebar_border,
                );
                draw_clip_label(&clips, bounds, &clip.label, theme.view_fg);
            }
            draw_track_divider(&clips, LABEL_WIDTH, y, content_width, theme.sidebar_shade);
        }
    }
    painter.rect_filled(rect(0.0, 0.0, LABEL_WIDTH, RULER_HEIGHT), 0, theme.view_bg);
    painter.rect_filled(
        rect(LABEL_WIDTH - 1.0, 0.0, 1.0, height),
        0,
        theme.sidebar_border,
    );
    painter.rect_filled(rect(0.0, RULER_HEIGHT, width, 1.0), 0, theme.sidebar_border);
    let content = painter.with_clip_rect(rect(LABEL_WIDTH, 0.0, content_width, height));
    draw_playhead(
        &content,
        viewport.scale.time_to_x(playhead, LABEL_WIDTH),
        viewport.scale.frame_width(Time {
            seconds: Fraction::from(1) / frame_rate,
        }),
        height,
        Color::BLUE1,
    );
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
