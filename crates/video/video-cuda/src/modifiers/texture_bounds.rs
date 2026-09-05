use super::RasterModifierRuntime;
use crate::visual_source::VisualModifierContext;
use shrimply_evaluation::resolve_scalar;
use shrimply_video_modifiers::texture_bounds::TextureBoundsModifier;

impl RasterModifierRuntime for TextureBoundsModifier {
    fn apply_raster(
        &self,
        mut input: crate::layer::RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<crate::layer::RasterVisual, String> {
        let resolve = |value, context: &mut VisualModifierContext<'_>| {
            resolve_scalar(value, context.evaluation, context.expressions)
        };
        let edges = [
            resolve(&self.edges.top, context),
            resolve(&self.edges.right, context),
            resolve(&self.edges.bottom, context),
            resolve(&self.edges.left, context),
        ];
        let address_mode = self.address_mode.value_at(context.evaluation.local_time());
        input.push_spatial(move |state| {
            for (current, added) in state.bounds.edges.iter_mut().zip(edges) {
                *current += added;
            }
            state.bounds.address_mode = address_mode;
        });
        Ok(input)
    }
}
