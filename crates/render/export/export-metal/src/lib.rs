#![cfg(target_os = "macos")]

use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use core::settings::VideoEncoder;
use ffmpeg::format::Pixel;
use ffmpeg_next as ffmpeg;
use shrimply_export_core::video::{
    self as core, ExportAudioEncoder, ExportContainer, ExportProgress, ExportVideoCodec,
    FrameTiming, RenderedFrame, VideoBackend,
};
use shrimply_math_core::Fraction;
use shrimply_preview_render_metal::ExportRenderer;
use shrimply_project_document::project::{Project, Time};

pub fn settings_backend() -> &'static core::settings::Encoders {
    use core::settings::Encoders;
    static ENCODERS: std::sync::OnceLock<Encoders> = std::sync::OnceLock::new();
    ENCODERS.get_or_init(|| {
        Encoders::probe(|encoder| {
            core::hardware_encoder_opens(encoder.format().ffmpeg, Pixel::NV12, |_, _| Ok(()))
        })
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoToolboxRateControl {
    AverageBitrate,
    ConstantBitrate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoToolboxProfile {
    H264Baseline,
    H264Main,
    H264High,
    HevcMain,
    HevcMain10,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum H264Entropy {
    Cavlc,
    Cabac,
}

#[derive(Clone, Debug)]
pub struct ExportSettings {
    pub maximum_temporal_decoders: usize,
    pub path: PathBuf,
    pub overwrite: bool,
    pub video_encoder: VideoEncoder,
    pub container: ExportContainer,
    pub fps: Fraction,
    pub background_alpha: u8,
    pub rate_control: VideoToolboxRateControl,
    pub profile: VideoToolboxProfile,
    pub bitrate_kbps: u32,
    pub keyframe_interval_seconds: u32,
    pub b_frames: u32,
    pub h264_entropy: H264Entropy,
    pub prioritize_speed: bool,
    pub power_efficient: bool,
    pub spatial_aq: bool,
    pub max_reference_frames: u32,
    pub audio_encoder: ExportAudioEncoder,
    pub audio_sample_rate: u32,
    pub audio_bitrate_kbps: u32,
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
        video_encoder: settings.video_encoder,
        container: settings.container,
        fps: settings.fps,
        background_alpha: settings.background_alpha,
        rate_control: match settings.rate_control {
            VideoToolboxRateControl::AverageBitrate => core::ExportRateControl::VariableBitrate,
            VideoToolboxRateControl::ConstantBitrate => core::ExportRateControl::ConstantBitrate,
        },
        constant_qp: 0,
        bitrate_kbps: settings.bitrate_kbps,
        max_bitrate_kbps: 0,
        target_quality: 0,
        keyframe_interval_seconds: settings.keyframe_interval_seconds,
        b_frames: settings.b_frames,
        audio_encoder: settings.audio_encoder,
        audio_sample_rate: settings.audio_sample_rate,
        audio_bitrate_kbps: settings.audio_bitrate_kbps,
    };
    core::export_project(
        project,
        core_settings,
        VideoToolboxBackend::new(settings),
        cancelled,
        progress,
    )
}

struct VideoToolboxBackend {
    settings: ExportSettings,
    renderer: Option<ExportRenderer>,
    scaler: Option<ffmpeg::software::scaling::Context>,
}

impl VideoToolboxBackend {
    fn new(settings: ExportSettings) -> Self {
        Self {
            settings,
            renderer: None,
            scaler: None,
        }
    }

    fn rgba_frame(
        &mut self,
        project: &Project,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<ffmpeg::frame::Video, String> {
        let pixels = self
            .renderer
            .as_mut()
            .ok_or("Metal export renderer was not prepared")?
            .render_rgba(project, position, cancelled)?;
        core::rgba_frame(
            project.canvas_size.width,
            project.canvas_size.height,
            &pixels,
        )
    }
}

impl VideoBackend for VideoToolboxBackend {
    fn name(&self) -> &'static str {
        "metal"
    }

    fn validate(&self, _project: &Project, settings: &core::ExportSettings) -> Result<(), String> {
        match (settings.video_encoder.codec(), self.settings.profile) {
            (
                ExportVideoCodec::H264,
                VideoToolboxProfile::HevcMain | VideoToolboxProfile::HevcMain10,
            ) => Err("An HEVC profile cannot be used for H.264 export".into()),
            (
                ExportVideoCodec::H265,
                VideoToolboxProfile::H264Baseline
                | VideoToolboxProfile::H264Main
                | VideoToolboxProfile::H264High,
            ) => Err("An H.264 profile cannot be used for HEVC export".into()),
            _ => Ok(()),
        }
    }

    fn prepare(
        &mut self,
        _project: &Project,
        settings: &core::ExportSettings,
        _cancelled: &AtomicBool,
    ) -> Result<(), String> {
        self.renderer = Some(ExportRenderer::new(
            settings.background_alpha,
            self.settings.maximum_temporal_decoders,
        ));
        Ok(())
    }

    fn open_video_encoder(
        &mut self,
        project: &Project,
        settings: &core::ExportSettings,
        global_header: bool,
    ) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
        let name = settings.video_encoder.format().ffmpeg;
        let codec = ffmpeg::codec::encoder::find_by_name(name)
            .ok_or_else(|| format!("FFmpeg encoder {name} was not found"))?;
        let pixel = if self.settings.profile == VideoToolboxProfile::HevcMain10 {
            Pixel::P010LE
        } else {
            Pixel::NV12
        };
        let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|error| error.to_string())?;
        core::configure_video_encoder(&mut encoder, project, settings, pixel, global_header)?;
        let mut options = ffmpeg::Dictionary::new();
        options.set("allow_sw", "0");
        options.set("require_sw", "0");
        options.set("profile", profile_name(self.settings.profile));
        if settings.video_encoder.codec() == ExportVideoCodec::H264 {
            options.set(
                "coder",
                match self.settings.h264_entropy {
                    H264Entropy::Cavlc => "cavlc",
                    H264Entropy::Cabac => "cabac",
                },
            );
        }
        options.set(
            "prio_speed",
            if self.settings.prioritize_speed {
                "1"
            } else {
                "0"
            },
        );
        options.set(
            "power_efficient",
            if self.settings.power_efficient {
                "1"
            } else {
                "0"
            },
        );
        options.set(
            "spatial_aq",
            if self.settings.spatial_aq { "1" } else { "0" },
        );
        options.set(
            "max_ref_frames",
            &self.settings.max_reference_frames.to_string(),
        );
        if self.settings.rate_control == VideoToolboxRateControl::ConstantBitrate {
            options.set("constant_bit_rate", "1");
        }
        encoder
            .open_as_with(codec, options)
            .map_err(|error| format!("Could not open {name}: {error}"))
    }

    fn render_video_frame(
        &mut self,
        project: &Project,
        _settings: &core::ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String> {
        let rgba = self.rgba_frame(project, position, cancelled)?;
        let pixel = if self.settings.profile == VideoToolboxProfile::HevcMain10 {
            Pixel::P010LE
        } else {
            Pixel::NV12
        };
        let frame = core::rgba_to_bt709(&mut self.scaler, &rgba, pixel)?;
        Ok(RenderedFrame {
            frame,
            timing: FrameTiming::default(),
        })
    }

    fn render_rgba_frame(
        &mut self,
        project: &Project,
        _settings: &core::ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String> {
        Ok(RenderedFrame {
            frame: self.rgba_frame(project, position, cancelled)?,
            timing: FrameTiming::default(),
        })
    }
}

fn profile_name(profile: VideoToolboxProfile) -> &'static str {
    match profile {
        VideoToolboxProfile::H264Baseline => "baseline",
        VideoToolboxProfile::H264Main => "main",
        VideoToolboxProfile::H264High => "high",
        VideoToolboxProfile::HevcMain => "main",
        VideoToolboxProfile::HevcMain10 => "main10",
    }
}
