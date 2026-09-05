use glam::{IVec2, vec2};

use crate::Rect;

pub fn fullscreen_pointer_positions_close(a: glam::Vec2, b: glam::Vec2) -> bool {
    const REVEAL_POINTER_THRESHOLD: f32 = 1.0;
    (a.x - b.x).abs() <= REVEAL_POINTER_THRESHOLD && (a.y - b.y).abs() <= REVEAL_POINTER_THRESHOLD
}

pub fn video_content_rect(surface: IVec2, canvas: glam::UVec2, padding_px: u32) -> Rect {
    let surface_width = surface.x.max(1) as f32;
    let surface_height = surface.y.max(1) as f32;
    let padding = padding_px as f32;
    let available_width = (surface_width - padding * 2.0).max(1.0);
    let available_height = (surface_height - padding * 2.0).max(1.0);
    let available_aspect = available_width / available_height;
    let canvas_aspect = canvas.x.max(1) as f32 / canvas.y.max(1) as f32;
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
