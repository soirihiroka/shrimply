use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use cached::{Cached, stores::LruCache};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use ffmpeg::{ChannelLayout, codec, encoder, format, frame};
use ffmpeg_next as ffmpeg;
use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_project_document::project::Time;
use uuid::Uuid;

const RECORDING_DIR: &str = "media/recordings";
const CHANNELS: usize = 2;
const OPUS_RATE: u32 = 48_000;
const TTS_REFERENCE_RATE: u32 = 24_000;
const TTS_REFERENCE_MAX_SECONDS: usize = 15;
const TTS_REFERENCE_CACHE_ENTRIES: usize = 4;
const WAV_HEADER_BYTES: usize = 44;
const WAV_FORMAT_FLOAT: u16 = 3;
const WAV_BITS_PER_SAMPLE: u16 = 32;

static TTS_REFERENCE_WAV_CACHE: LazyLock<Mutex<LruCache<AssetSnapshot, Vec<u8>>>> =
    LazyLock::new(|| {
        Mutex::new(
            LruCache::builder()
                .max_size(TTS_REFERENCE_CACHE_ENTRIES)
                .build()
                .expect("valid TTS reference WAV cache size"),
        )
    });

pub struct MicRecording {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
}

pub struct FinishedRecording {
    pub path: PathBuf,
    pub duration: Time,
}

pub struct MicRecordingSnapshot {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl MicRecording {
    pub fn start() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "No default audio input device".to_string())?;
        let input_config = device
            .default_input_config()
            .map_err(|error| error.to_string())?;
        let config = input_config.config();
        let input_channels = config.channels as usize;
        let sample_rate = config.sample_rate;
        let samples = Arc::new(Mutex::new(Vec::new()));
        let stream = build_input_stream(
            &device,
            &config,
            input_config.sample_format(),
            input_channels,
            samples.clone(),
        )?;

        stream.play().map_err(|error| error.to_string())?;
        tracing::info!(
            "Microphone recording started: device={}, sample_format={:?}, sample_rate={}, channels={}",
            device,
            input_config.sample_format(),
            sample_rate,
            input_channels
        );

        Ok(Self {
            stream,
            samples,
            sample_rate,
        })
    }

    pub fn finish(self) -> Result<FinishedRecording, String> {
        let Self {
            stream,
            samples,
            sample_rate,
        } = self;
        drop(stream);

        let samples = samples
            .lock()
            .map_err(|_| "Recording buffer died".to_string())?;
        if samples.len() < CHANNELS {
            return Err("Recording captured no audio".to_string());
        }

        let directory = shrimply_project_document::project::project_directory().join(RECORDING_DIR);
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let path = directory.join(format!("{}.opus", Uuid::new_v4()));
        if let Err(error) = write_opus(&path, &samples, sample_rate) {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        let frames = samples.len() / CHANNELS;
        let duration = Time::from_nanos(
            ((frames as u128 * 1_000_000_000_u128) / sample_rate as u128).min(u64::MAX as u128)
                as u64,
        );
        tracing::info!(
            "Microphone recording saved: {} frames={} sample_rate={}",
            path.display(),
            frames,
            sample_rate
        );

        Ok(FinishedRecording { path, duration })
    }

    pub fn snapshot(&self) -> MicRecordingSnapshot {
        let samples = self
            .samples
            .lock()
            .map(|samples| samples.clone())
            .unwrap_or_default();
        MicRecordingSnapshot {
            samples,
            sample_rate: self.sample_rate,
        }
    }
}

pub fn transcode_to_opus(input_path: &Path, output_path: &Path) -> Result<Time, String> {
    let (samples, sample_rate) = decode_stereo(input_path, None, None)?;
    write_opus(output_path, &samples, sample_rate)?;
    let frames = i64::try_from(samples.len() / CHANNELS)
        .map_err(|_| "generated audio is too long".to_string())?;
    Ok(Time::from_fraction(frames, i64::from(sample_rate)))
}

