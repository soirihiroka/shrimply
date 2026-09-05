use crate::generated::{GeneratedTransition, GeneratedVisual};
use shrimply_core::timeline_value::*;
use shrimply_evaluation::{TransformEvaluation, TransformExpressionCache, VisualEvaluation};
use shrimply_project::project::{CanvasSize, Color, ShapeItem, ShapeKind, ShapeRoundingStrategy};
use skia_safe::{
    BlendMode, BlurStyle, Canvas, Color as SkiaColor, MaskFilter, Paint, PaintStyle, Path,
    PathBuilder, PathDirection, Point, Rect, canvas::SaveLayerRec,
};

pub fn prepare(
    canvas_size: CanvasSize,
    surface_size: CanvasSize,
    item: &shrimply_project::project::VideoItem,
    evaluation: VisualEvaluation,
    transition: Option<GeneratedTransition>,
    expressions: &mut TransformExpressionCache,
) -> PreparedShape {
    let shrimply_project::project::VideoItemContent::Shape(shape) = &item.content else {
        panic!("shape preparation received non-shape item")
    };
    let rotation_degrees = shrimply_evaluation::resolve_scalar(
        &item.transform.rotation_degrees,
        &evaluation,
        expressions,
    );
    let mut decoration = decoration_outset(shape, &evaluation, expressions, rotation_degrees);
    if transition.is_some_and(|transition| {
        matches!(
            transition.kind,
            shrimply_project::project::VisualTransitionKind::Write
                | shrimply_project::project::VisualTransitionKind::Create
                | shrimply_project::project::VisualTransitionKind::FacetAssembly
                | shrimply_project::project::VisualTransitionKind::Coalesce
                | shrimply_project::project::VisualTransitionKind::ContourCurrent
                | shrimply_project::project::VisualTransitionKind::SoftRefraction
                | shrimply_project::project::VisualTransitionKind::MorphologicalResolve
                | shrimply_project::project::VisualTransitionKind::LivingFill
                | shrimply_project::project::VisualTransitionKind::Diffusion
                | shrimply_project::project::VisualTransitionKind::ReverseDiffusion
        )
    }) {
        let canvas_extent = canvas_size.width.max(canvas_size.height).max(1) as f32;
        let trace_outset = match transition.map(|value| value.kind) {
            Some(shrimply_project::project::VisualTransitionKind::FacetAssembly) => {
                canvas_extent * 0.3
            }
            Some(
                shrimply_project::project::VisualTransitionKind::Coalesce
                | shrimply_project::project::VisualTransitionKind::SoftRefraction
                | shrimply_project::project::VisualTransitionKind::MorphologicalResolve
                | shrimply_project::project::VisualTransitionKind::Diffusion
                | shrimply_project::project::VisualTransitionKind::ReverseDiffusion,
            ) => canvas_extent * 0.05,
            _ => canvas_size.height.max(1) as f32 / 1080.0,
        };
        for edge in &mut decoration {
            *edge = edge.max(trace_outset);
        }
    }
    let decoration_offset = glam::Vec2::new(decoration[3], decoration[0]);
    PreparedShape {
        canvas_size,
        surface_size,
        content_offset: decoration_offset,
        // Resolve before the frame's audio-readiness gate. Drawing must not
        // discover new asynchronous mouth-analysis requests after acceptance.
        shape: resolve_shape_draw(shape, rotation_degrees, &evaluation, expressions),
        evaluation,
        transition,
    }
}

pub fn decoration_outset(
    shape: &ShapeItem,
    evaluation: &TransformEvaluation,
    expressions: &mut TransformExpressionCache,
    rotation_degrees: f32,
) -> [f32; 4] {
    let outline_width = number_value(&shape.outline_width, evaluation, expressions).max(0.0);
    let shadow_distance = number_value(&shape.shadow_distance, evaluation, expressions).max(0.0);
    let shadow_direction = number_value(&shape.shadow_direction_degrees, evaluation, expressions);
    let shadow_width = number_value(&shape.shadow_width, evaluation, expressions).max(0.0);
    let shadow_blur = number_value(&shape.shadow_blur, evaluation, expressions).max(0.0);
    let shadow_offset = shrimply_math_media::rotate_degrees(
        shrimply_math_media::polar_degrees(shadow_distance, shadow_direction),
        -rotation_degrees,
    );
    shrimply_math_media::decoration_outset(outline_width, shadow_offset, shadow_width, shadow_blur)
}

pub struct PreparedShape {
    pub canvas_size: CanvasSize,
    pub surface_size: CanvasSize,
    pub content_offset: glam::Vec2,
    shape: ShapeDraw,
    pub evaluation: shrimply_evaluation::VisualEvaluation,
    transition: Option<GeneratedTransition>,
}

impl GeneratedVisual for PreparedShape {
    fn draw(
        &self,
        canvas: &Canvas,
        _evaluation: &TransformEvaluation,
        _expressions: &mut TransformExpressionCache,
        path_effect: Option<&skia_safe::PathEffect>,
    ) {
        canvas.save();
        canvas.translate((self.content_offset.x, self.content_offset.y));
        draw_shape(
            canvas,
            &self.shape,
            1.0,
            self.transition,
            self.canvas_size.height.max(1) as f32 * (2.0 / 1080.0),
            path_effect,
        );
        canvas.restore();
    }
}

