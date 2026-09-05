use std::ptr::NonNull;

use skia_safe::{
    Font, Paint, PathBuilder, Point, RRect, canvas::PointMode, paint::Style as PaintStyle,
};

pub use shrimply_math_color::Color;
pub use shrimply_math_geometry::{Rect, UVec2, Vec2, vec2};

pub type TimelinePainter = TimelinePainterInner;

#[derive(Clone, Copy)]
pub struct Stroke {
    pub width: f32,
    pub color: Color,
}

impl Stroke {
    pub fn none() -> Self {
        Self {
            width: 0.0,
            color: Color::<f32>::TRANSPARENT,
        }
    }

    pub fn new(width: f32, color: Color) -> Self {
        Self { width, color }
    }
}

#[derive(Clone, Copy)]
pub enum StrokeKind {
    Inside,
}

#[derive(Clone, Copy)]
pub enum Align2 {
    LeftTop,
    CenterCenter,
}

impl Align2 {
    pub const LEFT_TOP: Self = Self::LeftTop;
    pub const CENTER_CENTER: Self = Self::CenterCenter;

    fn anchor_size(self, pos: Vec2, size: Vec2) -> Rect {
        match self {
            Self::LeftTop => Rect::from_min_size(pos, size),
            Self::CenterCenter => Rect::from_center_size(pos, size),
        }
    }
}

#[derive(Clone)]
pub struct FontId {
    size: f32,
}

impl FontId {
    pub fn proportional(size: f32) -> Self {
        Self { size }
    }
}

pub struct TimelinePainterInner {
    canvas: NonNull<skia_safe::Canvas>,
    restore_clip: bool,
}

pub trait CornerRadius {
    fn into_f64(self) -> f64;
}

impl CornerRadius for f64 {
    fn into_f64(self) -> f64 {
        self
    }
}

impl CornerRadius for f32 {
    fn into_f64(self) -> f64 {
        f64::from(self)
    }
}

impl CornerRadius for i32 {
    fn into_f64(self) -> f64 {
        f64::from(self)
    }
}

impl CornerRadius for u8 {
    fn into_f64(self) -> f64 {
        f64::from(self)
    }
}

impl Clone for TimelinePainterInner {
    fn clone(&self) -> Self {
        Self {
            canvas: self.canvas,
            restore_clip: false,
        }
    }
}

impl TimelinePainterInner {
    pub fn new(canvas: &skia_safe::Canvas) -> Self {
        Self {
            canvas: NonNull::from(canvas),
            restore_clip: false,
        }
    }

    pub fn canvas(&self) -> &skia_safe::Canvas {
        unsafe { self.canvas.as_ref() }
    }

