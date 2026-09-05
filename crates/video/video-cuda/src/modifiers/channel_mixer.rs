use super::RasterModifierRuntime;
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_video_modifiers::channel_mixer::ChannelMixerModifier;

impl RasterModifierRuntime for ChannelMixerModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        input.push_pixel(Box::new(shrimply_video_core::modifiers::channel_mixer(
            self,
            context.evaluation,
            context.expressions,
        )));
        Ok(input)
    }
}
