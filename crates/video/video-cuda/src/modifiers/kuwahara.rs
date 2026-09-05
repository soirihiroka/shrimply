use super::RasterModifierRuntime;
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::kuwahara::KuwaharaModifier;

impl RasterModifierRuntime for KuwaharaModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        let effect =
            shrimply_video_core::modifiers::kuwahara(self, context.evaluation, context.expressions);
        if !effect.is_identity() {
            input.push_pixel(Box::new(effect));
        }
        Ok(input)
    }
}
