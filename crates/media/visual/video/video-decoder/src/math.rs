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

/// Frame-number timestamp conventions from OpenCV's cap_ffmpeg_impl.hpp.
/// Display timestamps are normalized once, then mapped with trunc(fps * sec + 0.5).
#[derive(Clone, Copy)]
pub(crate) struct FrameNumberClock {
    rate: Fraction,
    time_base: ffmpeg::Rational,
    stream_start_time: i64,
    timestamp_offset: i64,
    first_frame_number: i64,
}

const MICROSECOND_TIME_BASE: sys::AVRational = sys::AVRational {
    num: 1,
    den: sys::AV_TIME_BASE,
};

impl FrameNumberClock {
    pub(crate) fn new(
        first: &ffmpeg::frame::Video,
        rate: Fraction,
        time_base: ffmpeg::Rational,
        stream_start_time: i64,
        container_start_time: i64,
    ) -> Option<Self> {
        let timestamp = first.pts().unwrap_or_else(|| first.packet().dts);
        if timestamp == sys::AV_NOPTS_VALUE {
            return None;
        }
        let mut minimum_start = container_start_time;
        if minimum_start == sys::AV_NOPTS_VALUE {
            minimum_start = i64::MAX;
        }
        if stream_start_time != sys::AV_NOPTS_VALUE {
            minimum_start = minimum_start.min(unsafe {
                sys::av_rescale_q(stream_start_time, time_base.into(), MICROSECOND_TIME_BASE)
            });
        }
        minimum_start = minimum_start
            .min(unsafe { sys::av_rescale_q(timestamp, time_base.into(), MICROSECOND_TIME_BASE) });
        let mut clock = Self {
            rate,
            time_base,
            stream_start_time,
            timestamp_offset: minimum_start.min(0).checked_neg()?,
            first_frame_number: 0,
        };
        clock.first_frame_number = clock.frame_number(first)?;
        Some(clock)
    }

    fn frame_number(&self, frame: &ffmpeg::frame::Video) -> Option<i64> {
        let timestamp = frame.pts().unwrap_or_else(|| frame.packet().dts);
        if timestamp == sys::AV_NOPTS_VALUE {
            return None;
        }
        let timestamp =
            unsafe { sys::av_rescale_q(timestamp, self.time_base.into(), MICROSECOND_TIME_BASE) }
                .checked_add(self.timestamp_offset)?;
        let mut timestamp =
            unsafe { sys::av_rescale_q(timestamp, MICROSECOND_TIME_BASE, self.time_base.into()) };
        if self.timestamp_offset == 0 && self.stream_start_time != sys::AV_NOPTS_VALUE {
            timestamp = timestamp.checked_sub(self.stream_start_time)?;
        }
        let time = Time::from_fraction(
            timestamp.checked_mul(i64::from(self.time_base.numerator()))?,
            i64::from(self.time_base.denominator()),
        );
        rounded_tick(time, self.rate)
    }

    pub(crate) fn ordinal(&self, frame: &ffmpeg::frame::Video) -> Option<i64> {
        self.frame_number(frame)?
            .checked_sub(self.first_frame_number)
    }

    pub(crate) fn seek_timestamp(&self, index: u64, preroll: u64) -> Option<i64> {
        let position =
            shrimply_math_core::time_from_frame(index.saturating_sub(preroll), self.rate)?;
        let ticks = rounded_tick(
            position,
            shrimply_math_core::fraction_new(
                i64::from(self.time_base.denominator()),
                i64::from(self.time_base.numerator()),
            ),
        )?;
        let offset = unsafe {
            sys::av_rescale_q(
                self.timestamp_offset,
                MICROSECOND_TIME_BASE,
                self.time_base.into(),
            )
        };
        let start = if self.stream_start_time == sys::AV_NOPTS_VALUE {
            0
        } else {
            self.stream_start_time
        };
        start.checked_add(ticks)?.checked_sub(offset)
    }
}

/// OpenCV get_total_frames uses stream nb_frames or rounded duration * FPS.
pub(crate) fn estimated_frame_count(
    container_duration: i64,
    stream_duration: i64,
    time_base: ffmpeg::Rational,
    rate: Fraction,
) -> Option<i64> {
    const MINIMUM_DURATION_MICROSECONDS: i64 = 25;
    let duration = if container_duration >= MINIMUM_DURATION_MICROSECONDS {
        Time::from_fraction(container_duration, i64::from(sys::AV_TIME_BASE))
    } else if stream_duration > 0 {
        Time::from_fraction(
            stream_duration.checked_mul(i64::from(time_base.numerator()))?,
            i64::from(time_base.denominator()),
        )
    } else {
        Time::ZERO
    };
    rounded_tick(duration, rate)
}

fn rounded_tick(time: Time, rate: Fraction) -> Option<i64> {
    let (time_numerator, time_denominator) = fraction_ratio_i128(time.seconds)?;
    let (rate_numerator, rate_denominator) = fraction_ratio_i128(rate)?;
    let denominator = time_denominator.checked_mul(rate_denominator)?;
    let numerator = time_numerator
        .checked_mul(rate_numerator)?
        .checked_mul(2)?
        .checked_add(denominator)?;
    i64::try_from(numerator / denominator.checked_mul(2)?).ok()
}

/// OpenCV get_fps accepts a metadata rate at or above eps_zero = 0.000025.
pub(crate) fn usable_metadata_frame_rate(rate: ffmpeg::Rational) -> bool {
    const MINIMUM_RATE_RECIPROCAL: i64 = 40_000;
    rate.numerator() > 0
        && rate.denominator() > 0
        && i64::from(rate.numerator()) * MINIMUM_RATE_RECIPROCAL >= i64::from(rate.denominator())
}
