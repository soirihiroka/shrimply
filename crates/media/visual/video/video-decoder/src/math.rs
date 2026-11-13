use ffmpeg_next::{self as ffmpeg, sys};
use shrimply_math_core::{Fraction, Time, fraction_ratio_i128};

pub(crate) fn frame_time(
    frame: &ffmpeg::frame::Video,
    time_base: ffmpeg::Rational,
    stream_start_time: i64,
    frame_index: usize,
    last_position: Option<Time>,
    frame_duration: Time,
    timestamp_required: bool,
) -> Result<Time, String> {
    let Some(timestamp) = frame.timestamp() else {
        if let Some(last_position) = last_position {
            return Ok(last_position.saturating_add(frame_duration));
        }
        if timestamp_required {
            return Err(
                "video decoder returned a frame without a timestamp after random access"
                    .to_string(),
            );
        }
        return Ok(Time {
            seconds: frame_duration.seconds
                * Fraction::from(
                    u64::try_from(frame_index).expect("decoded frame index exceeds u64"),
                ),
        });
    };
    let start_time = if stream_start_time == sys::AV_NOPTS_VALUE {
        0
    } else {
        stream_start_time
    };
    Ok(Time::from_fraction(
        timestamp
            .saturating_sub(start_time)
            .saturating_mul(i64::from(time_base.numerator())),
        i64::from(time_base.denominator()),
    ))
}

pub(crate) fn source_time_to_stream_timestamp(
    position: Time,
    time_base: ffmpeg::Rational,
    stream_start_time: i64,
) -> i64 {
    let (position_numerator, position_denominator) =
        fraction_ratio_i128(position.seconds).expect("decoder position must be finite");
    let numerator = position_numerator.saturating_mul(i128::from(time_base.denominator()));
    let denominator = position_denominator.saturating_mul(i128::from(time_base.numerator()));
    let timestamp = if denominator == 0 {
        0
    } else {
        numerator / denominator
    };
    let timestamp = if stream_start_time == sys::AV_NOPTS_VALUE {
        timestamp
    } else {
        timestamp.saturating_add(i128::from(stream_start_time))
    };
    timestamp.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}
