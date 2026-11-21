use ffmpeg::packet::Mut as _;
use ffmpeg_next as ffmpeg;
use libc::EAGAIN;
use shrimply_project_document::project::{AudioItem, AudioSpeedMethod, RepeatStrategy, Time};
use std::path::Path;

const CHANNELS: usize = 2;
const SAMPLE_RATE: u32 = 48_000;
const AV_TIME_BASE: i64 = 1_000_000;
const OPUS_BIT_RATE: usize = 192_000;
const OPUS_DEFAULT_FRAME_SIZE: usize = 960;

pub(super) fn transcode(input: &Path, output: &Path) -> Result<(), String> {
    ffmpeg::init().map_err(|error| format!("Could not initialize FFmpeg: {error}"))?;
    let duration = duration(input)?;
    let item = AudioItem::builder(Time::ZERO, duration)
        .source_duration(duration)
        .repeat_strategy(RepeatStrategy::Empty)
        .speed_method(AudioSpeedMethod::Naive)
        .file(input)
        .build();
    let mut renderer = super::streaming::OfflineAudioRenderer::new(&item, SAMPLE_RATE)?;
    let samples = renderer.render(&item, Time::ZERO, duration)?;
    encode(&samples, output)
}

fn duration(path: &Path) -> Result<Time, String> {
    let input = shrimply_media_info::demux::open(path)
        .map_err(|error| format!("Could not open Pneuma output: {error}"))?;
    let duration = input.duration();
    if duration > 0 {
        return Ok(Time::from_fraction(duration, AV_TIME_BASE));
    }
    let stream = input
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .ok_or_else(|| "Pneuma output has no audio stream".to_string())?;
    let duration = stream.duration();
    if duration <= 0 {
        return Err("Pneuma output does not report its duration".to_string());
    }
    let time_base = stream.time_base();
    let numerator = i128::from(duration) * i128::from(time_base.numerator());
    let numerator = numerator.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
    Ok(Time::from_fraction(
        numerator,
        i64::from(time_base.denominator()),
    ))
}

fn encode(samples: &[f32], path: &Path) -> Result<(), String> {
    let codec = ffmpeg::codec::encoder::find_by_name("libopus")
        .ok_or_else(|| "FFmpeg encoder libopus was not found".to_string())?;
    let sample_format = ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed);
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .audio()
        .map_err(|error| format!("Could not configure Opus encoder: {error}"))?;
    encoder.set_rate(SAMPLE_RATE as i32);
    encoder.set_channel_layout(ffmpeg::channel_layout::ChannelLayout::STEREO);
    encoder.set_format(sample_format);
    encoder.set_time_base(ffmpeg::Rational(1, SAMPLE_RATE as i32));
    encoder.set_bit_rate(OPUS_BIT_RATE);

    let mut output = ffmpeg::format::output(path)
        .map_err(|error| format!("Could not create Pneuma Opus cache: {error}"))?;
    if output
        .format()
        .flags()
        .contains(ffmpeg::format::Flags::GLOBAL_HEADER)
    {
        unsafe {
            (*encoder.as_mut_ptr()).flags |= ffmpeg::sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    let mut encoder = encoder
        .open_as(codec)
        .map_err(|error| format!("Could not open Opus encoder: {error}"))?;
    let stream_index = {
        let mut stream = output
            .add_stream_with(encoder.as_ref())
            .map_err(|error| format!("Could not add Opus cache stream: {error}"))?;
        stream.set_time_base(ffmpeg::Rational(1, SAMPLE_RATE as i32));
        stream.index()
    };
    output
        .write_header()
        .map_err(|error| format!("Could not write Opus cache header: {error}"))?;
    let stream_time_base = output
        .stream(stream_index)
        .expect("new Opus stream is missing")
        .time_base();
    let frame_size = usize::try_from(encoder.frame_size())
        .ok()
        .filter(|size| *size > 0)
        .unwrap_or(OPUS_DEFAULT_FRAME_SIZE);
    let total_frames = samples.len() / CHANNELS;
    let mut offset = 0;
    let mut pts = 0;
    while offset < total_frames {
        let frames = frame_size.min(total_frames - offset);
        let mut frame = ffmpeg::frame::Audio::new(
            sample_format,
            frame_size,
            ffmpeg::channel_layout::ChannelLayout::STEREO,
        );
        frame.set_rate(SAMPLE_RATE);
        frame.set_pts(Some(pts));
        for (index, sample) in frame.plane_mut::<(f32, f32)>(0).iter_mut().enumerate() {
            *sample = if index < frames {
                (
                    samples[(offset + index) * CHANNELS].clamp(-1.0, 1.0),
                    samples[(offset + index) * CHANNELS + 1].clamp(-1.0, 1.0),
                )
            } else {
                (0.0, 0.0)
            };
        }
        encoder
            .send_frame(&frame)
            .map_err(|error| format!("Could not encode Pneuma Opus cache: {error}"))?;
        write_packets(&mut encoder, &mut output, stream_index, stream_time_base)?;
        offset += frames;
        pts += frame_size as i64;
    }
    encoder
        .send_eof()
        .map_err(|error| format!("Could not finalize Pneuma Opus cache: {error}"))?;
    write_packets(&mut encoder, &mut output, stream_index, stream_time_base)?;
    output
        .write_trailer()
        .map_err(|error| format!("Could not finish Pneuma Opus cache: {error}"))
}

fn write_packets(
    encoder: &mut ffmpeg::codec::encoder::audio::Encoder,
    output: &mut ffmpeg::format::context::Output,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
) -> Result<(), String> {
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                packet.set_stream(stream_index);
                packet.rescale_ts(encoder.time_base(), stream_time_base);
                if packet.size() == 0 {
                    let result = unsafe {
                        ffmpeg::sys::av_interleaved_write_frame(
                            output.as_mut_ptr(),
                            packet.as_mut_ptr(),
                        )
                    };
                    if result != 0 {
                        return Err(format!(
                            "Could not write Pneuma Opus cache: {}",
                            ffmpeg::Error::from(result)
                        ));
                    }
                } else {
                    packet
                        .write_interleaved(output)
                        .map_err(|error| format!("Could not write Pneuma Opus cache: {error}"))?;
                }
            }
            Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => return Ok(()),
            Err(ffmpeg::Error::Eof) => return Ok(()),
            Err(error) => return Err(format!("Could not receive Opus packet: {error}")),
        }
    }
}
