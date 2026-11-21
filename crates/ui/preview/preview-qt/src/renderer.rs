use super::*;
use glam::IVec2;

pub(crate) struct ToolkitPreviewRenderer {
    renderer: VideoRenderer,
}

impl ToolkitPreviewRenderer {
    pub(crate) fn new() -> Result<Self, String> {
        Ok(Self {
            renderer: VideoRenderer::new()?,
        })
    }

    pub(crate) fn render(
        &mut self,
        frame: Option<&CompositedVideoFrame>,
        surface: IVec2,
        pixels_per_point: f32,
        appearance: Appearance,
        draw_overlay: impl FnOnce(&skia_safe::Canvas),
    ) -> Result<(), String> {
        self.renderer
            .render(surface, pixels_per_point, frame, appearance, |painter| {
                draw_overlay(painter.canvas());
            })
    }

    pub(crate) fn destroy(&mut self) {
        self.renderer.destroy();
    }
}
