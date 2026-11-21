use glam::{Mat3, Vec2};
use shrimply_components_skia::icon::{self, Icon};
use shrimply_math_color::Color;
use shrimply_math_geometry::Rect;
use skia_safe::{
    Canvas as PreviewCanvas, ImageFilter, Paint as SkiaPaint, PaintStyle, PathBuilder, Point,
    RRect, TileMode,
    canvas::SaveLayerRec,
    color_filters, image_filters,
    paint::{Cap, Join},
};

use crate::Cursor;

pub const KEYPOINT_RADIUS: f32 = 4.5;
pub const KEYPOINT_HIT_RADIUS: f32 = 11.0;
pub const CONTROL_LINE_WIDTH: f32 = 2.0;
pub const ROTATION_HANDLE_OFFSET: f32 = 25.0;

const OUTLINE_BLUR_SIGMA: f32 = 0.75;
const OUTLINE_ALPHA_GAIN: f32 = 3.8;
const OUTLINE_TINT_MIX: f32 = 0.70;
const LIGHT_ACCENT_LUMA_THRESHOLD: f32 = 0.72;
const HANDLE_FILL_COLOR: Color = Color::WHITE;
const HANDLE_STROKE_WIDTH: f32 = CONTROL_LINE_WIDTH;

const EDGE_HANDLE_HALF_LENGTH: f32 = 9.0;
const EDGE_HANDLE_HALF_THICKNESS: f32 = 3.0;
const EDGE_HANDLE_MARGIN_FACTOR: f32 = 0.25;

const ROTATION_HANDLE_RADIUS: f32 = 8.5;
const ROTATION_ICON_NAME: &str = "view-refresh-symbolic";
const ROTATION_ICON_SIZE: f32 = 12.0;
const ROTATION_ICON_VIEWBOX: f32 = 16.0;
// Optical center of the circular arc in `view-refresh-symbolic.svg` (the arrowhead extends right).
const ROTATION_ICON_ARC_CENTER: Vec2 = Vec2::new(7.5, 8.5);
const ROTATION_CLEARANCE: f32 = ROTATION_HANDLE_RADIUS + 4.0;

const ANCHOR_RING_RADIUS: f32 = 7.0;
const ANCHOR_TICK_LENGTH: f32 = 3.5;
const ANCHOR_STROKE_WIDTH: f32 = 1.75;
const ANCHOR_DOT_RADIUS: f32 = 1.75;

pub fn selection_color() -> Color {
    shrimply_cross_ui_theme::current().accent_bg
}

pub fn selection_foreground() -> Color {
    shrimply_cross_ui_theme::current().accent_fg
}

/// Composites `draw` as a single control layer with a symmetric accent-tinted contrast outline
/// so every overlay primitive stays solid, continuous, and legible on any background.
pub fn with_control_outline(canvas: &PreviewCanvas, draw: impl FnOnce(&PreviewCanvas)) {
    let mut paint = SkiaPaint::default();
    if let Some(filter) = control_outline_filter() {
        paint.set_image_filter(filter);
    }
    canvas.save_layer(&SaveLayerRec::default().paint(&paint));
    draw(canvas);
    canvas.restore();
}

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

impl BoundsHandle {
    pub const fn opposite(self) -> Self {
        match self {
            Self::TopLeft => Self::BottomRight,
            Self::Top => Self::Bottom,
            Self::TopRight => Self::BottomLeft,
            Self::Right => Self::Left,
            Self::BottomRight => Self::TopLeft,
            Self::Bottom => Self::Top,
            Self::BottomLeft => Self::TopRight,
            Self::Left => Self::Right,
        }
    }

    pub const fn axes(self) -> (i8, i8) {
        match self {
            Self::TopLeft => (-1, -1),
            Self::Top => (0, -1),
            Self::TopRight => (1, -1),
            Self::Right => (1, 0),
            Self::BottomRight => (1, 1),
            Self::Bottom => (0, 1),
            Self::BottomLeft => (-1, 1),
            Self::Left => (-1, 0),
        }
    }
}

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

pub fn hit_bounds_handle(pointer: Vec2, transform: Mat3, bounds: Rect) -> Option<BoundsHandle> {
    BOUNDS_HANDLES.into_iter().find(|&handle| {
        hit_keypoint(
            pointer,
            transform.transform_point2(bounds_handle_position(handle, bounds)),
        )
    })
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

/// Computes `(stem_start, knob_center)` in screen space, flipping inward when the outward knob
/// would clip outside `visible_rect`.
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
        visible_rect.min + Vec2::splat(ROTATION_CLEARANCE),
        visible_rect.max - Vec2::splat(ROTATION_CLEARANCE),
    );
    let valid_inset = inset.min.x <= inset.max.x && inset.min.y <= inset.max.y;
    if valid_inset && inset.contains(outside) {
        return (top, outside);
    }
    let inside = top - outward * ROTATION_HANDLE_OFFSET;
    let min_half_height =
        ROTATION_HANDLE_OFFSET + ROTATION_HANDLE_RADIUS + ANCHOR_RING_RADIUS + ANCHOR_TICK_LENGTH;
    if valid_inset && top.distance(center) >= min_half_height && inset.contains(inside) {
        (top, inside)
    } else {
        (top, outside)
    }
}

pub fn draw_control_line(canvas: &PreviewCanvas, start: Vec2, end: Vec2, color: Color) {
    canvas.draw_line(
        sk_point(start),
        sk_point(end),
        &stroke_paint(color, CONTROL_LINE_WIDTH, Cap::Butt, Join::Miter),
    );
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
    canvas.draw_path(
        &path.detach(),
        &stroke_paint(color, CONTROL_LINE_WIDTH, Cap::Butt, Join::Miter),
    );
}