impl PreparedShape {
    pub fn morph_scene(&self) -> Option<crate::vector_morph::MorphScene> {
        let shape = &self.shape;
        let path = shape_path(shape, Rect::from_xywh(0.0, 0.0, shape.size.x, shape.size.y))
            .with_transform(&skia_safe::Matrix::translate((
                self.content_offset.x,
                self.content_offset.y,
            )));
        let mut appearance = Vec::new();
        if shape.shadow_offset.length_squared() > f32::EPSILON
            || shape.shadow_width > 0.0
            || shape.shadow_blur > 0.0
        {
            let mut paint = fill_paint(shape.shadow_color, 1.0);
            if shape.shadow_width > 0.0 {
                paint.set_style(PaintStyle::StrokeAndFill);
                paint.set_stroke_width(shape.shadow_width);
            }
            if shape.shadow_blur > 0.0 {
                paint.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, shape.shadow_blur, true));
            }
            appearance.push(crate::vector_morph::MorphPaintLayer {
                paint,
                offset: shape.shadow_offset,
            });
        }
        appearance.push(crate::vector_morph::MorphPaintLayer {
            paint: fill_paint(shape.fill, 1.0),
            offset: glam::Vec2::ZERO,
        });
        if shape.outline_width > 0.0 {
            let mut paint = fill_paint(shape.outline_color, 1.0);
            paint.set_style(PaintStyle::Stroke);
            paint.set_stroke_width(shape.outline_width);
            appearance.push(crate::vector_morph::MorphPaintLayer {
                paint,
                offset: glam::Vec2::ZERO,
            });
        }
        Some(crate::vector_morph::MorphScene {
            objects: vec![crate::vector_morph::MorphObject {
                path: crate::vector_morph::skia_path_to_morph(&path),
                appearance,
            }],
            evaluation: self.evaluation.clone(),
            canvas_size: self.canvas_size,
        })
    }
}

#[derive(Clone, Copy)]
struct ShapeDraw {
    shape: ShapeKind,
    size: glam::Vec2,
    star_points: u32,
    star_inner_radius: f32,
    arrow_shaft_width: f32,
    arrow_head_length: f32,
    cross_arm_thickness: f32,
    ellipse_inner_radius: f32,
    ellipse_completion_degrees: f32,
    fill: Color<u8>,
    outline_color: Color<u8>,
    outline_width: f32,
    corner_radius: f32,
    rounding_strategy: ShapeRoundingStrategy,
    shadow_color: Color<u8>,
    shadow_offset: glam::Vec2,
    shadow_width: f32,
    shadow_blur: f32,
}

fn draw_shape(
    canvas: &Canvas,
    shape: &ShapeDraw,
    opacity: f32,
    transition: Option<GeneratedTransition>,
    fallback_trace_width: f32,
    path_effect: Option<&skia_safe::PathEffect>,
) {
    let rect = Rect::from_xywh(0.0, 0.0, shape.size.x, shape.size.y);
    let affected = path_effect.map(|effect| {
        crate::shaky_path::apply(
            &shape_path(shape, rect),
            effect,
            rect.with_outset((shape.size.x, shape.size.y)),
        )
    });
    if let Some(transition) = transition {
        draw_shape_transition(
            canvas,
            shape,
            rect,
            opacity,
            transition,
            fallback_trace_width,
            affected.as_ref(),
        );
        return;
    }
    if let Some(path) = affected {
        draw_shape_path(canvas, shape, &path, opacity);
        return;
    }
    draw_shape_path(canvas, shape, &shape_path(shape, rect), opacity);
}

fn resolve_shape_draw(
    shape_item: &ShapeItem,
    rotation: f32,
    eval: &TransformEvaluation,
    expression_cache: &mut TransformExpressionCache,
) -> ShapeDraw {
    ShapeDraw {
        shape: shape_item.shape.value_at(eval.local_time()),
        size: vec2_value(&shape_item.size, eval, expression_cache).max(glam::Vec2::splat(1.0)),
        star_points: shrimply_evaluation::resolve(&shape_item.star_points, eval, expression_cache)
            .clamp(3, 32),
        star_inner_radius: number_value(
            &shape_item.star_inner_radius_percent,
            eval,
            expression_cache,
        )
        .clamp(5.0, 95.0)
            / 100.0,
        arrow_shaft_width: number_value(
            &shape_item.arrow_shaft_width_percent,
            eval,
            expression_cache,
        )
        .clamp(5.0, 95.0)
            / 100.0,
        arrow_head_length: number_value(
            &shape_item.arrow_head_length_percent,
            eval,
            expression_cache,
        )
        .clamp(5.0, 95.0)
            / 100.0,
        cross_arm_thickness: number_value(
            &shape_item.cross_arm_thickness_percent,
            eval,
            expression_cache,
        )
        .clamp(5.0, 95.0)
            / 100.0,
        ellipse_inner_radius: number_value(
            &shape_item.ellipse_inner_radius_percent,
            eval,
            expression_cache,
        )
        .clamp(0.0, 95.0)
            / 100.0,
        ellipse_completion_degrees: number_value(
            &shape_item.ellipse_completion_degrees,
            eval,
            expression_cache,
        )
        .clamp(0.0, 360.0),
        fill: shrimply_evaluation::resolve_color(&shape_item.fill, eval, expression_cache),
        outline_color: shrimply_evaluation::resolve_color(
            &shape_item.outline_color,
            eval,
            expression_cache,
        ),
        outline_width: number_value(&shape_item.outline_width, eval, expression_cache).max(0.0),
        corner_radius: number_value(&shape_item.corner_radius, eval, expression_cache).max(0.0),
        rounding_strategy: shape_item.rounding_strategy.value_at(eval.local_time()),
        shadow_color: shrimply_evaluation::resolve_color(
            &shape_item.shadow_color,
            eval,
            expression_cache,
        ),
        shadow_offset: shrimply_math_media::rotate_degrees(
            shrimply_math_media::polar_degrees(
                number_value(&shape_item.shadow_distance, eval, expression_cache).max(0.0),
                number_value(&shape_item.shadow_direction_degrees, eval, expression_cache),
            ),
            -rotation,
        ),
        shadow_width: number_value(&shape_item.shadow_width, eval, expression_cache).max(0.0),
        shadow_blur: number_value(&shape_item.shadow_blur, eval, expression_cache).max(0.0),
    }
}

