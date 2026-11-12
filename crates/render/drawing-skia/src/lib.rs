pub mod path;
pub mod shape;
use std::rc::Rc;

use glam::Vec2;
use shrimply_math_color::{Color, LayerBlendMode};
use shrimply_math_geometry::{ComposedTransform2D, Rect};
use skia_safe::{
    BlendMode, ColorFilter, Data, Font, FontMgr, FontStyle, Paint as SkiaPaint, PaintStyle,
    PathBuilder, Point, RuntimeEffect, canvas::SaveLayerRec,
};

#[derive(Clone)]
pub enum CanvasOperation {
    Transform(ComposedTransform2D),
    MotionBlur(Rc<[ComposedTransform2D]>),
    Opacity(f32),
    Hsv {
        hue_turns: f32,
        saturation: f32,
        value: f32,
    },
    Repeat {
        copies_x: u32,
        copies_y: u32,
        step: Vec2,
        row_offset: Vec2,
    },
}

pub fn draw_with_operations(
    canvas: &skia_safe::Canvas,
    operations: &[CanvasOperation],
    mut draw: impl FnMut(&skia_safe::Canvas),
) {
    draw_operations(canvas, operations, &mut draw);
}

pub fn draw_composited(
    canvas: &skia_safe::Canvas,
    opacity: f32,
    blend_mode: LayerBlendMode,
    draw: impl FnOnce(&skia_safe::Canvas),
) {
    let mut paint = SkiaPaint::default();
    paint.set_alpha_f(opacity.clamp(0.0, 1.0));
    paint.set_blender(layer_blender(blend_mode));
    canvas.save_layer(&SaveLayerRec::default().paint(&paint));
    draw(canvas);
    canvas.restore();
}

fn draw_operations(
    canvas: &skia_safe::Canvas,
    operations: &[CanvasOperation],
    draw: &mut impl FnMut(&skia_safe::Canvas),
) {
    let Some((operation, preceding)) = operations.split_last() else {
        draw(canvas);
        return;
    };
    match operation {
        CanvasOperation::Transform(transform) => {
            canvas.save();
            canvas.concat(&shrimply_math_geometry::to_skia_matrix(transform.matrix));
            draw_operations(canvas, preceding, draw);
            canvas.restore();
        }
        CanvasOperation::MotionBlur(transforms) => {
            let weight = 1.0 / transforms.len().max(1) as f32;
            for transform in transforms.iter() {
                let mut paint = SkiaPaint::default();
                paint.set_alpha_f(weight);
                paint.set_blend_mode(BlendMode::Plus);
                canvas.save_layer(&SaveLayerRec::default().paint(&paint));
                canvas.concat(&shrimply_math_geometry::to_skia_matrix(transform.matrix));
                draw_operations(canvas, preceding, draw);
                canvas.restore();
            }
        }
        CanvasOperation::Opacity(opacity) => {
            canvas.save_layer_alpha_f(None, opacity.clamp(0.0, 1.0));
            draw_operations(canvas, preceding, draw);
            canvas.restore();
        }
        CanvasOperation::Hsv {
            hue_turns,
            saturation,
            value,
        } => {
            let mut paint = SkiaPaint::default();
            paint.set_color_filter(hsv_color_filter(*hue_turns, *saturation, *value));
            canvas.save_layer(&SaveLayerRec::default().paint(&paint));
            draw_operations(canvas, preceding, draw);
            canvas.restore();
        }
        CanvasOperation::Repeat {
            copies_x,
            copies_y,
            step,
            row_offset,
        } => {
            for y in 0..(*copies_y).max(1) {
                for x in 0..(*copies_x).max(1) {
                    canvas.save();
                    let offset =
                        Vec2::new(x as f32 * step.x, y as f32 * step.y) + y as f32 * *row_offset;
                    canvas.translate((offset.x, offset.y));
                    draw_operations(canvas, preceding, draw);
                    canvas.restore();
                }
            }
        }
    }
}

const HSV_SKSL: &str = include_str!("shaders/hsv.sksl");

thread_local! {
    static HSV_EFFECT: Result<RuntimeEffect, String> =
        RuntimeEffect::make_for_color_filter(HSV_SKSL, None);
}

pub fn hsv_color_filter(hue_turns: f32, saturation: f32, value: f32) -> ColorFilter {
    let mut uniforms = [0_u8; 12];
    uniforms[..4].copy_from_slice(&hue_turns.to_ne_bytes());
    uniforms[4..8].copy_from_slice(&saturation.to_ne_bytes());
    uniforms[8..].copy_from_slice(&value.to_ne_bytes());
    HSV_EFFECT.with(|effect| {
        effect
            .as_ref()
            .unwrap_or_else(|error| panic!("compile vector HSV color filter: {error}"))
            .make_color_filter(Data::new_copy(&uniforms), None)
            .expect("create vector HSV color filter from valid uniforms")
    })
}

const LAYER_BLEND_SKSL: &str = include_str!("shaders/layer_blend.sksl");

thread_local! {
    static LAYER_BLEND_EFFECT: Result<RuntimeEffect, String> =
        RuntimeEffect::make_for_blender(LAYER_BLEND_SKSL, None);
}

