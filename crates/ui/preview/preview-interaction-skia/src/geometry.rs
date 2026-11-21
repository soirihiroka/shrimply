use glam::{IVec2, vec2};
use shrimply_components_skia::Rect;
use shrimply_preview_provider_skia::PreviewViewport;
use shrimply_project_document::project::CanvasSize;

pub fn video_content_rect(surface: IVec2, canvas: CanvasSize, padding_px: u32) -> Rect {
    shrimply_preview_provider_skia::math::video_content_rect(
        surface,
        glam::UVec2::new(canvas.width, canvas.height),
        padding_px,
    )
}

pub fn preview_viewport(surface: IVec2, canvas: CanvasSize, padding_px: u32) -> PreviewViewport {
    let surface_rect = Rect::from_min_size(
        glam::Vec2::ZERO,
        vec2(surface.x.max(1) as f32, surface.y.max(1) as f32),
    );
    PreviewViewport::new(
        vec2(canvas.width.max(1) as f32, canvas.height.max(1) as f32),
        video_content_rect(surface, canvas, padding_px),
    )
    .with_surface_rect(surface_rect)
}

pub(crate) use shrimply_preview_provider_skia::math::caption_preview_scale;
