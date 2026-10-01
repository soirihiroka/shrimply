use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ffmpeg::format::Pixel;
use ffmpeg::sys;
use ffmpeg_next as ffmpeg;
use libc::EAGAIN;
use serde::Serialize;
use shrimply_audio_engine::streaming;
use shrimply_math_core::{Fraction, frame_count, time_from_frame};
use shrimply_project_document::project::{self, Project, Time};

const AUDIO_CHANNELS: usize = 2;
const DEFAULT_AUDIO_FRAME_SIZE: usize = 1024;
const EXPORT_DEBUG_FRAME_PERIOD: u64 = 300;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportVideoCodec {
    H264,
    H265,
    Gif,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportContainer {
    Mp4,
    Mkv,
    Gif,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportAudioEncoder {
    FdkAac,
    Aac,
    #[cfg(target_os = "macos")]
    AudioToolboxAac,
    Opus,
}

#[derive(Clone, Debug)]
pub struct ExportSettings {
    pub path: PathBuf,
    pub video_codec: ExportVideoCodec,
    pub container: ExportContainer,
    pub fps: Fraction,
    pub background_alpha: u8,
    pub bitrate_kbps: u32,
    pub keyframe_interval_seconds: u32,
    pub b_frames: u32,
    pub audio_encoder: ExportAudioEncoder,
    pub audio_sample_rate: u32,
    pub audio_bitrate_kbps: u32,
}

#[derive(Clone, Debug)]
pub enum ExportProgress {
    MixingAudio {
        current_frame: u64,
        total_frames: u64,
    },
    SettingUp(&'static str),
    EncodingAudio {
        current_frame: u64,
        total_frames: u64,
    },
    EncodingVideo {
        current_frame: u64,
        total_frames: u64,
        fps_milli: u64,
    },
    Finalizing,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTiming {
    pub compositor_ns: Option<u64>,
    pub conversion_ns: Option<u64>,
}

pub struct RenderedFrame {
    pub frame: ffmpeg::frame::Video,
    pub timing: FrameTiming,
}

pub trait VideoBackend {
    fn name(&self) -> &'static str;

    fn encoder_label(&self, codec: ExportVideoCodec) -> &'static str;

    fn validate(&self, project: &Project, settings: &ExportSettings) -> Result<(), String>;

    fn prepare(
        &mut self,
        project: &Project,
        settings: &ExportSettings,
        cancelled: &AtomicBool,
    ) -> Result<(), String>;

    fn open_video_encoder(
        &mut self,
        project: &Project,
        settings: &ExportSettings,
        global_header: bool,
    ) -> Result<ffmpeg::codec::encoder::video::Encoder, String>;

    fn render_video_frame(
        &mut self,
        project: &Project,
        settings: &ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String>;

    fn render_rgba_frame(
        &mut self,
        project: &Project,
        settings: &ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String>;

    fn decoder_session_count(&self) -> Option<usize> {
        None
    }

    fn shutdown(&mut self) {}
}

#[derive(Serialize)]
struct ExportBenchmark {
    version: u32,
    output: PathBuf,
    backend: String,
    encoder: String,
    canvas_width: u32,
    canvas_height: u32,
    project_duration: Time,
    fps_numerator: i64,
    fps_denominator: i64,
    video_codec: String,
    decoder_sessions: Option<usize>,
    frame_count: u64,
    total_elapsed_ns: u128,
    stages: ExportStageBenchmark,
    frames: Vec<ExportFrameBenchmark>,
}

#[derive(Serialize)]
struct ExportStageBenchmark {
    audio_mix_ns: u128,
    setup_ns: u128,
    audio_encode_ns: u128,
    video_encode_ns: u128,
    finalize_ns: u128,
}

#[derive(Serialize)]
struct ExportFrameBenchmark {
    index: u64,
    position: Time,
    render_ns: u128,
    compositor_gpu_ns: Option<u64>,
    conversion_gpu_ns: Option<u64>,
    encoder_send_ns: u128,
    packet_drain_and_mux_ns: u128,
    packets_received: usize,
    total_ns: u128,
}

pub fn export_project<B, F>(
    project: Project,
    settings: ExportSettings,
    mut backend: B,
    cancelled: Arc<AtomicBool>,
    progress: F,
) -> Result<(), String>
where
    B: VideoBackend,
    F: FnMut(ExportProgress),
{
    let result = export_project_inner(&project, &settings, &mut backend, &cancelled, progress);
    backend.shutdown();
    if result.is_ok() && cancelled.load(Ordering::Relaxed) {
        Err("Export cancelled".to_string())
    } else {
        result
    }
}

fn export_project_inner<B, F>(
    project: &Project,
    settings: &ExportSettings,
    backend: &mut B,
    cancelled: &AtomicBool,
    mut progress: F,
) -> Result<(), String>
where
    B: VideoBackend,
    F: FnMut(ExportProgress),
{
    let export_started = Instant::now();
    check_cancelled(cancelled)?;
    ffmpeg::init().map_err(|error| error.to_string())?;
    project.validate()?;
    shrimply_visual_core::sam2::validate_cache(project)?;
    shrimply_visual_core::transparent_fill::validate_cache(project)?;
    validate_settings(project, settings)?;
    backend.validate(project, settings)?;
    crate::ensure_output_is_not_an_asset(project, &settings.path)?;
    let assets = crate::snapshot_assets(project)?;

    progress(ExportProgress::SettingUp("Stabilizing source video"));
    shrimply_visual_core::stabilization::ensure_project(project)?;
    crate::ensure_assets_current(&assets)?;
    check_cancelled(cancelled)?;

    let _span = tracing::info_span!(
        "video_export",
        path = %settings.path.display(),
        backend = backend.name(),
        duration = %export_duration(project).as_label(),
        fps.numerator = project::fraction_numerator(settings.fps),
        fps.denominator = project::fraction_denominator(settings.fps),
        video_codec = ?settings.video_codec,
        audio_encoder = ?settings.audio_encoder,
    )
    .entered();
    tracing::info!("video export started");

    let audio_mix_started = Instant::now();
    let audio_samples = if settings.video_codec == ExportVideoCodec::Gif {
        Vec::new()
    } else {
        mix_entire_audio_track(
            project,
            settings.audio_sample_rate,
            cancelled,
            &mut progress,
        )?
    };
    crate::ensure_assets_current(&assets)?;
    let audio_mix_ns = audio_mix_started.elapsed().as_nanos();

    let parent = settings
        .path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let suffix = format!(".{}", extension_for_container(settings.container));
    let temporary = tempfile::Builder::new()
        .prefix(".shrimply-video-")
        .suffix(&suffix)
        .tempfile_in(parent)
        .map_err(|error| format!("Could not prepare video export: {error}"))?
        .into_temp_path();

    let setup_started = Instant::now();
    progress(ExportProgress::SettingUp("Opening output file"));
    let mut output = ffmpeg::format::output_as(&temporary, container_name(settings.container))
        .map_err(|error| error.to_string())?;
    let global_header = output_needs_global_header(&output);
    progress(ExportProgress::SettingUp("Preparing video renderer"));
    backend.prepare(project, settings, cancelled)?;
    check_cancelled(cancelled)?;

    progress(ExportProgress::SettingUp("Opening video encoder"));
    let mut video_encoder = if settings.video_codec == ExportVideoCodec::Gif {
        open_gif_encoder(project, settings, global_header)?
    } else {
        backend.open_video_encoder(project, settings, global_header)?
    };
    let video_time_base = video_time_base(settings.fps)?;
    let video_stream_index = {
        let mut stream = output
            .add_stream_with(video_encoder.as_ref())
            .map_err(|error| error.to_string())?;
        if settings.container == ExportContainer::Mp4
            && settings.video_codec == ExportVideoCodec::H265
        {
            // FFmpeg defaults to `hev1`; `hvc1` is required for reliable QuickTime playback.
            unsafe {
                (*stream.parameters().as_mut_ptr()).codec_tag = u32::from_le_bytes(*b"hvc1");
            }
        }
        stream.set_time_base(video_time_base);
        stream.set_rate(video_frame_rate(settings.fps)?);
        stream.set_avg_frame_rate(video_frame_rate(settings.fps)?);
        stream.index()
    };

    let audio_time_base = ffmpeg::Rational(1, settings.audio_sample_rate as i32);
    let (mut audio_encoder, audio_stream_index) = if settings.video_codec == ExportVideoCodec::Gif {
        (None, None)
    } else {
        progress(ExportProgress::SettingUp("Opening audio encoder"));
        let encoder = open_audio_encoder(settings, global_header)?;
        let stream_index = {
            let mut stream = output
                .add_stream_with(encoder.as_ref())
                .map_err(|error| error.to_string())?;
            stream.set_time_base(audio_time_base);
            stream.index()
        };
        (Some(encoder), Some(stream_index))
    };

    progress(ExportProgress::SettingUp("Writing media header"));
    check_cancelled(cancelled)?;
    output.write_header().map_err(|error| error.to_string())?;
    let video_stream_time_base = output
        .stream(video_stream_index)
        .ok_or("Video stream disappeared after writing the header")?
        .time_base();
    let audio_stream_time_base = audio_stream_index
        .map(|stream_index| {
            output
                .stream(stream_index)
                .ok_or_else(|| "Audio stream disappeared after writing the header".to_string())
                .map(|stream| stream.time_base())
        })
        .transpose()?;
    let setup_ns = setup_started.elapsed().as_nanos();

    let audio_encode_started = Instant::now();
    let mut audio_packets = match (
        audio_encoder.as_mut(),
        audio_stream_index,
        audio_stream_time_base,
    ) {
        (Some(encoder), Some(stream_index), Some(stream_time_base)) => encode_audio_packets(
            encoder,
            &audio_samples,
            settings,
            stream_index,
            stream_time_base,
            cancelled,
            &mut progress,
        )?,
        _ => VecDeque::new(),
    };
    let audio_encode_ns = audio_encode_started.elapsed().as_nanos();

    let video_encode_started = Instant::now();
    let frames = encode_video_packets(
        project,
        settings,
        backend,
        &mut video_encoder,
        video_stream_index,
        video_stream_time_base,
        audio_stream_index.zip(audio_stream_time_base),
        &mut audio_packets,
        &mut output,
        cancelled,
        &assets,
        &mut progress,
    )?;
    let video_encode_ns = video_encode_started.elapsed().as_nanos();

    progress(ExportProgress::Finalizing);
    let finalize_started = Instant::now();
    while let Some(mut packet) = audio_packets.pop_front() {
        check_cancelled(cancelled)?;
        crate::ensure_assets_current(&assets)?;
        packet.set_stream(audio_stream_index.expect("audio packets require an audio stream"));
        packet
            .write_interleaved(&mut output)
            .map_err(|error| format!("Could not write final audio packet: {error}"))?;
    }
    crate::verify_assets_current(&assets)?;
    output
        .write_trailer()
        .map_err(|error| format!("Could not write export trailer: {error}"))?;
    drop(output);
    check_cancelled(cancelled)?;
    temporary
        .persist(&settings.path)
        .map_err(|error| format!("Could not save video export: {error}"))?;
    let finalize_ns = finalize_started.elapsed().as_nanos();

    let benchmark = ExportBenchmark {
        version: 3,
        output: settings.path.clone(),
        backend: backend.name().to_string(),
        encoder: backend.encoder_label(settings.video_codec).to_string(),
        canvas_width: project.canvas_size.width,
        canvas_height: project.canvas_size.height,
        project_duration: export_duration(project),
        fps_numerator: project::fraction_numerator(settings.fps),
        fps_denominator: project::fraction_denominator(settings.fps),
        video_codec: format!("{:?}", settings.video_codec),
        decoder_sessions: backend.decoder_session_count(),
        frame_count: frames.len() as u64,
        total_elapsed_ns: export_started.elapsed().as_nanos(),
        stages: ExportStageBenchmark {
            audio_mix_ns,
            setup_ns,
            audio_encode_ns,
            video_encode_ns,
            finalize_ns,
        },
        frames,
    };
    write_benchmark(settings, &benchmark);
    tracing::info!("video export finished");
    Ok(())
}

pub fn extension_for_container(container: ExportContainer) -> &'static str {
    match container {
        ExportContainer::Mp4 => "mp4",
        ExportContainer::Mkv => "mkv",
        ExportContainer::Gif => "gif",
    }
}

fn container_name(container: ExportContainer) -> &'static str {
    match container {
        ExportContainer::Mp4 => "mp4",
        ExportContainer::Mkv => "matroska",
        ExportContainer::Gif => "gif",
    }
}

pub fn export_duration(project: &Project) -> Time {
    project
        .video_tracks
        .iter()
        .flat_map(|track| track.items.iter().map(|item| item.end))
        .chain(
            project
                .audio_tracks
                .iter()
                .flat_map(|track| track.items.iter().map(|item| item.end)),
        )
        .max()
        .unwrap_or(Time::ZERO)
}

fn validate_settings(project: &Project, settings: &ExportSettings) -> Result<(), String> {
    if project.canvas_size.width == 0 || project.canvas_size.height == 0 {
        return Err("Project canvas size must be larger than zero".to_string());
    }
    if settings.video_codec != ExportVideoCodec::Gif
        && (!project.canvas_size.width.is_multiple_of(2)
            || !project.canvas_size.height.is_multiple_of(2))
    {
        return Err("H.264 and H.265 export require an even canvas width and height".to_string());
    }
    if settings.audio_sample_rate == 0 {
        return Err("Audio sample rate must be larger than zero".to_string());
    }
    if project::fraction_numerator(settings.fps) <= 0
        || project::fraction_denominator(settings.fps) <= 0
    {
        return Err("Frame rate must be larger than zero".to_string());
    }
    if (settings.video_codec == ExportVideoCodec::Gif)
        != (settings.container == ExportContainer::Gif)
    {
        return Err("GIF encoding requires the GIF container".to_string());
    }
    Ok(())
}

fn mix_entire_audio_track<F>(
    project: &Project,
    sample_rate: u32,
    cancelled: &AtomicBool,
    progress: &mut F,
) -> Result<Vec<f32>, String>
where
    F: FnMut(ExportProgress),
{
    streaming::mix_project_offline(project, sample_rate, |current_frame, total_frames| {
        progress(ExportProgress::MixingAudio {
            current_frame,
            total_frames,
        });
        !cancelled.load(Ordering::Relaxed)
    })
}

fn open_gif_encoder(
    project: &Project,
    settings: &ExportSettings,
    global_header: bool,
) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
    let codec = ffmpeg::codec::encoder::find_by_name("gif")
        .ok_or_else(|| "FFmpeg encoder gif was not found".to_string())?;
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| error.to_string())?;
    configure_video_encoder(&mut encoder, project, settings, Pixel::PAL8, global_header)?;
    encoder
        .open_as_with(codec, ffmpeg::Dictionary::new())
        .map_err(|error| format!("Could not open gif: {error}"))
}

pub fn configure_video_encoder(
    encoder: &mut ffmpeg::codec::encoder::video::Video,
    project: &Project,
    settings: &ExportSettings,
    pixel: Pixel,
    global_header: bool,
) -> Result<(), String> {
    encoder.set_width(project.canvas_size.width);
    encoder.set_height(project.canvas_size.height);
    encoder.set_time_base(video_time_base(settings.fps)?);
    encoder.set_frame_rate(Some(video_frame_rate(settings.fps)?));
    encoder.set_format(pixel);
    encoder.set_gop(video_gop(settings));
    encoder.set_max_b_frames(settings.b_frames as usize);
    encoder.set_bit_rate(settings.bitrate_kbps as usize * 1_000);
    unsafe {
        set_bt709_video_metadata(encoder.as_mut_ptr());
        if global_header {
            (*encoder.as_mut_ptr()).flags |= sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    Ok(())
}

fn open_audio_encoder(
    settings: &ExportSettings,
    global_header: bool,
) -> Result<ffmpeg::codec::encoder::audio::Encoder, String> {
    let encoder_name = audio_encoder_name(settings.audio_encoder);
    let codec = ffmpeg::codec::encoder::find_by_name(encoder_name)
        .ok_or_else(|| format!("FFmpeg encoder {encoder_name} was not found"))?;
    let sample_format = audio_sample_format(settings.audio_encoder);
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .audio()
        .map_err(|error| error.to_string())?;
    encoder.set_rate(settings.audio_sample_rate as i32);
    encoder.set_channel_layout(ffmpeg::channel_layout::ChannelLayout::STEREO);
    encoder.set_format(sample_format);
    encoder.set_time_base(ffmpeg::Rational(1, settings.audio_sample_rate as i32));
    encoder.set_bit_rate(settings.audio_bitrate_kbps as usize * 1_000);
    unsafe {
        if global_header {
            (*encoder.as_mut_ptr()).flags |= sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    encoder
        .open_as_with(codec, ffmpeg::Dictionary::new())
        .map_err(|error| format!("Could not open {encoder_name}: {error}"))
}

fn encode_audio_packets(
    encoder: &mut ffmpeg::codec::encoder::audio::Encoder,
    samples: &[f32],
    settings: &ExportSettings,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    cancelled: &AtomicBool,
    progress: &mut dyn FnMut(ExportProgress),
) -> Result<VecDeque<ffmpeg::Packet>, String> {
    let input_frames = samples.len() / AUDIO_CHANNELS;
    progress(ExportProgress::EncodingAudio {
        current_frame: 0,
        total_frames: input_frames as u64,
    });
    let encoder_time_base = encoder.time_base();
    let frame_size = usize::try_from(encoder.frame_size())
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_AUDIO_FRAME_SIZE);
    let mut packets = VecDeque::new();
    let mut next_pts = 0_i64;
    let sample_format = audio_sample_format(settings.audio_encoder);
    let mut start_frame = 0;
    while start_frame < input_frames {
        check_cancelled(cancelled)?;
        let frames = frame_size.min(input_frames - start_frame);
        let mut frame = ffmpeg::frame::Audio::new(
            sample_format,
            frame_size,
            ffmpeg::channel_layout::ChannelLayout::STEREO,
        );
        frame.set_rate(settings.audio_sample_rate);
        frame.set_pts(Some(next_pts));
        fill_audio_frame(
            &mut frame,
            sample_format,
            &samples[start_frame * AUDIO_CHANNELS..],
            frames,
        );
        encoder
            .send_frame(&frame)
            .map_err(|error| error.to_string())?;
        receive_audio_packets(
            encoder,
            stream_index,
            encoder_time_base,
            stream_time_base,
            &mut packets,
        )?;
        start_frame += frames;
        next_pts += frame_size as i64;
        progress(ExportProgress::EncodingAudio {
            current_frame: start_frame.min(input_frames) as u64,
            total_frames: input_frames as u64,
        });
    }
    encoder.send_eof().map_err(|error| error.to_string())?;
    receive_audio_packets(
        encoder,
        stream_index,
        encoder_time_base,
        stream_time_base,
        &mut packets,
    )?;
    Ok(packets)
}

#[allow(clippy::too_many_arguments)]
fn encode_video_packets<B: VideoBackend>(
    project: &Project,
    settings: &ExportSettings,
    backend: &mut B,
    encoder: &mut ffmpeg::codec::encoder::video::Encoder,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    audio_stream: Option<(usize, ffmpeg::Rational)>,
    audio_packets: &mut VecDeque<ffmpeg::Packet>,
    output: &mut ffmpeg::format::context::Output,
    cancelled: &AtomicBool,
    assets: &[project::AssetSnapshot],
    progress: &mut dyn FnMut(ExportProgress),
) -> Result<Vec<ExportFrameBenchmark>, String> {
    let total = frame_count(export_duration(project), settings.fps)
        .ok_or_else(|| "export duration and frame rate exceed the exact range".to_string())?;
    let encoder_time_base = encoder.time_base();
    progress(ExportProgress::EncodingVideo {
        current_frame: 0,
        total_frames: total,
        fps_milli: 0,
    });
    let mut benchmarks = Vec::new();
    let mut fps_window = VecDeque::from([(0, Instant::now())]);
    for frame_index in 0..total {
        check_cancelled(cancelled)?;
        crate::ensure_assets_current(assets)?;
        let started = Instant::now();
        let position = time_from_frame(frame_index, settings.fps)
            .ok_or_else(|| "export frame exceeds the exact range".to_string())?;
        let render_started = Instant::now();
        let RenderedFrame { mut frame, timing } = if settings.video_codec == ExportVideoCodec::Gif {
            let rendered = backend.render_rgba_frame(project, settings, position, cancelled)?;
            RenderedFrame {
                frame: rgba_to_gif_frame(&rendered.frame)?,
                timing: rendered.timing,
            }
        } else {
            backend.render_video_frame(project, settings, position, cancelled)?
        };
        let render_ns = render_started.elapsed().as_nanos();
        frame.set_pts(Some(frame_index as i64));
        unsafe { (*frame.as_mut_ptr()).duration = 1 };
        let encoder_send_started = Instant::now();
        encoder
            .send_frame(&frame)
            .map_err(|error| error.to_string())?;
        let encoder_send_ns = encoder_send_started.elapsed().as_nanos();
        let packet_started = Instant::now();
        let packets_received = receive_video_packets(
            encoder,
            stream_index,
            encoder_time_base,
            stream_time_base,
            audio_stream,
            audio_packets,
            output,
        )?;
        let current_frame = frame_index + 1;
        let completed_at = Instant::now();
        fps_window.push_back((current_frame, completed_at));
        while fps_window.len() > 2
            && completed_at.duration_since(fps_window[1].1) >= Duration::from_secs(1)
        {
            fps_window.pop_front();
        }
        let (window_frame, window_started) = fps_window.front().copied().expect("FPS sample");
        progress(ExportProgress::EncodingVideo {
            current_frame,
            total_frames: total,
            fps_milli: crate::math::frames_per_second_milli(
                current_frame - window_frame,
                completed_at.duration_since(window_started),
            ),
        });
        if should_log_export_frame(frame_index, total) {
            tracing::debug!(
                frame = current_frame,
                total,
                position = position.as_label(),
                packets_received,
                "export frame encoded"
            );
        }
        benchmarks.push(ExportFrameBenchmark {
            index: frame_index,
            position,
            render_ns,
            compositor_gpu_ns: timing.compositor_ns,
            conversion_gpu_ns: timing.conversion_ns,
            encoder_send_ns,
            packet_drain_and_mux_ns: packet_started.elapsed().as_nanos(),
            packets_received,
            total_ns: started.elapsed().as_nanos(),
        });
    }
    crate::ensure_assets_current(assets)?;
    encoder.send_eof().map_err(|error| error.to_string())?;
    receive_video_packets(
        encoder,
        stream_index,
        encoder_time_base,
        stream_time_base,
        audio_stream,
        audio_packets,
        output,
    )?;
    Ok(benchmarks)
}

pub fn rgba_to_gif_frame(source: &ffmpeg::frame::Video) -> Result<ffmpeg::frame::Video, String> {
    if source.format() != Pixel::RGBA {
        return Err("GIF conversion requires an RGBA frame".to_string());
    }
    let width = source.width() as usize;
    let height = source.height() as usize;
    let row_bytes = width * std::mem::size_of::<u32>();
    let mut rgba = Vec::with_capacity(row_bytes * height);
    for row in source.data(0).chunks_exact(source.stride(0)).take(height) {
        rgba.extend_from_slice(&row[..row_bytes]);
    }
    let quantized = shrimply_math_color::quantize_gif_rgba(&rgba, width, height);
    let mut frame = ffmpeg::frame::Video::new(Pixel::PAL8, source.width(), source.height());
    let destination_stride = frame.stride(0);
    for (source_row, destination_row) in quantized
        .indices
        .chunks_exact(width)
        .zip(frame.data_mut(0).chunks_exact_mut(destination_stride))
    {
        destination_row[..width].copy_from_slice(source_row);
    }
    let palette = unsafe { (*frame.as_mut_ptr()).data[1] };
    if palette.is_null() {
        return Err("FFmpeg did not allocate a GIF palette".to_string());
    }
    let palette = unsafe { std::slice::from_raw_parts_mut(palette, sys::AVPALETTE_SIZE as usize) };
    for (color, entry) in quantized
        .palette
        .into_iter()
        .zip(palette.chunks_exact_mut(std::mem::size_of::<u32>()))
    {
        entry.copy_from_slice(&color.to_ne_bytes());
    }
    Ok(frame)
}

fn receive_audio_packets(
    encoder: &mut ffmpeg::codec::encoder::audio::Encoder,
    stream_index: usize,
    encoder_time_base: ffmpeg::Rational,
    stream_time_base: ffmpeg::Rational,
    packets: &mut VecDeque<ffmpeg::Packet>,
) -> Result<(), String> {
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                packet.set_stream(stream_index);
                packet.rescale_ts(encoder_time_base, stream_time_base);
                packets.push_back(packet);
            }
            Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => return Ok(()),
            Err(ffmpeg::Error::Eof) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn receive_video_packets(
    encoder: &mut ffmpeg::codec::encoder::video::Encoder,
    stream_index: usize,
    encoder_time_base: ffmpeg::Rational,
    stream_time_base: ffmpeg::Rational,
    audio_stream: Option<(usize, ffmpeg::Rational)>,
    audio_packets: &mut VecDeque<ffmpeg::Packet>,
    output: &mut ffmpeg::format::context::Output,
) -> Result<usize, String> {
    let mut written = 0;
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                packet.set_stream(stream_index);
                if packet.duration() == 0 {
                    packet.set_duration(1);
                }
                packet.rescale_ts(encoder_time_base, stream_time_base);
                if let Some((audio_stream_index, audio_time_base)) = audio_stream {
                    write_audio_until(
                        &packet,
                        stream_time_base,
                        audio_stream_index,
                        audio_time_base,
                        audio_packets,
                        output,
                    )?;
                }
                packet
                    .write_interleaved(output)
                    .map_err(|error| error.to_string())?;
                written += 1;
            }
            Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => return Ok(written),
            Err(ffmpeg::Error::Eof) => return Ok(written),
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn write_audio_until(
    video_packet: &ffmpeg::Packet,
    video_time_base: ffmpeg::Rational,
    audio_stream_index: usize,
    audio_time_base: ffmpeg::Rational,
    audio_packets: &mut VecDeque<ffmpeg::Packet>,
    output: &mut ffmpeg::format::context::Output,
) -> Result<(), String> {
    while audio_packets.front().is_some_and(|packet| {
        packet_is_before_or_at(packet, audio_time_base, video_packet, video_time_base)
    }) {
        let mut packet = audio_packets.pop_front().expect("front packet exists");
        packet.set_stream(audio_stream_index);
        packet
            .write_interleaved(output)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn packet_is_before_or_at(
    left: &ffmpeg::Packet,
    left_time_base: ffmpeg::Rational,
    right: &ffmpeg::Packet,
    right_time_base: ffmpeg::Rational,
) -> bool {
    let left_ts = left.dts().or_else(|| left.pts()).unwrap_or(0) as i128;
    let right_ts = right.dts().or_else(|| right.pts()).unwrap_or(0) as i128;
    left_ts * left_time_base.0 as i128 * right_time_base.1 as i128
        <= right_ts * right_time_base.0 as i128 * left_time_base.1 as i128
}

fn fill_audio_frame(
    frame: &mut ffmpeg::frame::Audio,
    sample_format: ffmpeg::format::Sample,
    samples: &[f32],
    frames: usize,
) {
    let frame_samples = frame.samples();
    match sample_format {
        ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar) => {
            for (index, sample) in frame.plane_mut::<f32>(0).iter_mut().enumerate() {
                *sample = if index < frames {
                    samples[index * AUDIO_CHANNELS].clamp(-1.0, 1.0)
                } else {
                    0.0
                };
            }
            for (index, sample) in frame.plane_mut::<f32>(1).iter_mut().enumerate() {
                *sample = if index < frames {
                    samples[index * AUDIO_CHANNELS + 1].clamp(-1.0, 1.0)
                } else {
                    0.0
                };
            }
        }
        ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed) => {
            for (index, sample) in frame.plane_mut::<(f32, f32)>(0).iter_mut().enumerate() {
                *sample = if index < frames {
                    (
                        samples[index * AUDIO_CHANNELS].clamp(-1.0, 1.0),
                        samples[index * AUDIO_CHANNELS + 1].clamp(-1.0, 1.0),
                    )
                } else {
                    (0.0, 0.0)
                };
            }
        }
        ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed) => {
            for (index, sample) in frame.plane_mut::<(i16, i16)>(0).iter_mut().enumerate() {
                *sample = if index < frames {
                    (
                        sample_i16(samples[index * AUDIO_CHANNELS]),
                        sample_i16(samples[index * AUDIO_CHANNELS + 1]),
                    )
                } else {
                    (0, 0)
                };
            }
        }
        _ => panic!("unsupported export audio sample format"),
    }
    debug_assert_eq!(frame_samples, frame.samples());
}

fn sample_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn audio_encoder_name(encoder: ExportAudioEncoder) -> &'static str {
    match encoder {
        #[cfg(not(windows))]
        ExportAudioEncoder::FdkAac => "libfdk_aac",
        #[cfg(windows)]
        ExportAudioEncoder::FdkAac => "aac",
        ExportAudioEncoder::Aac => "aac",
        #[cfg(target_os = "macos")]
        ExportAudioEncoder::AudioToolboxAac => "aac_at",
        ExportAudioEncoder::Opus => "libopus",
    }
}

fn audio_sample_format(encoder: ExportAudioEncoder) -> ffmpeg::format::Sample {
    match encoder {
        #[cfg(not(windows))]
        ExportAudioEncoder::FdkAac => {
            ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed)
        }
        #[cfg(windows)]
        ExportAudioEncoder::FdkAac => {
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar)
        }
        ExportAudioEncoder::Aac => {
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar)
        }
        #[cfg(target_os = "macos")]
        ExportAudioEncoder::AudioToolboxAac => {
            ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed)
        }
        ExportAudioEncoder::Opus => {
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed)
        }
    }
}

