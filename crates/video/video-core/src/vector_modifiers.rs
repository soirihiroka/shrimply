//! Evaluation shared by vector renderers, before their rasterization boundary.
use crate::generated::{TextMaskOperation, VectorOperation};
use shrimply_evaluation::{
    TransformExpressionCache, VisualEvaluation, resolve_scalar, resolve_vec2,
};
use shrimply_math_geometry::ResolvedTransform2D;
use shrimply_video_modifiers::{
    VectorModifierEffect, opacity::OpacityModifier, transform::TransformModifier,
};

pub fn transform(
    effect: &TransformModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> ResolvedTransform2D {
    ResolvedTransform2D {
        position: resolve_vec2(effect.position(), evaluation, expressions),
        anchor: resolve_vec2(effect.anchor(), evaluation, expressions),
        scale: resolve_vec2(effect.scale(), evaluation, expressions),
        shear: resolve_vec2(effect.shear(), evaluation, expressions),
        rotation_degrees: resolve_scalar(effect.rotation_degrees(), evaluation, expressions),
    }
}

pub fn opacity(
    effect: &OpacityModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> f32 {
    resolve_scalar(&effect.opacity, evaluation, expressions).clamp(0.0, 1.0)
}

pub fn operation(
    effect: &VectorModifierEffect,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> Option<VectorOperation> {
    Some(match effect {
        VectorModifierEffect::Transform(effect) => {
            VectorOperation::Transform(transform(effect, evaluation, expressions).composed())
        }
        VectorModifierEffect::Opacity(effect) => {
            VectorOperation::Opacity(opacity(effect, evaluation, expressions))
        }
        VectorModifierEffect::Repeat(effect) => {
            use shrimply_video_modifiers::repeat::RepeatOffsetAxis;
            let step = resolve_vec2(&effect.step, evaluation, expressions);
            let copies_x = resolve_scalar(&effect.copies_x, evaluation, expressions)
                .round()
                .max(1.0) as u32;
            let copies_y = resolve_scalar(&effect.copies_y, evaluation, expressions)
                .round()
                .max(1.0) as u32;
            let row_offset = resolve_scalar(&effect.row_offset, evaluation, expressions);
            let row_offset = match effect.row_offset_axis.value_at(evaluation.local_time()) {
                RepeatOffsetAxis::X => glam::Vec2::new(row_offset, 0.0),
                RepeatOffsetAxis::Y => glam::Vec2::new(0.0, row_offset),
            };
            VectorOperation::Repeat {
                copies_x,
                copies_y,
                step,
                row_offset,
            }
        }
        VectorModifierEffect::ShakyPath(effect) => {
            const MIN_STEP_SIZE: f32 = 0.1;
            let amplitude = resolve_scalar(&effect.amplitude, evaluation, expressions).max(0.0);
            if amplitude <= f32::EPSILON {
                return None;
            }
            let step_size =
                resolve_scalar(&effect.step_size, evaluation, expressions).max(MIN_STEP_SIZE);
            let evolution = resolve_scalar(&effect.evolution, evaluation, expressions);
            let seed = resolve_scalar(&effect.seed, evaluation, expressions)
                .round()
                .clamp(0.0, u32::MAX as f32) as u32;
            VectorOperation::ShakyPath {
                amplitude,
                step_size,
                seed: shrimply_math_media::shaky_path_seed(seed, evolution),
            }
        }
        VectorModifierEffect::Hsv(effect) => {
            const DEGREES_PER_TURN: f32 = 360.0;
            VectorOperation::Hsv {
                hue_turns: resolve_scalar(&effect.hue_degrees, evaluation, expressions)
                    / DEGREES_PER_TURN,
                saturation: resolve_scalar(&effect.saturation, evaluation, expressions)
                    .clamp(0.0, 2.0),
                value: resolve_scalar(&effect.value, evaluation, expressions).clamp(0.0, 2.0),
            }
        }
        // PathOffset is a no-op in the existing vector pipeline.
        VectorModifierEffect::PathOffset(_) => return None,
        VectorModifierEffect::TextMask(effect) => {
            let amount = resolve_scalar(&effect.amount, evaluation, expressions).clamp(0.0, 1.0);
            if amount >= 1.0 {
                return None;
            }
            VectorOperation::TextMask(TextMaskOperation {
                amount,
                partial_mode: effect.partial_mode,
                direction: effect.direction,
            })
        }
    })
}
