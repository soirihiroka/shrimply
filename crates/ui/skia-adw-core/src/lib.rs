use std::time::Duration;

pub use shrimply_math_color::Color;
pub use shrimply_math_geometry::{Rect, Vec2};
use skia_safe::{
    Canvas, Color4f, Paint, Point, RRect, TileMode, gradient, paint::Style as PaintStyle,
};

pub mod audio_meter;
pub mod button;
pub mod canvas;
pub mod cursor;
pub mod font_grid;
pub mod icon;
pub mod math;
pub mod skia_font;
pub mod skia_system_font;
pub mod slider;
pub mod spinner;

pub const OVERSHOOT_MAX_DISTANCE: f64 = 100.0;
pub const OVERSHOOT_FRICTION: f64 = 20.0;
pub const OVERSHOOT_VISIBLE_DISTANCE: f64 = 0.1;

const OVERSHOOT_SMALL_LENGTH: f64 = 0.03;
const OVERSHOOT_BIG_LENGTH: f64 = 0.50;
const OVERSHOOT_SMALL_ALPHA: f32 = 0.12;
const OVERSHOOT_BIG_ALPHA: f32 = 0.05;
const SCROLLBAR_ALONG_MARGIN: f32 = 9.0;
const SCROLLBAR_RESTING_EDGE_MARGIN: f32 = 4.0;
const SCROLLBAR_HOVER_EDGE_MARGIN: f32 = 8.0;
const SCROLLBAR_THICKNESS: f32 = 8.0;
const SCROLLBAR_RESTING_THICKNESS: f32 = 3.0;
const SCROLLBAR_MIN_THUMB_LENGTH: f64 = 40.0;
const SCROLLBAR_TROUGH_ALPHA: f32 = 0.10;
const SCROLLBAR_THUMB_ALPHA: f32 = 0.20;
const SCROLLBAR_THUMB_HOVER_ALPHA: f32 = 0.40;
const SCROLLBAR_THUMB_ACTIVE_ALPHA: f32 = 0.60;
const SCROLLBAR_OUTLINE_ALPHA: f32 = 0.35;
const SCROLLBAR_OUTLINE_HOVER_ALPHA: f32 = 0.60;
const SCROLL_ANIMATION_DURATION: Duration = Duration::from_millis(200);
const SCROLLBAR_TRANSITION_DURATION: Duration = Duration::from_millis(200);

#[derive(Clone, Copy)]
pub enum Edge {
    Left,
    Right,
}

#[derive(Clone, Copy)]
pub struct ScrollbarState {
    pub expansion: f32,
    pub active: bool,
}

#[derive(Clone, Copy)]
pub struct Scrollbar {
    pub axis: Axis,
    pub bounds: Rect,
    pub content_length: f64,
    pub viewport_length: f64,
    pub value: f64,
    pub color: Color,
    pub outline_color: Color,
    pub state: ScrollbarState,
}

#[derive(Clone, Copy)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy)]
pub struct ScrollbarMetrics {
    pub track: Rect,
    pub thumb: Rect,
    pub max_value: f64,
}

#[derive(Clone, Copy)]
pub struct ScrollAnimation {
    pub source: f64,
    pub target: f64,
    pub elapsed: Duration,
}

pub fn overshoot_distance(distance: f64, elapsed: Duration) -> f64 {
    let t = elapsed.as_secs_f64();
    let half_friction = OVERSHOOT_FRICTION / 2.0;
    distance * (-half_friction * t).exp() * (1.0 + half_friction * t)
}

pub fn scrollbar_metrics(scrollbar: Scrollbar) -> Option<ScrollbarMetrics> {
    if scrollbar.bounds.width() <= 0.0
        || scrollbar.bounds.height() <= 0.0
        || scrollbar.content_length <= 0.0
        || scrollbar.viewport_length <= 0.0
    {
        return None;
    }

    let track = scrollbar_track(scrollbar);
    let track_length = match scrollbar.axis {
        Axis::Horizontal => track.width(),
        Axis::Vertical => track.height(),
    };
    if track_length <= 0.0 {
        return None;
    }

    let content_length = scrollbar.content_length.max(scrollbar.viewport_length);
    let max_value = (content_length - scrollbar.viewport_length).max(0.0);
    let thumb_length = (f64::from(track_length) * scrollbar.viewport_length / content_length)
        .clamp(
            SCROLLBAR_MIN_THUMB_LENGTH.min(f64::from(track_length)),
            f64::from(track_length),
        );
    let max_travel = (f64::from(track_length) - thumb_length).max(0.0);
    let progress = if max_value <= f64::EPSILON || max_travel <= f64::EPSILON {
        0.0
    } else {
        (scrollbar.value / max_value).clamp(0.0, 1.0)
    };
    let thumb_offset = progress * max_travel;
    let thickness = scrollbar_thickness(scrollbar.state);
    let thumb = match scrollbar.axis {
        Axis::Horizontal => Rect::from_xywh(
            track.left() + thumb_offset as f32,
            track.top(),
            thumb_length as f32,
            thickness,
        ),
        Axis::Vertical => Rect::from_xywh(
            track.left(),
            track.top() + thumb_offset as f32,
            thickness,
            thumb_length as f32,
        ),
    };

    Some(ScrollbarMetrics {
        track,
        thumb,
        max_value,
    })
}

