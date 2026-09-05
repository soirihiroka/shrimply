use super::RasterModifierRuntime;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::sampling::SamplingModifier;

impl RasterModifierRuntime for SamplingModifier {
    fn apply_raster(
        &self,
        mut input: crate::layer::RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<crate::layer::RasterVisual, String> {
        let method = shrimply_video_core::raster_modifiers::sampling(
            self,
            context.evaluation,
            context.expressions,
            context.accuracy.content_accurate(),
        );
        input.push_spatial(move |state| state.sampling = method);
        Ok(input)
    }
}