pub fn save_wav_as_opus(wav: &[u8], output_path: &Path) -> Result<Time, String> {
    if !wav.starts_with(b"RIFF") || wav.get(8..12) != Some(b"WAVE") {
        return Err("Server returned invalid WAV audio".to_string());
    }
    if output_path.exists() {
        return Err(format!(
            "generated audio destination already exists: {}",
            output_path.display()
        ));
    }
    let directory = output_path
        .parent()
        .ok_or_else(|| "generated audio destination has no parent directory".to_string())?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
    let id = Uuid::new_v4();
    let wav_path = directory.join(format!(".{id}.wav"));
    let encoded_path = directory.join(format!(".{id}.opus"));
    fs::write(&wav_path, wav)
        .map_err(|error| format!("Could not save generated audio: {error}"))?;
    let converted = transcode_to_opus(&wav_path, &encoded_path).and_then(|duration| {
        if duration <= Time::ZERO {
            return Err("Generated file has no valid audio".to_string());
        }
        fs::rename(&encoded_path, output_path)
            .map_err(|error| format!("Could not finalize audio: {error}"))?;
        Ok(duration)
    });
    let _ = fs::remove_file(&wav_path);
    if converted.is_err() {
        let _ = fs::remove_file(&encoded_path);
        let _ = fs::remove_file(output_path);
    }
    converted
}

