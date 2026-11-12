use crate::generated::{GeneratedTransition, GeneratedVisual};
use shrimply_drawing_skia::shape::{ShapeDraw, draw_shape_path, fill_paint, shape_path};
use shrimply_project_document::project::{CanvasSize, Color, ShapeItem};
use shrimply_project_evaluation::{
    TransformEvaluation, TransformExpressionCache, VisualEvaluation,
};
use shrimply_property_model::timeline_value::*;
use skia_safe::{
    BlendMode, BlurStyle, Canvas, Color as SkiaColor, MaskFilter, Paint, PaintStyle, Path, Point,
    Rect, canvas::SaveLayerRec,
};

pub fn prepare(
    canvas_size: CanvasSize,
    surface_size: CanvasSize,
    item: &shrimply_project_document::project::VideoItem,
    evaluation: VisualEvaluation,
    transition: Option<GeneratedTransition>,
    expressions: &mut TransformExpressionCache,
) -> PreparedShape {
    let shrimply_project_document::project::VideoItemContent::Shape(shape) = &item.content else {
        panic!("shape preparation received non-shape item")
    };
    let rotation_degrees = shrimply_project_evaluation::resolve_scalar(
        &item.transform.rotation_degrees,
        &evaluation,
        expressions,
    );
    let mut decoration = decoration_outset(shape, &evaluation, expressions, rotation_degrees);
    if transition.is_some_and(|transition| {
        matches!(
            transition.kind,
            shrimply_project_document::project::VisualTransitionKind::Write
                | shrimply_project_document::project::VisualTransitionKind::Create
                | shrimply_project_document::project::VisualTransitionKind::FacetAssembly
                | shrimply_project_document::project::VisualTransitionKind::Coalesce
                | shrimply_project_document::project::VisualTransitionKind::ContourCurrent
                | shrimply_project_document::project::VisualTransitionKind::SoftRefraction
                | shrimply_project_document::project::VisualTransitionKind::MorphologicalResolve
                | shrimply_project_document::project::VisualTransitionKind::LivingFill
                | shrimply_project_document::project::VisualTransitionKind::Diffusion
                | shrimply_project_document::project::VisualTransitionKind::ReverseDiffusion
        )
    }) {
        let canvas_extent = canvas_size.width.max(canvas_size.height).max(1) as f32;
        let trace_outset = match transition.map(|value| value.kind) {
            Some(shrimply_project_document::project::VisualTransitionKind::FacetAssembly) => {
                canvas_extent * 0.3
            }
            Some(
                shrimply_project_document::project::VisualTransitionKind::Coalesce
                | shrimply_project_document::project::VisualTransitionKind::SoftRefraction
                | shrimply_project_document::project::VisualTransitionKind::MorphologicalResolve
                | shrimply_project_document::project::VisualTransitionKind::Diffusion
                | shrimply_project_document::project::VisualTransitionKind::ReverseDiffusion,
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
    pub evaluation: shrimply_project_evaluation::VisualEvaluation,
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
        star_points: shrimply_project_evaluation::resolve(
            &shape_item.star_points,
            eval,
            expression_cache,
        )
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
        fill: shrimply_project_evaluation::resolve_color(&shape_item.fill, eval, expression_cache),
        outline_color: shrimply_project_evaluation::resolve_color(
            &shape_item.outline_color,
            eval,
            expression_cache,
        ),
        outline_width: number_value(&shape_item.outline_width, eval, expression_cache).max(0.0),
        corner_radius: number_value(&shape_item.corner_radius, eval, expression_cache).max(0.0),
        rounding_strategy: shape_item.rounding_strategy.value_at(eval.local_time()),
        shadow_color: shrimply_project_evaluation::resolve_color(
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
        shrimply_project_document::project::VisualTransitionKind::Write => {
            let trace_color = if shape.outline_width > 0.0 {
                shape.outline_color
            } else {
                shape.fill
            };
            if progress < 0.5 {
                let partial = crate::path_transition::partial_path(
                    full,
                    progress * 2.0,
                    transition.side == shrimply_project_document::project::TransitionSide::Outro,
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
        shrimply_project_document::project::VisualTransitionKind::Create => {
            let partial = crate::path_transition::partial_path(full, progress, false);
            draw_shape_path(canvas, shape, &partial, opacity);
        }
        shrimply_project_document::project::VisualTransitionKind::FacetAssembly => {
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
        shrimply_project_document::project::VisualTransitionKind::Coalesce => {
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
        shrimply_project_document::project::VisualTransitionKind::ContourCurrent => {
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
        shrimply_project_document::project::VisualTransitionKind::SoftRefraction => {
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
        shrimply_project_document::project::VisualTransitionKind::MorphologicalResolve => {
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
        shrimply_project_document::project::VisualTransitionKind::LivingFill => {
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
        shrimply_project_document::project::VisualTransitionKind::Diffusion
        | shrimply_project_document::project::VisualTransitionKind::ReverseDiffusion => {
            let diffused = crate::path_transition::diffused_path(
                full,
                rect.with_outset((rect.width(), rect.height())),
                progress,
                (transition.kind
                    == shrimply_project_document::project::VisualTransitionKind::ReverseDiffusion)
                    != (transition.side
                        == shrimply_project_document::project::TransitionSide::Outro),
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

fn number_value(
    number: &TimelineValue<f32>,
    eval: &TransformEvaluation,
    expression_cache: &mut TransformExpressionCache,
) -> f32 {
    shrimply_project_evaluation::resolve_scalar(number, eval, expression_cache)
}

fn vec2_value(
    value: &TimelineValue<glam::Vec2>,
    eval: &TransformEvaluation,
    expression_cache: &mut TransformExpressionCache,
) -> glam::Vec2 {
    shrimply_project_evaluation::resolve_vec2(value, eval, expression_cache)
}
