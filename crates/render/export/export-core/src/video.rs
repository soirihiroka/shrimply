use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ffmpeg::format::Pixel;
use ffmpeg::sys;
use ffmpeg_next as ffmpeg;
use libc::EAGAIN;
use serde::{Deserialize, Serialize};
use shrimply_math_core::{Fraction, frame_count, time_from_frame};
use shrimply_project_document::project::{self, Project, Time};

const EXPORT_DEBUG_FRAME_PERIOD: u64 = 300;
const VIDEO_ENCODER_STALL_TIMEOUT: Duration = Duration::from_secs(5);
// AVC and HEVC bound decoded-picture buffering to 16 pictures.
const AVC_HEVC_REORDER_CAPACITY: u32 = 16;
pub(crate) mod mux;
pub mod progress;
pub mod settings;
use settings::VideoEncoder;
mod software;

pub use software::rgba_to_bt709;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportVideoCodec {
    H264,
    H265,
    Gif,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportContainer {
    Mp4,
    Mkv,
    Gif,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportRateControl {
    ConstantQp,
    ConstantBitrate,
    VariableBitrate,
    VariableBitrateTargetQuality,
    Lossless,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportAudioEncoder {
    ShrimplyAac,
    Aac,
    #[cfg(target_os = "macos")]
    AudioToolboxAac,
    Opus,
}

impl Default for ExportAudioEncoder {
    fn default() -> Self {
        available_audio_encoders()
            .next()
            .map_or(Self::Aac, |(encoder, _)| encoder)
    }
}

#[derive(Clone, Debug)]
pub struct ExportSettings {
    pub path: PathBuf,
    pub overwrite: bool,
    pub video_encoder: VideoEncoder,
    pub container: ExportContainer,
    pub fps: Fraction,
    pub background_alpha: u8,
    pub rate_control: ExportRateControl,
    pub constant_qp: u32,
    pub bitrate_kbps: u32,
    pub max_bitrate_kbps: u32,
    pub target_quality: u32,
    pub keyframe_interval_seconds: u32,
    pub b_frames: u32,
    pub audio_encoder: ExportAudioEncoder,
    pub audio_sample_rate: u32,
    pub audio_bitrate_kbps: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "phase", content = "details", rename_all = "snake_case")]
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
    backend: B,
    cancelled: Arc<AtomicBool>,
    progress: F,
) -> Result<(), String>
where
    B: VideoBackend,
    F: FnMut(ExportProgress),
{
    export_project_to(project, settings, backend, None, cancelled, progress)
}

type OpenStream<'a> = &'a mut dyn FnMut() -> Result<ffmpeg::format::context::StreamIo, String>;

pub(crate) fn export_project_to<B, F>(
    project: Project,
    settings: ExportSettings,
    mut backend: B,
    stream: Option<OpenStream<'_>>,
    cancelled: Arc<AtomicBool>,
    progress: F,
) -> Result<(), String>
where
    B: VideoBackend,
    F: FnMut(ExportProgress),
{
    let result = export_project_inner(
        &project,
        &settings,
        &mut backend,
        stream,
        &cancelled,
        progress,
    );
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
    stream: Option<OpenStream<'_>>,
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
    let duration = export_duration(project);
    let total_frames = frame_count(duration, settings.fps)
        .filter(|&frames| i64::try_from(frames).is_ok())
        .ok_or("Export duration and frame rate exceed the exact range")?;
    let software = matches!(
        settings.video_encoder,
        VideoEncoder::X264 | VideoEncoder::X265
    );
    if !software {
        backend.validate(project, settings)?;
    }
    if stream.is_none() {
        crate::ensure_output_is_not_an_asset(project, &settings.path)?;
    }
    let assets = crate::snapshot_assets(project)?;

    progress(ExportProgress::SettingUp("Stabilizing source video"));
    shrimply_visual_core::stabilization::ensure_project(project)?;
    crate::ensure_assets_current(&assets)?;
    check_cancelled(cancelled)?;

    let _span = tracing::info_span!(
        "video_export",
        path = %settings.path.display(),
        backend = backend.name(),
        duration = %duration.as_label(),
        fps.numerator = project::fraction_numerator(settings.fps),
        fps.denominator = project::fraction_denominator(settings.fps),
        video_codec = ?settings.video_encoder.codec(),
        audio_encoder = ?settings.audio_encoder,
    )
    .entered();
    tracing::info!("video export started");

    let setup_started = Instant::now();
    progress(ExportProgress::SettingUp("Preparing video renderer"));
    backend.prepare(project, settings, cancelled)?;
    check_cancelled(cancelled)?;
    let format = container_name(settings.container);
    let global_header = crate::output::needs_global_header(format)?;
    progress(ExportProgress::SettingUp("Opening video encoder"));
    let mut video_encoder = if settings.video_encoder == VideoEncoder::Gif {
        open_gif_encoder(project, settings, global_header)?
    } else if software {
        software::open(project, settings, global_header)?
    } else {
        backend.open_video_encoder(project, settings, global_header)?
    };
    let encoder_label = video_encoder
        .codec()
        .ok_or("Missing open video encoder")?
        .name()
        .to_owned();
    let audio_encoder = if settings.video_encoder == VideoEncoder::Gif {
        None
    } else {
        progress(ExportProgress::SettingUp("Opening audio encoder"));
        Some(open_audio_encoder(settings, global_header)?)
    };
    check_cancelled(cancelled)?;
    progress(ExportProgress::SettingUp("Opening output file"));
    let (temporary, mut output) = if let Some(open) = stream {
        let mut stream = open()?;
        let seekable = unsafe { (*stream.as_mut_ptr()).seekable } != 0;
        let mut output = ffmpeg::format::output_to_stream(stream, None, Some(format))
            .map_err(|error| error.to_string())?;
        if !seekable && settings.container == ExportContainer::Mp4 {
            // Delay the first fragment's header so it can account for
            // video decode lead and audio encoder priming in the edit lists.
            ffmpeg_check(
                unsafe {
                    sys::av_opt_set(
                        (*output.as_mut_ptr()).priv_data,
                        c"movflags".as_ptr(),
                        c"delay_moov+frag_keyframe+default_base_moof".as_ptr(),
                        0,
                    )
                },
                "Configure streaming MP4",
            )?;
        }
        (None, output)
    } else {
        let temporary = crate::output::temporary_sibling(&settings.path)
            .map_err(|error| format!("Could not prepare video export: {error}"))?;
        let output =
            ffmpeg::format::output_as(&temporary, format).map_err(|error| error.to_string())?;
        (Some(temporary), output)
    };
    let video_time_base = video_time_base(settings.fps)?;
    let video_stream_index = {
        let mut stream = output
            .add_stream_with(video_encoder.as_ref())
            .map_err(|error| error.to_string())?;
        if settings.container == ExportContainer::Mp4
            && settings.video_encoder.codec() == ExportVideoCodec::H265
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

    let audio = if let Some(encoder) = audio_encoder {
        let index = encoder.add_stream(&mut output)?;
        let packets = crate::audio::AudioPackets::new(encoder, project, &assets);
        Some(mux::AudioTrack::new(index, packets, cancelled))
    } else {
        None
    };

    // MediaCodec can provide global headers with the first real packet.
    let wait_for_header = settings.video_encoder != VideoEncoder::Gif
        && unsafe { (*video_encoder.as_ptr()).extradata_size == 0 };
    progress(ExportProgress::SettingUp(if wait_for_header {
        "Preparing media header"
    } else {
        "Writing media header"
    }));
    check_cancelled(cancelled)?;
    let mut output = mux::Muxer::new(
        output,
        video_stream_index,
        audio,
        wait_for_header,
        (settings.video_encoder != VideoEncoder::Gif).then(|| {
            crate::math::decode_order::DecodeOrder::new(total_frames, AVC_HEVC_REORDER_CAPACITY)
        }),
    )?;
    let setup_ns = setup_started.elapsed().as_nanos();

    let video_encode_started = Instant::now();
    let frames = encode_video_packets(
        project,
        settings,
        backend,
        &mut video_encoder,
        &mut output,
        total_frames,
        cancelled,
        &assets,
        &mut progress,
    )?;
    let (mix_during_video_ns, audio_during_video_ns) = output.audio_timings();
    let video_encode_ns = video_encode_started
        .elapsed()
        .as_nanos()
        .saturating_sub(mix_during_video_ns + audio_during_video_ns);

    progress(ExportProgress::Finalizing);
    let finalize_started = Instant::now();
    output.finish()?;
    crate::verify_assets_current(&assets)?;
    let (audio_mix_ns, audio_encode_ns) = output.audio_timings();
    let audio_during_finalize_ns =
        (audio_mix_ns - mix_during_video_ns) + (audio_encode_ns - audio_during_video_ns);
    drop(output);
    check_cancelled(cancelled)?;
    if let Some(temporary) = temporary {
        if settings.overwrite {
            temporary.persist(&settings.path)
        } else {
            temporary.persist_noclobber(&settings.path)
        }
        .map_err(|error| format!("Could not save video export: {error}"))?;
        let finalize_ns = finalize_started
            .elapsed()
            .as_nanos()
            .saturating_sub(audio_during_finalize_ns);

        let benchmark = ExportBenchmark {
            version: 3,
            output: settings.path.clone(),
            backend: backend.name().to_string(),
            encoder: encoder_label,
            canvas_width: project.canvas_size.width,
            canvas_height: project.canvas_size.height,
            project_duration: duration,
            fps_numerator: project::fraction_numerator(settings.fps),
            fps_denominator: project::fraction_denominator(settings.fps),
            video_codec: format!("{:?}", settings.video_encoder.codec()),
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
    }
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

pub fn mime_type(container: ExportContainer) -> &'static str {
    match container {
        ExportContainer::Mp4 => "video/mp4",
        ExportContainer::Mkv => "video/x-matroska",
        ExportContainer::Gif => "image/gif",
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
    if settings.video_encoder != VideoEncoder::Gif {
        if settings.rate_control == ExportRateControl::ConstantQp
            && settings.constant_qp > settings::MAX_CONSTANT_QP
        {
            return Err("Constant QP must be between 0 and 51".into());
        }
        if matches!(
            settings.rate_control,
            ExportRateControl::ConstantBitrate
                | ExportRateControl::VariableBitrate
                | ExportRateControl::VariableBitrateTargetQuality
        ) && settings.bitrate_kbps == 0
        {
            return Err("Video bitrate must be larger than zero".into());
        }
    }
    if project.canvas_size.width == 0 || project.canvas_size.height == 0 {
        return Err("Project canvas size must be larger than zero".to_string());
    }
    if settings.video_encoder != VideoEncoder::Gif
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
    if (settings.video_encoder == VideoEncoder::Gif) != (settings.container == ExportContainer::Gif)
    {
        return Err("GIF encoding requires the GIF container".to_string());
    }
    Ok(())
}

fn open_gif_encoder(
    project: &Project,
    settings: &ExportSettings,
    global_header: bool,
) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
    let codec = ffmpeg::codec::encoder::find_by_name(settings.video_encoder.format().ffmpeg)
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

/// Open a small native encoding session to detect drivers and encoder devices,
/// rather than relying on the names compiled into FFmpeg.
pub fn hardware_encoder_opens(
    name: &str,
    pixel: Pixel,
    configure: impl FnOnce(
        &mut ffmpeg::codec::encoder::video::Video,
        &mut ffmpeg::Dictionary<'static>,
    ) -> Result<(), String>,
) -> bool {
    let Ok(mut encoder) = probe_encoder(name, pixel) else {
        return false;
    };
    let mut options = ffmpeg::Dictionary::new();
    configure(&mut encoder, &mut options).is_ok() && encoder.open_with(options).is_ok()
}

pub(crate) fn probe_encoder(
    name: &str,
    pixel: Pixel,
) -> Result<ffmpeg::codec::encoder::video::Video, String> {
    let codec = ffmpeg::codec::encoder::find_by_name(name)
        .ok_or_else(|| format!("FFmpeg encoder {name} is unavailable"))?;
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| error.to_string())?;
    encoder.set_width(256);
    encoder.set_height(256);
    encoder.set_format(pixel);
    encoder.set_time_base(ffmpeg::Rational(1, 30));
    encoder.set_frame_rate(Some(ffmpeg::Rational(30, 1)));
    encoder.set_bit_rate(1_000_000);
    Ok(encoder)
}

fn open_audio_encoder(
    settings: &ExportSettings,
    global_header: bool,
) -> Result<crate::audio::Encoder, String> {
    let bitrate = settings
        .audio_bitrate_kbps
        .checked_mul(1_000)
        .ok_or("Audio bitrate out of range")?;
    let Some((name, format)) = ffmpeg_audio_encoder(settings.audio_encoder) else {
        return crate::audio::Encoder::shrimply(settings.audio_sample_rate, bitrate);
    };
    let codec = ffmpeg::codec::encoder::find_by_name(name)
        .ok_or_else(|| format!("FFmpeg encoder {name} was not found"))?;
    crate::audio::open_encoder(
        codec,
        settings.audio_sample_rate,
        format,
        Some(bitrate as usize),
        global_header,
    )
    .map(crate::audio::Encoder::Ffmpeg)
}

#[allow(clippy::too_many_arguments)]
fn encode_video_packets<B: VideoBackend>(
    project: &Project,
    settings: &ExportSettings,
    backend: &mut B,
    encoder: &mut ffmpeg::codec::encoder::video::Encoder,
    output: &mut mux::Muxer<'_>,
    total: u64,
    cancelled: &AtomicBool,
    assets: &[project::AssetSnapshot],
    progress: &mut dyn FnMut(ExportProgress),
) -> Result<Vec<ExportFrameBenchmark>, String> {
    progress(ExportProgress::EncodingVideo {
        current_frame: 0,
        total_frames: total,
        fps_milli: 0,
    });
    let mut benchmarks = Vec::new();
    let mut scaler = None;
    let mut fps_window = VecDeque::from([(0, Instant::now())]);
    for frame_index in 0..total {
        check_cancelled(cancelled)?;
        crate::ensure_assets_current(assets)?;
        let started = Instant::now();
        let position = time_from_frame(frame_index, settings.fps)
            .ok_or_else(|| "export frame exceeds the exact range".to_string())?;
        let render_started = Instant::now();
        let RenderedFrame { mut frame, timing } = if settings.video_encoder == VideoEncoder::Gif {
            let rendered = backend.render_rgba_frame(project, settings, position, cancelled)?;
            RenderedFrame {
                frame: rgba_to_gif_frame(&rendered.frame)?,
                timing: rendered.timing,
            }
        } else if matches!(
            settings.video_encoder,
            VideoEncoder::X264 | VideoEncoder::X265
        ) {
            let mut rendered = backend.render_rgba_frame(project, settings, position, cancelled)?;
            rendered.frame = rgba_to_bt709(&mut scaler, &rendered.frame, encoder.format())?;
            rendered
        } else {
            backend.render_video_frame(project, settings, position, cancelled)?
        };
        let render_ns = render_started.elapsed().as_nanos();
        frame.set_pts(Some(frame_index as i64));
        unsafe { (*frame.as_mut_ptr()).duration = 1 };
        let encoder_send_started = Instant::now();
        let drained = send_video_frame(encoder, output, Some(&frame), cancelled)?;
        let encoder_send_ns = encoder_send_started.elapsed().as_nanos();
        let packet_started = Instant::now();
        let packets_received = drained + receive_video_packets(encoder, output, cancelled, false)?;
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
    send_video_frame(encoder, output, None, cancelled)?;
    receive_video_packets(encoder, output, cancelled, true)?;
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
    for (color, entry) in quantized.palette.into_iter().zip(
        palette
            .as_chunks_mut::<{ std::mem::size_of::<u32>() }>()
            .0
            .iter_mut(),
    ) {
        entry.copy_from_slice(&color.to_ne_bytes());
    }
    Ok(frame)
}

/// Package an explicit GPU readback for CPU-only image encoders such as GIF.
pub fn rgba_frame(width: u32, height: u32, bytes: &[u8]) -> Result<ffmpeg::frame::Video, String> {
    let row_bytes = (width as usize)
        .checked_mul(size_of::<u32>())
        .ok_or("RGBA row size overflow")?;
    if row_bytes.checked_mul(height as usize) != Some(bytes.len()) || row_bytes == 0 || height == 0
    {
        return Err("RGBA readback does not match the frame dimensions".into());
    }
    let mut frame = ffmpeg::frame::Video::new(Pixel::RGBA, width, height);
    let stride = frame.stride(0);
    for (destination, source) in frame
        .data_mut(0)
        .chunks_mut(stride)
        .zip(bytes.chunks_exact(row_bytes))
    {
        destination[..row_bytes].copy_from_slice(source);
    }
    Ok(frame)
}

fn send_video_frame(
    encoder: &mut ffmpeg::codec::encoder::video::Encoder,
    output: &mut mux::Muxer<'_>,
    frame: Option<&ffmpeg::frame::Video>,
    cancelled: &AtomicBool,
) -> Result<usize, String> {
    let mut written = 0;
    let mut last_progress = Instant::now();
    loop {
        check_cancelled(cancelled)?;
        let result = match frame {
            Some(frame) => encoder.send_frame(frame),
            None => encoder.send_eof(),
        };
        match result {
            Ok(()) => return Ok(written),
            Err(ffmpeg::Error::Eof) if frame.is_none() => return Ok(written),
            Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => {
                let drained = receive_video_packets(encoder, output, cancelled, false)?;
                written += drained;
                if drained != 0 {
                    last_progress = Instant::now();
                } else if last_progress.elapsed() >= VIDEO_ENCODER_STALL_TIMEOUT {
                    return Err("Video encoder stalled while accepting frames".into());
                } else {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn receive_video_packets(
    encoder: &mut ffmpeg::codec::encoder::video::Encoder,
    output: &mut mux::Muxer<'_>,
    cancelled: &AtomicBool,
    draining: bool,
) -> Result<usize, String> {
    let mut written = 0;
    let mut last_packet = Instant::now();
    loop {
        check_cancelled(cancelled)?;
        let mut packet = ffmpeg::Packet::empty();
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                output.write_video(packet, encoder)?;
                written += 1;
                last_packet = Instant::now();
            }
            Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => {
                if !draining {
                    return Ok(written);
                }
                if last_packet.elapsed() >= VIDEO_ENCODER_STALL_TIMEOUT {
                    return Err("Video encoder stalled while draining packets".into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(ffmpeg::Error::Eof) if draining => return Ok(written),
            Err(error) => return Err(format!("Receive encoded video: {error}")),
        }
    }
}

pub fn available_audio_encoders() -> impl Iterator<Item = (ExportAudioEncoder, &'static str)> {
    [
        #[cfg(target_os = "macos")]
        (ExportAudioEncoder::AudioToolboxAac, "AudioToolbox AAC"),
        (ExportAudioEncoder::Aac, "AAC"),
        (ExportAudioEncoder::ShrimplyAac, "Shrimply AAC"),
        (ExportAudioEncoder::Opus, "Opus"),
    ]
    .into_iter()
    .filter(|(encoder, _)| {
        ffmpeg_audio_encoder(*encoder)
            .is_none_or(|(name, _)| ffmpeg::codec::encoder::find_by_name(name).is_some())
    })
}

fn ffmpeg_audio_encoder(
    encoder: ExportAudioEncoder,
) -> Option<(&'static str, ffmpeg::format::Sample)> {
    use ffmpeg::format::{Sample, sample::Type};
    Some(match encoder {
        ExportAudioEncoder::ShrimplyAac => return None,
        ExportAudioEncoder::Aac => ("aac", Sample::F32(Type::Planar)),
        #[cfg(target_os = "macos")]
        ExportAudioEncoder::AudioToolboxAac => ("aac_at", Sample::I16(Type::Packed)),
        ExportAudioEncoder::Opus => ("libopus", Sample::F32(Type::Packed)),
    })
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
