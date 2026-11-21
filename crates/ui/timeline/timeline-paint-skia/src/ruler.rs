use super::canvas::{Align2, Color, FontId, Rect, vec2};

use crate::math::nice_tick_step;
use shrimply_math_core::{Fraction, Time};

use super::canvas::TimelinePainter;

#[derive(Clone, Copy)]
pub struct TimelineScale {
    pub scroll_seconds: Time,
    pub seconds_per_pixel: Fraction,
}

impl TimelineScale {
    pub fn new(scroll_seconds: Time, seconds_per_pixel: Fraction) -> Self {
        Self {
            scroll_seconds,
            seconds_per_pixel,
        }
    }

    pub fn time_to_x(self, time: Time, x: f64) -> f64 {
        x + shrimply_math_core::time_ratio_f64(
            time.signed_sub(self.scroll_seconds),
            Time {
                seconds: self.seconds_per_pixel,
            },
        )
    }

    pub fn frame_width(self, frame_step_seconds: Time) -> f64 {
        if self.seconds_per_pixel <= Fraction::from(0) {
            return 1.0;
        }
        shrimply_math_core::time_ratio_f64(
            frame_step_seconds,
            Time {
                seconds: self.seconds_per_pixel,
            },
        )
    }
}

pub struct RulerStyle {
    pub height: f64,
    pub frame_tick_min_width: f64,
    pub grid_color: Color,
    pub label_color: Color,
}

pub struct RulerDraw {
    pub scale: TimelineScale,
    pub timeline_x: f64,
    pub timeline_width: f64,
    pub content_height: f64,
    pub frame_step_seconds: Time,
    pub frame_rate: Fraction,
    pub hide_zero_label: bool,
    pub style: RulerStyle,
}

pub fn draw(painter: &TimelinePainter, draw: RulerDraw) {
    for tick in visible_ticks(
        draw.scale,
        draw.timeline_width,
        draw.frame_step_seconds,
        draw.frame_rate,
        draw.hide_zero_label,
        draw.style.frame_tick_min_width,
    ) {
        let x = draw.scale.time_to_x(tick.time, draw.timeline_x);
        painter.rect_filled(
            rect(
                x,
                draw.style.height - 5.0,
                1.0,
                draw.content_height - draw.style.height,
            ),
            0,
            draw.style.grid_color,
        );
        if let Some(label) = tick.label {
            painter.text(
                vec2((x + 3.0) as f32, 7.0),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(12.0),
                draw.style.label_color,
            );
        }
    }
}

struct RulerTick {
    time: Time,
    label: Option<String>,
}

fn visible_ticks(
    scale: TimelineScale,
    timeline_width: f64,
    frame_step_seconds: Time,
    frame_rate: Fraction,
    hide_zero_label: bool,
    frame_tick_min_width: f64,
) -> Vec<RulerTick> {
    let start = scale.scroll_seconds.max(Time::ZERO);
    let end = scale
        .scroll_seconds
        .saturating_add(
            Time {
                seconds: scale.seconds_per_pixel,
            }
            .scaled(Fraction::from(timeline_width)),
        )
        .max(start);
    let frame_pixels = scale.frame_width(frame_step_seconds);
    if frame_pixels >= frame_tick_min_width {
        return visible_frame_ticks(start, end, frame_rate, frame_pixels, hide_zero_label);
    }

    let step = nice_tick_step(scale.seconds_per_pixel);
    let first =
        shrimply_math_core::fraction_floor_i64(start.seconds / step).expect("finite ruler start");
    let mut tick = Time {
        seconds: Fraction::from(first) * step,
    };
    let mut ticks = Vec::new();
    while tick <= end {
        if tick >= Time::ZERO {
            ticks.push(RulerTick {
                time: tick,
                label: (!hide_zero_label || tick > Time::ZERO).then(|| timeline_tick(tick)),
            });
        }
        tick = tick.saturating_add(Time { seconds: step });
    }
    ticks
}

fn visible_frame_ticks(
    start: Time,
    end: Time,
    frame_rate: Fraction,
    frame_pixels: f64,
    hide_zero_label: bool,
) -> Vec<RulerTick> {
    let start_frame = shrimply_math_core::nonnegative_frame_index(start, frame_rate)
        .expect("project frame rate must be positive");
    let end_frame = shrimply_math_core::frame_count(end, frame_rate)
        .expect("project frame rate must be positive");
    let label_every = (48.0 / frame_pixels.max(1.0)).ceil().max(1.0) as u64;
    let mut ticks = Vec::new();
    for frame in start_frame..=end_frame {
        ticks.push(RulerTick {
            time: shrimply_math_core::time_from_frame(frame, frame_rate)
                .expect("visible project frame must have an exact time"),
            label: ((!hide_zero_label || frame > 0) && frame % label_every == 0)
                .then(|| frame.to_string()),
        });
    }
    ticks
}

fn timeline_tick(time: Time) -> String {
    let seconds = time.as_secs_f64();
    if seconds < 60.0 {
        return format!("{seconds:.1}s");
    }
    let total = shrimply_math_core::fraction_round_nonnegative_u64(time.seconds);
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect::from_xywh(
        x as f32,
        y as f32,
        width.max(0.0) as f32,
        height.max(0.0) as f32,
    )
}
