use std::collections::VecDeque;
use std::ffi::CString;
use std::path::PathBuf;
use std::ptr;

use ffmpeg::sys;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::Pixel;
use libc::EAGAIN;
use serde::Serialize;
use shrimply_math_core::{Fraction, frame_count, time_from_frame};

use shrimply_audio::streaming;
use shrimply_project::project::{self, Project, Time};
use shrimply_video_cuda::compositor::{
    EXPORT_ASSETS_LOADING, RenderResourceConfig, VideoExportRenderer,
};
use shrimply_video_cuda::gpu::ExportPixelFormat;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const AUDIO_CHANNELS: usize = 2;
const DEFAULT_AUDIO_FRAME_SIZE: usize = 1024;
const VIDEO_HW_POOL_SIZE: i32 = 32;
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
pub enum ExportRateControl {
    ConstantQp,
    ConstantBitrate,
    VariableBitrate,
    VariableBitrateTargetQuality,
    Lossless,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportPreset {
    P1,
    P2,
    P3,
    P4,
    P5,
    P6,
    P7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportTuning {
    UltraHighQuality,
    HighQuality,
    LowLatency,
    UltraLowLatency,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportMultipass {
    SinglePass,
    QuarterResolution,
    FullResolution,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportProfile {
    Main,
    Main10,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportAudioEncoder {
    FdkAac,
    Aac,
    Opus,
}

#[derive(Clone, Debug)]
pub struct ExportSettings {
    pub path: PathBuf,
    pub video_codec: ExportVideoCodec,
    pub container: ExportContainer,
    pub fps: Fraction,
    pub background_alpha: u8,
    pub rate_control: ExportRateControl,
    pub constant_qp: u32,
    pub bitrate_kbps: u32,
    pub max_bitrate_kbps: u32,
    pub target_quality: u32,
    pub keyframe_interval_seconds: u32,
    pub preset: ExportPreset,
    pub tuning: ExportTuning,
    pub multipass: ExportMultipass,
    pub profile: ExportProfile,
    pub look_ahead: bool,
    pub adaptive_quantization: bool,
    pub b_frames: u32,
    pub b_frame_as_reference: bool,
    pub audio_encoder: ExportAudioEncoder,
    pub audio_sample_rate: u32,
    pub audio_bitrate_kbps: u32,
    pub maximum_temporal_decoders: usize,
    pub gpu_host_memory_gib: Fraction,
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

#[derive(Serialize)]
struct ExportBenchmark {
    version: u32,
    output: PathBuf,
    canvas_width: u32,
    canvas_height: u32,
    project_duration: Time,
    fps_numerator: i64,
    fps_denominator: i64,
    video_codec: String,
    rate_control: String,
    preset: String,
    decoder_sessions: usize,
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
    hardware_frame_ns: u128,
    convert_ns: u128,
    compositor_gpu_ns: u64,
    conversion_gpu_ns: u64,
    encoder_send_ns: u128,
    packet_drain_and_mux_ns: u128,
    packets_received: usize,
    total_ns: u128,
}

pub fn export_project<F>(
    project: Project,
    settings: ExportSettings,
    cancelled: Arc<AtomicBool>,
    progress: F,
) -> Result<(), String>
where
    F: FnMut(ExportProgress),
{
    let path = settings.path.clone();
    let output_opened = Arc::new(AtomicBool::new(false));
    let result = export_project_inner(
        project,
        settings,
        cancelled.clone(),
        output_opened.clone(),
        progress,
    );
    let result = if result.is_ok() && cancelled.load(Ordering::Relaxed) {
        Err("Export cancelled".to_string())
    } else {
        result
    };
    if result.is_err() && output_opened.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn export_project_inner<F>(
    project: Project,
    settings: ExportSettings,
    cancelled: Arc<AtomicBool>,
    output_opened: Arc<AtomicBool>,
    mut progress: F,
) -> Result<(), String>
where
    F: FnMut(ExportProgress),
{
    let export_started = Instant::now();
    let mut benchmark_path = settings.path.as_os_str().to_os_string();
    benchmark_path.push(".benchmark.json");
    let benchmark_path = PathBuf::from(benchmark_path);
    check_cancelled(&cancelled)?;
    ffmpeg::init().map_err(|error| error.to_string())?;
    project.validate()?;
    shrimply_video_cuda::validate_sam2_cache(&project)?;
    shrimply_video_cuda::validate_transparent_fill_cache(&project)?;
    validate_settings(&project, &settings)?;
    crate::ensure_output_is_not_an_asset(&project, &settings.path)?;
    let assets = crate::snapshot_assets(&project)?;
    progress(ExportProgress::SettingUp("Stabilizing source video"));
    shrimply_video_cuda::video_stabilization::ensure_project(&project)?;
    crate::ensure_assets_current(&assets)?;
    check_cancelled(&cancelled)?;
    let _span = tracing::info_span!(
        "video_export",
        path = %settings.path.display(),
        duration = %export_duration(&project).as_label(),
        fps.numerator = project::fraction_numerator(settings.fps),
        fps.denominator = project::fraction_denominator(settings.fps),
        video_codec = ?settings.video_codec,
        audio_encoder = ?settings.audio_encoder,
    )
    .entered();

    tracing::info!("video export started");

    let audio_mix_started = Instant::now();
    let audio_samples = mix_entire_audio_track(
        &project,
        settings.audio_sample_rate,
        &cancelled,
        &mut progress,
    )?;
    crate::ensure_assets_current(&assets)?;
    let audio_mix_ns = audio_mix_started.elapsed().as_nanos();
    check_cancelled(&cancelled)?;
    let setup_started = Instant::now();
    progress(ExportProgress::SettingUp("Opening output file"));
    let mut output = ffmpeg::format::output(&settings.path).map_err(|error| error.to_string())?;
    output_opened.store(true, Ordering::Relaxed);
    match std::fs::remove_file(&benchmark_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Could not remove stale export benchmark {}: {error}",
                benchmark_path.display()
            ));
        }
    }
    let global_header = output_needs_global_header(&output);

    progress(ExportProgress::SettingUp("Preparing video renderer"));
    check_cancelled(&cancelled)?;
    let video_time_base = video_time_base(settings.fps)?;
    let mut renderer = VideoExportRenderer::new_with_resources(
        settings.audio_sample_rate,
        RenderResourceConfig {
            maximum_temporal_decoders: settings.maximum_temporal_decoders,
            gpu_host_memory_gib: settings.gpu_host_memory_gib,
        },
    )?;
    let mut frame_target = match settings.video_codec {
        ExportVideoCodec::Gif => VideoFrameTarget::Gif,
        ExportVideoCodec::H264 | ExportVideoCodec::H265 => {
            progress(ExportProgress::SettingUp("Preparing hardware frames"));
            VideoFrameTarget::Nvenc(HwFrameContext::new(&project, &settings)?)
        }
    };
    check_cancelled(&cancelled)?;
    progress(ExportProgress::SettingUp("Opening video encoder"));
    check_cancelled(&cancelled)?;
    let mut video_encoder = open_video_encoder(&project, &settings, global_header, &frame_target)?;
    let video_stream_index = {
        let mut stream = output
            .add_stream_with(video_encoder.as_ref())
            .map_err(|error| error.to_string())?;
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
        check_cancelled(&cancelled)?;
        let encoder = open_audio_encoder(&settings, global_header)?;
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
    check_cancelled(&cancelled)?;
    output.write_header().map_err(|error| error.to_string())?;
    let video_stream_time_base = output
        .stream(video_stream_index)
        .ok_or_else(|| "Video stream disappeared after writing the header".to_string())?
        .time_base();
    let audio_stream_time_base = if let Some(stream_index) = audio_stream_index {
        Some(
            output
                .stream(stream_index)
                .ok_or_else(|| "Audio stream disappeared after writing the header".to_string())?
                .time_base(),
        )
    } else {
        None
    };
    tracing::debug!(
        "export stream time bases: video_requested={}/{} video_actual={}/{} audio_actual={audio_stream_time_base:?}",
        video_time_base.0,
        video_time_base.1,
        video_stream_time_base.0,
        video_stream_time_base.1,
    );
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
            &settings,
            stream_index,
            stream_time_base,
            &cancelled,
            &mut progress,
        )?,
        _ => VecDeque::new(),
    };
    let audio_encode_ns = audio_encode_started.elapsed().as_nanos();

    let video_encode_started = Instant::now();
    let frame_benchmarks = encode_video_packets(
        &project,
        &settings,
        &mut renderer,
        &mut frame_target,
        &mut video_encoder,
        video_stream_index,
        video_stream_time_base,
        audio_stream_index.zip(audio_stream_time_base),
        &mut audio_packets,
        &mut output,
        &cancelled,
        &assets,
        &mut progress,
    )?;
    let video_encode_ns = video_encode_started.elapsed().as_nanos();

    progress(ExportProgress::Finalizing);
    let finalize_started = Instant::now();
    let _finalize_span = tracing::debug_span!(
        "video_export.finalize",
        queued_audio_packets = audio_packets.len(),
    )
    .entered();
    let mut final_audio_packets = 0_usize;
    while let Some(mut packet) = audio_packets.pop_front() {
        check_cancelled(&cancelled)?;
        crate::ensure_assets_current(&assets)?;
        let audio_stream_index = audio_stream_index.expect("audio packets require an audio stream");
        let pts = packet.pts();
        let dts = packet.dts();
        let duration = packet.duration();
        packet.set_stream(audio_stream_index);
        packet
            .write_interleaved(&mut output)
            .map_err(|error| {
                format!(
                    "Could not write final audio packet {final_audio_packets}: pts={pts:?} dts={dts:?} duration={duration} error={error}"
                )
            })?;
        final_audio_packets += 1;
    }
    crate::verify_assets_current(&assets)?;
    output
        .write_trailer()
        .map_err(|error| format!("Could not write export trailer: {error}"))?;
    drop(output);
    match std::fs::metadata(&settings.path) {
        Ok(metadata) => tracing::debug!(bytes = metadata.len(), "export output finalized"),
        Err(error) => tracing::warn!("export output metadata unavailable: {error}",),
    }
    let finalize_ns = finalize_started.elapsed().as_nanos();
    let benchmark = ExportBenchmark {
        version: 2,
        output: settings.path.clone(),
        canvas_width: project.canvas_size.width,
        canvas_height: project.canvas_size.height,
        project_duration: export_duration(&project),
        fps_numerator: project::fraction_numerator(settings.fps),
        fps_denominator: project::fraction_denominator(settings.fps),
        video_codec: format!("{:?}", settings.video_codec),
        rate_control: format!("{:?}", settings.rate_control),
        preset: format!("{:?}", settings.preset),
        decoder_sessions: renderer.decoder_session_count(),
        frame_count: frame_benchmarks.len() as u64,
        total_elapsed_ns: export_started.elapsed().as_nanos(),
        stages: ExportStageBenchmark {
            audio_mix_ns,
            setup_ns,
            audio_encode_ns,
            video_encode_ns,
            finalize_ns,
        },
        frames: frame_benchmarks,
    };
    let benchmark_json = serde_json::to_vec_pretty(&benchmark)
        .map_err(|error| format!("Could not serialize export benchmark: {error}"))?;
    std::fs::write(&benchmark_path, benchmark_json).map_err(|error| {
        format!(
            "Could not write export benchmark {}: {error}",
            benchmark_path.display()
        )
    })?;
    tracing::info!(path = %benchmark_path.display(), "export benchmark written");
    renderer.shutdown();
    std::mem::forget(renderer);
    drop(video_encoder);
    drop(frame_target);
    tracing::info!(final_audio_packets, "video export finished");
    Ok(())
}

pub fn extension_for_container(container: ExportContainer) -> &'static str {
    match container {
        ExportContainer::Mp4 => "mp4",
        ExportContainer::Mkv => "mkv",
        ExportContainer::Gif => "gif",
    }
}

fn export_duration(project: &Project) -> Time {
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
        return Err("NVENC export requires an even canvas width and height".to_string());
    }
    if settings.audio_sample_rate == 0 {
        return Err("Audio sample rate must be larger than zero".to_string());
    }
    let fps_num = project::fraction_numerator(settings.fps);
    let fps_den = project::fraction_denominator(settings.fps);
    if fps_num <= 0 || fps_den <= 0 {
        return Err("Frame rate must be larger than zero".to_string());
    }
    if settings.profile == ExportProfile::Main10 && settings.video_codec == ExportVideoCodec::H264 {
        return Err("H.264 Main10 export is not supported by this NVENC path".to_string());
    }
    if (settings.video_codec == ExportVideoCodec::Gif)
        != (settings.container == ExportContainer::Gif)
    {
        return Err("GIF encoding requires the GIF container".to_string());
    }
    Ok(())
}

enum VideoFrameTarget {
    Nvenc(HwFrameContext),
    Gif,
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
    let samples =
        streaming::mix_project_offline(project, sample_rate, |current_frame, total_frames| {
            progress(ExportProgress::MixingAudio {
                current_frame,
                total_frames,
            });
            !cancelled.load(Ordering::Relaxed)
        })?;
    Ok(samples)
}

fn open_video_encoder(
    project: &Project,
    settings: &ExportSettings,
    global_header: bool,
    frame_target: &VideoFrameTarget,
) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
    if settings.video_codec == ExportVideoCodec::Gif {
        let codec = ffmpeg::codec::encoder::find_by_name("gif")
            .ok_or_else(|| "FFmpeg encoder gif was not found".to_string())?;
        let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|error| error.to_string())?;
        encoder.set_width(project.canvas_size.width.max(1));
        encoder.set_height(project.canvas_size.height.max(1));
        encoder.set_time_base(video_time_base(settings.fps)?);
        encoder.set_frame_rate(Some(video_frame_rate(settings.fps)?));
        encoder.set_format(Pixel::PAL8);
        unsafe {
            if global_header {
                (*encoder.as_mut_ptr()).flags |= sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }
        return encoder
            .open_as_with(codec, ffmpeg::Dictionary::new())
            .map_err(|error| format!("Could not open gif: {error}"));
    }

