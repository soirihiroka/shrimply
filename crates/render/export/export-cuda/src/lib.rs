#![cfg(not(target_os = "macos"))]

use std::{
    ffi::CString,
    path::PathBuf,
    ptr,
    sync::{Arc, atomic::AtomicBool},
};

use ffmpeg::format::Pixel;
use ffmpeg::sys;
use ffmpeg_next as ffmpeg;
use shrimply_export_core::video::{self as core, FrameTiming, RenderedFrame, VideoBackend};
use shrimply_math_core::Fraction;
use shrimply_project_document::project::{self, Project, Time};
use shrimply_visual_cuda::compositor::{
    EXPORT_ASSETS_LOADING, RenderResourceConfig, VideoExportRenderer,
};
use shrimply_visual_cuda::gpu::{CompositedVideoFrame, ExportPixelFormat};

const VIDEO_HW_POOL_SIZE: i32 = 32;

pub use shrimply_export_core::video::{
    ExportAudioEncoder, ExportContainer, ExportProgress, ExportVideoCodec, extension_for_container,
};

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

#[derive(Clone, Debug)]
pub struct ExportSettings {
    pub path: PathBuf,
    pub overwrite: bool,
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

pub fn export_project<F>(
    project: Project,
    settings: ExportSettings,
    cancelled: Arc<AtomicBool>,
    progress: F,
) -> Result<(), String>
where
    F: FnMut(ExportProgress),
{
    let core_settings = core::ExportSettings {
        path: settings.path.clone(),
        overwrite: settings.overwrite,
        video_codec: settings.video_codec,
        container: settings.container,
        fps: settings.fps,
        background_alpha: settings.background_alpha,
        bitrate_kbps: settings.bitrate_kbps,
        keyframe_interval_seconds: settings.keyframe_interval_seconds,
        b_frames: settings.b_frames,
        audio_encoder: settings.audio_encoder,
        audio_sample_rate: settings.audio_sample_rate,
        audio_bitrate_kbps: settings.audio_bitrate_kbps,
    };
    core::export_project(
        project,
        core_settings,
        CudaBackend::new(settings),
        cancelled,
        progress,
    )
}

struct CudaBackend {
    settings: ExportSettings,
    renderer: Option<VideoExportRenderer>,
    hardware_frames: Option<HwFrameContext>,
}

impl CudaBackend {
    fn new(settings: ExportSettings) -> Self {
        Self {
            settings,
            renderer: None,
            hardware_frames: None,
        }
    }

