//! Ordered raster state changes surrounding the shared pixel kernels.
use shrimply_evaluation::{TransformExpressionCache, VisualEvaluation};
use shrimply_render_core::{VideoSampleMethod, effects::PixelEffect};
use shrimply_video_modifiers::RasterModifierEffect;

pub enum Operation {
    Pixel(PixelEffect),
    Transform(shrimply_math_geometry::ComposedTransform2D),
    Opacity(f32),
    Sampling(VideoSampleMethod),
}

pub struct Modifier {
    pub operation: Operation,
    pub alpha_mask: Option<crate::alpha_mask::ResolvedShapeAlphaMask>,
}

pub fn modifier(
    modifier: &shrimply_project::project::VisualModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
    content_accurate: bool,
) -> Result<Option<Modifier>, String> {
    let shrimply_video_modifiers::ModifierEffect::Raster(effect) = &modifier.effect else {
        return Ok(None);
    };
    let alpha_mask = modifier
        .alpha_mask
        .as_ref()
        .filter(|mask| mask.enabled)
        .map(|mask| crate::alpha_mask::resolve(mask, evaluation, expressions));
    Ok(
        operation(effect, evaluation, expressions, content_accurate)?.map(|operation| Modifier {
            operation,
            alpha_mask,
        }),
    )
}

pub fn sampling(
    effect: &shrimply_video_modifiers::sampling::SamplingModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
    content_accurate: bool,
) -> VideoSampleMethod {
    crate::generated::sampling(
        shrimply_evaluation::resolve(&effect.method, evaluation, expressions),
        content_accurate,
    )
}

pub fn operation(
    effect: &RasterModifierEffect,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
    content_accurate: bool,
) -> Result<Option<Operation>, String> {
    Ok(Some(match effect {
        RasterModifierEffect::CornerPin(effect) => Operation::Pixel(crate::modifiers::corner_pin(
            effect,
            evaluation,
            expressions,
        )?),
        RasterModifierEffect::Transform(effect) => Operation::Transform(
            crate::vector_modifiers::transform(effect, evaluation, expressions).composed(),
        ),
        RasterModifierEffect::Opacity(effect) => Operation::Opacity(
            crate::vector_modifiers::opacity(effect, evaluation, expressions),
        ),
        RasterModifierEffect::Sampling(effect) => {
            Operation::Sampling(sampling(effect, evaluation, expressions, content_accurate))
        }
        _ => {
            return Ok(
                crate::modifiers::pixel_effect(effect, evaluation, expressions)
                    .map(Operation::Pixel),
            );
        }
    }))
}
