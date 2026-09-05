use glam::{IVec2, vec2};
use shrimply_preview_core::PreviewViewport;
use shrimply_project::project::CanvasSize;
use shrimply_skia_adw_core::Rect;

pub fn video_content_rect(surface: IVec2, canvas: CanvasSize, padding_px: u32) -> Rect {
    shrimply_preview_core::math::video_content_rect(
        surface,
        glam::UVec2::new(canvas.width, canvas.height),
        padding_px,
    )
}

pub fn preview_viewport(surface: IVec2, canvas: CanvasSize, padding_px: u32) -> PreviewViewport {
    PreviewViewport::new(
        vec2(canvas.width.max(1) as f32, canvas.height.max(1) as f32),
        video_content_rect(surface, canvas, padding_px),
    )
}
