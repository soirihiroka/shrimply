use shrimply_project::project::TextureAddressMode;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedVisualBounds {
    /// Signed top, right, bottom, and left pixels. Positive values outset; negative values inset.
    pub edges: [f32; 4],
    /// Sequential crop applied by crop modifiers after the signed base edges.
    pub modifier_crop: [f32; 4],
    pub modifier_crop_pixels: [f32; 4],
    pub address_mode: TextureAddressMode,
}

impl Default for ResolvedVisualBounds {
    fn default() -> Self {
        Self {
            edges: [0.0; 4],
            modifier_crop: [0.0; 4],
            modifier_crop_pixels: [0.0; 4],
            address_mode: TextureAddressMode::Transparent,
        }
    }
}

pub fn sampling_bounds(
    bounds: ResolvedVisualBounds,
    source_size: glam::Vec2,
) -> ([f32; 4], [f32; 4]) {
    crate::math::signed_edges_bounds(
        bounds.edges,
        bounds.modifier_crop,
        bounds.modifier_crop_pixels,
        source_size,
    )
}

pub fn source_size_for_frame_size(
    bounds: ResolvedVisualBounds,
    frame_size: glam::Vec2,
) -> glam::Vec2 {
    crate::math::source_size_for_signed_frame(
        bounds.edges,
        bounds.modifier_crop,
        bounds.modifier_crop_pixels,
        frame_size,
    )
}
