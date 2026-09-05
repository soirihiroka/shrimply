use super::RasterModifierRuntime;
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::chromatic_aberration::ChromaticAberrationModifier;

impl RasterModifierRuntime for ChromaticAberrationModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        input.push_pixel(Box::new(
            shrimply_video_core::modifiers::chromatic_aberration(
                self,
                context.evaluation,
                context.expressions,
            ),
        ));
        Ok(input)
    }
}
