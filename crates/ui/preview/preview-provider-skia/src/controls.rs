use glam::{Mat3, Vec2};
use shrimply_components_skia::icon::{self, Icon};
use shrimply_math_color::Color;
use shrimply_math_geometry::Rect;
use skia_safe::{
    Canvas as PreviewCanvas, Paint as SkiaPaint, PaintStyle, PathBuilder, Point, RRect,
    paint::{Cap, Join},
};

use crate::Cursor;

pub const KEYPOINT_RADIUS: f32 = 4.5;
pub const KEYPOINT_HIT_RADIUS: f32 = 11.0;
pub const CONTROL_LINE_WIDTH: f32 = 1.5;
pub const CONTROL_SHADOW_WIDTH: f32 = 4.0;
pub const ROTATION_HANDLE_OFFSET: f32 = 24.0;

const CONTROL_CASING_WIDTH: f32 = 3.0;
const CONTROL_OUTER_RIM_WIDTH: f32 = 4.0;
const OUTER_RIM_COLOR: Color = Color::new(0.0, 0.0, 0.0, 0.35);
const INNER_CASING_COLOR: Color = Color::new(1.0, 1.0, 1.0, 0.90);
const HANDLE_FILL_COLOR: Color = Color::WHITE;

const HANDLE_STROKE_WIDTH: f32 = 1.5;
const HANDLE_CASING_EXTRA: f32 = 0.75;
const HANDLE_RIM_EXTRA: f32 = 0.75;

const EDGE_HANDLE_HALF_LENGTH: f32 = 7.5;
const EDGE_HANDLE_HALF_THICKNESS: f32 = 2.75;
const EDGE_HANDLE_EDGE_MARGIN_FACTOR: f32 = 0.25;

const ROTATION_HANDLE_RADIUS: f32 = 9.0;
const ROTATION_HANDLE_CASING_EXTRA: f32 = 1.0;
const ROTATION_HANDLE_RIM_EXTRA: f32 = 0.75;
const ROTATION_ICON_NAME: &str = "view-refresh-symbolic";
const ROTATION_ICON_SIZE: f32 = 12.0;
const ROTATION_VIEWPORT_MARGIN: f32 = 2.0;
const ROTATION_HANDLE_CLEARANCE: f32 = ROTATION_HANDLE_RADIUS
    + ROTATION_HANDLE_CASING_EXTRA
    + ROTATION_HANDLE_RIM_EXTRA
    + ROTATION_VIEWPORT_MARGIN;

const ANCHOR_RING_RADIUS: f32 = 7.0;
const ANCHOR_TICK_LENGTH: f32 = 3.5;
const ANCHOR_STROKE_WIDTH: f32 = 1.75;
const ANCHOR_CENTER_DOT_RADIUS: f32 = 1.75;
const ANCHOR_CASING_STROKE_EXTRA: f32 = 1.5;
const ANCHOR_RIM_STROKE_EXTRA: f32 = 2.5;
const ANCHOR_CASING_DOT_EXTRA: f32 = 0.75;
const ANCHOR_RIM_DOT_EXTRA: f32 = 1.25;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundsHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

pub const BOUNDS_HANDLES: [BoundsHandle; 8] = [
    BoundsHandle::TopLeft,
    BoundsHandle::Top,
    BoundsHandle::TopRight,
    BoundsHandle::Right,
    BoundsHandle::BottomRight,
    BoundsHandle::Bottom,
    BoundsHandle::BottomLeft,
    BoundsHandle::Left,
];

pub fn bounds_handle_position(handle: BoundsHandle, bounds: Rect) -> Vec2 {
    let center = bounds.center();
    match handle {
        BoundsHandle::TopLeft => bounds.min,
        BoundsHandle::Top => Vec2::new(center.x, bounds.min.y),
        BoundsHandle::TopRight => Vec2::new(bounds.max.x, bounds.min.y),
        BoundsHandle::Right => Vec2::new(bounds.max.x, center.y),
        BoundsHandle::BottomRight => bounds.max,
        BoundsHandle::Bottom => Vec2::new(center.x, bounds.max.y),
        BoundsHandle::BottomLeft => Vec2::new(bounds.min.x, bounds.max.y),
        BoundsHandle::Left => Vec2::new(bounds.min.x, center.y),
    }
}