pub fn transcode_to_wav(input_path: &Path) -> Result<Vec<u8>, String> {
    let source = Asset::new(input_path).snapshot()?;
    if let Some(wav) = TTS_REFERENCE_WAV_CACHE
        .lock()
        .expect("TTS reference WAV cache lock is poisoned")
        .cache_get(&source)
        .cloned()
    {
        tracing::debug!(path = %input_path.display(), wav_bytes = wav.len(), "Using cached TTS reference WAV");
        return Ok(wav);
    }
    let maximum_samples = TTS_REFERENCE_MAX_SECONDS * TTS_REFERENCE_RATE as usize * CHANNELS;
    let (stereo, _) = decode_stereo(input_path, Some(TTS_REFERENCE_RATE), Some(maximum_samples))?;
    let samples = stereo
        .chunks_exact(CHANNELS)
        .map(|frame| (frame[0] + frame[1]) * 0.5)
        .collect::<Vec<_>>();
    let data_size = u32::try_from(samples.len().saturating_mul(size_of::<f32>()))
        .map_err(|_| "reference audio is too large".to_string())?;
    let riff_size = data_size
        .checked_add((WAV_HEADER_BYTES - 8) as u32)
        .ok_or_else(|| "reference audio is too large".to_string())?;
    let mut wav = Vec::with_capacity(WAV_HEADER_BYTES + data_size as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&WAV_FORMAT_FLOAT.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&TTS_REFERENCE_RATE.to_le_bytes());
    wav.extend_from_slice(&(TTS_REFERENCE_RATE * size_of::<f32>() as u32).to_le_bytes());
    wav.extend_from_slice(&(size_of::<f32>() as u16).to_le_bytes());
    wav.extend_from_slice(&WAV_BITS_PER_SAMPLE.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    source.verify_current()?;
    TTS_REFERENCE_WAV_CACHE
        .lock()
        .expect("TTS reference WAV cache lock is poisoned")
        .cache_set(source, wav.clone());
    Ok(wav)
}

fn decode_stereo(
    input_path: &Path,
    output_rate: Option<u32>,
    maximum_samples: Option<usize>,
) -> Result<(Vec<f32>, u32), String> {
    ffmpeg::init().map_err(|error| error.to_string())?;
    let mut input = format::input(input_path)
        .map_err(|error| format!("could not open {}: {error}", input_path.display()))?;
    let (stream_index, parameters) = {
        let stream = input
            .streams()
            .find(|stream| stream.parameters().medium() == ffmpeg::media::Type::Audio)
            .ok_or_else(|| format!("{} has no audio stream", input_path.display()))?;
        (stream.index(), stream.parameters())
    };
    let context = ffmpeg::codec::context::Context::from_parameters(parameters)
        .map_err(|error| error.to_string())?;
    let mut decoder = context
        .decoder()
        .audio()
        .map_err(|error| error.to_string())?;
    if decoder.channel_layout().is_empty() {
        decoder.set_channel_layout(ChannelLayout::default(decoder.channels() as i32));
    }
    let output_rate = output_rate.unwrap_or_else(|| decoder.rate().max(1));
    let mut resampler = decoder
        .resampler(
            format::Sample::F32(format::sample::Type::Packed),
            ChannelLayout::STEREO,
            output_rate,
        )
        .map_err(|error| error.to_string())?;
    let mut samples = Vec::new();

    for (stream, packet) in input.packets() {
        if stream.index() != stream_index {
            continue;
        }
        decoder
            .send_packet(&packet)
            .map_err(|error| error.to_string())?;
        receive_resampled_samples(&mut decoder, &mut resampler, &mut samples, maximum_samples)?;
        if maximum_samples.is_some_and(|maximum| samples.len() >= maximum) {
            break;
        }
    }
    if maximum_samples.is_none_or(|maximum| samples.len() < maximum) {
        decoder.send_eof().map_err(|error| error.to_string())?;
        receive_resampled_samples(&mut decoder, &mut resampler, &mut samples, maximum_samples)?;
    }
    while maximum_samples.is_none_or(|maximum| samples.len() < maximum) {
        let Some(delay_before) = resampler.delay() else {
            break;
        };
        let capacity = (delay_before.output as usize).max(1);
        let mut frame = frame::Audio::new(
            format::Sample::F32(format::sample::Type::Packed),
            capacity,
            ChannelLayout::STEREO,
        );
        frame.set_rate(output_rate);
        let delay = resampler
            .flush(&mut frame)
            .map_err(|error| format!("could not flush WAV resampler: {error}"))?;
        append_frame_f32(&frame, &mut samples)?;
        match delay {
            Some(remaining) if remaining.output < delay_before.output => {}
            Some(remaining) => {
                tracing::debug!(
                    before = delay_before.output,
                    after = remaining.output,
                    "WAV resampler retained a non-draining filter delay"
                );
                break;
            }
            None => break,
        }
    }
    if samples.is_empty() {
        return Err(format!("{} decoded no audio", input_path.display()));
    }
    if let Some(maximum) = maximum_samples {
        samples.truncate(maximum);
    }
    Ok((samples, output_rate))
}

fn receive_resampled_samples(
    decoder: &mut ffmpeg::decoder::Audio,
    resampler: &mut ffmpeg::software::resampling::Context,
    samples: &mut Vec<f32>,
    maximum_samples: Option<usize>,
) -> Result<(), String> {
    let mut decoded = frame::Audio::empty();
    while decoder.receive_frame(&mut decoded).is_ok() {
        if decoded.channel_layout().is_empty() {
            decoded.set_channel_layout(ChannelLayout::default(decoded.channels() as i32));
        }
        let mut stereo = frame::Audio::empty();
        resampler
            .run(&decoded, &mut stereo)
            .map_err(|error| format!("could not convert generated WAV: {error}"))?;
        append_frame_f32(&stereo, samples)?;
        if maximum_samples.is_some_and(|maximum| samples.len() >= maximum) {
            break;
        }
    }
    Ok(())
}

fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    input_channels: usize,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, String> {
    match sample_format {
        cpal::SampleFormat::F32 => {
            build_typed_input_stream::<f32>(device, config, input_channels, samples)
        }
        cpal::SampleFormat::F64 => {
            build_typed_input_stream::<f64>(device, config, input_channels, samples)
        }
        cpal::SampleFormat::I8 => {
            build_typed_input_stream::<i8>(device, config, input_channels, samples)
        }
        cpal::SampleFormat::I16 => {
            build_typed_input_stream::<i16>(device, config, input_channels, samples)
        }
        cpal::SampleFormat::I24 => {
            build_typed_input_stream::<cpal::I24>(device, config, input_channels, samples)
        }
        cpal::SampleFormat::I32 => {
            build_typed_input_stream::<i32>(device, config, input_channels, samples)
        }
        cpal::SampleFormat::I64 => {
            build_typed_input_stream::<i64>(device, config, input_channels, samples)
        }
        cpal::SampleFormat::U8 => {
            build_typed_input_stream::<u8>(device, config, input_channels, samples)
        }
        cpal::SampleFormat::U16 => {
            build_typed_input_stream::<u16>(device, config, input_channels, samples)
        }
        cpal::SampleFormat::U32 => {
            build_typed_input_stream::<u32>(device, config, input_channels, samples)
        }
        cpal::SampleFormat::U64 => {
            build_typed_input_stream::<u64>(device, config, input_channels, samples)
        }
        other => Err(format!("Unsupported audio input sample format {other:?}")),
    }
}

fn build_typed_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    input_channels: usize,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, String>
where
    T: Sample + SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            *config,
            move |data: &[T], _| append_input_samples(data, input_channels, &samples),
            |error| tracing::error!("Microphone input stream error: {error}"),
            None,
        )
        .map_err(|error| error.to_string())
}