    let VideoFrameTarget::Nvenc(hw_frames) = frame_target else {
        return Err("NVENC export requires CUDA hardware frames".to_string());
    };
    let encoder_name = match settings.video_codec {
        ExportVideoCodec::H264 => "h264_nvenc",
        ExportVideoCodec::H265 => "hevc_nvenc",
        ExportVideoCodec::Gif => unreachable!("GIF encoder returned above"),
    };
    let codec = ffmpeg::codec::encoder::find_by_name(encoder_name)
        .ok_or_else(|| format!("FFmpeg encoder {encoder_name} was not found"))?;
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| error.to_string())?;

    encoder.set_width(project.canvas_size.width.max(1));
    encoder.set_height(project.canvas_size.height.max(1));
    encoder.set_time_base(video_time_base(settings.fps)?);
    encoder.set_frame_rate(Some(video_frame_rate(settings.fps)?));
    encoder.set_gop(video_gop(settings));
    encoder.set_max_b_frames(settings.b_frames as usize);
    encoder.set_bit_rate(settings.bitrate_kbps as usize * 1_000);
    encoder.set_max_bit_rate(settings.max_bitrate_kbps as usize * 1_000);

    encoder.set_format(Pixel::CUDA);
    unsafe {
        let ctx = encoder.as_mut_ptr();
        (*ctx).sw_pix_fmt = hw_frames.pixel_format().sw_format();
        (*ctx).hw_device_ctx = hw_frames.device_ref()?;
        (*ctx).hw_frames_ctx = hw_frames.encoder_ref()?;
        set_bt709_video_metadata(ctx);
        if global_header {
            (*ctx).flags |= sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    let encoder = encoder
        .open_as_with(codec, video_options(settings))
        .map_err(|error| format!("Could not open {encoder_name}: {error}"))?;
    tracing::info!(
        "Opened NVENC encoder {encoder_name}: size={}x{} fps={}/{} time_base={}/{} pix_fmt=CUDA sw_format={:?} gop={} max_b_frames={}",
        encoder.width(),
        encoder.height(),
        project::fraction_numerator(settings.fps),
        project::fraction_denominator(settings.fps),
        encoder.time_base().0,
        encoder.time_base().1,
        hw_frames.pixel_format().sw_format(),
        video_gop(settings),
        settings.b_frames
    );
    Ok(encoder)
}

fn open_audio_encoder(
    settings: &ExportSettings,
    global_header: bool,
) -> Result<ffmpeg::codec::encoder::audio::Encoder, String> {
    let encoder_name = match settings.audio_encoder {
        ExportAudioEncoder::FdkAac => "libfdk_aac",
        ExportAudioEncoder::Aac => "aac",
        ExportAudioEncoder::Opus => "libopus",
    };
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

fn rgba_to_gif_frame(source: &ffmpeg::frame::Video) -> Result<ffmpeg::frame::Video, String> {
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

#[allow(clippy::too_many_arguments)]
fn encode_video_packets(
    project: &Project,
    settings: &ExportSettings,
    renderer: &mut VideoExportRenderer,
    frame_target: &mut VideoFrameTarget,
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
    let frame_count = frame_count(export_duration(project), settings.fps)
        .ok_or_else(|| "export duration and frame rate exceed the exact range".to_string())?;
    let encoder_time_base = encoder.time_base();
    tracing::debug!(
        "export video encode start: frames={frame_count} time_base={}/{} stream_time_base={}/{}",
        encoder_time_base.0,
        encoder_time_base.1,
        stream_time_base.0,
        stream_time_base.1
    );
    progress(ExportProgress::EncodingVideo {
        current_frame: 0,
        total_frames: frame_count,
        fps_milli: 0,
    });
    let mut benchmarks = Vec::new();
    let mut fps_window = VecDeque::from([(0, Instant::now())]);

    for frame_index in 0..frame_count {
        check_cancelled(cancelled)?;
        crate::ensure_assets_current(assets)?;
        let frame_started = Instant::now();
        let position = time_from_frame(frame_index, settings.fps)
            .ok_or_else(|| "export frame exceeds the exact range".to_string())?;
        let render_started = Instant::now();
        let composited = loop {
            match renderer.render(project, position, settings.background_alpha) {
                Ok(frame) => break frame,
                Err(error) if error == EXPORT_ASSETS_LOADING => {
                    check_cancelled(cancelled)?;
                    crate::ensure_assets_current(assets)?;
                    std::thread::yield_now();
                }
                Err(error) => return Err(error),
            }
        };
        let render_ns = render_started.elapsed().as_nanos();
        let log_frame = should_log_export_frame(frame_index, frame_count);
        if log_frame {
            tracing::debug!(
                "export frame render: index={} of {} position={} composited={}",
                frame_index + 1,
                frame_count,
                position.as_label(),
                composited.debug_label()
            );
        }
        let hardware_frame_started = Instant::now();
        let mut frame = match &*frame_target {
            VideoFrameTarget::Nvenc(hw_frames) => hw_frames.frame()?,
            VideoFrameTarget::Gif => ffmpeg::frame::Video::new(
                Pixel::RGBA,
                project.canvas_size.width,
                project.canvas_size.height,
            ),
        };
        let hardware_frame_ns = hardware_frame_started.elapsed().as_nanos();
        let convert_started = Instant::now();
        let gpu_timing = match &mut *frame_target {
            VideoFrameTarget::Nvenc(hw_frames) => {
                renderer.copy_to_hw_frame(composited, &mut frame, hw_frames.pixel_format())?
            }
            VideoFrameTarget::Gif => {
                let timing = renderer.copy_to_rgba_frame(composited, &mut frame)?;
                frame = rgba_to_gif_frame(&frame)?;
                timing
            }
        };
        let convert_ns = convert_started.elapsed().as_nanos();
        if let VideoFrameTarget::Nvenc(hw_frames) = &*frame_target {
            set_bt709_frame_metadata(&mut frame);
            if log_frame {
                log_export_hw_frame(frame_index, &frame, hw_frames.pixel_format());
            }
        }
        frame.set_pts(Some(frame_index as i64));
        unsafe {
            (*frame.as_mut_ptr()).duration = 1;
        }
        let encoder_send_started = Instant::now();
        encoder
            .send_frame(&frame)
            .map_err(|error| error.to_string())?;
        let encoder_send_ns = encoder_send_started.elapsed().as_nanos();
        let packet_drain_started = Instant::now();
        let packets = receive_video_packets(
            encoder,
            stream_index,
            encoder_time_base,
            stream_time_base,
            audio_stream,
            audio_packets,
            output,
        )?;
        let packet_drain_and_mux_ns = packet_drain_started.elapsed().as_nanos();
        if log_frame {
            tracing::debug!(
                "export frame encode: index={} pts={} packets_received_after_send={packets}",
                frame_index + 1,
                frame_index
            );
        }
        let current_frame = frame_index + 1;
        let completed_at = Instant::now();
        fps_window.push_back((current_frame, completed_at));
        while fps_window.len() > 2
            && completed_at.duration_since(fps_window[1].1) >= Duration::from_secs(1)
        {
            fps_window.pop_front();
        }
        let (window_frame, window_started) = fps_window
            .front()
            .copied()
            .expect("FPS window always contains its initial sample");
        progress(ExportProgress::EncodingVideo {
            current_frame,
            total_frames: frame_count,
            fps_milli: crate::math::frames_per_second_milli(
                current_frame - window_frame,
                completed_at.duration_since(window_started),
            ),
        });
        benchmarks.push(ExportFrameBenchmark {
            index: frame_index,
            position,
            render_ns,
            hardware_frame_ns,
            convert_ns,
            compositor_gpu_ns: gpu_timing.compositor_ns,
            conversion_gpu_ns: gpu_timing.conversion_ns,
            encoder_send_ns,
            packet_drain_and_mux_ns,
            packets_received: packets,
            total_ns: frame_started.elapsed().as_nanos(),
        });
    }

    crate::ensure_assets_current(assets)?;
    encoder.send_eof().map_err(|error| error.to_string())?;
    let flushed = receive_video_packets(
        encoder,
        stream_index,
        encoder_time_base,
        stream_time_base,
        audio_stream,
        audio_packets,
        output,
    )?;
    tracing::debug!("export video flush: packets_received={flushed}");
    Ok(benchmarks)
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err("Export cancelled".to_string())
    } else {
        Ok(())
    }
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

fn set_bt709_frame_metadata(frame: &mut ffmpeg::frame::Video) {
    unsafe {
        let raw = frame.as_mut_ptr();
        (*raw).color_primaries = sys::AVColorPrimaries::AVCOL_PRI_BT709;
        (*raw).color_trc = sys::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        (*raw).colorspace = sys::AVColorSpace::AVCOL_SPC_BT709;
        (*raw).color_range = sys::AVColorRange::AVCOL_RANGE_MPEG;
        (*raw).chroma_location = sys::AVChromaLocation::AVCHROMA_LOC_LEFT;
    }
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

fn should_log_export_frame(frame_index: u64, frame_count: u64) -> bool {
    frame_index < 3
        || frame_index + 1 == frame_count
        || (frame_index + 1).is_multiple_of(EXPORT_DEBUG_FRAME_PERIOD)
}

fn log_export_hw_frame(
    frame_index: u64,
    frame: &ffmpeg::frame::Video,
    pixel_format: ExportPixelFormat,
) {
    unsafe {
        let raw = frame.as_ptr();
        if raw.is_null() {
            tracing::debug!("export hw frame: index={} null=true", frame_index + 1);
            return;
        }
        tracing::debug!(
            "export hw frame: index={} format={} expected_sw_format={:?} size={}x{} data0={:p} data1={:p} linesize0={} linesize1={} hw_frames_ctx={:p}",
            frame_index + 1,
            (*raw).format,
            pixel_format.sw_format(),
            (*raw).width,
            (*raw).height,
            (*raw).data[0],
            (*raw).data[1],
            (*raw).linesize[0],
            (*raw).linesize[1],
            (*raw).hw_frames_ctx
        );
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
        let Some(mut packet) = audio_packets.pop_front() else {
            break;
        };
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
            let left = frame.plane_mut::<f32>(0);
            for index in 0..frame_samples {
                left[index] = if index < frames {
                    samples[index * AUDIO_CHANNELS].clamp(-1.0, 1.0)
                } else {
                    0.0
                };
            }
            let right = frame.plane_mut::<f32>(1);
            for index in 0..frame_samples {
                right[index] = if index < frames {
                    samples[index * AUDIO_CHANNELS + 1].clamp(-1.0, 1.0)
                } else {
                    0.0
                };
            }
        }
        ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed) => {
            let packed = frame.plane_mut::<(f32, f32)>(0);
            for index in 0..frame_samples {
                packed[index] = if index < frames {
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
            let packed = frame.plane_mut::<(i16, i16)>(0);
            for index in 0..frame_samples {
                packed[index] = if index < frames {
                    (
                        sample_i16(samples[index * AUDIO_CHANNELS]),
                        sample_i16(samples[index * AUDIO_CHANNELS + 1]),
                    )
                } else {
                    (0, 0)
                };
            }
        }
        _ => {}
    }
}

fn sample_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn audio_sample_format(encoder: ExportAudioEncoder) -> ffmpeg::format::Sample {
    match encoder {
        ExportAudioEncoder::FdkAac => {
            ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed)
        }
        ExportAudioEncoder::Aac => {
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar)
        }
        ExportAudioEncoder::Opus => {
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed)
        }
    }
}

fn video_options(settings: &ExportSettings) -> ffmpeg::Dictionary<'static> {
    let mut options = ffmpeg::Dictionary::new();
    options.set("preset", preset_name(settings.preset));
    options.set("tune", tuning_name(settings));
    options.set("multipass", multipass_name(settings.multipass));
    options.set("profile", profile_name(settings));
    options.set("rc", rate_control_name(settings.rate_control));
    options.set("bf", &settings.b_frames.to_string());
    options.set(
        "b_ref_mode",
        if settings.b_frame_as_reference {
            "middle"
        } else {
            "disabled"
        },
    );
    options.set(
        "spatial-aq",
        if settings.adaptive_quantization {
            "1"
        } else {
            "0"
        },
    );
    options.set(
        "temporal-aq",
        if settings.adaptive_quantization {
            "1"
        } else {
            "0"
        },
    );
    options.set("rc-lookahead", if settings.look_ahead { "8" } else { "0" });
    match settings.rate_control {
        ExportRateControl::ConstantQp => options.set("qp", &settings.constant_qp.to_string()),
        ExportRateControl::VariableBitrateTargetQuality => {
            options.set("cq", &settings.target_quality.to_string())
        }
        ExportRateControl::Lossless => options.set("qp", "0"),
        ExportRateControl::ConstantBitrate | ExportRateControl::VariableBitrate => {}
    }
    options
}