pub fn bounds_handle_cursor(handle: BoundsHandle) -> Cursor {
    match handle {
        BoundsHandle::Top | BoundsHandle::Bottom => Cursor::ResizeVertical,
        BoundsHandle::Left | BoundsHandle::Right => Cursor::ResizeHorizontal,
        BoundsHandle::TopLeft | BoundsHandle::BottomRight => Cursor::ResizeDiagonalDown,
        BoundsHandle::TopRight | BoundsHandle::BottomLeft => Cursor::ResizeDiagonalUp,
    }
}

pub fn hit_keypoint(pointer: Vec2, position: Vec2) -> bool {
    pointer.distance_squared(position) <= KEYPOINT_HIT_RADIUS * KEYPOINT_HIT_RADIUS
}

/// Computes the screen-space `(stem_start, knob_center)` for a transformed box's rotation handle,
/// flipping inward from the top edge when the outward knob would clip outside `visible_rect`.
pub fn rotation_handle_segment(transform: Mat3, bounds: Rect, visible_rect: Rect) -> (Vec2, Vec2) {
    let [top_left, top_right, bottom_right, bottom_left] = rect_corners(transform, bounds);
    let top = (top_left + top_right) * 0.5;
    let center = (top_left + top_right + bottom_left + bottom_right) * 0.25;
    let top_edge = top_right - top_left;
    let outward = if top_edge.length_squared() > f32::EPSILON {
        let normal = Vec2::new(top_edge.y, -top_edge.x).normalize();
        if normal.dot(top - center) < 0.0 {
            -normal
        } else {
            normal
        }
    } else {
        (top - center).try_normalize().unwrap_or(-Vec2::Y)
    };
    let outside = top + outward * ROTATION_HANDLE_OFFSET;
    let inset = Rect::from_min_max(
        visible_rect.min + Vec2::splat(ROTATION_HANDLE_CLEARANCE),
        visible_rect.max - Vec2::splat(ROTATION_HANDLE_CLEARANCE),
    );
    let fits_outside =
        inset.min.x <= inset.max.x && inset.min.y <= inset.max.y && inset.contains(outside);
    if fits_outside {
        (top, outside)
    } else {
        let inside = top - outward * ROTATION_HANDLE_OFFSET;
        let min_half_height = ROTATION_HANDLE_OFFSET
            + ROTATION_HANDLE_RADIUS
            + ANCHOR_RING_RADIUS
            + ANCHOR_TICK_LENGTH;
        if top.distance(center) >= min_half_height
            && inset.min.x <= inset.max.x
            && inset.min.y <= inset.max.y
            && inset.contains(inside)
        {
            (top, inside)
        } else {
            (top, outside)
        }
    }
}

/// Unified Google Slides-style transform selector with layered multi-background-visible outlines.
#[derive(Clone, Copy, Debug)]
pub struct TransformBoxOverlay {
    transform: Mat3,
    bounds: Rect,
    color: Color,
    foreground: Color,
    resize_handles: bool,
    rotation: Option<(Vec2, Vec2)>,
    anchor: Option<Vec2>,
}

impl TransformBoxOverlay {
    pub const fn new(transform: Mat3, bounds: Rect, color: Color, foreground: Color) -> Self {
        Self {
            transform,
            bounds,
            color,
            foreground,
            resize_handles: false,
            rotation: None,
            anchor: None,
        }
    }

    pub const fn with_resize_handles(mut self, enabled: bool) -> Self {
        self.resize_handles = enabled;
        self
    }

    pub const fn with_rotation(mut self, segment: Option<(Vec2, Vec2)>) -> Self {
        self.rotation = segment;
        self
    }

    pub const fn with_anchor(mut self, anchor: Option<Vec2>) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn draw(&self, canvas: &PreviewCanvas) {
        let corners = rect_corners(self.transform, self.bounds);
        // Back layer: draw bounding box and rotation stem together in unified passes so the
        // stem's outer casing never cuts across the top edge or top resize handle.
        let mut path = PathBuilder::new();
        path.move_to(sk_point(corners[0]));
        for &corner in &corners[1..] {
            path.line_to(sk_point(corner));
        }
        path.close();
        if let Some((stem, knob)) = self.rotation {
            path.move_to(sk_point(stem));
            path.line_to(sk_point(knob));
        }
        let path = path.detach();
        draw_layered_stroke(canvas, &path, self.color);

        if self.resize_handles {
            draw_bounds_handles(canvas, self.transform, self.bounds, self.color);
        }
        if let Some((_, knob)) = self.rotation {
            draw_rotation_handle(canvas, knob, self.color, self.foreground);
        }
        if let Some(anchor) = self.anchor {
            draw_anchor(canvas, anchor, self.color);
        }
    }
}