pub fn video_time_base(fps: Fraction) -> Result<ffmpeg::Rational, String> {
    Ok(ffmpeg::Rational(
        checked_i32(project::fraction_denominator(fps), "frame-rate denominator")?,
        checked_i32(project::fraction_numerator(fps), "frame-rate numerator")?,
    ))
}

pub fn video_frame_rate(fps: Fraction) -> Result<ffmpeg::Rational, String> {
    Ok(ffmpeg::Rational(
        checked_i32(project::fraction_numerator(fps), "frame-rate numerator")?,
        checked_i32(project::fraction_denominator(fps), "frame-rate denominator")?,
    ))
}

fn checked_i32(value: i64, label: &str) -> Result<i32, String> {
    i32::try_from(value).map_err(|_| format!("{label} is out of range"))
}

pub fn video_gop(settings: &ExportSettings) -> u32 {
    if settings.keyframe_interval_seconds == 0 {
        return 250;
    }
    let fps_num = project::fraction_numerator(settings.fps).max(1) as u128;
    let fps_den = project::fraction_denominator(settings.fps).max(1) as u128;
    ((settings.keyframe_interval_seconds as u128 * fps_num) / fps_den)
        .max(1)
        .min(u32::MAX as u128) as u32
}

fn set_bt709_video_metadata(ctx: *mut sys::AVCodecContext) {
    unsafe {
        (*ctx).color_primaries = sys::AVColorPrimaries::AVCOL_PRI_BT709;
        (*ctx).color_trc = sys::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        (*ctx).colorspace = sys::AVColorSpace::AVCOL_SPC_BT709;
        (*ctx).color_range = sys::AVColorRange::AVCOL_RANGE_MPEG;
        (*ctx).chroma_sample_location = sys::AVChromaLocation::AVCHROMA_LOC_LEFT;
    }
}

