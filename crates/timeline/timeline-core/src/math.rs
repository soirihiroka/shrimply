use shrimply_math_core::{BigFraction, fraction_from_f64};
pub use shrimply_math_media::*;

pub fn toggle_timeline_zoom(
    view: &mut crate::TimelineViewState,
    duration: Time,
    position: Time,
    width: f64,
    minimum: f64,
) {
    const DETAIL_SCALE: f64 = 0.05;
    const MAX_DETAIL_SECONDS: i64 = 10;
    const GLOBAL_THRESHOLD: f64 = 0.8;
    if width <= 0.0 {
        return;
    }
    let global = (duration.as_secs_f64() / width).clamp(minimum, crate::MAX_SECONDS_PER_PIXEL);
    let detail = (global * DETAIL_SCALE)
        .min(Time::from_seconds(MAX_DETAIL_SECONDS).as_secs_f64() / width)
        .max(minimum);
    let zoomed_in = view.seconds_per_pixel < global * GLOBAL_THRESHOLD;
    view.seconds_per_pixel = if zoomed_in { global } else { detail };
    view.scroll_seconds = if zoomed_in {
        0.0
    } else {
        (position.as_secs_f64() - width * detail / 2.0).max(0.0)
    };
}

pub fn time_at_x(view: crate::view::TimelineViewState, x: f64) -> Time {
    Time::from_big_fraction(
        BigFraction::from_fraction(fraction_from_f64(view.scroll_seconds))
            + BigFraction::from_fraction(fraction_from_f64(x - crate::geometry::timeline_x()))
                * BigFraction::from_fraction(fraction_from_f64(view.seconds_per_pixel)),
    )
    .max(Time::ZERO)
}

pub fn snap_distance(view: crate::view::TimelineViewState, radius_pixels: f64) -> Time {
    Time::from_big_fraction(
        BigFraction::from_fraction(fraction_from_f64(view.seconds_per_pixel))
            * BigFraction::from_fraction(fraction_from_f64(radius_pixels)),
    )
}

/// Keep the time under the pointer fixed as the scale changes.
pub fn zoom_at_x(
    view: &mut crate::view::TimelineViewState,
    x: f64,
    factor: f64,
    minimum_seconds_per_pixel: f64,
) {
    if !factor.is_finite() || factor <= 0.0 {
        return;
    }
    let x = x.max(crate::geometry::timeline_x());
    let anchor = time_at_x(*view, x);
    view.seconds_per_pixel = (view.seconds_per_pixel / factor).clamp(
        minimum_seconds_per_pixel,
        crate::metrics::MAX_SECONDS_PER_PIXEL,
    );
    view.scroll_seconds = Time::from_big_fraction(
        BigFraction::from_fraction(anchor.seconds)
            - BigFraction::from_fraction(fraction_from_f64(x - crate::geometry::timeline_x()))
                * BigFraction::from_fraction(fraction_from_f64(view.seconds_per_pixel)),
    )
    .max(Time::ZERO)
    .as_secs_f64();
}

pub fn scroll_zoom_factor(delta: f64) -> f64 {
    (delta / crate::metrics::SCROLL_PIXELS_PER_STEP).exp()
}

pub fn pinch_zoom_factor(magnification: f64) -> f64 {
    magnification.exp()
}

pub fn scrollbar_wheel_pages(delta: f64) -> f64 {
    const PAGE_FRACTION_PER_STEP: f64 = 0.25;
    delta / crate::metrics::SCROLL_PIXELS_PER_STEP * PAGE_FRACTION_PER_STEP
}

pub(crate) fn track_row_at_y(y: f64) -> Option<usize> {
    (y.is_finite() && y >= crate::metrics::RULER_HEIGHT).then(|| {
        ((y - crate::metrics::RULER_HEIGHT) / crate::metrics::TRACK_HEIGHT).floor() as usize
    })
}
