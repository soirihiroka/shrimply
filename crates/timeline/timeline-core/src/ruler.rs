use super::renderer::{Align2, Color, FontId, Rect, vec2};

use crate::time_format;

use super::renderer::TimelinePainter;

#[derive(Clone, Copy)]
pub struct TimelineScale {
    pub scroll_seconds: f64,
    pub seconds_per_pixel: f64,
}

impl TimelineScale {
    pub fn new(scroll_seconds: f64, seconds_per_pixel: f64) -> Self {
        Self {
            scroll_seconds,
            seconds_per_pixel,
        }
    }

    pub fn time_to_x(self, time_seconds: f64, x: f64) -> f64 {
        x + (time_seconds - self.scroll_seconds) / self.seconds_per_pixel
    }

    pub fn frame_width(self, frame_step_seconds: f64) -> f64 {
        if self.seconds_per_pixel <= 0.0 {
            return 1.0;
        }
        frame_step_seconds / self.seconds_per_pixel
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
    pub frame_step_seconds: f64,
    pub frame_rate: crate::Fraction,
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
        let x = draw
            .scale
            .time_to_x(tick.time.as_secs_f64(), draw.timeline_x);
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
    time: crate::project::Time,
    label: Option<String>,
}

fn visible_ticks(
    scale: TimelineScale,
    timeline_width: f64,
    frame_step_seconds: f64,
    frame_rate: crate::Fraction,
    hide_zero_label: bool,
    frame_tick_min_width: f64,
) -> Vec<RulerTick> {
    let start = scale.scroll_seconds.max(0.0);
    let end = (scale.scroll_seconds + timeline_width * scale.seconds_per_pixel).max(start);
    let frame_pixels = scale.frame_width(frame_step_seconds);
    if frame_pixels >= frame_tick_min_width {
        return visible_frame_ticks(start, end, frame_rate, frame_pixels, hide_zero_label);
    }

    let step = nice_tick_step(scale.seconds_per_pixel * 120.0);
    let mut tick = (start / step).floor() * step;
    let mut ticks = Vec::new();
    while tick <= end {
        if tick >= 0.0 {
            ticks.push(RulerTick {
                time: crate::project::Time::from_seconds_f64(tick),
                label: (!hide_zero_label || tick > f64::EPSILON)
                    .then(|| time_format::timeline_tick(tick)),
            });
        }
        tick += step;
    }
    ticks
}

fn visible_frame_ticks(
    start: f64,
    end: f64,
    frame_rate: crate::Fraction,
    frame_pixels: f64,
    hide_zero_label: bool,
) -> Vec<RulerTick> {
    let start_frame = shrimply_math_core::nonnegative_frame_index(
        crate::project::Time::from_seconds_f64(start),
        frame_rate,
    )
    .expect("project frame rate must be positive");
    let end_frame =
        shrimply_math_core::frame_count(crate::project::Time::from_seconds_f64(end), frame_rate)
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

fn nice_tick_step(raw: f64) -> f64 {
    let raw = raw.max(0.001);
    let magnitude = 10_f64.powf(raw.log10().floor());
    for multiplier in [1.0, 2.0, 5.0, 10.0] {
        let step = magnitude * multiplier;
        if step >= raw {
            return step;
        }
    }
    magnitude * 10.0
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect::from_min_size(
        vec2(x as f32, y as f32),
        vec2(width.max(0.0) as f32, height.max(0.0) as f32),
    )
}
