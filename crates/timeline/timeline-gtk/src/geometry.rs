use super::*;

pub(super) fn timeline_track_content_height(
    project: &Project,
    virtual_tracks: &[(TrackKind, usize)],
) -> f64 {
    (items::track_rows(project).len() + virtual_tracks.len() + 1).max(1) as f64 * TRACK_HEIGHT
}

pub(super) fn visible_track_height(height: f64) -> f64 {
    (height - RULER_HEIGHT).max(0.0)
}

pub(super) fn max_scroll_y(track_content_height: f64, height: f64) -> f64 {
    (track_content_height - visible_track_height(height)).max(0.0)
}

pub(crate) fn timeline_x() -> f64 {
    LABEL_WIDTH + TIMELINE_PADDING_LEFT
}

pub(crate) fn timeline_width(width: f64) -> f64 {
    (width - timeline_x() - TIMELINE_PADDING_RIGHT).max(0.0)
}

pub(super) fn vertical_scrollbar(
    view: TimelineViewState,
    width: f64,
    height: f64,
    track_content_height: f64,
    state: shrimply_skia_adw_core::ScrollbarState,
) -> Option<shrimply_skia_adw_core::Scrollbar> {
    let visible_height = visible_track_height(height);
    if visible_height <= 0.0 || track_content_height <= visible_height {
        return None;
    }

    Some(shrimply_skia_adw_core::Scrollbar {
        axis: shrimply_skia_adw_core::Axis::Vertical,
        bounds: scrollbar_bounds(timeline_width(width), height),
        content_length: track_content_height,
        viewport_length: visible_height,
        value: view.scroll_y,
        color: crate::theme::current().view_fg,
        outline_color: crate::theme::current().scrollbar_outline,
        state,
    })
}

pub(super) fn horizontal_scrollbar(
    view: TimelineViewState,
    timeline_width: f64,
    height: f64,
    duration_seconds: f64,
    state: shrimply_skia_adw_core::ScrollbarState,
) -> shrimply_skia_adw_core::Scrollbar {
    let visible_seconds = timeline_width * view.seconds_per_pixel;
    shrimply_skia_adw_core::Scrollbar {
        axis: shrimply_skia_adw_core::Axis::Horizontal,
        bounds: scrollbar_bounds(timeline_width, height),
        content_length: duration_seconds.max(visible_seconds) + visible_seconds * 2.0,
        viewport_length: visible_seconds,
        value: view.scroll_seconds,
        color: crate::theme::current().view_fg,
        outline_color: crate::theme::current().scrollbar_outline,
        state,
    }
}

pub(super) fn ruler_scale(view: TimelineViewState) -> ruler::TimelineScale {
    ruler::TimelineScale::new(view.scroll_seconds, view.seconds_per_pixel)
}

pub(super) fn scrollbar_bounds(timeline_width: f64, height: f64) -> shrimply_skia_adw_core::Rect {
    shrimply_skia_adw_core::Rect::from_xywh(
        timeline_x() as f32,
        RULER_HEIGHT as f32,
        timeline_width.max(0.0) as f32,
        visible_track_height(height) as f32,
    )
}

pub(super) fn x_to_time(x: f64, scroll_seconds: f64, seconds_per_pixel: f64) -> f64 {
    scroll_seconds + (x - timeline_x()) * seconds_per_pixel
}

pub(super) fn time_to_x(time_seconds: f64, x: f64, view: TimelineViewState) -> f64 {
    ruler_scale(view).time_to_x(time_seconds, x)
}

pub(super) fn frame_step_seconds(project: &Project) -> f64 {
    frame_step(project).as_secs_f64()
}

pub(super) fn frame_step(project: &Project) -> Time {
    project.frame_step()
}

pub(crate) fn waveform_chunks_per_second_from_frame_step(frame_step_seconds: f64) -> u32 {
    if frame_step_seconds.is_finite() && frame_step_seconds > 0.0 {
        (f64::from(WAVEFORM_CHUNKS_PER_FRAME) / frame_step_seconds)
            .round()
            .clamp(1.0, f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

pub(super) fn min_seconds_per_pixel(frame_step_seconds: f64) -> f64 {
    (frame_step_seconds / MAX_FRAME_PIXEL_WIDTH).clamp(MIN_SECONDS_PER_PIXEL, MAX_SECONDS_PER_PIXEL)
}

pub(super) fn frame_width(view: TimelineViewState, frame_step_seconds: f64) -> f64 {
    ruler_scale(view).frame_width(frame_step_seconds)
}

pub(super) fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect::from_min_size(
        vec2(x as f32, y as f32),
        vec2(width.max(0.0) as f32, height.max(0.0) as f32),
    )
}
