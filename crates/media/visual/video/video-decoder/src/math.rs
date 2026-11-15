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

/// Match OpenCV's display-matrix angle convention and cvRound tie handling.
pub(crate) fn display_rotation(data: &[u8]) -> Option<i32> {
    const ELEMENTS: usize = 9;
    let bytes = data.get(..ELEMENTS * size_of::<i32>())?;
    let matrix: [i32; ELEMENTS] = std::array::from_fn(|index| {
        i32::from_ne_bytes(
            bytes[index * 4..(index + 1) * 4]
                .try_into()
                .expect("display matrix element"),
        )
    });
    let angle = unsafe { sys::av_display_rotation_get(matrix.as_ptr()) };
    angle
        .is_finite()
        .then(|| (-(angle.round_ties_even() as i32)).rem_euclid(360))
}

/// OpenCV's frame-number seek rounds PTS to the nearest nominal frame. Request
/// its interval midpoint so a container's rounded time base does not select
/// the preceding frame when the nominal boundary falls just before its PTS.
pub(crate) fn frame_center_time(frame: u64, frame_rate: Fraction) -> Option<Time> {
    let start = shrimply_math_core::time_from_frame(frame, frame_rate)?;
    let duration = shrimply_math_core::time_from_frame(1, frame_rate)?;
    Some(Time {
        seconds: start.seconds + duration.seconds / Fraction::from(2_u64),
    })
}

/// VideoCapture fills forward from a preroll frame to recover random-access units.
pub(crate) fn frame_seek_time(frame: u64, preroll: u64, frame_rate: Fraction) -> Option<Time> {
    shrimply_math_core::time_from_frame(frame.saturating_sub(preroll), frame_rate)
}

/// OpenCV get_fps accepts a metadata rate at or above eps_zero = 0.000025.
pub(crate) fn usable_metadata_frame_rate(rate: ffmpeg::Rational) -> bool {
    const MINIMUM_RATE_RECIPROCAL: i64 = 40_000;
    rate.numerator() > 0
        && rate.denominator() > 0
        && i64::from(rate.numerator()) * MINIMUM_RATE_RECIPROCAL >= i64::from(rate.denominator())
}
