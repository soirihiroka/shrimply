use shrimply_math_core::{Fraction, Time, fraction_floor_i64};

pub fn timestamp(time: Time, base: ffmpeg_next::Rational) -> i64 {
    fraction_floor_i64(
        time.seconds * Fraction::from(base.denominator()) / Fraction::from(base.numerator()),
    )
    .expect("media timestamp exceeds i64")
}

pub fn frame_time(timestamp: i64, base: ffmpeg_next::Rational) -> Time {
    Time {
        seconds: Fraction::from(timestamp) * Fraction::from(base.numerator())
            / Fraction::from(base.denominator()),
    }
}
