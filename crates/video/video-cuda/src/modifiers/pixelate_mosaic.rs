use super::RasterModifierRuntime;
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::pixelate_mosaic::PixelateMosaicModifier;

impl RasterModifierRuntime for PixelateMosaicModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        input.push_pixel(Box::new(shrimply_video_core::modifiers::pixelate_mosaic(
            self,
            context.evaluation,
            context.expressions,
        )));
        Ok(input)
    }
}