fn append_input_samples<T>(data: &[T], input_channels: usize, samples: &Arc<Mutex<Vec<f32>>>)
where
    T: Sample,
    f32: FromSample<T>,
{
    if input_channels == 0 {
        return;
    }
    let Ok(mut output) = samples.lock() else {
        return;
    };
    output.reserve(data.len() / input_channels * CHANNELS);
    for frame in data.chunks(input_channels) {
        let left = frame.first().copied().map(Sample::to_sample).unwrap_or(0.0);
        let right = frame.get(1).copied().map(Sample::to_sample).unwrap_or(left);
        output.push(left);
        output.push(right);
    }
}

fn write_opus(path: &Path, samples: &[f32], input_rate: u32) -> Result<(), String> {
    ffmpeg::init().map_err(|error| error.to_string())?;
    let mut output = format::output(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    let codec = encoder::find_by_name("libopus")
        .ok_or_else(|| "FFmpeg libopus encoder not found".to_string())?
        .audio()
        .map_err(|error| error.to_string())?;
    let encoder_format = preferred_opus_format(codec)?;
    let encoder_rate = preferred_opus_rate(codec);
    let global = output
        .format()
        .flags()
        .contains(format::flag::Flags::GLOBAL_HEADER);

    let (stream_index, out_time_base, mut audio_encoder) = {
        let mut stream = output
            .add_stream(codec)
            .map_err(|error| format!("could not add Opus stream: {error}"))?;
        let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|error| error.to_string())?;
        let mut audio_encoder = context
            .encoder()
            .audio()
            .map_err(|error| error.to_string())?;
        if global {
            audio_encoder.set_flags(codec::flag::Flags::GLOBAL_HEADER);
        }
        audio_encoder.set_rate(encoder_rate as i32);
        audio_encoder.set_channel_layout(ChannelLayout::STEREO);
        audio_encoder.set_format(encoder_format);
        audio_encoder.set_bit_rate(96_000);
        audio_encoder.set_time_base((1, encoder_rate as i32));
        stream.set_time_base((1, encoder_rate as i32));
        let audio_encoder = audio_encoder
            .open_as(codec)
            .map_err(|error| format!("could not open Opus encoder: {error}"))?;
        stream.set_parameters(&audio_encoder);
        (stream.index(), stream.time_base(), audio_encoder)
    };

    output
        .write_header()
        .map_err(|error| format!("could not write {} header: {error}", path.display()))?;

    let mut resampler = ffmpeg::software::resampling::Context::get(
        format::Sample::F32(format::sample::Type::Packed),
        ChannelLayout::STEREO,
        input_rate,
        encoder_format,
        ChannelLayout::STEREO,
        encoder_rate,
    )
    .map_err(|error| error.to_string())?;
    let frame_size = (audio_encoder.frame_size() as usize).max(960);
    let mut pending = Vec::new();
    let mut next_pts = 0_i64;

    for chunk in samples.chunks(frame_size * CHANNELS) {
        let mut input = frame::Audio::new(
            format::Sample::F32(format::sample::Type::Packed),
            chunk.len() / CHANNELS,
            ChannelLayout::STEREO,
        );
        input.set_rate(input_rate);
        fill_packed_f32_frame(&mut input, chunk);
        let mut resampled = frame::Audio::new(
            encoder_format,
            resampled_capacity(input.samples(), input_rate, encoder_rate),
            ChannelLayout::STEREO,
        );
        resampled.set_rate(encoder_rate);
        resampler
            .run(&input, &mut resampled)
            .map_err(|error| format!("could not resample microphone audio: {error}"))?;
        append_frame_f32(&resampled, &mut pending)?;
        encode_pending(
            &mut pending,
            frame_size,
            encoder_format,
            &mut audio_encoder,
            &mut output,
            stream_index,
            out_time_base,
            &mut next_pts,
            false,
        )?;
    }

    while let Some(delay_before) = resampler.delay() {
        let flush_samples = (delay_before.output as usize).max(frame_size);
        let mut resampled = frame::Audio::new(encoder_format, flush_samples, ChannelLayout::STEREO);
        resampled.set_rate(encoder_rate);
        let delay = resampler
            .flush(&mut resampled)
            .map_err(|error| format!("could not flush microphone resampler: {error}"))?;
        append_frame_f32(&resampled, &mut pending)?;
        match delay {
            Some(remaining) if remaining.output < delay_before.output => {}
            Some(remaining) => {
                tracing::debug!(
                    before = delay_before.output,
                    after = remaining.output,
                    "Opus resampler retained a non-draining filter delay"
                );
                break;
            }
            None => break,
        }
    }
    encode_pending(
        &mut pending,
        frame_size,
        encoder_format,
        &mut audio_encoder,
        &mut output,
        stream_index,
        out_time_base,
        &mut next_pts,
        true,
    )?;

    audio_encoder
        .send_eof()
        .map_err(|error| error.to_string())?;
    receive_packets(&mut audio_encoder, &mut output, stream_index, out_time_base)?;
    output
        .write_trailer()
        .map_err(|error| format!("could not write {} trailer: {error}", path.display()))
}

