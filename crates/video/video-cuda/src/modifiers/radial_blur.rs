use super::RasterModifierRuntime;
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::radial_blur::RadialBlurModifier;

impl RasterModifierRuntime for RadialBlurModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        input.push_pixel(Box::new(shrimply_video_core::modifiers::radial_blur(
            self,
            context.evaluation,
            context.expressions,
        )));
        Ok(input)
    }
}