pub fn draw_scrollbar(canvas: &Canvas, scrollbar: Scrollbar) -> Option<ScrollbarMetrics> {
    let metrics = scrollbar_metrics(scrollbar)?;
    if scrollbar.state.expansion > 0.0 {
        draw_rounded_rect(
            canvas,
            scrollbar_track(scrollbar),
            SCROLLBAR_THICKNESS / 2.0,
            scrollbar
                .color
                .alpha_multiply(SCROLLBAR_TROUGH_ALPHA * scrollbar.state.expansion),
        );
    }

    draw_rounded_rect(
        canvas,
        metrics.thumb,
        scrollbar_thickness(scrollbar.state) / 2.0,
        scrollbar
            .color
            .alpha_multiply(scrollbar_thumb_alpha(scrollbar.state)),
    );
    draw_rounded_rect_stroke(
        canvas,
        metrics.thumb,
        scrollbar_thickness(scrollbar.state) / 2.0,
        1.0,
        scrollbar
            .outline_color
            .alpha_multiply(scrollbar_outline_alpha(scrollbar.state)),
    );

    Some(metrics)
}

pub fn animate_scroll(animation: ScrollAnimation) -> (f64, bool) {
    if animation.elapsed >= SCROLL_ANIMATION_DURATION {
        return (animation.target, false);
    }

    let t =
        (animation.elapsed.as_secs_f64() / SCROLL_ANIMATION_DURATION.as_secs_f64()).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    (
        animation.source + eased * (animation.target - animation.source),
        true,
    )
}

pub fn animate_scrollbar_expansion(source: f32, target: f32, elapsed: Duration) -> (f32, bool) {
    if elapsed >= SCROLLBAR_TRANSITION_DURATION {
        return (target, false);
    }

    let t = (elapsed.as_secs_f32() / SCROLLBAR_TRANSITION_DURATION.as_secs_f32()).clamp(0.0, 1.0);
    (source + t * (target - source), true)
}

pub fn draw_overshoot(canvas: &Canvas, viewport: Rect, edge: Edge, distance: f64, color: Color) {
    if viewport.width() <= 0.0 || viewport.height() <= 0.0 || distance <= 0.0 {
        return;
    }

    let distance = distance
        .clamp(0.0, OVERSHOOT_MAX_DISTANCE)
        .min(f64::from(viewport.width()));
    if distance <= 0.0 {
        return;
    }

    draw_overshoot_layer(
        canvas,
        viewport,
        edge,
        distance,
        OVERSHOOT_BIG_LENGTH,
        (
            &[
                color.alpha_multiply(OVERSHOOT_BIG_ALPHA),
                color.with_alpha(0.0),
            ],
            &[0.0, 1.0],
        ),
    );
    draw_overshoot_layer(
        canvas,
        viewport,
        edge,
        distance,
        OVERSHOOT_SMALL_LENGTH,
        (
            &[
                color.alpha_multiply(OVERSHOOT_SMALL_ALPHA),
                color.alpha_multiply(OVERSHOOT_SMALL_ALPHA),
                color.with_alpha(0.0),
            ],
            &[0.0, 0.85, 1.0],
        ),
    );
}

fn scrollbar_track(scrollbar: Scrollbar) -> Rect {
    let thickness = scrollbar_thickness(scrollbar.state);
    let edge_margin = scrollbar_edge_margin(scrollbar.state);
    match scrollbar.axis {
        Axis::Horizontal => Rect::from_xywh(
            scrollbar.bounds.left() + SCROLLBAR_ALONG_MARGIN,
            scrollbar.bounds.bottom() - edge_margin - thickness,
            (scrollbar.bounds.width() - SCROLLBAR_ALONG_MARGIN * 2.0).max(0.0),
            thickness,
        ),
        Axis::Vertical => Rect::from_xywh(
            scrollbar.bounds.right() - edge_margin - thickness,
            scrollbar.bounds.top() + SCROLLBAR_ALONG_MARGIN,
            thickness,
            (scrollbar.bounds.height() - SCROLLBAR_ALONG_MARGIN * 2.0).max(0.0),
        ),
    }
}

