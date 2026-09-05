use super::RasterModifierRuntime;
use crate::visual_source::VisualModifierContext;
use shrimply_evaluation::resolve_scalar;
use shrimply_video_modifiers::crop::CropModifier;

impl RasterModifierRuntime for CropModifier {
    fn apply_raster(
        &self,
        mut input: crate::layer::RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<crate::layer::RasterVisual, String> {
        let resolve = |value, context: &mut VisualModifierContext<'_>| {
            resolve_scalar(value, context.evaluation, context.expressions)
        };
        let (pixels, edges) = match self {
            CropModifier::Percentage(edges) => (false, edges),
            CropModifier::Pixels(edges) => (true, edges),
        };
        let crop = [
            resolve(&edges.top, context),
            resolve(&edges.right, context),
            resolve(&edges.bottom, context),
            resolve(&edges.left, context),
        ];
        if pixels {
            input.push_spatial(move |state| {
                for (current, added) in state.bounds.modifier_crop_pixels.iter_mut().zip(crop) {
                    *current += added.max(0.0);
                }
            });
        } else {
            let crop = crate::math::normalized_crop(
                crop.map(|value| (value / 100.0).clamp(0.0, 0.999_99)),
            );
            input.push_spatial(move |state| {
                (
                    state.bounds.modifier_crop,
                    state.bounds.modifier_crop_pixels,
                ) = crate::math::compose_fractional_crop(
                    state.bounds.modifier_crop,
                    state.bounds.modifier_crop_pixels,
                    crop,
                );
            });
        }
        Ok(input)
    }
}
