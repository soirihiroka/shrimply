use super::RasterModifierRuntime;
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::film_grain::FilmGrainModifier;

impl RasterModifierRuntime for FilmGrainModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        let effect = shrimply_video_core::modifiers::film_grain(
            self,
            context.evaluation,
            context.expressions,
        );
        input.push_pixel(Box::new(effect));
        Ok(input)
    }
}
