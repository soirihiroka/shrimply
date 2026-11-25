use std::path::Path;

use ffmpeg::packet::Mut as _;
use ffmpeg_next as ffmpeg;
use libc::EAGAIN;

use shrimply_audio_engine::streaming;
use shrimply_project_document::project::Project;

const CHANNELS: usize = 2;
const SAMPLE_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Wav,
    Flac,
    Mp3,
    Ogg,
    Opus,
}

#[derive(Clone, Copy, Debug)]
pub enum ExportProgress {
    Mixing {
        completed_frames: u64,
        total_frames: u64,
    },
    Encoding {
        completed_frames: u64,
        total_frames: u64,
    },
}

impl From<ExportProgress> for crate::video::ExportProgress {
    fn from(progress: ExportProgress) -> Self {
        match progress {
            ExportProgress::Mixing {
                completed_frames,
                total_frames,
            } => Self::MixingAudio {
                current_frame: completed_frames,
                total_frames,
            },
            ExportProgress::Encoding {
                completed_frames,
                total_frames,
            } => Self::EncodingAudio {
                current_frame: completed_frames,
                total_frames,
            },
        }
    }
}

impl Format {
    pub const ALL: [Self; 5] = [Self::Wav, Self::Flac, Self::Mp3, Self::Ogg, Self::Opus];

    pub fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV",
            Self::Flac => "FLAC",
            Self::Mp3 => "MP3",
            Self::Ogg => "OGG Vorbis",
            Self::Opus => "Opus",
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Wav => "audio/wav",
            Self::Flac => "audio/flac",
            Self::Mp3 => "audio/mpeg",
            Self::Ogg | Self::Opus => "audio/ogg",
        }
    }

    pub fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Flac,
            2 => Self::Mp3,
            3 => Self::Ogg,
            4 => Self::Opus,
            _ => Self::Wav,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
            Self::Opus => "opus",
        }
    }
}

pub fn export(project: &Project, path: &Path, format: Format) -> Result<(), String> {
    export_with_progress(project, path, format, |_| true)
}

pub fn export_with_progress(
    project: &Project,
    path: &Path,
    format: Format,
    mut progress: impl FnMut(ExportProgress) -> bool,
) -> Result<(), String> {
    crate::ensure_output_is_not_an_asset(project, path)?;
    let assets = crate::snapshot_assets(project)?;
    let mut output_opened = false;
    let result = export_inner(
        project,
        format,
        &assets,
        || {
            let output = ffmpeg::format::output(path)
                .map_err(|error| format!("Could not create the {format:?} output file: {error}"))?;
            output_opened = true;
            Ok(output)
        },
        &mut progress,
    );
    if result.is_err() && output_opened {
        let _ = std::fs::remove_file(path);
    }
    result
}

pub fn export_to_stream(
    project: &Project,
    format: Format,
    open: impl FnOnce() -> Result<ffmpeg::format::context::StreamIo, String>,
    mut progress: impl FnMut(ExportProgress) -> bool,
) -> Result<(), String> {
    let container = match format {
        Format::Ogg | Format::Opus => "ogg",
        _ => format.extension(),
    };
    let assets = crate::snapshot_assets(project)?;
    export_inner(
        project,
        format,
        &assets,
        || {
            ffmpeg::format::output_to_stream(open()?, None, Some(container))
                .map_err(|error| format!("Could not create the {format:?} output file: {error}"))
        },
        &mut progress,
    )
}