fn preset_name(preset: ExportPreset) -> &'static str {
    match preset {
        ExportPreset::P1 => "p1",
        ExportPreset::P2 => "p2",
        ExportPreset::P3 => "p3",
        ExportPreset::P4 => "p4",
        ExportPreset::P5 => "p5",
        ExportPreset::P6 => "p6",
        ExportPreset::P7 => "p7",
    }
}

fn tuning_name(settings: &ExportSettings) -> &'static str {
    if settings.rate_control == ExportRateControl::Lossless {
        return "lossless";
    }
    match settings.tuning {
        ExportTuning::UltraHighQuality => "uhq",
        ExportTuning::HighQuality => "hq",
        ExportTuning::LowLatency => "ll",
        ExportTuning::UltraLowLatency => "ull",
    }
}

fn multipass_name(multipass: ExportMultipass) -> &'static str {
    match multipass {
        ExportMultipass::SinglePass => "disabled",
        ExportMultipass::QuarterResolution => "qres",
        ExportMultipass::FullResolution => "fullres",
    }
}

fn profile_name(settings: &ExportSettings) -> &'static str {
    match (settings.video_codec, settings.profile) {
        (ExportVideoCodec::H264, ExportProfile::Main) => "main",
        (ExportVideoCodec::H265, ExportProfile::Main10) => "main10",
        _ => "main",
    }
}