pub fn draw_control_line(canvas: &PreviewCanvas, start: Vec2, end: Vec2, color: Color) {
    let mut path = PathBuilder::new();
    path.move_to(sk_point(start));
    path.line_to(sk_point(end));
    draw_layered_stroke(canvas, &path.detach(), color);
}

pub fn draw_control_polyline(canvas: &PreviewCanvas, points: &[Vec2], closed: bool, color: Color) {
    let Some(&first) = points.first() else {
        return;
    };
    let mut path = PathBuilder::new();
    path.move_to(sk_point(first));
    for &next in &points[1..] {
        path.line_to(sk_point(next));
    }
    if closed {
        path.close();
    }
    draw_layered_stroke(canvas, &path.detach(), color);
}

pub fn draw_control_rect(canvas: &PreviewCanvas, transform: Mat3, bounds: Rect, color: Color) {
    let corners = rect_corners(transform, bounds);
    draw_control_polyline(canvas, &corners, true, color);
}

pub fn draw_keypoint(canvas: &PreviewCanvas, position: Vec2, color: Color) {
    draw_corner_handle(canvas, position, color);
}

pub fn draw_keypoints(canvas: &PreviewCanvas, positions: &[Vec2], color: Color) {
    for &position in positions {
        draw_corner_handle(canvas, position, color);
    }
}

pub fn draw_bounds_handles(canvas: &PreviewCanvas, transform: Mat3, bounds: Rect, color: Color) {
    let [top_left, top_right, bottom_right, bottom_left] = rect_corners(transform, bounds);
    let edges = [
        (top_left, top_right),
        (top_right, bottom_right),
        (bottom_right, bottom_left),
        (bottom_left, top_left),
    ];
    for (start, end) in edges {
        draw_edge_handle(canvas, (start + end) * 0.5, end - start, color);
    }
    for corner in [top_left, top_right, bottom_right, bottom_left] {
        draw_corner_handle(canvas, corner, color);
    }
}

pub fn draw_rotation_handle(
    canvas: &PreviewCanvas,
    position: Vec2,
    color: Color,
    foreground: Color,
) {
    let casing_radius = ROTATION_HANDLE_RADIUS + ROTATION_HANDLE_CASING_EXTRA;
    let rim_radius = casing_radius + ROTATION_HANDLE_RIM_EXTRA;
    canvas.draw_circle(sk_point(position), rim_radius, &fill_paint(OUTER_RIM_COLOR));
    canvas.draw_circle(
        sk_point(position),
        casing_radius,
        &fill_paint(INNER_CASING_COLOR),
    );
    canvas.draw_circle(
        sk_point(position),
        ROTATION_HANDLE_RADIUS,
        &fill_paint(color),
    );
    let half = Vec2::splat(ROTATION_ICON_SIZE * 0.5);
    icon::draw(
        canvas,
        Icon(ROTATION_ICON_NAME),
        Rect::from_min_max(position - half, position + half),
        foreground,
    );
}

pub fn draw_anchor(canvas: &PreviewCanvas, position: Vec2, color: Color) {
    let passes = [
        (
            OUTER_RIM_COLOR,
            ANCHOR_STROKE_WIDTH + ANCHOR_RIM_STROKE_EXTRA,
            ANCHOR_CENTER_DOT_RADIUS + ANCHOR_RIM_DOT_EXTRA,
        ),
        (
            INNER_CASING_COLOR,
            ANCHOR_STROKE_WIDTH + ANCHOR_CASING_STROKE_EXTRA,
            ANCHOR_CENTER_DOT_RADIUS + ANCHOR_CASING_DOT_EXTRA,
        ),
        (color, ANCHOR_STROKE_WIDTH, ANCHOR_CENTER_DOT_RADIUS),
    ];
    for (pass_color, stroke_width, dot_radius) in passes {
        let stroke = round_stroke_paint(pass_color, stroke_width);
        canvas.draw_circle(sk_point(position), ANCHOR_RING_RADIUS, &stroke);
        for direction in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
            let start = position + direction * ANCHOR_RING_RADIUS;
            let end = position + direction * (ANCHOR_RING_RADIUS + ANCHOR_TICK_LENGTH);
            canvas.draw_line(sk_point(start), sk_point(end), &stroke);
        }
        canvas.draw_circle(sk_point(position), dot_radius, &fill_paint(pass_color));
    }
}

