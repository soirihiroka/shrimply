use super::RasterModifierRuntime;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::transform::TransformModifier;

impl RasterModifierRuntime for TransformModifier {
    fn apply_raster(
        &self,
        mut input: crate::layer::RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<crate::layer::RasterVisual, String> {
        let modifier = shrimply_video_core::vector_modifiers::transform(
            self,
            context.evaluation,
            context.expressions,
        );
        input.push_spatial(move |state| {
            state.transform = modifier.composed().compose(state.transform);
        });
        Ok(input)
    }
}
