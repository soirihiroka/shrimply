use shrimply_render_core::LayerBlendMode;

use super::VisualFrame;

pub(crate) struct LayeredImageGpuLayer<'a> {
    pub source: &'a VisualFrame,
    pub clipping_base: Option<(&'a VisualFrame, f32)>,
    pub mode: LayerBlendMode,
    pub opacity: f32,
    pub noise_seed: u32,
}
