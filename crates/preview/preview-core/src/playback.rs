//! Playback display and control timing shared by native preview adapters.
use std::time::Duration;

pub const STEP_REPEAT_TICK: Duration = Duration::from_millis(200);
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;
const LOADING_INDICATOR_FRAME_RATE: u64 = 24;
pub const LOADING_INDICATOR_DELAY: Duration =
    Duration::from_nanos(NANOSECONDS_PER_SECOND / LOADING_INDICATOR_FRAME_RATE);

pub fn playback_speed_label(speed: shrimply_timeline_value::Fraction) -> String {
    use shrimply_timeline_value::{fraction_denominator, fraction_numerator};
    if fraction_denominator(speed) == 1 {
        format!("x{}", fraction_numerator(speed))
    } else {
        format!(
            "x{}/{}",
            fraction_numerator(speed),
            fraction_denominator(speed)
        )
    }
}

pub fn rendered_frame_rate_label(render_elapsed: Duration) -> Option<String> {
    use shrimply_math_core::{fraction_round_nonnegative_u64, frame_rate_from_duration};
    const MAX_DISPLAYED_FRAME_RATE: u64 = 999;
    frame_rate_from_duration(render_elapsed).map(|frame_rate| {
        fraction_round_nonnegative_u64(frame_rate)
            .min(MAX_DISPLAYED_FRAME_RATE)
            .to_string()
    })
}