fn resampled_capacity(input_samples: usize, input_rate: u32, output_rate: u32) -> usize {
    let samples = input_samples as u128 * output_rate as u128;
    samples
        .div_ceil(input_rate.max(1) as u128)
        .saturating_add(256)
        .min(usize::MAX as u128) as usize
}

fn preferred_opus_format(codec: ffmpeg::codec::Audio) -> Result<format::Sample, String> {
    let formats = codec
        .formats()
        .ok_or_else(|| "Opus encoder did not report supported sample formats".to_string())?
        .collect::<Vec<_>>();
    [
        format::Sample::F32(format::sample::Type::Packed),
        format::Sample::F32(format::sample::Type::Planar),
        format::Sample::I16(format::sample::Type::Packed),
        format::Sample::I16(format::sample::Type::Planar),
    ]
    .into_iter()
    .find(|format| formats.contains(format))
    .ok_or_else(|| "Opus encoder does not support f32 or i16 audio".to_string())
}

fn preferred_opus_rate(codec: ffmpeg::codec::Audio) -> u32 {
    codec
        .rates()
        .and_then(|mut rates| rates.find(|rate| *rate == OPUS_RATE as i32))
        .map(|rate| rate as u32)
        .unwrap_or(OPUS_RATE)
}

fn fill_packed_f32_frame(frame: &mut frame::Audio, samples: &[f32]) {
    for (dst, src) in frame
        .plane_mut::<(f32, f32)>(0)
        .iter_mut()
        .zip(samples.chunks_exact(CHANNELS))
    {
        *dst = (src[0], src[1]);
    }
}

