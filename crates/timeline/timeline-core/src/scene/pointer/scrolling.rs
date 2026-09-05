use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_scroll(
    view: &mut TimelineViewState,
    delta: Vec2,
    ctrl: bool,
    pointer: Option<Vec2>,
    timeline_width: f64,
    height: f64,
    track_content_height: f64,
    duration_seconds: f64,
    frame_step_seconds: f64,
) -> Option<(TimelineOverscrollEdge, f64)> {
    if delta.x.abs().max(delta.y.abs()) <= f32::EPSILON {
        return None;
    }

    let min_seconds_per_pixel = min_seconds_per_pixel(frame_step_seconds);
    view.clamp(
        duration_seconds,
        timeline_width,
        min_seconds_per_pixel,
        track_content_height,
        height,
    );
    let pointer_x = pointer
        .map(|pointer| f64::from(pointer.x))
        .unwrap_or_else(|| timeline_x() + timeline_width / 2.0);

    if ctrl {
        if pointer_x < timeline_x() {
            return None;
        }
        let pointer_seconds = x_to_time(pointer_x, view.scroll_seconds, view.seconds_per_pixel);
        let delta = if delta.x.abs() > delta.y.abs() {
            delta.x as f64
        } else {
            delta.y as f64
        };
        let zoom = if delta < 0.0 { 0.8 } else { 1.25 };
        view.seconds_per_pixel =
            (view.seconds_per_pixel * zoom).clamp(min_seconds_per_pixel, MAX_SECONDS_PER_PIXEL);
        view.scroll_seconds = pointer_seconds - (pointer_x - timeline_x()) * view.seconds_per_pixel;
        view.clamp(
            duration_seconds,
            timeline_width,
            min_seconds_per_pixel,
            track_content_height,
            height,
        );
        None
    } else {
        let delta = if delta.x.abs() > f32::EPSILON {
            delta.x as f64
        } else {
            delta.y as f64
        };
        let target = view.scroll_seconds + delta * view.seconds_per_pixel;
        let max_scroll =
            max_horizontal_scroll_seconds(duration_seconds, timeline_width, view.seconds_per_pixel);
        let overscroll = if target < 0.0 {
            Some((
                TimelineOverscrollEdge::Left,
                (-target / view.seconds_per_pixel)
                    .clamp(1.0, shrimply_skia_adw_core::OVERSHOOT_MAX_DISTANCE),
            ))
        } else if target > max_scroll {
            Some((
                TimelineOverscrollEdge::Right,
                ((target - max_scroll) / view.seconds_per_pixel)
                    .clamp(1.0, shrimply_skia_adw_core::OVERSHOOT_MAX_DISTANCE),
            ))
        } else {
            None
        };
        view.scroll_seconds = target;
        view.clamp(
            duration_seconds,
            timeline_width,
            min_seconds_per_pixel,
            track_content_height,
            height,
        );
        overscroll
    }
}

pub(super) fn max_horizontal_scroll_seconds(
    duration_seconds: f64,
    timeline_width: f64,
    seconds_per_pixel: f64,
) -> f64 {
    let visible_seconds = timeline_width * seconds_per_pixel;
    duration_seconds.max(visible_seconds) + visible_seconds
}

pub(super) fn begin_slider_action(
    view: &mut TimelineViewState,
    scrollbar_lifecycle: &mut shrimply_skia_adw_core::slider::Lifecycle,
    x: f64,
    y: f64,
    timeline_width: f64,
    height: f64,
    duration_seconds: f64,
) -> (DragMode, bool) {
    let scrollbar = horizontal_scrollbar(
        *view,
        timeline_width,
        height,
        duration_seconds,
        shrimply_skia_adw_core::slider::idle_state(),
    );
    let mut scroll_seconds = view.scroll_seconds;
    match scrollbar_lifecycle.begin(scrollbar, Vec2::new(x as f32, y as f32), |value| {
        scroll_seconds = value;
    }) {
        shrimply_skia_adw_core::slider::Begin::Drag => {
            view.scroll_seconds = scroll_seconds;
            (DragMode::SliderMove, false)
        }
        shrimply_skia_adw_core::slider::Begin::None => (DragMode::None, false),
    }
}

pub(super) fn begin_vertical_slider_action(
    view: &mut TimelineViewState,
    scrollbar_lifecycle: &mut shrimply_skia_adw_core::slider::Lifecycle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    track_content_height: f64,
) -> (DragMode, bool) {
    let Some(scrollbar) = vertical_scrollbar(
        *view,
        width,
        height,
        track_content_height,
        shrimply_skia_adw_core::slider::idle_state(),
    ) else {
        return (DragMode::None, false);
    };

    let mut scroll_y = view.scroll_y;
    match scrollbar_lifecycle.begin(scrollbar, Vec2::new(x as f32, y as f32), |value| {
        scroll_y = value;
    }) {
        shrimply_skia_adw_core::slider::Begin::Drag => {
            view.scroll_y = scroll_y;
            (DragMode::VerticalSliderMove, false)
        }
        shrimply_skia_adw_core::slider::Begin::None => (DragMode::None, false),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update_slider_drag(
    view: &mut TimelineViewState,
    scrollbar_lifecycle: &mut shrimply_skia_adw_core::slider::Lifecycle,
    x: f64,
    timeline_width: f64,
    height: f64,
    track_content_height: f64,
    duration_seconds: f64,
    min_seconds_per_pixel: f64,
    drag_mode: DragMode,
) {
    if !matches!(drag_mode, DragMode::SliderMove) {
        return;
    }

    let scrollbar = horizontal_scrollbar(
        *view,
        timeline_width,
        height,
        duration_seconds,
        shrimply_skia_adw_core::slider::idle_state(),
    );
    let mut scroll_seconds = view.scroll_seconds;
    scrollbar_lifecycle.drag_by(scrollbar, x - view.drag_start_x, |value| {
        scroll_seconds = value;
    });
    view.scroll_seconds = scroll_seconds;
    view.clamp(
        duration_seconds,
        timeline_width,
        min_seconds_per_pixel,
        track_content_height,
        height,
    );
}

pub(super) fn update_vertical_slider_drag(
    view: &mut TimelineViewState,
    scrollbar_lifecycle: &mut shrimply_skia_adw_core::slider::Lifecycle,
    y: f64,
    width: f64,
    height: f64,
    track_content_height: f64,
) {
    let Some(scrollbar) = vertical_scrollbar(
        *view,
        width,
        height,
        track_content_height,
        shrimply_skia_adw_core::slider::idle_state(),
    ) else {
        return;
    };
    let mut scroll_y = view.scroll_y;
    scrollbar_lifecycle.drag_by(scrollbar, y - view.drag_start_y, |value| {
        scroll_y = value;
    });
    view.scroll_y = scroll_y;
    view.clamp_y(track_content_height, height);
}
