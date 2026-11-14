use shrimply_math_core::{Fraction, fraction_as_f64};

const TARGET_TICK_PIXELS: f64 = 120.0;

pub fn visible_time_bounds(
    view: crate::Viewport,
    width: f64,
) -> (shrimply_math_core::Time, shrimply_math_core::Time) {
    use shrimply_math_core::Time;
    let margin = Fraction::from(crate::ITEM_BORDER_STROKE_WIDTH);
    let scale = view.scale;
    (
        scale.scroll_seconds.saturating_sub(Time {
            seconds: margin * scale.seconds_per_pixel,
        }),
        scale.scroll_seconds.saturating_add(Time {
            seconds: (Fraction::from(width.max(0.0)) + margin) * scale.seconds_per_pixel,
        }),
    )
}

pub fn nice_tick_step(seconds_per_pixel: Fraction) -> Fraction {
    // Preserve the desktop ruler's rounding at visual zoom thresholds. Only
    // spacing selection uses drawing coordinates; tick timestamps stay exact.
    let raw = (fraction_as_f64(seconds_per_pixel) * TARGET_TICK_PIXELS).max(0.001);
    let magnitude = 10_f64.powf(raw.log10().floor());
    for multiplier in [1.0, 2.0, 5.0, 10.0] {
        let step = magnitude * multiplier;
        if step >= raw {
            return Fraction::from(step);
        }
    }
    Fraction::from(magnitude * 10.0)
}

pub fn item_rect(
    start: shrimply_math_core::Time,
    end: shrimply_math_core::Time,
    x: f64,
    view: impl Into<crate::Viewport>,
) -> (f64, f64) {
    let view = view.into();
    let left = view.scale.time_to_x(start, x);
    let right = view.scale.time_to_x(end, x);
    (left, (right - left).max(crate::MIN_ITEM_PIXEL_WIDTH))
}

pub fn preview_media_time_offset(
    offset: shrimply_math_core::Time,
    old_start: shrimply_math_core::Time,
    new_start: shrimply_math_core::Time,
    playback_speed: Fraction,
    _repeat_strategy: crate::project::RepeatStrategy,
    _source_duration: shrimply_math_core::Time,
) -> shrimply_math_core::Time {
    offset.saturating_add(crate::project::scaled_time_delta(
        new_start.signed_sub(old_start),
        playback_speed,
    ))
}

pub fn row_screen_y(row: usize, view: crate::Viewport) -> f64 {
    crate::RULER_HEIGHT + row as f64 * crate::TRACK_HEIGHT - view.scroll_y
}

pub fn track_label_button_y(row_y: f64) -> f64 {
    row_y + ((crate::TRACK_HEIGHT - crate::TRACK_LABEL_BUTTON_SIZE) / 2.0).max(0.0)
}
