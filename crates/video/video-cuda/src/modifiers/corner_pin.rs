use super::RasterModifierRuntime;
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::corner_pin::CornerPinModifier;

impl RasterModifierRuntime for CornerPinModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        let effect = shrimply_video_core::modifiers::corner_pin(
            self,
            context.evaluation,
            context.expressions,
        )?;
        if !effect.is_identity() {
            input.push_pixel(Box::new(effect));
        }
        Ok(input)
    }
}
