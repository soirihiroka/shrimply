use glam::{IVec2, vec2};
use shrimply_preview_core::PreviewViewport;
use shrimply_project::project::CanvasSize;
use shrimply_skia_adw_core::Rect;

pub fn video_content_rect(surface: IVec2, canvas: CanvasSize, padding_px: u32) -> Rect {
    let surface_width = surface.x.max(1) as f32;
    let surface_height = surface.y.max(1) as f32;
    let padding = padding_px as f32;
    let available_width = (surface_width - padding * 2.0).max(1.0);
    let available_height = (surface_height - padding * 2.0).max(1.0);
    let available_aspect = available_width / available_height;
    let canvas_aspect = canvas.width.max(1) as f32 / canvas.height.max(1) as f32;
    let (width, height) = if available_aspect > canvas_aspect {
        (available_height * canvas_aspect, available_height)
    } else {
        (available_width, available_width / canvas_aspect)
    };
    Rect::from_min_size(
        vec2(
            (surface_width - width) * 0.5,
            (surface_height - height) * 0.5,
        ),
        vec2(width, height),
    )
}

pub fn preview_viewport(surface: IVec2, canvas: CanvasSize, padding_px: u32) -> PreviewViewport {
    PreviewViewport::new(
        vec2(canvas.width.max(1) as f32, canvas.height.max(1) as f32),
        video_content_rect(surface, canvas, padding_px),
    )
}
