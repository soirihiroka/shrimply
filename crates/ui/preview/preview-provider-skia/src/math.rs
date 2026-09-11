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

pub const MIN_PREVIEW_ZOOM: f32 = 1.0;
pub const MAX_PREVIEW_ZOOM: f32 = 8.0;
pub const PREVIEW_ZOOM_PER_STEP: f32 = 1.2;
pub const PREVIEW_DRAG_THRESHOLD: f32 = 3.0;
pub const SCROLL_PIXELS_PER_STEP: f32 = 120.0;

pub fn padded_preview_rect(surface: glam::Vec2, padding: u32) -> Rect {
    let size = (surface - glam::Vec2::splat(padding as f32 * 2.0)).max(glam::Vec2::ONE);
    Rect::from_min_size((surface - size) * 0.5, size)
}

pub fn zoomed_preview_rect(fit: Rect, bounds: Rect, zoom: f32, center: glam::Vec2) -> Rect {
    let size = fit.size() * zoom;
    let overflow = (size - bounds.size()).max(glam::Vec2::ZERO) * 0.5;
    let min = (bounds.center() - center * size).clamp(
        bounds.center() - size * 0.5 - overflow,
        bounds.center() - size * 0.5 + overflow,
    );
    Rect::from_min_size(min, size)
}

pub fn preview_center(rect: Rect, bounds: Rect) -> glam::Vec2 {
    (bounds.center() - rect.min) / rect.size()
}

pub fn preview_dragged(origin: glam::Vec2, position: glam::Vec2) -> bool {
    origin.distance_squared(position) >= PREVIEW_DRAG_THRESHOLD * PREVIEW_DRAG_THRESHOLD
}

pub fn preview_zoom(zoom: f32, steps: f32) -> f32 {
    (zoom * PREVIEW_ZOOM_PER_STEP.powf(-steps)).clamp(MIN_PREVIEW_ZOOM, MAX_PREVIEW_ZOOM)
}

pub fn cursor_zoom_rect(rect: Rect, previous_zoom: f32, zoom: f32, cursor: glam::Vec2) -> Rect {
    let ratio = zoom / previous_zoom;
    Rect::from_min_size(cursor + (rect.min - cursor) * ratio, rect.size() * ratio)
}
