use crate::video::ExportProgress;
use std::{collections::VecDeque, path::Path, time::Instant};

use ffmpeg_next::{self as ffmpeg, packet::Mut as _, sys};
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

    fn container(self) -> &'static str {
        match self {
            Self::Opus => "ogg",
            _ => self.extension(),
        }
    }
}

pub fn export_with_progress(
    project: &Project,
    path: &Path,
    format: Format,
    mut progress: impl FnMut(ExportProgress) -> bool,
) -> Result<(), String> {
    crate::ensure_output_is_not_an_asset(project, path)?;
    let assets = crate::snapshot_assets(project)?;
    let temporary = crate::output::temporary_sibling(path)
        .map_err(|error| format!("Could not prepare audio export: {error}"))?;
    export_inner(
        project,
        format,
        &assets,
        || {
            ffmpeg::format::output_as(&temporary, format.container())
                .map_err(|error| format!("Could not create the {format:?} output file: {error}"))
        },
        &mut progress,
    )?;
    temporary
        .persist(path)
        .map_err(|error| format!("Could not save audio export: {error}"))
}

pub fn export_to_stream(
    project: &Project,
    format: Format,
    open: impl FnOnce() -> Result<ffmpeg::format::context::StreamIo, String>,
    mut progress: impl FnMut(ExportProgress) -> bool,
) -> Result<(), String> {
    let assets = crate::snapshot_assets(project)?;
    export_inner(
        project,
        format,
        &assets,
        || {
            ffmpeg::format::output_to_stream(open()?, None, Some(format.container()))
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
    if !progress(ExportProgress::EncodingAudio {
        current_frame: 0,
        total_frames,
    }) {
        return Err("Export cancelled".into());
    }
    let encoder = open_encoder(
        codec,
        SAMPLE_RATE,
        sample_format,
        matches!(format, Format::Mp3 | Format::Ogg | Format::Opus).then_some(192_000),
        crate::output::needs_global_header(format.container())?,
    )?;
    let mut packets = AudioPackets::new(Encoder::Ffmpeg(encoder), project, assets);
    let mut packet = packets
        .next(&mut *progress)?
        .ok_or("Audio encoder produced no packets")?;
    let mut output = open()?;
    let stream_index = packets.encoder.add_stream(&mut output)?;
    output
        .write_header()
        .map_err(|error| format!("Could not write the {format:?} file header: {error}"))?;
    let stream_time_base = output.stream(stream_index).unwrap().time_base();
    loop {
        crate::write_packet(
            &mut output,
            packet,
            stream_index,
            packets.time_base(),
            stream_time_base,
        )
        .map_err(|error| format!("Could not write {format:?} audio data: {error}"))?;
        let Some(next) = packets.next(&mut *progress)? else {
            break;
        };
        packet = next;
    }
    crate::verify_assets_current(assets)?;
    output
        .write_trailer()
        .map_err(|error| format!("Could not finalize the {format:?} file: {error}"))
}

pub(crate) fn open_encoder(
    codec: ffmpeg::Codec,
    sample_rate: u32,
    sample_format: ffmpeg::format::Sample,
    bitrate: Option<usize>,
    global_header: bool,
) -> Result<ffmpeg::codec::encoder::audio::Encoder, String> {
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .audio()
        .map_err(|error| format!("Configure {} audio encoder: {error}", codec.name()))?;
    encoder.set_rate(sample_rate as i32);
    encoder.set_channel_layout(ffmpeg::channel_layout::ChannelLayout::STEREO);
    encoder.set_format(sample_format);
    encoder.set_time_base(ffmpeg::Rational(1, sample_rate as i32));
    if let Some(bitrate) = bitrate {
        encoder.set_bit_rate(bitrate);
    }
    if global_header {
        unsafe {
            (*encoder.as_mut_ptr()).flags |= ffmpeg::sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    encoder
        .open_as(codec)
        .map_err(|error| format!("Open {} audio encoder: {error}", codec.name()))
}

pub(crate) enum Encoder {
    Ffmpeg(ffmpeg::codec::encoder::audio::Encoder),
    Shrimply {
        codec: Box<shrimply_aac::Encoder>,
        rate: u32,
        bitrate: u32,
        pts: i64,
    },
}

impl Encoder {
    pub fn shrimply(rate: u32, bitrate: u32) -> Result<Self, String> {
        Ok(Self::Shrimply {
            codec: Box::new(shrimply_aac::Encoder::new(rate, bitrate)?),
            rate,
            bitrate,
            pts: -(shrimply_aac::FRAME_SAMPLES as i64),
        })
    }

    pub fn add_stream(
        &self,
        output: &mut ffmpeg::format::context::Output,
    ) -> Result<usize, String> {
        let mut stream = match self {
            Self::Ffmpeg(codec) => output.add_stream_with(codec.as_ref()),
            Self::Shrimply { .. } => output.add_stream(None),
        }
        .map_err(|error| format!("Could not add audio stream: {error}"))?;
        stream.set_time_base(self.time_base());
        if let Self::Shrimply {
            codec,
            rate,
            bitrate,
            ..
        } = self
        {
            let configuration = codec.configuration();
            // The AVStream owns codecpar, including the padded FFmpeg allocation.
            unsafe {
                let mut parameters = stream.parameters();
                let parameters = &mut *parameters.as_mut_ptr();
                parameters.extradata = sys::av_mallocz(
                    configuration.len() + sys::AV_INPUT_BUFFER_PADDING_SIZE as usize,
                )
                .cast();
                if parameters.extradata.is_null() {
                    return Err("Could not allocate Shrimply AAC configuration".into());
                }
                std::ptr::copy_nonoverlapping(
                    configuration.as_ptr(),
                    parameters.extradata,
                    configuration.len(),
                );
                parameters.extradata_size = configuration.len() as i32;
                parameters.codec_type = sys::AVMediaType::AVMEDIA_TYPE_AUDIO;
                parameters.codec_id = sys::AVCodecID::AV_CODEC_ID_AAC;
                parameters.profile = sys::AV_PROFILE_AAC_LOW;
                parameters.sample_rate = *rate as i32;
                parameters.bit_rate = i64::from(*bitrate);
                parameters.frame_size = shrimply_aac::FRAME_SAMPLES as i32;
                parameters.initial_padding = parameters.frame_size;
                sys::av_channel_layout_default(&mut parameters.ch_layout, CHANNELS as i32);
            }
        }
        Ok(stream.index())
    }

    fn time_base(&self) -> ffmpeg::Rational {
        match self {
            Self::Ffmpeg(codec) => codec.time_base(),
            Self::Shrimply { rate, .. } => ffmpeg::Rational(1, *rate as i32),
        }
    }
}

pub(crate) struct AudioPackets<'a> {
    encoder: Encoder,
    mixer: streaming::OfflineProjectMix<'a>,
    assets: &'a [shrimply_project_document::project::AssetSnapshot],
    samples: VecDeque<f32>,
    frame_size: usize,
    submitted: u64,
    draining: bool,
    pub mix_ns: u128,
    pub encode_ns: u128,
}

impl<'a> AudioPackets<'a> {
    pub fn new(
        encoder: Encoder,
        project: &'a Project,
        assets: &'a [shrimply_project_document::project::AssetSnapshot],
    ) -> Self {
        let (rate, frame_size) = match &encoder {
            Encoder::Ffmpeg(codec) => (
                codec.rate(),
                usize::try_from(codec.frame_size())
                    .ok()
                    .filter(|&size| size > 0)
                    .unwrap_or(1024),
            ),
            Encoder::Shrimply { rate, .. } => (*rate, shrimply_aac::FRAME_SAMPLES),
        };
        Self {
            mixer: streaming::OfflineProjectMix::new(project, rate),
            frame_size,
            encoder,
            assets,
            samples: VecDeque::new(),
            submitted: 0,
            draining: false,
            mix_ns: 0,
            encode_ns: 0,
        }
    }

    pub fn time_base(&self) -> ffmpeg::Rational {
        self.encoder.time_base()
    }

    pub fn next(
        &mut self,
        mut progress: impl FnMut(ExportProgress) -> bool,
    ) -> Result<Option<ffmpeg::Packet>, String> {
        let started = Instant::now();
        let initial_mix_ns = self.mix_ns;
        let mut packet = ffmpeg::Packet::empty();
        let packet = loop {
            if let Encoder::Ffmpeg(codec) = &mut self.encoder {
                match codec.receive_packet(&mut packet) {
                    Ok(()) => break Some(packet),
                    Err(ffmpeg::Error::Eof) if self.draining => break None,
                    Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN && !self.draining => {}
                    Err(error) => return Err(format!("Receive encoded audio: {error}")),
                }
            }
            while !self.draining && self.samples.len() < self.frame_size * CHANNELS {
                crate::ensure_assets_current(self.assets)?;
                let started = Instant::now();
                let total_frames = self.mixer.total_frames();
                let samples = self.mixer.next_chunk(|_, _| {
                    progress(ExportProgress::EncodingAudio {
                        current_frame: self.submitted,
                        total_frames,
                    })
                })?;
                self.mix_ns += started.elapsed().as_nanos();
                let Some(samples) = samples else {
                    break;
                };
                self.samples.extend(samples);
            }
            let frames = self.frame_size.min(self.samples.len() / CHANNELS);
            self.draining = frames == 0;
            let source_pts = i64::try_from(self.submitted)
                .map_err(|_| "Audio timestamp exceeds the exact range")?;
            let samples = &self.samples.make_contiguous()[..frames * CHANNELS];
            let encoded = match &mut self.encoder {
                Encoder::Ffmpeg(codec) if self.draining => {
                    codec
                        .send_eof()
                        .map_err(|error| format!("Finalize audio encoder: {error}"))?;
                    continue;
                }
                Encoder::Ffmpeg(codec) => {
                    let mut frame = ffmpeg::frame::Audio::new(
                        codec.format(),
                        frames,
                        ffmpeg::channel_layout::ChannelLayout::STEREO,
                    );
                    frame.set_rate(codec.rate());
                    frame.set_pts(Some(source_pts));
                    fill_frame(&mut frame, samples);
                    codec
                        .send_frame(&frame)
                        .map_err(|error| format!("Encode audio frame: {error}"))?;
                    None
                }
                Encoder::Shrimply { codec, pts, .. } => {
                    if self.draining && (source_pts == 0 || *pts >= source_pts) {
                        break None;
                    }
                    let mut block = [[0.0; CHANNELS]; shrimply_aac::FRAME_SAMPLES];
                    block[..frames].copy_from_slice(samples.as_chunks::<CHANNELS>().0);
                    codec
                        .encode(&block)
                        .map(|data| {
                            let total = if self.draining { source_pts } else { i64::MAX };
                            let result = aac_packet(&data, *pts, total);
                            *pts += shrimply_aac::FRAME_SAMPLES as i64;
                            result
                        })
                        .transpose()?
                }
            };
            drop(self.samples.drain(..frames * CHANNELS));
            self.submitted += frames as u64;
            if !progress(ExportProgress::EncodingAudio {
                current_frame: self.submitted,
                total_frames: self.mixer.total_frames(),
            }) {
                return Err("Export cancelled".into());
            }
            if encoded.is_some() {
                break encoded;
            }
        };
        self.encode_ns += started
            .elapsed()
            .as_nanos()
            .saturating_sub(self.mix_ns - initial_mix_ns);
        Ok(packet)
    }
}

fn aac_packet(data: &[u8], pts: i64, total: i64) -> Result<ffmpeg::Packet, String> {
    let frame_size = shrimply_aac::FRAME_SAMPLES as i64;
    let duration = total.saturating_sub(pts).min(frame_size);
    let mut packet = ffmpeg::Packet::copy(data);
    packet.set_pts(Some(pts));
    packet.set_dts(Some(pts));
    packet.set_duration(duration);
    packet.set_flags(ffmpeg::packet::Flags::KEY);

    let leading = if pts < 0 { frame_size as u32 } else { 0 };
    let trailing = (frame_size - duration) as u32;
    if leading != 0 || trailing != 0 {
        unsafe {
            let skip = sys::av_packet_new_side_data(
                packet.as_mut_ptr(),
                sys::AVPacketSideDataType::AV_PKT_DATA_SKIP_SAMPLES,
                10,
            );
            if skip.is_null() {
                return Err("Could not allocate Shrimply AAC packet padding information".into());
            }
            let skip = std::slice::from_raw_parts_mut(skip, 10);
            skip[..4].copy_from_slice(&leading.to_le_bytes());
            skip[4..8].copy_from_slice(&trailing.to_le_bytes());
            skip[8..].fill(0);
        }
    }
    Ok(packet)
}

fn fill_frame(frame: &mut ffmpeg::frame::Audio, samples: &[f32]) {
    match frame.format() {
        ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed) => {
            for (sample, stereo) in frame
                .plane_mut::<(i16, i16)>(0)
                .iter_mut()
                .zip(samples.as_chunks::<CHANNELS>().0.iter())
            {
                *sample = (to_i16(stereo[0]), to_i16(stereo[1]));
            }
        }
        ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar) => {
            for channel in 0..CHANNELS {
                for (sample, value) in frame
                    .plane_mut::<f32>(channel)
                    .iter_mut()
                    .zip(samples[channel..].iter().step_by(CHANNELS))
                {
                    *sample = value.clamp(-1.0, 1.0);
                }
            }
        }
        ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed) => {
            for (sample, stereo) in frame
                .plane_mut::<(f32, f32)>(0)
                .iter_mut()
                .zip(samples.as_chunks::<CHANNELS>().0.iter())
            {
                *sample = (stereo[0].clamp(-1.0, 1.0), stereo[1].clamp(-1.0, 1.0));
            }
        }
        _ => unreachable!(),
    }
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
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
        let duration = Time::from_fraction(5, 4);
        let sample_frames = duration.as_sample_frame(SAMPLE_RATE);
        let mut random = fastrand::Rng::with_seed(0);
        for _ in 0..sample_frames as usize * CHANNELS {
            let sample = random.f32() * 4.0 - 2.0;
            source
                .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
                .unwrap();
        }
        source.finalize().unwrap();

        let item = AudioItem::builder(Time::ZERO, duration)
            .id(Default::default())
            .source_duration(duration)
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
            export_with_progress(&project, &path, format, |_| true).unwrap();
            if matches!(format, Format::Wav) {
                assert_eq!(
                    u64::from(hound::WavReader::open(&path).unwrap().duration()),
                    sample_frames
                );
            }
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

        let path = directory.join("audio.mp4");
        let assets = crate::snapshot_assets(&project).unwrap();
        let encoder = Encoder::shrimply(SAMPLE_RATE, 192_000).unwrap();
        let mut output = ffmpeg::format::output_as(&path, "mp4").unwrap();
        let index = encoder.add_stream(&mut output).unwrap();
        output.write_header().unwrap();
        let time_base = output.stream(index).unwrap().time_base();
        let mut packets = AudioPackets::new(encoder, &project, &assets);
        let mut pts = -(shrimply_aac::FRAME_SAMPLES as i64);
        while let Some(packet) = packets.next(|_| true).unwrap() {
            assert_eq!(packet.pts(), Some(pts));
            pts += packet.duration();
            crate::write_packet(&mut output, packet, index, packets.time_base(), time_base)
                .unwrap();
        }
        assert_eq!(pts, sample_frames as i64);
        output.write_trailer().unwrap();
        drop(output);

        let mut input = ffmpeg::format::input(&path).unwrap();
        let stream = input.streams().best(ffmpeg::media::Type::Audio).unwrap();
        assert_eq!(stream.parameters().id(), ffmpeg::codec::Id::AAC);
        assert_eq!(stream.time_base(), ffmpeg::Rational(1, SAMPLE_RATE as i32));
        assert_eq!(stream.duration(), sample_frames as i64);
        let mut decoder = ffmpeg::codec::Context::from_parameters(stream.parameters())
            .unwrap()
            .decoder()
            .audio()
            .unwrap();
        let mut decoded = 0;
        let mut frame = ffmpeg::frame::Audio::empty();
        for (_, packet) in input.packets() {
            decoder.send_packet(&packet).unwrap();
            while decoder.receive_frame(&mut frame).is_ok() {
                decoded += frame.samples() as u64;
            }
        }
        decoder.send_eof().unwrap();
        while decoder.receive_frame(&mut frame).is_ok() {
            decoded += frame.samples() as u64;
        }
        assert!(
            (sample_frames..sample_frames + shrimply_aac::FRAME_SAMPLES as u64).contains(&decoded)
        );

        std::fs::remove_dir_all(directory).unwrap();
    }
}