fn scrollbar_thickness(state: ScrollbarState) -> f32 {
    SCROLLBAR_RESTING_THICKNESS
        + (SCROLLBAR_THICKNESS - SCROLLBAR_RESTING_THICKNESS) * state.expansion.clamp(0.0, 1.0)
}

fn scrollbar_edge_margin(state: ScrollbarState) -> f32 {
    SCROLLBAR_RESTING_EDGE_MARGIN
        + (SCROLLBAR_HOVER_EDGE_MARGIN - SCROLLBAR_RESTING_EDGE_MARGIN)
            * state.expansion.clamp(0.0, 1.0)
}

fn scrollbar_thumb_alpha(state: ScrollbarState) -> f32 {
    if state.active {
        SCROLLBAR_THUMB_ACTIVE_ALPHA
    } else {
        SCROLLBAR_THUMB_ALPHA
            + (SCROLLBAR_THUMB_HOVER_ALPHA - SCROLLBAR_THUMB_ALPHA)
                * state.expansion.clamp(0.0, 1.0)
    }
}

fn scrollbar_outline_alpha(state: ScrollbarState) -> f32 {
    SCROLLBAR_OUTLINE_ALPHA
        + (SCROLLBAR_OUTLINE_HOVER_ALPHA - SCROLLBAR_OUTLINE_ALPHA)
            * state.expansion.clamp(0.0, 1.0)
}

fn draw_overshoot_layer(
    canvas: &Canvas,
    viewport: Rect,
    edge: Edge,
    distance: f64,
    width_fraction: f64,
    gradient: (&[Color], &[f32]),
) {
    let width = distance * width_fraction;
    let viewport_x = f64::from(viewport.left());
    let viewport_y = f64::from(viewport.top());
    let viewport_right = f64::from(viewport.right());
    let viewport_height = f64::from(viewport.height());
    let (x, center_x) = match edge {
        Edge::Left => (viewport_x, viewport_x),
        Edge::Right => (viewport_right - width, viewport_right),
    };
    draw_elliptical_radial_gradient(
        canvas,
        Rect::from_xywh(x as f32, viewport.top(), width as f32, viewport.height()),
        Point::new(center_x as f32, (viewport_y + viewport_height / 2.0) as f32),
        width as f32,
        (viewport_height / 2.0) as f32,
        gradient.0,
        gradient.1,
    );
}

fn draw_rounded_rect(canvas: &Canvas, rect: Rect, radius: f32, color: Color) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 || color.a <= 0.0 {
        return;
    }

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);
    paint.set_color(color);
    canvas.draw_rrect(
        RRect::new_rect_xy(skia_safe::Rect::from(rect), radius, radius),
        &paint,
    );
}

fn draw_rounded_rect_stroke(canvas: &Canvas, rect: Rect, radius: f32, width: f32, color: Color) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 || width <= 0.0 || color.a <= 0.0 {
        return;
    }

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width);
    paint.set_color(color);
    canvas.draw_rrect(
        RRect::new_rect_xy(skia_safe::Rect::from(rect), radius, radius),
        &paint,
    );
}

fn draw_elliptical_radial_gradient(
    canvas: &Canvas,
    rect: Rect,
    center: Point,
    radius_x: f32,
    radius_y: f32,
    colors: &[Color],
    positions: &[f32],
) {
    if rect.width() <= 0.0
        || rect.height() <= 0.0
        || radius_x <= 0.0
        || radius_y <= 0.0
        || colors.len() < 2
        || colors.len() != positions.len()
        || colors.iter().all(|color| color.a <= 0.0)
    {
        return;
    }

    let colors: Vec<Color4f> = colors.iter().copied().map(Into::into).collect();
    let gradient_colors = gradient::Colors::new(&colors, Some(positions), TileMode::Clamp, None);
    let gradient = gradient::Gradient::new(gradient_colors, gradient::Interpolation::default());
    let Some(shader) =
        gradient::shaders::radial_gradient((Point::new(0.0, 0.0), 1.0), &gradient, None)
    else {
        return;
    };

    let local_rect = skia_safe::Rect::from_xywh(
        (rect.left() - center.x) / radius_x,
        (rect.top() - center.y) / radius_y,
        rect.width() / radius_x,
        rect.height() / radius_y,
    );

    canvas.save();
    canvas.clip_rect(
        skia_safe::Rect::from(rect),
        skia_safe::ClipOp::Intersect,
        true,
    );
    canvas.translate((center.x, center.y));
    canvas.scale((radius_x, radius_y));

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);
    paint.set_shader(shader);
    canvas.draw_rect(local_rect, &paint);
    canvas.restore();
}
