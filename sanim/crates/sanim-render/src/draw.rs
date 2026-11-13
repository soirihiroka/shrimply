use sanim_core::{math::to_f64, *};
use shrimply_drawing_skia::{
    path::PreparedPath,
    shape::{self, ShapeRoundingStrategy},
};
use skia_safe::{Canvas, Paint, PathBuilder, Rect};
use std::{
    rc::{Rc, Weak as PathWeak},
    sync::{Arc, Weak},
};

const MAX_GEOMETRY_CACHE_BYTES: usize = 64 * 1024 * 1024;
// Many elements share a small number of paths (for example chart dots).
// Weak identity hints therefore have a separate, still bounded entry budget.
const MAX_GEOMETRY_CACHE_ENTRIES: usize = 1024;
const MAX_GEOMETRY_HINT_ENTRIES: usize = 8192;

pub(crate) struct GeometryHint {
    // A weak allocation guard prevents pointer reuse, without retaining scene data.
    pub(super) _geometry: Weak<Geometry>,
    pub(super) path: PathWeak<PreparedPath>,
}

/// Prepare a leaf only when it survives drawing's visibility/reveal gates.
/// The bounded weak hint avoids rehashing immutable text outlines each frame.
pub(crate) fn prepared_path(
    element: &Element,
    renderer: &mut crate::Renderer,
) -> Result<Rc<PreparedPath>, String> {
    let identity = Arc::as_ptr(&element.geometry) as usize;
    if let Some(path) = renderer
        .geometry_hints
        .get(&identity)
        .and_then(|hint| hint.path.upgrade())
    {
        renderer.geometry_key_hits += 1;
        renderer.geometry_cache_hits += 1;
        return Ok(path);
    }
    // Rectangle child state does not change its surface path.
    let key = if let Geometry::Rectangle {
        width,
        height,
        radius,
        ..
    } = element.geometry.as_ref()
    {
        digest(
            &serde_json::to_vec(&("Rectangle", width, height, radius))
                .map_err(|error| error.to_string())?,
        )
    } else {
        digest(&serde_json::to_vec(element.geometry.as_ref()).map_err(|error| error.to_string())?)
    };
    let prepared = if let Some(path) = renderer.geometry_cache.get(&key) {
        renderer.geometry_cache_hits += 1;
        path.clone()
    } else {
        let path = match element.geometry.as_ref() {
            Geometry::Circle { radius } => {
                let radius = to_f64(*radius)? as f32;
                shape::ellipse_path(
                    glam::Vec2::splat(radius * 2.),
                    0.,
                    360.,
                    0.,
                    ShapeRoundingStrategy::Circular,
                )
                .with_transform(&skia_safe::Matrix::translate((-radius, -radius)))
            }
            Geometry::Rectangle {
                width,
                height,
                radius,
                ..
            } => {
                let w = to_f64(*width)? as f32;
                let h = to_f64(*height)? as f32;
                shape::rect_path(
                    Rect::from_xywh(-w / 2., -h / 2., w, h),
                    to_f64(radius.resolve(*width, *height))? as f32,
                    ShapeRoundingStrategy::Circular,
                )
            }
            Geometry::Line { end, .. } => {
                let mut path = PathBuilder::new();
                path.move_to((0., 0.));
                path.line_to((end.values[0] as f32, end.values[1] as f32));
                path.detach()
            }
            Geometry::Polyline { points, .. } => {
                let mut path = PathBuilder::new();
                let first = points
                    .first()
                    .ok_or("Polyline requires at least two points")?;
                path.move_to((first.values[0] as f32, first.values[1] as f32));
                for point in points.iter().skip(1) {
                    path.line_to((point.values[0] as f32, point.values[1] as f32));
                }
                path.detach()
            }
            Geometry::Polygon { points } => shape::polygon_path(
                points
                    .iter()
                    .map(|p| glam::Vec2::new(p.values[0] as f32, p.values[1] as f32))
                    .collect(),
                0.,
                ShapeRoundingStrategy::Circular,
            ),
            Geometry::Text { outline, .. } => {
                let mut path = PathBuilder::new();
                let point = |p: [f64; 2]| (p[0] as f32, p[1] as f32);
                for segment in outline.iter() {
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
                path.detach()
            }
            Geometry::Group { .. } => unreachable!("groups handled before geometry preparation"),
        };
        if !path.is_finite() {
            return Err(format!(
                "geometry '{}' exceeds Skia numeric range",
                element.id
            ));
        }
        let prepared = PreparedPath::new(path)
            .map_err(|error| format!("geometry '{}': {error}", element.id))?;
        renderer.geometries_prepared += 1;
        let prepared = Rc::new(prepared);
        let bytes = prepared.approximate_bytes_used();
        if bytes <= MAX_GEOMETRY_CACHE_BYTES {
            if renderer.geometry_cache.len() >= MAX_GEOMETRY_CACHE_ENTRIES
                || renderer.geometry_cache_bytes.saturating_add(bytes) > MAX_GEOMETRY_CACHE_BYTES
            {
                renderer.geometry_cache.clear();
                renderer.geometry_hints.clear();
                renderer.geometry_cache_bytes = 0;
            }
            renderer.geometry_cache.insert(key, prepared.clone());
            renderer.geometry_cache_bytes += bytes;
        }
        prepared
    };
    // Keep admitted hot identities when a larger scene exceeds the hint budget.
    // Clearing everything here makes every subsequent frame rehash every leaf.
    if renderer.geometry_hints.len() < MAX_GEOMETRY_HINT_ENTRIES {
        renderer.geometry_hints.insert(
            identity,
            GeometryHint {
                _geometry: Arc::downgrade(&element.geometry),
                path: Rc::downgrade(&prepared),
            },
        );
    }
    Ok(prepared)
}

pub(crate) struct Drawing<'a> {
    pub renderer: &'a mut crate::Renderer,
    pub pixels: usize,
    pub work: usize,
}

