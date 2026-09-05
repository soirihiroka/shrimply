use std::f32::consts::{PI, TAU};
use std::time::Duration;

use skia_safe::{Canvas, Paint, PaintCap, paint::Style as PaintStyle};

use super::{Color, Rect, math};

const MAX_RADIUS: f32 = 32.0;
const SPIN_DURATION: Duration = Duration::from_millis(1_200);
const N_CYCLES: u32 = 53;
const START_ANGLE: f32 = PI * 0.35;
const CIRCLE_OPACITY: f32 = 0.15;
const MIN_ARC_LENGTH: f32 = PI * 0.015;
const MAX_ARC_LENGTH: f32 = PI * 0.9;
const IDLE_DISTANCE: f32 = PI * 0.9;
const OVERLAP_DISTANCE: f32 = PI * 0.7;
const EXTEND_DISTANCE: f32 = PI * 1.1;
const CONTRACT_DISTANCE: f32 = PI * 1.35;

#[derive(Clone, Copy)]
pub struct Config {
    bounds: Rect,
    color: Color,
    elapsed: Duration,
}

impl Config {
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            color: Color::<f32>::WHITE,
            elapsed: Duration::ZERO,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn elapsed(mut self, elapsed: Duration) -> Self {
        self.elapsed = elapsed;
        self
    }
}

pub fn draw(canvas: &Canvas, config: Config) {
    if config.bounds.width() <= 0.0 || config.bounds.height() <= 0.0 || config.color.a <= 0.0 {
        return;
    }

    let radius = (config.bounds.width().min(config.bounds.height()) / 2.0)
        .floor()
        .min(MAX_RADIUS);
    let line_width = radius / 4.0;
    if radius < line_width / 2.0 {
        return;
    }

    let center = (
        config.bounds.center().x.round(),
        config.bounds.center().y.round(),
    );
    let path_radius = radius - line_width / 2.0;
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(line_width);
    paint.set_stroke_cap(PaintCap::Round);
    paint.set_color(config.color.alpha_multiply(CIRCLE_OPACITY));
    canvas.draw_circle(center, path_radius, &paint);

    let loop_nanos = SPIN_DURATION.as_nanos() * u128::from(N_CYCLES);
    let elapsed_nanos = config.elapsed.as_nanos() % loop_nanos;
    let progress = elapsed_nanos as f32 / SPIN_DURATION.as_nanos() as f32 * TAU;
    let start = progress + arc_start(progress) + START_ANGLE;
    let end = progress + arc_end(progress) + START_ANGLE;
    let sweep = (start - end).rem_euclid(TAU);
    let oval: skia_safe::Rect = Rect::from_xywh(
        center.0 - path_radius,
        center.1 - path_radius,
        path_radius * 2.0,
        path_radius * 2.0,
    )
    .into();
    paint.set_color(config.color);
    canvas.draw_arc(oval, end.to_degrees(), sweep.to_degrees(), false, &paint);
}

fn arc_start(angle: f32) -> f32 {
    let cycle = IDLE_DISTANCE + EXTEND_DISTANCE + CONTRACT_DISTANCE - OVERLAP_DISTANCE;
    let angle = angle.rem_euclid(cycle);
    let progress = if angle > EXTEND_DISTANCE {
        1.0
    } else {
        math::ease_in_out_sine(angle / EXTEND_DISTANCE)
    };
    MIN_ARC_LENGTH + (MAX_ARC_LENGTH - MIN_ARC_LENGTH) * progress - angle * MAX_ARC_LENGTH / cycle
}

fn arc_end(angle: f32) -> f32 {
    let cycle = IDLE_DISTANCE + EXTEND_DISTANCE + CONTRACT_DISTANCE - OVERLAP_DISTANCE;
    let angle = angle.rem_euclid(cycle);
    let progress = if angle < EXTEND_DISTANCE - OVERLAP_DISTANCE {
        0.0
    } else if angle > cycle - IDLE_DISTANCE {
        1.0
    } else {
        math::ease_in_out_sine((angle - EXTEND_DISTANCE + OVERLAP_DISTANCE) / CONTRACT_DISTANCE)
    };
    (MAX_ARC_LENGTH - MIN_ARC_LENGTH) * progress - angle * MAX_ARC_LENGTH / cycle
}
