use sanim_core::{math::to_f64, *};
use shrimply_drawing_skia::shape::{self, ShapeRoundingStrategy};
use skia_safe::{Canvas, Paint, PathBuilder, Rect};

pub(crate) fn element_draw(
    canvas: &Canvas,
    element: &Element,
    pixels: usize,
    work: &mut usize,
) -> Result<(), String> {
    *work = work.checked_sub(1).ok_or("render work budget exceeded")?;
    if element.opacity == Fraction::from(0) {
        return Ok(());
    }
    let save = canvas.save();
    let result = (|| {
        canvas.translate((
            element.position.values[0] as f32,
            element.position.values[1] as f32,
        ));
        canvas.rotate(to_f64(element.rotation)? as f32, None);
        let scale = to_f64(element.scale)? as f32;
        canvas.scale((scale, scale));
        if !canvas.local_to_device().is_finite() {
            return Err("composed transform exceeds Skia numeric range".into());
        }
        if let Geometry::Group { children } = &element.geometry {
            if element.opacity != Fraction::from(1) {
                *work = work
                    .checked_sub(pixels)
                    .ok_or("group compositing exceeds render work budget")?;
                canvas.save_layer_alpha_f(None, to_f64(element.opacity)? as f32);
            }
            for child in children {
                element_draw(canvas, child, pixels, work)?;
            }
            return Ok(());
        }
        let mut paint = paint(&element.fill, element.opacity)?;
        match &element.geometry {
            Geometry::Circle { radius } => {
                let radius = to_f64(*radius)? as f32;
                let path = shape::ellipse_path(
                    glam::Vec2::splat(radius * 2.),
                    0.,
                    360.,
                    0.,
                    ShapeRoundingStrategy::Circular,
                )
                .with_transform(&skia_safe::Matrix::translate((-radius, -radius)));
                canvas.draw_path(&path, &paint);
            }
            Geometry::Rectangle { width, height } => {
                let w = to_f64(*width)? as f32;
                let h = to_f64(*height)? as f32;
                canvas.draw_path(
                    &shape::rect_path(
                        Rect::from_xywh(-w / 2., -h / 2., w, h),
                        0.,
                        ShapeRoundingStrategy::Circular,
                    ),
                    &paint,
                );
            }
            Geometry::Line { end, width } => {
                paint
                    .set_style(skia_safe::paint::Style::Stroke)
                    .set_stroke_width(to_f64(*width)? as f32);
                canvas.draw_line(
                    (0., 0.),
                    (end.values[0] as f32, end.values[1] as f32),
                    &paint,
                );
            }
            Geometry::Polygon { points } => {
                let path = shape::polygon_path(
                    points
                        .iter()
                        .map(|p| glam::Vec2::new(p.values[0] as f32, p.values[1] as f32))
                        .collect(),
                    0.,
                    ShapeRoundingStrategy::Circular,
                );
                if !path.is_finite() {
                    return Err("polygon exceeds Skia numeric range".into());
                }
                canvas.draw_path(&path, &paint);
            }
            Geometry::Text { outline, .. } => {
                *work = work
                    .checked_sub(outline.len())
                    .ok_or("text rendering exceeds render work budget")?;
                let mut path = PathBuilder::new();
                let point = |p: [f64; 2]| (p[0] as f32, p[1] as f32);
                for segment in outline {
                    match segment {
                        TextSegment::Move(p) => {
                            path.move_to(point(*p));
                        }
                        TextSegment::Line(p) => {
                            path.line_to(point(*p));
                        }
                        TextSegment::Curve(a, b, c) => {
                            path.cubic_to(point(*a), point(*b), point(*c));
                        }
                        TextSegment::Close => {
                            path.close();
                        }
                    }
                }
                if !path.is_finite() {
                    return Err("text outline exceeds Skia numeric range".into());
                }
                canvas.draw_path(&path.detach(), &paint);
            }
            Geometry::Group { .. } => unreachable!("group handled before primitive drawing"),
        }
        Ok(())
    })();
    canvas.restore_to_count(save);
    result
}
pub(crate) fn paint(color: &Color, opacity: Fraction) -> Result<Paint, String> {
    let mut channels = [0.; 4];
    for (i, value) in color.0.iter().enumerate() {
        let value = to_f64(*value)?;
        if !(0.0..=1.0).contains(&value) {
            return Err("color channel outside [0,1]".into());
        }
        channels[i] = value as f32;
    }
    channels[3] *= to_f64(opacity)? as f32;
    Ok(shape::paint(
        skia_safe::Color4f::new(channels[0], channels[1], channels[2], channels[3]),
        1.,
    ))
}
