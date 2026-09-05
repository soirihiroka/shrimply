use super::RasterModifierRuntime;
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::mirror::MirrorModifier;

impl RasterModifierRuntime for MirrorModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        _: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        input.push_pixel(Box::new(shrimply_video_core::modifiers::mirror(self)));
        Ok(input)
    }
}