fn export_inner(
    project: &Project,
    format: Format,
    assets: &[shrimply_project_document::project::AssetSnapshot],
    open: impl FnOnce() -> Result<ffmpeg::format::context::Output, String>,
    progress: &mut impl FnMut(ExportProgress) -> bool,
) -> Result<(), String> {
    ffmpeg::init().map_err(|error| format!("Could not initialize FFmpeg: {error}"))?;
    let total_frames = project.duration().as_sample_frame(SAMPLE_RATE);
    if total_frames == 0 {
        return Err("The selected audio has no duration.".to_string());
    }

    let samples =
        streaming::mix_project_offline(project, SAMPLE_RATE, |completed_frames, total_frames| {
            progress(ExportProgress::Mixing {
                completed_frames,
                total_frames,
            })
        })?;
    crate::ensure_assets_current(assets)?;

    if !progress(ExportProgress::Encoding {
        completed_frames: 0,
        total_frames,
    }) {
        return Err("Export cancelled".to_string());
    }

    let encoder_name = match format {
        Format::Wav => "pcm_s16le",
        Format::Flac => "flac",
        Format::Mp3 => "libmp3lame",
        Format::Ogg => "libvorbis",
        Format::Opus => "libopus",
    };
    let codec = ffmpeg::codec::encoder::find_by_name(encoder_name)
        .ok_or_else(|| format!("FFmpeg encoder {encoder_name} was not found"))?;
    let sample_format = match format {
        Format::Wav | Format::Flac => {
            ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed)
        }
        Format::Mp3 | Format::Ogg => {
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar)
        }
        Format::Opus => ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
    };
    let supports_sample_format = codec
        .audio()
        .map_err(|error| format!("{encoder_name} is not an audio encoder: {error}"))?
        .formats()
        .ok_or_else(|| {
            format!("The {encoder_name} encoder did not report supported sample formats")
        })?
        .any(|supported| supported == sample_format);
    if !supports_sample_format {
        return Err(format!(
            "The {encoder_name} encoder does not support the {sample_format:?} sample format required for {format:?} export"
        ));
    }
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .audio()
        .map_err(|error| format!("Could not configure the {encoder_name} encoder: {error}"))?;
    encoder.set_rate(SAMPLE_RATE as i32);
    encoder.set_channel_layout(ffmpeg::channel_layout::ChannelLayout::STEREO);
    encoder.set_format(sample_format);
    encoder.set_time_base(ffmpeg::Rational(1, SAMPLE_RATE as i32));
    if matches!(format, Format::Mp3 | Format::Ogg | Format::Opus) {
        encoder.set_bit_rate(192_000);
    }

    let mut output = open()?;
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
        .map_err(|error| format!("Could not open {encoder_name}: {error}"))?;
    let stream_index = {
        let mut stream = output
            .add_stream_with(encoder.as_ref())
            .map_err(|error| format!("Could not add the {format:?} audio stream: {error}"))?;
        stream.set_time_base(ffmpeg::Rational(1, SAMPLE_RATE as i32));
        stream.index()
    };
    output
        .write_header()
        .map_err(|error| format!("Could not write the {format:?} file header: {error}"))?;
    let stream_time_base = output.stream(stream_index).unwrap().time_base();
    let frame_size = usize::try_from(encoder.frame_size())
        .ok()
        .filter(|size| *size > 0)
        .unwrap_or(1024);
    let mut offset = 0;
    let mut pts = 0;
    while offset < total_frames as usize {
        crate::ensure_assets_current(assets)?;
        let frames = frame_size.min(total_frames as usize - offset);
        let mut frame = ffmpeg::frame::Audio::new(
            sample_format,
            frame_size,
            ffmpeg::channel_layout::ChannelLayout::STEREO,
        );
        frame.set_rate(SAMPLE_RATE);
        frame.set_pts(Some(pts));
        fill_frame(
            &mut frame,
            sample_format,
            &samples[offset * CHANNELS..],
            frames,
        );
        encoder
            .send_frame(&frame)
            .map_err(|error| format!("Could not encode {format:?} audio: {error}"))?;
        write_packets(
            &mut encoder,
            &mut output,
            stream_index,
            stream_time_base,
            format,
        )?;
        offset += frames;
        pts += frame_size as i64;
        if !progress(ExportProgress::Encoding {
            completed_frames: offset as u64,
            total_frames,
        }) {
            return Err("Export cancelled".to_string());
        }
    }
    encoder
        .send_eof()
        .map_err(|error| format!("Could not finalize the {format:?} encoder: {error}"))?;
    write_packets(
        &mut encoder,
        &mut output,
        stream_index,
        stream_time_base,
        format,
    )?;
    crate::verify_assets_current(assets)?;
    output
        .write_trailer()
        .map_err(|error| format!("Could not finalize the {format:?} file: {error}"))
}