fn draw_corner_handle(canvas: &PreviewCanvas, position: Vec2, color: Color) {
    let casing_radius = KEYPOINT_RADIUS + HANDLE_STROKE_WIDTH * 0.5 + HANDLE_CASING_EXTRA;
    let rim_radius = casing_radius + HANDLE_RIM_EXTRA;
    canvas.draw_circle(sk_point(position), rim_radius, &fill_paint(OUTER_RIM_COLOR));
    canvas.draw_circle(
        sk_point(position),
        casing_radius,
        &fill_paint(INNER_CASING_COLOR),
    );
    canvas.draw_circle(
        sk_point(position),
        KEYPOINT_RADIUS,
        &fill_paint(HANDLE_FILL_COLOR),
    );
    canvas.draw_circle(
        sk_point(position),
        KEYPOINT_RADIUS,
        &round_stroke_paint(color, HANDLE_STROKE_WIDTH),
    );
}

fn draw_edge_handle(canvas: &PreviewCanvas, position: Vec2, edge: Vec2, color: Color) {
    let edge_length = edge.length();
    if !edge_length.is_finite() || edge_length <= f32::EPSILON {
        draw_corner_handle(canvas, position, color);
        return;
    }
    let max_half_length = (edge_length * EDGE_HANDLE_EDGE_MARGIN_FACTOR - KEYPOINT_RADIUS)
        .max(EDGE_HANDLE_HALF_THICKNESS);
    let half_length = EDGE_HANDLE_HALF_LENGTH.min(max_half_length);
    let angle_degrees = edge.y.atan2(edge.x).to_degrees();
    let casing_expand = HANDLE_STROKE_WIDTH * 0.5 + HANDLE_CASING_EXTRA;
    let rim_expand = casing_expand + HANDLE_RIM_EXTRA;

    canvas.save();
    canvas.translate((position.x, position.y));
    canvas.rotate(angle_degrees, None);
    canvas.draw_rrect(
        pill_rrect(half_length, EDGE_HANDLE_HALF_THICKNESS, rim_expand),
        &fill_paint(OUTER_RIM_COLOR),
    );
    canvas.draw_rrect(
        pill_rrect(half_length, EDGE_HANDLE_HALF_THICKNESS, casing_expand),
        &fill_paint(INNER_CASING_COLOR),
    );
    canvas.draw_rrect(
        pill_rrect(half_length, EDGE_HANDLE_HALF_THICKNESS, 0.0),
        &fill_paint(HANDLE_FILL_COLOR),
    );
    canvas.draw_rrect(
        pill_rrect(half_length, EDGE_HANDLE_HALF_THICKNESS, 0.0),
        &round_stroke_paint(color, HANDLE_STROKE_WIDTH),
    );
    canvas.restore();
}

fn pill_rrect(half_length: f32, half_thickness: f32, expand: f32) -> RRect {
    let half_width = half_length + expand;
    let half_height = half_thickness + expand;
    RRect::new_rect_xy(
        skia_safe::Rect::from_ltrb(-half_width, -half_height, half_width, half_height),
        half_height,
        half_height,
    )
}

fn draw_layered_stroke(canvas: &PreviewCanvas, path: &skia_safe::Path, color: Color) {
    canvas.draw_path(
        path,
        &round_stroke_paint(OUTER_RIM_COLOR, CONTROL_OUTER_RIM_WIDTH),
    );
    canvas.draw_path(
        path,
        &round_stroke_paint(INNER_CASING_COLOR, CONTROL_CASING_WIDTH),
    );
    canvas.draw_path(path, &round_stroke_paint(color, CONTROL_LINE_WIDTH));
}

fn rect_corners(transform: Mat3, bounds: Rect) -> [Vec2; 4] {
    [
        bounds.min,
        Vec2::new(bounds.max.x, bounds.min.y),
        bounds.max,
        Vec2::new(bounds.min.x, bounds.max.y),
    ]
    .map(|point| transform.transform_point2(point))
}

fn fill_paint(color: Color) -> SkiaPaint {
    let mut paint = SkiaPaint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);
    paint.set_color(color);
    paint
}

fn round_stroke_paint(color: Color, width: f32) -> SkiaPaint {
    let mut paint = fill_paint(color);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width);
    paint.set_stroke_cap(Cap::Round);
    paint.set_stroke_join(Join::Round);
    paint
}

fn sk_point(value: Vec2) -> Point {
    Point::new(value.x, value.y)
}