    fn render(
        &mut self,
        project: &Project,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<CompositedVideoFrame, String> {
        loop {
            let result = self
                .renderer
                .as_mut()
                .ok_or("CUDA export renderer was not prepared")?
                .render(project, position, self.settings.background_alpha);
            match result {
                Ok(frame) => return Ok(frame),
                Err(error) if error == EXPORT_ASSETS_LOADING => {
                    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err("Export cancelled".to_string());
                    }
                    std::thread::yield_now();
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl VideoBackend for CudaBackend {
    fn name(&self) -> &'static str {
        "cuda"
    }

    fn encoder_label(&self, codec: ExportVideoCodec) -> &'static str {
        match codec {
            ExportVideoCodec::H264 => "h264_nvenc",
            ExportVideoCodec::H265 => "hevc_nvenc",
            ExportVideoCodec::Gif => "gif",
        }
    }

    fn validate(&self, project: &Project, settings: &core::ExportSettings) -> Result<(), String> {
        if settings.video_codec != ExportVideoCodec::Gif
            && (!project.canvas_size.width.is_multiple_of(2)
                || !project.canvas_size.height.is_multiple_of(2))
        {
            return Err("NVENC export requires an even canvas width and height".to_string());
        }
        if self.settings.profile == ExportProfile::Main10
            && settings.video_codec == ExportVideoCodec::H264
        {
            return Err("H.264 Main10 export is not supported by NVENC".to_string());
        }
        Ok(())
    }

    fn prepare(
        &mut self,
        project: &Project,
        settings: &core::ExportSettings,
        _cancelled: &AtomicBool,
    ) -> Result<(), String> {
        self.renderer = Some(VideoExportRenderer::new_with_resources(
            settings.audio_sample_rate,
            RenderResourceConfig {
                maximum_temporal_decoders: self.settings.maximum_temporal_decoders,
                gpu_host_memory_gib: self.settings.gpu_host_memory_gib,
            },
        )?);
        if settings.video_codec != ExportVideoCodec::Gif {
            self.hardware_frames = Some(HwFrameContext::new(project, &self.settings)?);
        }
        Ok(())
    }

    fn open_video_encoder(
        &mut self,
        project: &Project,
        settings: &core::ExportSettings,
        global_header: bool,
    ) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
        let hardware_frames = self
            .hardware_frames
            .as_ref()
            .ok_or("NVENC export requires CUDA hardware frames")?;
        let encoder_name = self.encoder_label(settings.video_codec);
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
        encoder.set_max_bit_rate(self.settings.max_bitrate_kbps as usize * 1_000);
        encoder.set_format(Pixel::CUDA);
        unsafe {
            let context = encoder.as_mut_ptr();
            (*context).sw_pix_fmt = hardware_frames.pixel_format().sw_format();
            (*context).hw_device_ctx = hardware_frames.device_ref()?;
            (*context).hw_frames_ctx = hardware_frames.encoder_ref()?;
            set_bt709_video_metadata(context);
            if global_header {
                (*context).flags |= sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }
        encoder
            .open_as_with(codec, video_options(&self.settings))
            .map_err(|error| format!("Could not open {encoder_name}: {error}"))
    }

    fn render_video_frame(
        &mut self,
        project: &Project,
        _settings: &core::ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String> {
        let composited = self.render(project, position, cancelled)?;
        let hardware_frames = self
            .hardware_frames
            .as_ref()
            .ok_or("CUDA hardware frames were not prepared")?;
        let mut frame = hardware_frames.frame()?;
        let timing = self
            .renderer
            .as_mut()
            .expect("renderer was checked while rendering")
            .copy_to_hw_frame(composited, &mut frame, hardware_frames.pixel_format())?;
        set_bt709_frame_metadata(&mut frame);
        Ok(RenderedFrame {
            frame,
            timing: FrameTiming {
                compositor_ns: Some(timing.compositor_ns),
                conversion_ns: Some(timing.conversion_ns),
            },
        })
    }

    fn render_rgba_frame(
        &mut self,
        project: &Project,
        _settings: &core::ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String> {
        let composited = self.render(project, position, cancelled)?;
        let mut frame = ffmpeg::frame::Video::new(
            Pixel::RGBA,
            project.canvas_size.width,
            project.canvas_size.height,
        );
        let timing = self
            .renderer
            .as_mut()
            .expect("renderer was checked while rendering")
            .copy_to_rgba_frame(composited, &mut frame)?;
        Ok(RenderedFrame {
            frame,
            timing: FrameTiming {
                compositor_ns: Some(timing.compositor_ns),
                conversion_ns: Some(timing.conversion_ns),
            },
        })
    }

    fn decoder_session_count(&self) -> Option<usize> {
        self.renderer
            .as_ref()
            .map(VideoExportRenderer::decoder_session_count)
    }

    fn shutdown(&mut self) {
        if let Some(mut renderer) = self.renderer.take() {
            renderer.shutdown();
            std::mem::forget(renderer);
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
    let adaptive_quantization = if settings.adaptive_quantization {
        "1"
    } else {
        "0"
    };
    options.set("spatial-aq", adaptive_quantization);
    options.set("temporal-aq", adaptive_quantization);
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
        ExportRateControl::VariableBitrate | ExportRateControl::VariableBitrateTargetQuality => {
            "vbr"
        }
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

fn video_gop(settings: &core::ExportSettings) -> u32 {
    if settings.keyframe_interval_seconds == 0 {
        return 250;
    }
    let numerator = project::fraction_numerator(settings.fps).max(1) as u128;
    let denominator = project::fraction_denominator(settings.fps).max(1) as u128;
    ((settings.keyframe_interval_seconds as u128 * numerator) / denominator)
        .max(1)
        .min(u32::MAX as u128) as u32
}

fn set_bt709_video_metadata(context: *mut sys::AVCodecContext) {
    unsafe {
        (*context).color_primaries = sys::AVColorPrimaries::AVCOL_PRI_BT709;
        (*context).color_trc = sys::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        (*context).colorspace = sys::AVColorSpace::AVCOL_SPC_BT709;
        (*context).color_range = sys::AVColorRange::AVCOL_RANGE_MPEG;
    }
}

fn set_bt709_frame_metadata(frame: &mut ffmpeg::frame::Video) {
    unsafe {
        let frame = frame.as_mut_ptr();
        (*frame).color_primaries = sys::AVColorPrimaries::AVCOL_PRI_BT709;
        (*frame).color_trc = sys::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        (*frame).colorspace = sys::AVColorSpace::AVCOL_SPC_BT709;
        (*frame).color_range = sys::AVColorRange::AVCOL_RANGE_MPEG;
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
