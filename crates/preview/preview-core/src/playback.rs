//! Playback display and control timing shared by native preview adapters.
use std::time::Duration;

pub const STEP_REPEAT_TICK: Duration = Duration::from_millis(200);

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