    pub fn rect_filled(&self, rect: Rect, corner_radius: impl CornerRadius, fill: Color) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 || fill.is_transparent() {
            return;
        }
        let corner_radius = corner_radius.into_f64();

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(PaintStyle::Fill);
        paint.set_color(fill);
        self.draw_rect_shape(rect, corner_radius, &paint, false);
    }

    pub fn rect_stroke(
        &self,
        rect: Rect,
        corner_radius: impl CornerRadius,
        stroke: Stroke,
        _kind: StrokeKind,
    ) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 || stroke.width <= 0.0 {
            return;
        }
        let corner_radius = corner_radius.into_f64();

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(stroke.width);
        paint.set_color(stroke.color);
        self.draw_rect_shape(rect, corner_radius, &paint, true);
    }

    pub fn text(
        &self,
        pos: Vec2,
        align: Align2,
        text: impl AsRef<str>,
        font_id: FontId,
        color: Color,
    ) {
        self.text_rotated(pos, align, text, font_id, color, 0.0);
    }

    pub fn text_rotated(
        &self,
        pos: Vec2,
        align: Align2,
        text: impl AsRef<str>,
        font_id: FontId,
        color: Color,
        rotation_degrees: f32,
    ) {
        let text = text.as_ref();
        if text.is_empty() {
            return;
        }

        let font = skia_font(font_id.clone());
        let size = self.layout_no_wrap(text, font_id, color).size();
        if size.x <= 0.0 {
            return;
        }

        let (_, metrics) = font.metrics();
        let baseline_offset = -metrics.ascent;
        let anchored = align.anchor_size(pos, size);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(color);
        self.canvas().save();
        self.canvas()
            .rotate(rotation_degrees, Some((pos.x, pos.y).into()));
        self.canvas().draw_str(
            text,
            (anchored.min.x, anchored.min.y + baseline_offset),
            &font,
            &paint,
        );
        self.canvas().restore();
    }

    pub fn system_text(&self, pos: Vec2, text: impl AsRef<str>, font_id: FontId, color: Color) {
        let text = text.as_ref();
        if text.is_empty() {
            return;
        }

        crate::skia_system_font::paragraph(text, font_id.size, color)
            .paint(self.canvas(), point(pos));
    }

    pub fn system_text_ellipsized(
        &self,
        pos: Vec2,
        text: impl AsRef<str>,
        font_id: FontId,
        color: Color,
        max_width: f32,
    ) {
        let text = text.as_ref();
        if text.is_empty() || max_width <= 0.0 {
            return;
        }

        crate::skia_system_font::ellipsized_paragraph(text, font_id.size, color, max_width)
            .paint(self.canvas(), point(pos));
    }

    pub fn line_segment(&self, segment: [Vec2; 2], stroke: Stroke) {
        if stroke.width <= 0.0 || stroke.color.is_transparent() {
            return;
        }
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(stroke.width);
        paint.set_color(stroke.color);
        self.canvas()
            .draw_line(point(segment[0]), point(segment[1]), &paint);
    }

    pub fn line_segments(&self, segments: &[Vec2], stroke: Stroke) {
        if segments.len() < 2 || stroke.width <= 0.0 || stroke.color.is_transparent() {
            return;
        }
        let points = segments.iter().copied().map(point).collect::<Vec<_>>();
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_stroke_width(stroke.width);
        paint.set_color(stroke.color);
        self.canvas().draw_points(PointMode::Lines, &points, &paint);
    }

    pub fn circle_filled(&self, center: Vec2, radius: f32, fill: Color) {
        if radius <= 0.0 || fill.is_transparent() {
            return;
        }

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(PaintStyle::Fill);
        paint.set_color(fill);
        self.canvas()
            .draw_circle((center.x, center.y), radius, &paint);
    }

    pub fn circle_stroke(&self, center: Vec2, radius: f32, stroke: Stroke) {
        if radius <= 0.0 || stroke.width <= 0.0 || stroke.color.is_transparent() {
            return;
        }

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(stroke.width);
        paint.set_color(stroke.color);
        self.canvas()
            .draw_circle((center.x, center.y), radius, &paint);
    }

    pub fn convex_polygon(&self, points: &[Vec2], fill: Color, stroke: Stroke) {
        if points.is_empty() {
            return;
        }

        let mut path_builder = PathBuilder::new();
        let first = points[0];
        path_builder.move_to(point(first));
        for next in points.iter().skip(1).copied() {
            path_builder.line_to(point(next));
        }
        path_builder.close();
        let path = path_builder.snapshot();

        if !fill.is_transparent() {
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_style(PaintStyle::Fill);
            paint.set_color(fill);
            self.canvas().draw_path(&path, &paint);
        }
        if stroke.width > 0.0 && !stroke.color.is_transparent() {
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_style(PaintStyle::Stroke);
            paint.set_stroke_width(stroke.width);
            paint.set_color(stroke.color);
            self.canvas().draw_path(&path, &paint);
        }
    }

    pub fn layout_no_wrap(&self, text: &str, font_id: FontId, _color: Color) -> TimelineTextLayout {
        if text.is_empty() {
            return TimelineTextLayout::zero();
        }

        let fallback_height = font_id.size.max(1.0);
        let font = skia_font(font_id);
        let (width, bounds) = font.measure_str(text, None);
        TimelineTextLayout::new(width, bounds.height().max(fallback_height))
    }

    pub fn layout_system_text_no_wrap(
        &self,
        text: &str,
        font_id: FontId,
        color: Color,
    ) -> TimelineTextLayout {
        if text.is_empty() {
            return TimelineTextLayout::zero();
        }

        let paragraph = crate::skia_system_font::paragraph(text, font_id.size, color);
        TimelineTextLayout::new(paragraph.longest_line(), paragraph.height())
    }

    pub fn path_filled(&self, path: &skia_safe::Path, fill: Color) {
        if fill.is_transparent() {
            return;
        }

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(PaintStyle::Fill);
        paint.set_color(fill);
        self.canvas().draw_path(path, &paint);
    }

    pub fn path_stroke(&self, path: &skia_safe::Path, stroke: Stroke) {
        if stroke.width <= 0.0 || stroke.color.is_transparent() {
            return;
        }

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(stroke.width);
        paint.set_color(stroke.color);
        self.canvas().draw_path(path, &paint);
    }

    pub fn with_clip_rect(&self, rect: Rect) -> Self {
        let mut painter = self.clone();
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return painter;
        }

        painter.canvas().save();
        painter.canvas().clip_rect(
            skia_safe::Rect::from(rect),
            skia_safe::ClipOp::Intersect,
            true,
        );
        painter.restore_clip = true;
        painter
    }

    fn draw_rect_shape(&self, rect: Rect, corner_radius: f64, paint: &Paint, _stroke: bool) {
        let sk_rect = skia_safe::Rect::from(rect);
        if corner_radius > 0.0 {
            let corner_radius = corner_radius.max(0.0) as f32;
            let rounded = RRect::new_rect_xy(sk_rect, corner_radius, corner_radius);
            self.canvas().draw_rrect(rounded, paint);
            return;
        }
        self.canvas().draw_rect(sk_rect, paint);
    }
}

impl Drop for TimelinePainterInner {
    fn drop(&mut self) {
        if self.restore_clip {
            self.canvas().restore();
        }
    }
}

pub struct TimelineTextLayout {
    size: Vec2,
}

impl TimelineTextLayout {
    fn new(width: f32, height: f32) -> Self {
        Self {
            size: Vec2::new(width.max(0.0), height.max(0.0)),
        }
    }

    fn zero() -> Self {
        Self { size: Vec2::ZERO }
    }

    pub fn size(&self) -> Vec2 {
        self.size
    }
}

fn point(pos: Vec2) -> Point {
    Point::new(pos.x, pos.y)
}

fn skia_font(font_id: FontId) -> Font {
    crate::skia_font::font_with_families(&[], 400.0, font_id.size)
}