pub(crate) fn element_draw(
    canvas: &Canvas,
    element: &Element,
    inherited: Reveal,
    drawing: &mut Drawing<'_>,
) -> Result<(), String> {
    drawing.work = drawing
        .work
        .checked_sub(1)
        .ok_or("render work budget exceeded")?;
    let reveal = if inherited.progress < element.reveal.progress {
        inherited
    } else {
        element.reveal
    };
    if element.opacity == Fraction::from(0)
        || element.scale == Fraction::from(0)
        || element.horizontal.scale == Fraction::from(0)
        || reveal.progress == Fraction::from(0)
    {
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
        if element.horizontal.scale != Fraction::from(1) {
            let origin = to_f64(element.horizontal.origin.value)? as f32;
            canvas.translate((origin, 0.));
            canvas.scale((to_f64(element.horizontal.scale)? as f32, 1.));
            canvas.translate((-origin, 0.));
        }
        if !canvas.local_to_device().is_finite() {
            return Err("composed transform exceeds Skia numeric range".into());
        }
        if let Geometry::Group { children } = element.geometry.as_ref() {
            if element.opacity != Fraction::from(1) {
                drawing.work = drawing
                    .work
                    .checked_sub(drawing.pixels)
                    .ok_or("group compositing exceeds render work budget")?;
                canvas.save_layer_alpha_f(None, to_f64(element.opacity)? as f32);
            }
            // A group reveals its children in parallel, retaining each child's own reveal limit.
            for child in children {
                element_draw(canvas, child, reveal, drawing)?;
            }
            return Ok(());
        }
        let prepared = prepared_path(element, drawing.renderer)?;
        drawing.work = drawing
            .work
            .checked_sub(prepared.path.count_points())
            .ok_or("path rendering exceeds render work budget")?;
        let container = element
            .geometry
            .children()
            .is_some_and(|children| !children.is_empty());
        if container && element.opacity != Fraction::from(1) {
            drawing.work = drawing
                .work
                .checked_sub(drawing.pixels)
                .ok_or("container compositing exceeds render work budget")?;
            canvas.save_layer_alpha_f(None, to_f64(element.opacity)? as f32);
        }
        let own_opacity = if container {
            Fraction::from(1)
        } else {
            element.opacity
        };
        let mut paint = fill_paint(&element.fill, own_opacity, prepared.path.bounds())?;
        let progress = to_f64(reveal.progress)? as f32;
        if let Geometry::Line { width, .. } | Geometry::Polyline { width, .. } =
            element.geometry.as_ref()
        {
            paint
                .set_style(skia_safe::paint::Style::Stroke)
                .set_stroke_width(to_f64(*width)? as f32)
                .set_stroke_join(skia_safe::PaintJoin::Round);
            canvas.draw_path(&prepared.partial(progress, false), &paint);
        } else if progress >= 1.0 {
            canvas.draw_path(&prepared.path, &paint);
        } else {
            // Traces must stay inside the original bounds used by visual verification.
            canvas.clip_rect(prepared.path.bounds(), None, true);
            if paint.alpha_f() < 1.0 {
                drawing.work = drawing
                    .work
                    .checked_sub(drawing.pixels)
                    .ok_or("reveal compositing exceeds render work budget")?;
                canvas.save_layer_alpha_f(None, paint.alpha_f());
                paint.set_alpha_f(1.0);
            }
            let (trace_path, trace_opacity) = match reveal.kind {
                RevealKind::Create => {
                    let partial = prepared.partial(progress, false);
                    drawing.work = drawing
                        .work
                        .checked_sub(partial.count_points())
                        .ok_or("path rendering exceeds render work budget")?;
                    canvas.draw_path(&partial, &paint);
                    (partial, 1.0)
                }
                RevealKind::Write => {
                    let fill_progress = (progress * 2.0 - 1.0).clamp(0.0, 1.0);
                    if fill_progress > 0.0 {
                        drawing.work = drawing
                            .work
                            .checked_sub(prepared.path.count_points())
                            .ok_or("path rendering exceeds render work budget")?;
                        let mut fill = paint.clone();
                        fill.set_alpha_f(fill.alpha_f() * fill_progress);
                        canvas.draw_path(&prepared.path, &fill);
                    }
                    (
                        prepared.partial((progress * 2.0).min(1.0), false),
                        1.0 - fill_progress,
                    )
                }
            };
            const TRACE_WIDTH_PT: f32 = 1.0;
            paint
                .set_style(skia_safe::paint::Style::Stroke)
                .set_stroke_width(TRACE_WIDTH_PT);
            paint.set_alpha_f(paint.alpha_f() * trace_opacity);
            canvas.draw_path(&trace_path, &paint);
        }
        if let Some(stroke) = &element.stroke {
            let mut border = fill_paint(&stroke.paint, own_opacity, prepared.path.bounds())?;
            border
                .set_style(skia_safe::paint::Style::Stroke)
                .set_stroke_width(to_f64(stroke.width)? as f32);
            let border_path = prepared.partial(progress, false);
            drawing.work = drawing
                .work
                .checked_sub(border_path.count_points())
                .ok_or("border rendering exceeds render work budget")?;
            canvas.draw_path(&border_path, &border);
        }
        if let Some(children) = element.geometry.children() {
            for child in children {
                element_draw(canvas, child, reveal, drawing)?;
            }
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

fn fill_paint(fill: &sanim_core::Paint, opacity: Fraction, bounds: &Rect) -> Result<Paint, String> {
    match fill {
        sanim_core::Paint::Solid(color) => paint(color, opacity),
        sanim_core::Paint::LinearGradient { angle, stops } => {
            let points = math::paint::gradient_endpoints(
                [
                    bounds.left as f64,
                    bounds.top as f64,
                    bounds.right as f64,
                    bounds.bottom as f64,
                ],
                *angle,
            )?;
            let colors = stops
                .iter()
                .map(|stop| {
                    let values = stop
                        .color
                        .0
                        .map(|channel| to_f64(channel).map(|channel| channel as f32));
                    Ok(skia_safe::Color4f::new(
                        values[0].clone()?,
                        values[1].clone()?,
                        values[2].clone()?,
                        values[3].clone()?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let offsets = stops
                .iter()
                .map(|stop| to_f64(stop.offset).map(|offset| offset as f32))
                .collect::<Result<Vec<_>, _>>()?;
            let gradient_colors = skia_safe::gradient::Colors::new(
                &colors,
                Some(&offsets),
                skia_safe::TileMode::Clamp,
                None,
            );
            let gradient = skia_safe::gradient::Gradient::new(
                gradient_colors,
                skia_safe::gradient::Interpolation::default(),
            );
            let shader = skia_safe::gradient::shaders::linear_gradient(
                (
                    (points[0][0] as f32, points[0][1] as f32),
                    (points[1][0] as f32, points[1][1] as f32),
                ),
                &gradient,
                None,
            )
            .ok_or("Skia could not create the linear gradient shader")?;
            let mut paint = paint(&Color::parse("white")?, opacity)?;
            paint.set_shader(shader);
            Ok(paint)
        }
    }
}
