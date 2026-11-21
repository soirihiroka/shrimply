use super::*;
use glam::IVec2;
use shrimply_preview_runtime_cuda::{
    controller::Controller,
    overlay::{self, InteractionOverlay, PreviewOverlayFrame},
};
use shrimply_project_evaluation::TransformExpressionCache;
use std::cell::RefCell;

pub(crate) struct ToolkitPreviewRenderer {
    renderer: VideoRenderer,
}

pub(crate) fn toolkit_guide_viewport(
    project: &Project,
    preferences: &preferences_store::PreferencesSnapshot,
    width: f32,
    height: f32,
    fullscreen: bool,
) -> PreviewViewport {
    guides::viewport(
        IVec2::new(width as i32, height as i32),
        project.canvas_size,
        preferences.preview_padding_px,
        preferences.preview_guides_visible,
        fullscreen,
    )
}

impl ToolkitPreviewRenderer {
    pub(crate) fn new() -> Result<Self, String> {
        Ok(Self {
            renderer: VideoRenderer::new()?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        project: &Project,
        position: Time,
        frame: Option<&CompositedVideoFrame>,
        surface: IVec2,
        pixels_per_point: f32,
        background_color: Color,
        preferences: &preferences_store::PreferencesSnapshot,
        viewport: PreviewViewport,
        clip_rect: Rect,
        controller: &mut Controller,
        expression_cache: &RefCell<TransformExpressionCache>,
    ) -> Result<(), String> {
        let scale = pixels_per_point.max(1.0);
        let logical_surface = vec2(surface.x as f32 / scale, surface.y as f32 / scale);
        let content_rect = viewport.content_rect;
        self.renderer.render(
            surface,
            pixels_per_point,
            frame,
            Appearance {
                content_rect,
                clip_rect,
                shadow_size_px: preferences.preview_shadow_size_px,
                background_color,
                upsample_method: preferences.preview_upsample_method,
                downsample_method: preferences.preview_downsample_method,
            },
            |painter| {
                let surface_rect = Rect::from_min_size(vec2(0.0, 0.0), logical_surface);
                overlay::draw_overlay(
                    painter.canvas(),
                    &PreviewOverlayFrame {
                        project,
                        position,
                        captions: CaptionAppearance {
                            preview_rect: surface_rect,
                            font_size: preferences.caption_font_size,
                            background_color: preferences.caption_background_color,
                            bottom_inset: 0.0,
                        },
                        caption_split_hover: None,
                        interaction: InteractionOverlay {
                            viewport,
                            clip_rect,
                            surface_rect,
                            guides: preferences
                                .preview_guides_visible
                                .then_some(project.preview_guides.as_ref()),
                        },
                    },
                    controller,
                    expression_cache,
                    |_| {},
                );
            },
        )
    }

    pub(crate) fn destroy(&mut self) {
        self.renderer.destroy();
    }
}
