use super::RasterModifierRuntime;
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::color_correction::ColorCorrectionModifier;

impl RasterModifierRuntime for ColorCorrectionModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        input.push_pixel(Box::new(shrimply_video_core::modifiers::color_correction(
            self,
            context.evaluation,
            context.expressions,
        )));
        Ok(input)
    }
}