fn layer_blender(mode: LayerBlendMode) -> skia_safe::Blender {
    let mode = match mode {
        LayerBlendMode::PassThrough => 0_i32,
        LayerBlendMode::Normal => 1,
        LayerBlendMode::Dissolve => 2,
        LayerBlendMode::Darken => 3,
        LayerBlendMode::Multiply => 4,
        LayerBlendMode::ColorBurn => 5,
        LayerBlendMode::LinearBurn => 6,
        LayerBlendMode::DarkerColor => 7,
        LayerBlendMode::Lighten => 8,
        LayerBlendMode::Screen => 9,
        LayerBlendMode::ColorDodge => 10,
        LayerBlendMode::Add => 11,
        LayerBlendMode::LighterColor => 12,
        LayerBlendMode::Overlay => 13,
        LayerBlendMode::SoftLight => 14,
        LayerBlendMode::HardLight => 15,
        LayerBlendMode::VividLight => 16,
        LayerBlendMode::LinearLight => 17,
        LayerBlendMode::PinLight => 18,
        LayerBlendMode::HardMix => 19,
        LayerBlendMode::Difference => 20,
        LayerBlendMode::Exclusion => 21,
        LayerBlendMode::Subtract => 22,
        LayerBlendMode::Divide => 23,
        LayerBlendMode::Reflect => 28,
        LayerBlendMode::Hue => 24,
        LayerBlendMode::Saturation => 25,
        LayerBlendMode::Color => 26,
        LayerBlendMode::Luminosity => 27,
    };
    LAYER_BLEND_EFFECT.with(|effect| {
        effect
            .as_ref()
            .unwrap_or_else(|error| panic!("compile layer blender: {error}"))
            .make_blender(Data::new_copy(&mode.to_ne_bytes()), None)
            .expect("create layer blender from valid mode")
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub color: Color,
    pub width: f32,
}

impl Stroke {
    pub const fn new(color: Color, width: f32) -> Self {
        Self { color, width }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Paint {
    pub fill: Option<Color>,
    pub stroke: Option<Stroke>,
}

impl Paint {
    pub const fn fill(color: Color) -> Self {
        Self {
            fill: Some(color),
            stroke: None,
        }
    }

    pub const fn stroke(stroke: Stroke) -> Self {
        Self {
            fill: None,
            stroke: Some(stroke),
        }
    }
}

pub fn line(canvas: &skia_safe::Canvas, start: Vec2, end: Vec2, stroke: Stroke) {
    if stroke.width <= 0.0 || stroke.color.is_transparent() {
        return;
    }
    canvas.draw_line(point(start), point(end), &stroke_paint(stroke));
}

pub fn polyline(canvas: &skia_safe::Canvas, points: &[Vec2], closed: bool, paint: Paint) {
    let Some(first) = points.first() else {
        return;
    };
    let mut path = PathBuilder::new();
    path.move_to(point(*first));
    for &next in &points[1..] {
        path.line_to(point(next));
    }
    if closed {
        path.close();
    }
    let path = path.detach();
    if let Some(fill) = paint.fill {
        canvas.draw_path(&path, &fill_paint(fill));
    }
    if let Some(stroke) = paint.stroke {
        canvas.draw_path(&path, &stroke_paint(stroke));
    }
}

pub fn rect(canvas: &skia_safe::Canvas, rect: Rect, corner_radius: f32, paint: Paint) {
    let rect = skia_safe::Rect::from_xywh(rect.min.x, rect.min.y, rect.width(), rect.height());
    if let Some(fill) = paint.fill {
        canvas.draw_round_rect(rect, corner_radius, corner_radius, &fill_paint(fill));
    }
    if let Some(stroke) = paint.stroke {
        canvas.draw_round_rect(rect, corner_radius, corner_radius, &stroke_paint(stroke));
    }
}

pub fn circle(canvas: &skia_safe::Canvas, center: Vec2, radius: f32, paint: Paint) {
    if radius <= 0.0 {
        return;
    }
    if let Some(fill) = paint.fill {
        canvas.draw_circle(point(center), radius, &fill_paint(fill));
    }
    if let Some(stroke) = paint.stroke {
        canvas.draw_circle(point(center), radius, &stroke_paint(stroke));
    }
}

pub fn text(canvas: &skia_safe::Canvas, center: Vec2, text: &str, size: f32, color: Color) {
    if text.is_empty() || size <= 0.0 || color.is_transparent() {
        return;
    }
    let font = FontMgr::new()
        .legacy_make_typeface(None, FontStyle::default())
        .map(|typeface| Font::from_typeface(typeface, size))
        .unwrap_or_else(|| {
            let mut font = Font::default();
            font.set_size(size);
            font
        });
    let paint = fill_paint(color);
    let (width, bounds) = font.measure_str(text, Some(&paint));
    canvas.draw_str(
        text,
        Point::new(center.x - width * 0.5, center.y - bounds.center_y()),
        &font,
        &paint,
    );
}

fn fill_paint(color: Color) -> SkiaPaint {
    let mut paint = SkiaPaint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);
    paint.set_color(color);
    paint
}

fn stroke_paint(stroke: Stroke) -> SkiaPaint {
    let mut paint = fill_paint(stroke.color);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(stroke.width);
    paint
}

fn point(value: Vec2) -> Point {
    Point::new(value.x, value.y)
}