fn append_frame_f32(frame: &frame::Audio, samples: &mut Vec<f32>) -> Result<(), String> {
    match frame.format() {
        format::Sample::F32(format::sample::Type::Packed) => {
            for &(left, right) in frame.plane::<(f32, f32)>(0) {
                samples.push(left);
                samples.push(right);
            }
        }
        format::Sample::F32(format::sample::Type::Planar) => {
            let left = frame.plane::<f32>(0);
            let right = frame.plane::<f32>(1);
            for index in 0..frame.samples() {
                samples.push(left[index]);
                samples.push(right[index]);
            }
        }
        format::Sample::I16(format::sample::Type::Packed) => {
            for &(left, right) in frame.plane::<(i16, i16)>(0) {
                samples.push(left as f32 / i16::MAX as f32);
                samples.push(right as f32 / i16::MAX as f32);
            }
        }
        format::Sample::I16(format::sample::Type::Planar) => {
            let left = frame.plane::<i16>(0);
            let right = frame.plane::<i16>(1);
            for index in 0..frame.samples() {
                samples.push(left[index] as f32 / i16::MAX as f32);
                samples.push(right[index] as f32 / i16::MAX as f32);
            }
        }
        other => return Err(format!("Unsupported resampled recording format {other:?}")),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn encode_pending(
    pending: &mut Vec<f32>,
    frame_size: usize,
    encoder_format: format::Sample,
    encoder: &mut ffmpeg::codec::encoder::audio::Encoder,
    output: &mut format::context::Output,
    stream_index: usize,
    out_time_base: ffmpeg::Rational,
    next_pts: &mut i64,
    finish: bool,
) -> Result<(), String> {
    while pending.len() >= frame_size * CHANNELS {
        encode_frame(
            &pending[..frame_size * CHANNELS],
            encoder_format,
            encoder,
            output,
            stream_index,
            out_time_base,
            next_pts,
        )?;
        pending.drain(..frame_size * CHANNELS);
    }

    if finish && !pending.is_empty() {
        let mut final_samples = std::mem::take(pending);
        final_samples.resize(frame_size * CHANNELS, 0.0);
        encode_frame(
            &final_samples,
            encoder_format,
            encoder,
            output,
            stream_index,
            out_time_base,
            next_pts,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn encode_frame(
    samples: &[f32],
    encoder_format: format::Sample,
    encoder: &mut ffmpeg::codec::encoder::audio::Encoder,
    output: &mut format::context::Output,
    stream_index: usize,
    out_time_base: ffmpeg::Rational,
    next_pts: &mut i64,
) -> Result<(), String> {
    let frame_samples = samples.len() / CHANNELS;
    let mut frame = frame::Audio::new(encoder_format, frame_samples, ChannelLayout::STEREO);
    frame.set_rate(encoder.rate());
    frame.set_pts(Some(*next_pts));
    fill_encoder_frame(&mut frame, samples)?;
    *next_pts += frame_samples as i64;
    encoder
        .send_frame(&frame)
        .map_err(|error| error.to_string())?;
    receive_packets(encoder, output, stream_index, out_time_base)
}

fn fill_encoder_frame(frame: &mut frame::Audio, samples: &[f32]) -> Result<(), String> {
    match frame.format() {
        format::Sample::F32(format::sample::Type::Packed) => {
            for (dst, src) in frame
                .plane_mut::<(f32, f32)>(0)
                .iter_mut()
                .zip(samples.chunks_exact(CHANNELS))
            {
                *dst = (src[0], src[1]);
            }
        }
        format::Sample::F32(format::sample::Type::Planar) => {
            for (index, src) in samples.chunks_exact(CHANNELS).enumerate() {
                frame.plane_mut::<f32>(0)[index] = src[0];
                frame.plane_mut::<f32>(1)[index] = src[1];
            }
        }
        format::Sample::I16(format::sample::Type::Packed) => {
            for (dst, src) in frame
                .plane_mut::<(i16, i16)>(0)
                .iter_mut()
                .zip(samples.chunks_exact(CHANNELS))
            {
                *dst = (f32_to_i16(src[0]), f32_to_i16(src[1]));
            }
        }
        format::Sample::I16(format::sample::Type::Planar) => {
            for (index, src) in samples.chunks_exact(CHANNELS).enumerate() {
                frame.plane_mut::<i16>(0)[index] = f32_to_i16(src[0]);
                frame.plane_mut::<i16>(1)[index] = f32_to_i16(src[1]);
            }
        }
        other => return Err(format!("Unsupported Opus encoder format {other:?}")),
    }
    Ok(())
}

fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

fn receive_packets(
    encoder: &mut ffmpeg::codec::encoder::audio::Encoder,
    output: &mut format::context::Output,
    stream_index: usize,
    out_time_base: ffmpeg::Rational,
) -> Result<(), String> {
    let mut packet = ffmpeg::Packet::empty();
    while encoder.receive_packet(&mut packet).is_ok() {
        packet.set_stream(stream_index);
        packet.rescale_ts((1, encoder.rate() as i32), out_time_base);
        packet
            .write_interleaved(output)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