fn shape_path(shape: &ShapeDraw, rect: Rect) -> Path {
    match shape.shape {
        ShapeKind::Rect => match shape.rounding_strategy {
            ShapeRoundingStrategy::Circular => Path::rrect(
                skia_safe::RRect::new_rect_xy(
                    rect,
                    shape.corner_radius.max(0.0),
                    shape.corner_radius.max(0.0),
                ),
                None,
            ),
            ShapeRoundingStrategy::Continuous | ShapeRoundingStrategy::Chamfer => {
                rect_path(rect, shape.corner_radius, shape.rounding_strategy)
            }
        },
        ShapeKind::Ellipse => ellipse_path(shape, shape.ellipse_completion_degrees),
        ShapeKind::Triangle => triangle_path(rect, shape.corner_radius, shape.rounding_strategy),
        ShapeKind::Star => polygon_path(
            shrimply_math_media::fit_vertices(
                shrimply_math_media::star_vertices(
                    shape.star_points,
                    shape.star_inner_radius,
                    -std::f32::consts::FRAC_PI_2,
                ),
                glam::Vec2::ZERO,
                shape.size,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Arrow => polygon_path(
            shrimply_math_media::arrow_vertices(
                shape.size,
                shape.arrow_shaft_width,
                shape.arrow_head_length,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Diamond => polygon_path(
            shrimply_math_media::fit_vertices(
                shrimply_math_media::regular_polygon_vertices(4, -std::f32::consts::FRAC_PI_2),
                glam::Vec2::ZERO,
                shape.size,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Pentagon => polygon_path(
            shrimply_math_media::fit_vertices(
                shrimply_math_media::regular_polygon_vertices(5, -std::f32::consts::FRAC_PI_2),
                glam::Vec2::ZERO,
                shape.size,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Hexagon => polygon_path(
            shrimply_math_media::fit_vertices(
                shrimply_math_media::regular_polygon_vertices(6, 0.0),
                glam::Vec2::ZERO,
                shape.size,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Heart => heart_path(rect),
        ShapeKind::Octagon => polygon_path(
            shrimply_math_media::fit_vertices(
                shrimply_math_media::regular_polygon_vertices(8, std::f32::consts::FRAC_PI_8),
                glam::Vec2::ZERO,
                shape.size,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Cross => polygon_path(
            shrimply_math_media::cross_vertices(shape.size, shape.cross_arm_thickness),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
    }
}

fn ellipse_path(shape: &ShapeDraw, completion_degrees: f32) -> Path {
    let Some(segment) = shrimply_math_media::ellipse_segment(shape.size, completion_degrees) else {
        return Path::new();
    };
    let center = Point::new(segment.center.x, segment.center.y);
    let radius = Point::new(segment.radius.x, segment.radius.y);
    let oval = Rect::from_xywh(
        center.x - radius.x,
        center.y - radius.y,
        radius.x * 2.0,
        radius.y * 2.0,
    );
    if segment.sweep_radians >= std::f32::consts::TAU - f32::EPSILON {
        let mut builder = PathBuilder::new();
        builder.add_oval(oval, PathDirection::CW, None);
        if shape.ellipse_inner_radius > f32::EPSILON {
            let inner = Rect::from_xywh(
                center.x - radius.x * shape.ellipse_inner_radius,
                center.y - radius.y * shape.ellipse_inner_radius,
                radius.x * shape.ellipse_inner_radius * 2.0,
                radius.y * shape.ellipse_inner_radius * 2.0,
            );
            builder.add_oval(inner, PathDirection::CCW, None);
        }
        return builder.detach();
    }
    let start_degrees = segment.start_radians.to_degrees();
    let sweep_degrees = segment.sweep_radians.to_degrees();
    let point = |angle: f32, scale: f32| {
        Point::new(
            center.x + angle.cos() * radius.x * scale,
            center.y + angle.sin() * radius.y * scale,
        )
    };
    let start = segment.start_radians;
    let end = start + segment.sweep_radians;
    let start_radius_length = glam::Vec2::new(
        start.cos() * segment.radius.x,
        start.sin() * segment.radius.y,
    )
    .length();
    let end_radius_length =
        glam::Vec2::new(end.cos() * segment.radius.x, end.sin() * segment.radius.y).length();
    let radial_length =
        start_radius_length.min(end_radius_length) * (1.0 - shape.ellipse_inner_radius);
    let trim = shape.corner_radius.min(radial_length * 0.45);
    let outer_speed = (glam::Vec2::new(
        start.sin() * segment.radius.x,
        start.cos() * segment.radius.y,
    )
    .length()
        + glam::Vec2::new(end.sin() * segment.radius.x, end.cos() * segment.radius.y).length())
        * 0.5;
    let outer_trim_angle = (trim / outer_speed.max(f32::EPSILON)).min(segment.sweep_radians * 0.45);
    let start_radial_trim = trim / start_radius_length.max(f32::EPSILON);
    let end_radial_trim = trim / end_radius_length.max(f32::EPSILON);
    let mut builder = PathBuilder::new();
    if trim > f32::EPSILON {
        builder.move_to(point(start + outer_trim_angle, 1.0));
        builder.arc_to(
            oval,
            start_degrees + outer_trim_angle.to_degrees(),
            sweep_degrees - outer_trim_angle.to_degrees() * 2.0,
            false,
        );
        rounded_ellipse_corner(
            &mut builder,
            point(end - outer_trim_angle, 1.0),
            point(end, 1.0),
            point(end, 1.0 - end_radial_trim),
            shape.rounding_strategy,
        );
    } else {
        builder.arc_to(oval, start_degrees, sweep_degrees, true);
    }
    if shape.ellipse_inner_radius > f32::EPSILON {
        let inner_radius = Point::new(
            radius.x * shape.ellipse_inner_radius,
            radius.y * shape.ellipse_inner_radius,
        );
        let inner_oval = Rect::from_xywh(
            center.x - inner_radius.x,
            center.y - inner_radius.y,
            inner_radius.x * 2.0,
            inner_radius.y * 2.0,
        );
        if trim > f32::EPSILON {
            let inner_trim = trim.min(
                segment.radius.length() * shape.ellipse_inner_radius * segment.sweep_radians * 0.45,
            );
            let inner_speed =
                (glam::Vec2::new(start.sin() * inner_radius.x, start.cos() * inner_radius.y)
                    .length()
                    + glam::Vec2::new(end.sin() * inner_radius.x, end.cos() * inner_radius.y)
                        .length())
                    * 0.5;
            let inner_trim_angle =
                (inner_trim / inner_speed.max(f32::EPSILON)).min(segment.sweep_radians * 0.45);
            builder.line_to(point(end, shape.ellipse_inner_radius + end_radial_trim));
            rounded_ellipse_corner(
                &mut builder,
                point(end, shape.ellipse_inner_radius + end_radial_trim),
                point(end, shape.ellipse_inner_radius),
                point(end - inner_trim_angle, shape.ellipse_inner_radius),
                shape.rounding_strategy,
            );
            builder.arc_to(
                inner_oval,
                end.to_degrees() - inner_trim_angle.to_degrees(),
                -sweep_degrees + inner_trim_angle.to_degrees() * 2.0,
                false,
            );
            rounded_ellipse_corner(
                &mut builder,
                point(start + inner_trim_angle, shape.ellipse_inner_radius),
                point(start, shape.ellipse_inner_radius),
                point(start, shape.ellipse_inner_radius + start_radial_trim),
                shape.rounding_strategy,
            );
            builder.line_to(point(start, 1.0 - start_radial_trim));
            rounded_ellipse_corner(
                &mut builder,
                point(start, 1.0 - start_radial_trim),
                point(start, 1.0),
                point(start + outer_trim_angle, 1.0),
                shape.rounding_strategy,
            );
        } else {
            builder.arc_to(
                inner_oval,
                start_degrees + sweep_degrees,
                -sweep_degrees,
                false,
            );
        }
    } else if trim > f32::EPSILON {
        let center_trim = trim.min(start_radius_length.min(end_radius_length) * 0.45);
        builder.line_to(point(
            end,
            center_trim / end_radius_length.max(f32::EPSILON),
        ));
        rounded_ellipse_corner(
            &mut builder,
            point(end, center_trim / end_radius_length.max(f32::EPSILON)),
            center,
            point(start, center_trim / start_radius_length.max(f32::EPSILON)),
            shape.rounding_strategy,
        );
        builder.line_to(point(start, 1.0 - start_radial_trim));
        rounded_ellipse_corner(
            &mut builder,
            point(start, 1.0 - start_radial_trim),
            point(start, 1.0),
            point(start + outer_trim_angle, 1.0),
            shape.rounding_strategy,
        );
    } else {
        builder.line_to(center);
    }
    builder.close();
    builder.detach()
}

fn rounded_ellipse_corner(
    builder: &mut PathBuilder,
    entry: Point,
    corner: Point,
    exit: Point,
    strategy: ShapeRoundingStrategy,
) {
    match strategy {
        ShapeRoundingStrategy::Continuous => {
            builder.quad_to(corner, exit);
        }
        ShapeRoundingStrategy::Circular => {
            builder.conic_to(
                corner,
                exit,
                shrimply_math_media::corner_conic_weight(
                    glam::Vec2::new(entry.x - corner.x, entry.y - corner.y),
                    glam::Vec2::new(exit.x - corner.x, exit.y - corner.y),
                ),
            );
        }
        ShapeRoundingStrategy::Chamfer => {
            builder.line_to(exit);
        }
    }
}

fn draw_shape_transition(
    canvas: &Canvas,
    shape: &ShapeDraw,
    rect: Rect,
    opacity: f32,
    transition: GeneratedTransition,
    fallback_trace_width: f32,
    affected: Option<&Path>,
) {
    let original;
    let full = match affected {
        Some(path) => path,
        None => {
            original = shape_path(shape, rect);
            &original
        }
    };
    let progress = crate::path_transition::submobject_progress(transition, 0, 1);
    match transition.kind {
        shrimply_project::project::VisualTransitionKind::Write => {
            let trace_color = if shape.outline_width > 0.0 {
                shape.outline_color
            } else {
                shape.fill
            };
            if progress < 0.5 {
                let partial = crate::path_transition::partial_path(
                    full,
                    progress * 2.0,
                    transition.side == shrimply_project::project::TransitionSide::Outro,
                );
                let mut trace = fill_paint(trace_color, opacity);
                trace.set_style(PaintStyle::Stroke);
                trace.set_stroke_width(fallback_trace_width.max(1.0));
                canvas.draw_path(&partial, &trace);
                return;
            }

            let style_progress = (progress * 2.0 - 1.0).clamp(0.0, 1.0);
            if shape.shadow_offset.length_squared() > f32::EPSILON
                || shape.shadow_width > 0.0
                || shape.shadow_blur > 0.0
            {
                let mut shadow = fill_paint(shape.shadow_color, opacity * style_progress);
                if shape.shadow_width > 0.0 {
                    shadow.set_style(PaintStyle::StrokeAndFill);
                    shadow.set_stroke_width(shape.shadow_width);
                }
                if shape.shadow_blur > 0.0 {
                    shadow.set_mask_filter(MaskFilter::blur(
                        BlurStyle::Normal,
                        shape.shadow_blur,
                        true,
                    ));
                }
                canvas.save();
                canvas.translate((shape.shadow_offset.x, shape.shadow_offset.y));
                canvas.draw_path(full, &shadow);
                canvas.restore();
            }
            canvas.draw_path(full, &fill_paint(shape.fill, opacity * style_progress));
            let final_stroke_color = if shape.outline_width > 0.0 {
                shape.outline_color
            } else {
                trace_color
            };
            let mut stroke = fill_paint(
                trace_color.mix_oklaba(final_stroke_color, f64::from(style_progress)),
                opacity,
            );
            stroke.set_style(PaintStyle::Stroke);
            stroke.set_stroke_width(shrimply_math_media::lerp(
                fallback_trace_width.max(1.0),
                shape.outline_width,
                style_progress,
            ));
            if stroke.stroke_width() > 0.0 {
                canvas.draw_path(full, &stroke);
            }
        }
        shrimply_project::project::VisualTransitionKind::Create => {
            let partial = crate::path_transition::partial_path(full, progress, false);
            draw_shape_path(canvas, shape, &partial, opacity);
        }
        shrimply_project::project::VisualTransitionKind::FacetAssembly => {
            const FACETS: usize = 7;
            let extent = shape.size.max_element();
            let center = Point::new(rect.center_x(), rect.center_y());
            for facet in 0..FACETS {
                let (offset, rotation, scale, facet_opacity) =
                    shrimply_math_media::facet_transform(progress, facet, FACETS, extent);
                let clip = crate::path_transition::facet_clip(rect, facet, FACETS);
                canvas.save();
                canvas.translate((offset.x, offset.y));
                canvas.rotate(rotation, Some(center));
                canvas.translate((center.x, center.y));
                canvas.scale((scale, scale));
                canvas.translate((-center.x, -center.y));
                canvas.clip_path(&clip, None, true);
                draw_shape_path(canvas, shape, full, opacity * facet_opacity);
                canvas.restore();
            }

            let finished_opacity = ((progress - 0.82) / 0.18).clamp(0.0, 1.0);
            draw_shape_path(canvas, shape, full, opacity * finished_opacity);

            if let Some(glint) = crate::path_transition::facet_glint_clip(rect, progress) {
                canvas.save();
                canvas.clip_path(&glint, None, true);
                let glint_progress = ((progress - 0.62) / 0.28).clamp(0.0, 1.0);
                let glint_opacity = (std::f32::consts::PI * glint_progress).sin() * 0.42;
                let highlight = fill_paint(Color::<u8>::WHITE, opacity * glint_opacity);
                canvas.draw_path(full, &highlight);
                canvas.restore();
            }
        }
        shrimply_project::project::VisualTransitionKind::Coalesce => {
            let pools = crate::path_transition::coalesce_mask(
                rect,
                progress,
                transition.effect_detail.round() as usize,
            );
            let Some(filter) = crate::path_transition::coalesce_filter(
                rect.width().max(rect.height()),
                transition.effect_amount,
            ) else {
                draw_shape_path(canvas, shape, full, opacity);
                return;
            };
            canvas.save_layer(&SaveLayerRec::default());
            let mut filtered = Paint::default();
            filtered.set_image_filter(filter);
            canvas.save_layer(&SaveLayerRec::default().paint(&filtered));
            let mut mask = Paint::default();
            mask.set_anti_alias(true);
            mask.set_color(SkiaColor::WHITE);
            canvas.draw_path(&pools, &mask);
            canvas.restore();
            let mut source_in = Paint::default();
            source_in.set_blend_mode(BlendMode::SrcIn);
            canvas.save_layer(&SaveLayerRec::default().paint(&source_in));
            draw_shape_path(canvas, shape, full, opacity);
            canvas.restore();
            canvas.restore();
        }
        shrimply_project::project::VisualTransitionKind::ContourCurrent => {
            draw_shape_path(
                canvas,
                shape,
                full,
                opacity * shrimply_math_media::vector_reveal_opacity(progress),
            );
            canvas.draw_path(
                full,
                &crate::path_transition::contour_current_paint(
                    progress,
                    fallback_trace_width * 1.6 * transition.effect_amount,
                    transition.effect_detail,
                ),
            );
        }
        shrimply_project::project::VisualTransitionKind::SoftRefraction => {
            let Some(filter) = crate::path_transition::soft_refraction_filter(
                progress,
                rect.width().max(rect.height()),
                transition.effect_amount,
                transition.effect_detail,
            ) else {
                draw_shape_path(canvas, shape, full, opacity);
                return;
            };
            let mut filtered = Paint::default();
            filtered.set_alpha_f(shrimply_math_media::vector_reveal_opacity(progress));
            filtered.set_image_filter(filter);
            canvas.save_layer(&SaveLayerRec::default().paint(&filtered));
            draw_shape_path(canvas, shape, full, opacity);
            canvas.restore();
        }
        shrimply_project::project::VisualTransitionKind::MorphologicalResolve => {
            let Some(filter) = crate::path_transition::morphological_filter(
                progress,
                rect.width().max(rect.height()),
                transition.effect_amount,
                transition.effect_detail,
            ) else {
                draw_shape_path(canvas, shape, full, opacity);
                return;
            };
            let mut filtered = Paint::default();
            filtered.set_alpha_f(shrimply_math_media::vector_reveal_opacity(progress));
            filtered.set_image_filter(filter);
            canvas.save_layer(&SaveLayerRec::default().paint(&filtered));
            draw_shape_path(canvas, shape, full, opacity);
            canvas.restore();
        }
        shrimply_project::project::VisualTransitionKind::LivingFill => {
            let Some(mask) = crate::path_transition::living_fill_paint(
                rect,
                progress,
                transition.effect_amount,
                transition.effect_detail,
                transition.effect_angle_degrees,
            ) else {
                draw_shape_path(canvas, shape, full, opacity);
                return;
            };
            canvas.save_layer(&SaveLayerRec::default());
            canvas.draw_path(full, &mask);
            let mut source_in = Paint::default();
            source_in.set_blend_mode(BlendMode::SrcIn);
            canvas.save_layer(&SaveLayerRec::default().paint(&source_in));
            draw_shape_path(canvas, shape, full, opacity);
            canvas.restore();
            canvas.restore();
        }
        shrimply_project::project::VisualTransitionKind::Diffusion
        | shrimply_project::project::VisualTransitionKind::ReverseDiffusion => {
            let diffused = crate::path_transition::diffused_path(
                full,
                rect.with_outset((rect.width(), rect.height())),
                progress,
                (transition.kind
                    == shrimply_project::project::VisualTransitionKind::ReverseDiffusion)
                    != (transition.side == shrimply_project::project::TransitionSide::Outro),
                transition.effect_amount,
                transition.effect_detail,
                transition.effect_seed,
            );
            let transition_opacity = if transition.effect_fade {
                shrimply_math_media::vector_reveal_opacity(progress)
            } else {
                1.0
            };
            draw_shape_path(canvas, shape, &diffused, opacity * transition_opacity);
        }
        _ => draw_shape_path(canvas, shape, full, opacity),
    }
}

fn draw_shape_path(canvas: &Canvas, shape: &ShapeDraw, path: &Path, opacity: f32) {
    if opacity <= 0.0 {
        return;
    }
    if shape.shadow_offset.length_squared() > f32::EPSILON
        || shape.shadow_width > 0.0
        || shape.shadow_blur > 0.0
    {
        let mut shadow = fill_paint(shape.shadow_color, opacity);
        if shape.shadow_width > 0.0 {
            shadow.set_style(PaintStyle::StrokeAndFill);
            shadow.set_stroke_width(shape.shadow_width);
        }
        if shape.shadow_blur > 0.0 {
            shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, shape.shadow_blur, true));
        }
        canvas.save();
        canvas.translate((shape.shadow_offset.x, shape.shadow_offset.y));
        canvas.draw_path(path, &shadow);
        canvas.restore();
    }
    canvas.draw_path(path, &fill_paint(shape.fill, opacity));
    if shape.outline_width > 0.0 {
        let mut outline = fill_paint(shape.outline_color, opacity);
        outline.set_style(PaintStyle::Stroke);
        outline.set_stroke_width(shape.outline_width);
        canvas.draw_path(path, &outline);
    }
}

fn polygon_path(
    vertices: Vec<glam::Vec2>,
    corner_radius: f32,
    strategy: ShapeRoundingStrategy,
) -> Path {
    if corner_radius <= f32::EPSILON {
        return sharp_polygon_path(&vertices);
    }
    let circular = strategy == ShapeRoundingStrategy::Circular;
    let Some(corners) = shrimply_math_media::polygon_corners(&vertices, corner_radius, circular)
    else {
        return sharp_polygon_path(&vertices);
    };
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(corners[0].exit.x, corners[0].exit.y));
    for (index, corner) in corners
        .iter()
        .enumerate()
        .skip(1)
        .chain(corners.iter().enumerate().take(1))
    {
        builder.line_to(Point::new(corner.entry.x, corner.entry.y));
        let vertex = vertices[index];
        match strategy {
            ShapeRoundingStrategy::Continuous => {
                builder.quad_to(
                    Point::new(vertex.x, vertex.y),
                    Point::new(corner.exit.x, corner.exit.y),
                );
            }
            ShapeRoundingStrategy::Circular => {
                builder.conic_to(
                    Point::new(vertex.x, vertex.y),
                    Point::new(corner.exit.x, corner.exit.y),
                    corner.conic_weight,
                );
            }
            ShapeRoundingStrategy::Chamfer => {
                builder.line_to(Point::new(corner.exit.x, corner.exit.y));
            }
        }
    }
    builder.close();
    builder.detach()
}

fn sharp_polygon_path(vertices: &[glam::Vec2]) -> Path {
    let mut builder = PathBuilder::new();
    let Some(first) = vertices.first() else {
        return builder.detach();
    };
    builder.move_to(Point::new(first.x, first.y));
    for vertex in &vertices[1..] {
        builder.line_to(Point::new(vertex.x, vertex.y));
    }
    builder.close();
    builder.detach()
}

fn heart_path(rect: Rect) -> Path {
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(rect.center_x(), rect.bottom()));
    builder.cubic_to(
        Point::new(
            rect.left() + rect.width() * 0.45,
            rect.top() + rect.height() * 0.88,
        ),
        Point::new(rect.left(), rect.top() + rect.height() * 0.62),
        Point::new(rect.left(), rect.top() + rect.height() * 0.32),
    );
    builder.cubic_to(
        Point::new(rect.left(), rect.top() + rect.height() * 0.08),
        Point::new(rect.left() + rect.width() * 0.28, rect.top()),
        Point::new(rect.center_x(), rect.top() + rect.height() * 0.22),
    );
    builder.cubic_to(
        Point::new(rect.left() + rect.width() * 0.72, rect.top()),
        Point::new(rect.right(), rect.top() + rect.height() * 0.08),
        Point::new(rect.right(), rect.top() + rect.height() * 0.32),
    );
    builder.cubic_to(
        Point::new(rect.right(), rect.top() + rect.height() * 0.62),
        Point::new(
            rect.left() + rect.width() * 0.55,
            rect.top() + rect.height() * 0.88,
        ),
        Point::new(rect.center_x(), rect.bottom()),
    );
    builder.close();
    builder.detach()
}

fn number_value(
    number: &TimelineValue<f32>,
    eval: &TransformEvaluation,
    expression_cache: &mut TransformExpressionCache,
) -> f32 {
    shrimply_evaluation::resolve_scalar(number, eval, expression_cache)
}

fn vec2_value(
    value: &TimelineValue<glam::Vec2>,
    eval: &TransformEvaluation,
    expression_cache: &mut TransformExpressionCache,
) -> glam::Vec2 {
    shrimply_evaluation::resolve_vec2(value, eval, expression_cache)
}

fn rect_path(rect: Rect, corner_radius: f32, strategy: ShapeRoundingStrategy) -> Path {
    match strategy {
        ShapeRoundingStrategy::Chamfer => chamfer_rect_path(rect, corner_radius),
        ShapeRoundingStrategy::Continuous | ShapeRoundingStrategy::Circular => {
            continuous_rect_path(rect, corner_radius)
        }
    }
}

fn continuous_rect_path(rect: Rect, corner_radius: f32) -> Path {
    let radius = rect_corner_radius(rect, corner_radius);
    if radius <= f32::EPSILON {
        return sharp_rect_path(rect);
    }
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(rect.left() + radius, rect.top()));
    builder.line_to(Point::new(rect.right() - radius, rect.top()));
    add_continuous_rect_corner(
        &mut builder,
        Point::new(rect.right() - radius, rect.top() + radius),
        radius,
        -std::f32::consts::FRAC_PI_2,
        0.0,
    );
    builder.line_to(Point::new(rect.right(), rect.bottom() - radius));
    add_continuous_rect_corner(
        &mut builder,
        Point::new(rect.right() - radius, rect.bottom() - radius),
        radius,
        0.0,
        std::f32::consts::FRAC_PI_2,
    );
    builder.line_to(Point::new(rect.left() + radius, rect.bottom()));
    add_continuous_rect_corner(
        &mut builder,
        Point::new(rect.left() + radius, rect.bottom() - radius),
        radius,
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
    );
    builder.line_to(Point::new(rect.left(), rect.top() + radius));
    add_continuous_rect_corner(
        &mut builder,
        Point::new(rect.left() + radius, rect.top() + radius),
        radius,
        std::f32::consts::PI,
        std::f32::consts::PI + std::f32::consts::FRAC_PI_2,
    );
    builder.close();
    builder.detach()
}

fn add_continuous_rect_corner(
    builder: &mut PathBuilder,
    center: Point,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
) {
    const SAMPLES: u32 = 12;
    for step in 1..=SAMPLES {
        let amount = step as f32 / SAMPLES as f32;
        let angle = start_angle + (end_angle - start_angle) * amount;
        let cos = angle.cos();
        let sin = angle.sin();
        builder.line_to(Point::new(
            center.x + radius * cos.signum() * cos.abs().sqrt(),
            center.y + radius * sin.signum() * sin.abs().sqrt(),
        ));
    }
}

fn chamfer_rect_path(rect: Rect, corner_radius: f32) -> Path {
    let radius = rect_corner_radius(rect, corner_radius);
    if radius <= f32::EPSILON {
        return sharp_rect_path(rect);
    }
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(rect.left() + radius, rect.top()));
    builder.line_to(Point::new(rect.right() - radius, rect.top()));
    builder.line_to(Point::new(rect.right(), rect.top() + radius));
    builder.line_to(Point::new(rect.right(), rect.bottom() - radius));
    builder.line_to(Point::new(rect.right() - radius, rect.bottom()));
    builder.line_to(Point::new(rect.left() + radius, rect.bottom()));
    builder.line_to(Point::new(rect.left(), rect.bottom() - radius));
    builder.line_to(Point::new(rect.left(), rect.top() + radius));
    builder.close();
    builder.detach()
}

fn sharp_rect_path(rect: Rect) -> Path {
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(rect.left(), rect.top()));
    builder.line_to(Point::new(rect.right(), rect.top()));
    builder.line_to(Point::new(rect.right(), rect.bottom()));
    builder.line_to(Point::new(rect.left(), rect.bottom()));
    builder.close();
    builder.detach()
}

fn rect_corner_radius(rect: Rect, corner_radius: f32) -> f32 {
    corner_radius
        .max(0.0)
        .min(rect.width() * 0.5)
        .min(rect.height() * 0.5)
}

fn triangle_path(rect: Rect, corner_radius: f32, strategy: ShapeRoundingStrategy) -> Path {
    let top = Point::new(rect.center_x(), rect.top());
    let right = Point::new(rect.right(), rect.bottom());
    let left = Point::new(rect.left(), rect.bottom());
    if corner_radius <= 0.0 {
        return sharp_triangle_path(top, right, left);
    }
    match strategy {
        ShapeRoundingStrategy::Continuous => {
            continuous_triangle_path(top, right, left, corner_radius)
        }
        ShapeRoundingStrategy::Circular => circular_triangle_path(top, right, left, corner_radius),
        ShapeRoundingStrategy::Chamfer => chamfer_triangle_path(top, right, left, corner_radius),
    }
}

fn circular_triangle_path(top: Point, right: Point, left: Point, corner_radius: f32) -> Path {
    let Some(top_corner) = circular_triangle_corner(left, top, right, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(right_corner) = circular_triangle_corner(top, right, left, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(left_corner) = circular_triangle_corner(right, left, top, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let mut builder = PathBuilder::new();
    builder.move_to(top_corner.exit);
    builder.line_to(right_corner.entry);
    builder.conic_to(right, right_corner.exit, right_corner.weight);
    builder.line_to(left_corner.entry);
    builder.conic_to(left, left_corner.exit, left_corner.weight);
    builder.line_to(top_corner.entry);
    builder.conic_to(top, top_corner.exit, top_corner.weight);
    builder.close();
    builder.detach()
}

fn continuous_triangle_path(top: Point, right: Point, left: Point, corner_radius: f32) -> Path {
    let Some(top_corner) = triangle_corner(left, top, right, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(right_corner) = triangle_corner(top, right, left, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(left_corner) = triangle_corner(right, left, top, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let mut builder = PathBuilder::new();
    builder.move_to(top_corner.exit);
    builder.line_to(right_corner.entry);
    builder.quad_to(right, right_corner.exit);
    builder.line_to(left_corner.entry);
    builder.quad_to(left, left_corner.exit);
    builder.line_to(top_corner.entry);
    builder.quad_to(top, top_corner.exit);
    builder.close();
    builder.detach()
}

fn chamfer_triangle_path(top: Point, right: Point, left: Point, corner_radius: f32) -> Path {
    let Some(top_corner) = triangle_corner(left, top, right, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(right_corner) = triangle_corner(top, right, left, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(left_corner) = triangle_corner(right, left, top, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let mut builder = PathBuilder::new();
    builder.move_to(top_corner.exit);
    builder.line_to(right_corner.entry);
    builder.line_to(right_corner.exit);
    builder.line_to(left_corner.entry);
    builder.line_to(left_corner.exit);
    builder.line_to(top_corner.entry);
    builder.line_to(top_corner.exit);
    builder.close();
    builder.detach()
}

fn sharp_triangle_path(top: Point, right: Point, left: Point) -> Path {
    let mut builder = PathBuilder::new();
    builder.move_to(top);
    builder.line_to(right);
    builder.line_to(left);
    builder.close();
    builder.detach()
}

#[derive(Clone, Copy)]
struct TriangleCorner {
    entry: Point,
    exit: Point,
    weight: f32,
}

fn circular_triangle_corner(
    previous: Point,
    corner: Point,
    next: Point,
    radius: f32,
) -> Option<TriangleCorner> {
    let previous_x = previous.x - corner.x;
    let previous_y = previous.y - corner.y;
    let next_x = next.x - corner.x;
    let next_y = next.y - corner.y;
    let previous_length = previous_x.hypot(previous_y);
    let next_length = next_x.hypot(next_y);
    if previous_length <= f32::EPSILON || next_length <= f32::EPSILON {
        return None;
    }
    let dot = ((previous_x * next_x + previous_y * next_y) / (previous_length * next_length))
        .clamp(-1.0, 1.0);
    let half_angle = dot.acos() * 0.5;
    let tangent = half_angle.tan();
    if tangent <= f32::EPSILON {
        return None;
    }
    let distance = (radius / tangent)
        .min(previous_length * 0.45)
        .min(next_length * 0.45);
    let corner = triangle_corner_at_distance(
        previous,
        corner,
        next,
        previous_length,
        next_length,
        distance,
    )?;
    Some(TriangleCorner {
        weight: half_angle.cos().max(f32::EPSILON),
        ..corner
    })
}

fn triangle_corner(
    previous: Point,
    corner: Point,
    next: Point,
    distance: f32,
) -> Option<TriangleCorner> {
    let previous_length = (previous.x - corner.x).hypot(previous.y - corner.y);
    let next_length = (next.x - corner.x).hypot(next.y - corner.y);
    triangle_corner_at_distance(
        previous,
        corner,
        next,
        previous_length,
        next_length,
        distance,
    )
}

fn triangle_corner_at_distance(
    previous: Point,
    corner: Point,
    next: Point,
    previous_length: f32,
    next_length: f32,
    distance: f32,
) -> Option<TriangleCorner> {
    if previous_length <= f32::EPSILON || next_length <= f32::EPSILON {
        return None;
    }
    let distance = distance.min(previous_length * 0.45).min(next_length * 0.45);
    if distance <= f32::EPSILON {
        return None;
    }
    Some(TriangleCorner {
        entry: point_on_edge(corner, previous, previous_length, distance),
        exit: point_on_edge(corner, next, next_length, distance),
        weight: 1.0,
    })
}

fn point_on_edge(from: Point, to: Point, length: f32, distance: f32) -> Point {
    let amount = distance / length;
    Point::new(
        from.x + (to.x - from.x) * amount,
        from.y + (to.y - from.y) * amount,
    )
}

fn fill_paint(color: Color<u8>, opacity: f32) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);
    paint.set_color(color.alpha_multiply(opacity));
    paint
}
