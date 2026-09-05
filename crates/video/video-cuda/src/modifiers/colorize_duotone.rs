use super::RasterModifierRuntime;
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::colorize_duotone::ColorizeDuotoneModifier;

impl RasterModifierRuntime for ColorizeDuotoneModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        input.push_pixel(Box::new(shrimply_video_core::modifiers::colorize_duotone(
            self,
            context.evaluation,
            context.expressions,
        )));
        Ok(input)
    }
}
