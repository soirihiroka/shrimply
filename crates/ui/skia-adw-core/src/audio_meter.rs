use std::time::{Duration, Instant};

use shrimply_cross_ui_theme as theme;
use shrimply_math_color::Color;

use crate::canvas::{Align2, FontId, Rect, Stroke, TimelinePainter, vec2};

const MIN_DB: f32 = -60.0;
const YELLOW_DB: f32 = -18.0;
const RED_DB: f32 = -6.0;
const RELEASE_DB_PER_SECOND: f32 = 24.0;
const PEAK_HOLD: Duration = Duration::from_millis(1_500);
pub const DEFAULT_WIDTH: i32 = 54;
const PADDING_LEFT: f32 = 6.0;
const PADDING_RIGHT: f32 = 7.0;
const METER_VERTICAL_PADDING: f32 = 16.0;
const RULER_WIDTH: f32 = 32.0;
const CHANNEL_GAP: f32 = 1.0;
const FONT_SIZE: f32 = 8.0;
const RULER_LABEL_ALPHA: f32 = 0.55;
const RULER_TICK_ALPHA: f32 = 0.35;

#[derive(Clone, Copy)]
struct ChannelLevel {
    level_db: f32,
    peak_db: f32,
    peak_at: Instant,
}

pub struct AudioMeter {
    channels: [ChannelLevel; 2],
    last_frame: Instant,
}

impl Default for AudioMeter {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            channels: [ChannelLevel {
                level_db: MIN_DB,
                peak_db: MIN_DB,
                peak_at: now,
            }; 2],
            last_frame: now,
        }
    }
}

impl AudioMeter {
    pub fn draw(&self, canvas: &skia_safe::Canvas, width: f32, height: f32) {
        draw_meter(&TimelinePainter::new(canvas), width, height, self.channels);
    }

    pub fn update(&mut self, peaks: [f32; 2], now: Instant) {
        let release = RELEASE_DB_PER_SECOND * now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        for (channel, amplitude) in self.channels.iter_mut().zip(peaks) {
            let level = amplitude_to_db(amplitude);
            channel.level_db = if level >= channel.level_db {
                level
            } else {
                (channel.level_db - release).max(level).max(MIN_DB)
            };

            if level >= channel.peak_db {
                channel.peak_db = level;
                channel.peak_at = now;
            } else if now.duration_since(channel.peak_at) >= PEAK_HOLD {
                channel.peak_db = (channel.peak_db - release)
                    .max(channel.level_db)
                    .max(MIN_DB);
            }
        }
    }
}

fn amplitude_to_db(amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        MIN_DB
    } else {
        (20.0 * amplitude.log10()).clamp(MIN_DB, 0.0)
    }
}

fn draw_meter(painter: &TimelinePainter, width: f32, height: f32, channels: [ChannelLevel; 2]) {
    let top = METER_VERTICAL_PADDING.min((height - 1.0) / 2.0);
    let bottom = (height - METER_VERTICAL_PADDING).max(top + 1.0);
    let bars_right = (width - PADDING_RIGHT - RULER_WIDTH).max(PADDING_LEFT + CHANNEL_GAP + 2.0);
    let meter = Rect::from_min_size(
        vec2(PADDING_LEFT, top),
        vec2(bars_right - PADDING_LEFT, bottom - top),
    );
    painter.rect_filled(meter, 1.0, theme::current().sidebar_bg);

    let bar_left = meter.left();
    let bar_top = meter.top();
    let bar_bottom = meter.bottom();
    let bars_width = (meter.right() - bar_left - CHANNEL_GAP).max(2.0);
    let channel_width = bars_width / 2.0;
    let bars = [
        Rect::from_min_size(
            vec2(bar_left, bar_top),
            vec2(channel_width, bar_bottom - bar_top),
        ),
        Rect::from_min_size(
            vec2(bar_left + channel_width + CHANNEL_GAP, bar_top),
            vec2(channel_width, bar_bottom - bar_top),
        ),
    ];

    for (bar, channel) in bars.into_iter().zip(channels) {
        draw_level_band(
            painter,
            bar,
            channel.level_db,
            MIN_DB,
            YELLOW_DB,
            Color::GREEN3,
        );
        draw_level_band(
            painter,
            bar,
            channel.level_db,
            YELLOW_DB,
            RED_DB,
            Color::YELLOW3,
        );
        draw_level_band(painter, bar, channel.level_db, RED_DB, 0.0, Color::RED3);
        if channel.peak_db > MIN_DB {
            let peak_y = db_y(channel.peak_db, bar.top(), bar.bottom());
            let peak_color = if channel.peak_db >= RED_DB {
                Color::RED1
            } else {
                Color::YELLOW1
            };
            painter.rect_filled(
                Rect::from_min_size(vec2(bar.left(), peak_y - 1.0), vec2(bar.width(), 2.0)),
                0,
                peak_color,
            );
        }
    }

    draw_ruler(painter, bars_right, width, meter.top(), meter.bottom());
}

fn draw_level_band(
    painter: &TimelinePainter,
    bar: Rect,
    level_db: f32,
    low_db: f32,
    high_db: f32,
    color: Color,
) {
    let active_high = level_db.min(high_db);
    if active_high <= low_db {
        return;
    }
    let top = db_y(active_high, bar.top(), bar.bottom());
    let bottom = db_y(low_db, bar.top(), bar.bottom());
    painter.rect_filled(
        Rect::from_min_size(vec2(bar.left(), top), vec2(bar.width(), bottom - top)),
        0,
        color,
    );
}

fn draw_ruler(painter: &TimelinePainter, bars_right: f32, width: f32, top: f32, bottom: f32) {
    let height = bottom - top;
    let step = if height * 3.0 / -MIN_DB >= 14.0 {
        3
    } else if height * 6.0 / -MIN_DB >= 14.0 {
        6
    } else {
        12
    };
    let tick_start = bars_right + 5.0;
    let tick_end = tick_start + 3.0;
    let label_gap = 4.0;
    let font = FontId::proportional(FONT_SIZE);
    let foreground = theme::current().view_fg;
    let tick_color = foreground.alpha_multiply(RULER_TICK_ALPHA);
    let label_color = foreground.alpha_multiply(RULER_LABEL_ALPHA);

    for db in (0..=60).step_by(step) {
        let y = db_y(-(db as f32), top, bottom);
        painter.line_segment(
            [vec2(tick_start, y), vec2(tick_end, y)],
            Stroke::new(1.0, tick_color),
        );
        let label = if db == 0 {
            String::from("0")
        } else {
            format!("-{db}")
        };
        let size = painter
            .layout_no_wrap(&label, font.clone(), label_color)
            .size();
        if tick_end + label_gap + size.x <= width - PADDING_RIGHT {
            painter.text(
                vec2(tick_end + label_gap, y - size.y / 2.0),
                Align2::LEFT_TOP,
                label,
                font.clone(),
                label_color,
            );
        }
    }
}

fn db_y(db: f32, top: f32, bottom: f32) -> f32 {
    top + (-db.clamp(MIN_DB, 0.0) / -MIN_DB) * (bottom - top)
}