pub fn set_bt709_frame_metadata(frame: &mut ffmpeg::frame::Video) {
    unsafe {
        let raw = frame.as_mut_ptr();
        (*raw).color_primaries = sys::AVColorPrimaries::AVCOL_PRI_BT709;
        (*raw).color_trc = sys::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        (*raw).colorspace = sys::AVColorSpace::AVCOL_SPC_BT709;
        (*raw).color_range = sys::AVColorRange::AVCOL_RANGE_MPEG;
        (*raw).chroma_location = sys::AVChromaLocation::AVCHROMA_LOC_LEFT;
    }
}

pub fn ffmpeg_check(result: i32, operation: &str) -> Result<(), String> {
    if result >= 0 {
        Ok(())
    } else {
        Err(format!("{operation}: {}", ffmpeg::Error::from(result)))
    }
}

fn output_needs_global_header(output: &ffmpeg::format::context::Output) -> bool {
    unsafe {
        let format = (*output.as_ptr()).oformat;
        !format.is_null() && ((*format).flags & sys::AVFMT_GLOBALHEADER) != 0
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err("Export cancelled".to_string())
    } else {
        Ok(())
    }
}

fn should_log_export_frame(frame_index: u64, frame_count: u64) -> bool {
    frame_index < 3
        || frame_index + 1 == frame_count
        || (frame_index + 1).is_multiple_of(EXPORT_DEBUG_FRAME_PERIOD)
}

fn write_benchmark(settings: &ExportSettings, benchmark: &ExportBenchmark) {
    let mut path = settings.path.as_os_str().to_os_string();
    path.push(".benchmark.json");
    let path = PathBuf::from(path);
    let result = serde_json::to_vec_pretty(benchmark)
        .map_err(|error| error.to_string())
        .and_then(|data| std::fs::write(&path, data).map_err(|error| error.to_string()));
    match result {
        Ok(()) => tracing::info!(path = %path.display(), "export benchmark written"),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "could not write export benchmark")
        }
    }
}