fn rate_control_name(rate_control: ExportRateControl) -> &'static str {
    match rate_control {
        ExportRateControl::ConstantQp => "constqp",
        ExportRateControl::ConstantBitrate => "cbr",
        ExportRateControl::VariableBitrate => "vbr",
        ExportRateControl::VariableBitrateTargetQuality => "vbr",
        ExportRateControl::Lossless => "constqp",
    }
}

fn video_time_base(fps: Fraction) -> Result<ffmpeg::Rational, String> {
    Ok(ffmpeg::Rational(
        checked_i32(project::fraction_denominator(fps), "frame-rate denominator")?,
        checked_i32(project::fraction_numerator(fps), "frame-rate numerator")?,
    ))
}

fn video_frame_rate(fps: Fraction) -> Result<ffmpeg::Rational, String> {
    Ok(ffmpeg::Rational(
        checked_i32(project::fraction_numerator(fps), "frame-rate numerator")?,
        checked_i32(project::fraction_denominator(fps), "frame-rate denominator")?,
    ))
}

fn checked_i32(value: i64, label: &str) -> Result<i32, String> {
    i32::try_from(value).map_err(|_| format!("{label} is out of range"))
}

fn video_gop(settings: &ExportSettings) -> u32 {
    if settings.keyframe_interval_seconds == 0 {
        return 250;
    }
    let fps_num = project::fraction_numerator(settings.fps).max(1) as u128;
    let fps_den = project::fraction_denominator(settings.fps).max(1) as u128;
    ((settings.keyframe_interval_seconds as u128 * fps_num) / fps_den)
        .max(1)
        .min(u32::MAX as u128) as u32
}