fn fill_frame(
    frame: &mut ffmpeg::frame::Audio,
    format: ffmpeg::format::Sample,
    samples: &[f32],
    frames: usize,
) {
    match format {
        ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed) => {
            for (index, sample) in frame.plane_mut::<(i16, i16)>(0).iter_mut().enumerate() {
                *sample = if index < frames {
                    (
                        to_i16(samples[index * CHANNELS]),
                        to_i16(samples[index * CHANNELS + 1]),
                    )
                } else {
                    (0, 0)
                };
            }
        }
        ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar) => {
            for channel in 0..CHANNELS {
                for (index, sample) in frame.plane_mut::<f32>(channel).iter_mut().enumerate() {
                    *sample = if index < frames {
                        samples[index * CHANNELS + channel].clamp(-1.0, 1.0)
                    } else {
                        0.0
                    };
                }
            }
        }
        ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed) => {
            for (index, sample) in frame.plane_mut::<(f32, f32)>(0).iter_mut().enumerate() {
                *sample = if index < frames {
                    (
                        samples[index * CHANNELS].clamp(-1.0, 1.0),
                        samples[index * CHANNELS + 1].clamp(-1.0, 1.0),
                    )
                } else {
                    (0.0, 0.0)
                };
            }
        }
        _ => unreachable!(),
    }
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn write_packets(
    encoder: &mut ffmpeg::codec::encoder::audio::Encoder,
    output: &mut ffmpeg::format::context::Output,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    format: Format,
) -> Result<(), String> {
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                packet.set_stream(stream_index);
                packet.rescale_ts(encoder.time_base(), stream_time_base);
                let result = if packet.size() == 0 {
                    unsafe {
                        match ffmpeg::sys::av_interleaved_write_frame(
                            output.as_mut_ptr(),
                            packet.as_mut_ptr(),
                        ) {
                            0 => Ok(()),
                            error => Err(ffmpeg::Error::from(error)),
                        }
                    }
                } else {
                    packet.write_interleaved(output)
                };
                result
                    .map_err(|error| format!("Could not write {format:?} audio data: {error}"))?;
            }
            Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => return Ok(()),
            Err(ffmpeg::Error::Eof) => return Ok(()),
            Err(error) => return Err(format!("Could not receive {format:?} audio: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shrimply_project_document::project::{
        AudioItem, AudioTrack, Project, RepeatStrategy, Time,
    };

    #[test]
    fn exports_all_audio_formats() {
        let directory =
            std::env::temp_dir().join(format!("shrimply-audio-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let source_path = directory.join("source.wav");
        let mut source = hound::WavWriter::create(
            &source_path,
            hound::WavSpec {
                channels: CHANNELS as u16,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        let mut random = fastrand::Rng::with_seed(0);
        for _ in 0..SAMPLE_RATE as usize * CHANNELS {
            let sample = random.f32() * 4.0 - 2.0;
            source
                .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
                .unwrap();
        }
        source.finalize().unwrap();

        let item = AudioItem::builder(Time::ZERO, Time::from_fraction(1, 1))
            .id(Default::default())
            .source_duration(Time::from_fraction(1, 1))
            .repeat_strategy(RepeatStrategy::Empty)
            .file(&source_path)
            .build();
        let second_item = item.clone();
        let project = Project {
            name: "Audio export test".to_string(),
            audio_tracks: vec![
                AudioTrack {
                    items: vec![item],
                    ..Default::default()
                },
                AudioTrack {
                    items: vec![second_item],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mixed = streaming::mix_project_offline(&project, SAMPLE_RATE, |_, _| true).unwrap();
        assert!(mixed.iter().any(|sample| !(-1.0..=1.0).contains(sample)));
        for (format, encoder_name, container_name) in [
            (Format::Wav, "pcm_s16le", "wav"),
            (Format::Flac, "flac", "flac"),
            (Format::Mp3, "libmp3lame", "mp3"),
            (Format::Ogg, "libvorbis", "ogg"),
            (Format::Opus, "libopus", "ogg"),
        ] {
            let path = directory.join(format!("audio.{}", format.extension()));
            export(&project, &path, format).unwrap();
            let input = ffmpeg::format::input(&path).unwrap();
            assert_eq!(input.format().name(), container_name);
            let stream = input.streams().best(ffmpeg::media::Type::Audio).unwrap();
            let parameters = stream.parameters();
            let expected_codec = ffmpeg::codec::encoder::find_by_name(encoder_name)
                .unwrap()
                .id();
            assert_eq!(parameters.id(), expected_codec);
            unsafe {
                assert_eq!((*parameters.as_ptr()).sample_rate, SAMPLE_RATE as i32);
                assert_eq!(
                    (*parameters.as_ptr()).ch_layout.nb_channels,
                    CHANNELS as i32
                );
                if matches!(format, Format::Wav | Format::Flac) {
                    assert_eq!(
                        (*parameters.as_ptr())
                            .bits_per_raw_sample
                            .max((*parameters.as_ptr()).bits_per_coded_sample),
                        16
                    );
                }
            }
            assert!(input.duration() > 0);
        }

        std::fs::remove_dir_all(directory).unwrap();
    }
}
