use super::*;

pub fn timeline_track_content_height(
    project: &Project,
    virtual_tracks: &[(TrackKind, usize)],
) -> f64 {
    (items::track_rows(project).len() + virtual_tracks.len() + 1).max(1) as f64 * TRACK_HEIGHT
}

pub fn visible_track_height(height: f64) -> f64 {
    (height - RULER_HEIGHT).max(0.0)
}

pub fn max_scroll_y(track_content_height: f64, height: f64) -> f64 {
    (track_content_height - visible_track_height(height)).max(0.0)
}

pub fn timeline_x() -> f64 {
    LABEL_WIDTH + TIMELINE_PADDING_LEFT
}

pub fn timeline_width(width: f64) -> f64 {
    (width - timeline_x() - TIMELINE_PADDING_RIGHT).max(0.0)
}

pub fn vertical_scrollbar(
    view: TimelineViewState,
    width: f64,
    height: f64,
    track_content_height: f64,
    state: shrimply_components_skia::ScrollbarState,
) -> Option<shrimply_components_skia::Scrollbar> {
    let visible_height = visible_track_height(height);
    if visible_height <= 0.0 || track_content_height <= visible_height {
        return None;
    }

    Some(shrimply_components_skia::Scrollbar {
        axis: shrimply_components_skia::Axis::Vertical,
        bounds: scrollbar_bounds(timeline_width(width), height),
        content_length: track_content_height,
        viewport_length: visible_height,
        value: view.scroll_y,
        color: crate::theme::current().view_fg,
        outline_color: crate::theme::current().scrollbar_outline,
        state,
    })
}

pub fn horizontal_scrollbar(
    view: TimelineViewState,
    timeline_width: f64,
    height: f64,
    duration_seconds: f64,
    state: shrimply_components_skia::ScrollbarState,
) -> shrimply_components_skia::Scrollbar {
    let visible_seconds = timeline_width * view.seconds_per_pixel;
    shrimply_components_skia::Scrollbar {
        axis: shrimply_components_skia::Axis::Horizontal,
        bounds: scrollbar_bounds(timeline_width, height),
        content_length: duration_seconds.max(visible_seconds) + visible_seconds * 2.0,
        viewport_length: visible_seconds,
        value: view.scroll_seconds,
        color: crate::theme::current().view_fg,
        outline_color: crate::theme::current().scrollbar_outline,
        state,
    }
}

pub fn ruler_scale(view: TimelineViewState) -> ruler::TimelineScale {
    ruler::TimelineScale::new(
        Time::from_seconds_f64(view.scroll_seconds),
        Fraction::from(view.seconds_per_pixel),
    )
}

pub fn scrollbar_bounds(timeline_width: f64, height: f64) -> shrimply_components_skia::Rect {
    shrimply_components_skia::Rect::from_xywh(
        timeline_x() as f32,
        RULER_HEIGHT as f32,
        timeline_width.max(0.0) as f32,
        visible_track_height(height) as f32,
    )
}

pub fn x_to_time(x: f64, scroll_seconds: f64, seconds_per_pixel: f64) -> f64 {
    scroll_seconds + (x - timeline_x()) * seconds_per_pixel
}

pub use crate::math::{frame_width, time_to_x};

pub fn frame_step_seconds(project: &Project) -> f64 {
    frame_step(project).as_secs_f64()
}

pub fn frame_step(project: &Project) -> Time {
    project.frame_step()
}

pub fn waveform_chunks_per_second_from_frame_step(frame_step_seconds: f64) -> u32 {
    if frame_step_seconds.is_finite() && frame_step_seconds > 0.0 {
        (f64::from(WAVEFORM_CHUNKS_PER_FRAME) / frame_step_seconds)
            .round()
            .clamp(1.0, f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

pub fn min_seconds_per_pixel(frame_step_seconds: f64) -> f64 {
    (frame_step_seconds / MAX_FRAME_PIXEL_WIDTH).clamp(MIN_SECONDS_PER_PIXEL, MAX_SECONDS_PER_PIXEL)
}

pub fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect::from_min_size(
        vec2(x as f32, y as f32),
        vec2(width.max(0.0) as f32, height.max(0.0) as f32),
    )
}