fn output_needs_global_header(output: &ffmpeg::format::context::Output) -> bool {
    unsafe {
        let format = (*output.as_ptr()).oformat;
        !format.is_null() && ((*format).flags & sys::AVFMT_GLOBALHEADER) != 0
    }
}

struct HwFrameContext {
    device_ref: *mut sys::AVBufferRef,
    frames_ref: *mut sys::AVBufferRef,
    width: u32,
    height: u32,
    pixel_format: ExportPixelFormat,
}

impl HwFrameContext {
    fn new(project: &Project, settings: &ExportSettings) -> Result<Self, String> {
        let width = project.canvas_size.width.max(1);
        let height = project.canvas_size.height.max(1);
        let pixel_format = if settings.profile == ExportProfile::Main10 {
            ExportPixelFormat::P010
        } else {
            ExportPixelFormat::Nv12
        };
        let mut device_ref = ptr::null_mut();
        let mut options = ptr::null_mut();
        let option_key = CString::new("primary_ctx").expect("static FFmpeg option key");
        let option_value = CString::new("1").expect("static FFmpeg option value");
        ffmpeg_check(
            unsafe {
                sys::av_dict_set(&mut options, option_key.as_ptr(), option_value.as_ptr(), 0)
            },
            "configure FFmpeg CUDA primary context",
        )?;
        let create_result = unsafe {
            sys::av_hwdevice_ctx_create(
                &mut device_ref,
                sys::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA,
                ptr::null(),
                options,
                0,
            )
        };
        unsafe { sys::av_dict_free(&mut options) };
        ffmpeg_check(create_result, "create FFmpeg CUDA hardware device")?;
        if device_ref.is_null() {
            return Err("FFmpeg returned a null CUDA hardware device".to_string());
        }

        let frames_ref = unsafe { sys::av_hwframe_ctx_alloc(device_ref) };
        if frames_ref.is_null() {
            unsafe { sys::av_buffer_unref(&mut device_ref) };
            return Err("Could not allocate FFmpeg CUDA frame context".to_string());
        }
        unsafe {
            let frames = (*frames_ref).data.cast::<sys::AVHWFramesContext>();
            (*frames).format = sys::AVPixelFormat::AV_PIX_FMT_CUDA;
            (*frames).sw_format = pixel_format.sw_format();
            (*frames).width = width as i32;
            (*frames).height = height as i32;
            (*frames).initial_pool_size = VIDEO_HW_POOL_SIZE;
        }
        if let Err(error) = ffmpeg_check(
            unsafe { sys::av_hwframe_ctx_init(frames_ref) },
            "initialize FFmpeg CUDA frame context",
        ) {
            let mut frames_ref = frames_ref;
            unsafe {
                sys::av_buffer_unref(&mut frames_ref);
                sys::av_buffer_unref(&mut device_ref);
            }
            return Err(error);
        }

        tracing::debug!(
            "export hw frames context: device_ref={:p} frames_ref={:p} format=CUDA sw_format={:?} size={}x{} pool={}",
            device_ref,
            frames_ref,
            pixel_format.sw_format(),
            width,
            height,
            VIDEO_HW_POOL_SIZE
        );

        Ok(Self {
            device_ref,
            frames_ref,
            width,
            height,
            pixel_format,
        })
    }

