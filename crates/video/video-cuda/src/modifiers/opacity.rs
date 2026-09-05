use super::RasterModifierRuntime;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::opacity::OpacityModifier;

impl RasterModifierRuntime for OpacityModifier {
    fn apply_raster(
        &self,
        mut input: crate::layer::RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<crate::layer::RasterVisual, String> {
        let opacity = shrimply_video_core::vector_modifiers::opacity(
            self,
            context.evaluation,
            context.expressions,
        );
        input.push_spatial(move |state| state.compositing.opacity *= opacity);
        Ok(input)
    }
}