pub fn draw_control_rect(canvas: &PreviewCanvas, transform: Mat3, bounds: Rect, color: Color) {
    draw_control_polyline(canvas, &rect_corners(transform, bounds), true, color);
}

pub fn draw_keypoint(canvas: &PreviewCanvas, position: Vec2, color: Color) {
    canvas.draw_circle(
        sk_point(position),
        KEYPOINT_RADIUS,
        &fill_paint(HANDLE_FILL_COLOR),
    );
    canvas.draw_circle(
        sk_point(position),
        KEYPOINT_RADIUS,
        &stroke_paint(color, HANDLE_STROKE_WIDTH, Cap::Butt, Join::Miter),
    );
}

pub fn draw_keypoints(canvas: &PreviewCanvas, positions: &[Vec2], color: Color) {
    for &position in positions {
        draw_keypoint(canvas, position, color);
    }
}

pub fn draw_bounds_handles(canvas: &PreviewCanvas, transform: Mat3, bounds: Rect, color: Color) {
    let [top_left, top_right, bottom_right, bottom_left] = rect_corners(transform, bounds);
    for (start, end) in [
        (top_left, top_right),
        (top_right, bottom_right),
        (bottom_right, bottom_left),
        (bottom_left, top_left),
    ] {
        draw_edge_handle(canvas, (start + end) * 0.5, end - start, color);
    }
    for corner in [top_left, top_right, bottom_right, bottom_left] {
        draw_keypoint(canvas, corner, color);
    }
}

pub fn draw_rotation_handle(canvas: &PreviewCanvas, position: Vec2) {
    canvas.draw_circle(
        sk_point(position),
        ROTATION_HANDLE_RADIUS,
        &fill_paint(selection_color()),
    );
    let scale = ROTATION_ICON_SIZE / ROTATION_ICON_VIEWBOX;
    let origin = position - ROTATION_ICON_ARC_CENTER * scale;
    icon::draw(
        canvas,
        Icon(ROTATION_ICON_NAME),
        Rect::from_min_size(origin, Vec2::splat(ROTATION_ICON_SIZE)),
        selection_foreground(),
    );
}

pub fn draw_anchor(canvas: &PreviewCanvas, position: Vec2) {
    let color = selection_color();
    let stroke = stroke_paint(color, ANCHOR_STROKE_WIDTH, Cap::Round, Join::Round);
    canvas.draw_circle(sk_point(position), ANCHOR_RING_RADIUS, &stroke);
    for direction in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
        canvas.draw_line(
            sk_point(position + direction * ANCHOR_RING_RADIUS),
            sk_point(position + direction * (ANCHOR_RING_RADIUS + ANCHOR_TICK_LENGTH)),
            &stroke,
        );
    }
    canvas.draw_circle(sk_point(position), ANCHOR_DOT_RADIUS, &fill_paint(color));
}

fn draw_edge_handle(canvas: &PreviewCanvas, position: Vec2, edge: Vec2, color: Color) {
    let length = edge.length();
    if !length.is_finite() || length <= f32::EPSILON {
        draw_keypoint(canvas, position, color);
        return;
    }
    let max_half =
        (length * EDGE_HANDLE_MARGIN_FACTOR - KEYPOINT_RADIUS).max(EDGE_HANDLE_HALF_THICKNESS);
    let half_length = EDGE_HANDLE_HALF_LENGTH.min(max_half);
    let pill = RRect::new_rect_xy(
        skia_safe::Rect::from_ltrb(
            -half_length,
            -EDGE_HANDLE_HALF_THICKNESS,
            half_length,
            EDGE_HANDLE_HALF_THICKNESS,
        ),
        EDGE_HANDLE_HALF_THICKNESS,
        EDGE_HANDLE_HALF_THICKNESS,
    );
    canvas.save();
    canvas.translate((position.x, position.y));
    canvas.rotate(edge.y.atan2(edge.x).to_degrees(), None);
    canvas.draw_rrect(pill, &fill_paint(HANDLE_FILL_COLOR));
    canvas.draw_rrect(
        pill,
        &stroke_paint(color, HANDLE_STROKE_WIDTH, Cap::Butt, Join::Miter),
    );
    canvas.restore();
}

fn control_outline_color(accent: Color) -> Color<u8> {
    let target = if accent.rec709_luma() > LIGHT_ACCENT_LUMA_THRESHOLD {
        Color::BLACK
    } else {
        Color::WHITE
    };
    Color::from_srgba(accent.lerp(target, OUTLINE_TINT_MIX).to_array())
}

fn control_outline_filter() -> Option<ImageFilter> {
    let crop = image_filters::CropRect::default();
    let blur = image_filters::blur(
        (OUTLINE_BLUR_SIGMA, OUTLINE_BLUR_SIGMA),
        TileMode::Decal,
        None,
        crop,
    )?;
    let outline = control_outline_color(selection_color());
    let alpha = std::array::from_fn(|index| {
        ((index as f32 * OUTLINE_ALPHA_GAIN).min(f32::from(u8::MAX))).round() as u8
    });
    let red = [outline.r; 256];
    let green = [outline.g; 256];
    let blue = [outline.b; 256];
    let halo = image_filters::color_filter(
        color_filters::table_argb(Some(&alpha), Some(&red), Some(&green), Some(&blue))?,
        blur,
        crop,
    )?;
    image_filters::merge([Some(halo), None], crop)
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

fn stroke_paint(color: Color, width: f32, cap: Cap, join: Join) -> SkiaPaint {
    let mut paint = fill_paint(color);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width);
    paint.set_stroke_cap(cap);
    paint.set_stroke_join(join);
    paint
}

fn sk_point(value: Vec2) -> Point {
    Point::new(value.x, value.y)
}