    fn encoder_ref(&self) -> Result<*mut sys::AVBufferRef, String> {
        let reference = unsafe { sys::av_buffer_ref(self.frames_ref) };
        if reference.is_null() {
            Err("Could not retain FFmpeg CUDA frame context".to_string())
        } else {
            Ok(reference)
        }
    }

    fn device_ref(&self) -> Result<*mut sys::AVBufferRef, String> {
        let reference = unsafe { sys::av_buffer_ref(self.device_ref) };
        if reference.is_null() {
            Err("Could not retain FFmpeg CUDA device context".to_string())
        } else {
            Ok(reference)
        }
    }

    fn frame(&self) -> Result<ffmpeg::frame::Video, String> {
        let mut frame = ffmpeg::frame::Video::empty();
        ffmpeg_check(
            unsafe { sys::av_hwframe_get_buffer(self.frames_ref, frame.as_mut_ptr(), 0) },
            "allocate FFmpeg CUDA frame",
        )?;
        unsafe {
            (*frame.as_mut_ptr()).width = self.width as i32;
            (*frame.as_mut_ptr()).height = self.height as i32;
        }
        Ok(frame)
    }

    fn pixel_format(&self) -> ExportPixelFormat {
        self.pixel_format
    }
}

impl Drop for HwFrameContext {
    fn drop(&mut self) {
        unsafe {
            sys::av_buffer_unref(&mut self.frames_ref);
            sys::av_buffer_unref(&mut self.device_ref);
        }
    }
}

fn ffmpeg_check(result: i32, operation: &str) -> Result<(), String> {
    if result >= 0 {
        Ok(())
    } else {
        Err(format!("{operation}: {}", ffmpeg::Error::from(result)))
    }
}
