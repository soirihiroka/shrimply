//! Common native-host settings, exact frame-rate parsing, and field choices.
use super::{
    ExportAudioEncoder, ExportContainer, ExportRateControl, ExportSettings, ExportVideoCodec,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shrimply_project_document::project::Project;
pub use shrimply_project_document::project::frame_rate_choices as frame_rates;
use std::path::PathBuf;

pub const DEFAULT_VIDEO_BITRATE_KBPS: u32 = 10_000;
pub const DEFAULT_AUDIO_BITRATE_KBPS: u32 = 192;
pub const DEFAULT_AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const DEFAULT_CONSTANT_QP: u32 = 20;
pub const MAX_CONSTANT_QP: u32 = 51;

mod form;
pub use form::schema as schema_for;

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumIter)]
pub enum VideoEncoder {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    H264Nvenc,
    #[cfg(target_os = "linux")]
    H264Vaapi,
    #[cfg(target_os = "macos")]
    H264VideoToolbox,
    #[cfg(target_os = "android")]
    H264MediaCodecHardware,
    X264,
    #[cfg(target_os = "android")]
    H264MediaCodecSoftware,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    H265Nvenc,
    #[cfg(target_os = "linux")]
    H265Vaapi,
    #[cfg(target_os = "macos")]
    H265VideoToolbox,
    #[cfg(target_os = "android")]
    H265MediaCodecHardware,
    X265,
    #[cfg(target_os = "android")]
    H265MediaCodecSoftware,
    Gif,
}

pub struct Format {
    pub codec: ExportVideoCodec,
    pub value: &'static str,
    pub label: &'static str,
    pub ffmpeg: &'static str,
    pub hardware: bool,
}

impl VideoEncoder {
    pub fn format(self) -> Format {
        use ExportVideoCodec::*;
        let (codec, value, label, ffmpeg, hardware) = match self {
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            Self::H264Nvenc => (
                H264,
                "h264-nvenc",
                "H.264 (NVIDIA NVENC)",
                "h264_nvenc",
                true,
            ),
            #[cfg(target_os = "linux")]
            Self::H264Vaapi => (
                H264,
                "h264-vaapi",
                "H.264 (Intel VA-API)",
                "h264_vaapi",
                true,
            ),
            #[cfg(target_os = "macos")]
            Self::H264VideoToolbox => (
                H264,
                "h264-videotoolbox",
                "H.264 (VideoToolbox)",
                "h264_videotoolbox",
                true,
            ),
            #[cfg(target_os = "android")]
            Self::H264MediaCodecHardware => (
                H264,
                "h264-mediacodec-hw",
                "H.264 (MediaCodec HW)",
                "h264_mediacodec",
                true,
            ),
            Self::X264 => (H264, "h264-cpu", "H.264 (x264 CPU)", "libx264", false),
            #[cfg(target_os = "android")]
            Self::H264MediaCodecSoftware => (
                H264,
                "h264-mediacodec-cpu",
                "H.264 (MediaCodec CPU)",
                "h264_mediacodec",
                false,
            ),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            Self::H265Nvenc => (
                H265,
                "h265-nvenc",
                "H.265 (NVIDIA NVENC)",
                "hevc_nvenc",
                true,
            ),
            #[cfg(target_os = "linux")]
            Self::H265Vaapi => (
                H265,
                "h265-vaapi",
                "H.265 (Intel VA-API)",
                "hevc_vaapi",
                true,
            ),
            #[cfg(target_os = "macos")]
            Self::H265VideoToolbox => (
                H265,
                "h265-videotoolbox",
                "H.265 (VideoToolbox)",
                "hevc_videotoolbox",
                true,
            ),
            #[cfg(target_os = "android")]
            Self::H265MediaCodecHardware => (
                H265,
                "h265-mediacodec-hw",
                "H.265 (MediaCodec HW)",
                "hevc_mediacodec",
                true,
            ),
            Self::X265 => (H265, "h265-cpu", "H.265 (x265 CPU)", "libx265", false),
            #[cfg(target_os = "android")]
            Self::H265MediaCodecSoftware => (
                H265,
                "h265-mediacodec-cpu",
                "H.265 (MediaCodec CPU)",
                "hevc_mediacodec",
                false,
            ),
            Self::Gif => (Gif, "gif", "GIF (Software)", "gif", false),
        };
        Format {
            codec,
            value,
            label,
            ffmpeg,
            hardware,
        }
    }

    pub fn codec(self) -> ExportVideoCodec {
        self.format().codec
    }

    pub fn is_nvenc(self) -> bool {
        match self {
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            Self::H264Nvenc | Self::H265Nvenc => true,
            _ => false,
        }
    }

    fn is_vaapi(self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::H264Vaapi | Self::H265Vaapi => true,
            _ => false,
        }
    }

    fn is_mediacodec(self) -> bool {
        match self {
            #[cfg(target_os = "android")]
            Self::H264MediaCodecHardware
            | Self::H265MediaCodecHardware
            | Self::H264MediaCodecSoftware
            | Self::H265MediaCodecSoftware => true,
            _ => false,
        }
    }

    pub fn request_codec(value: &str) -> Option<ExportVideoCodec> {
        use strum::IntoEnumIterator;
        match value {
            "h264" => Some(ExportVideoCodec::H264),
            "h265" => Some(ExportVideoCodec::H265),
            value => Self::iter()
                .find(|encoder| encoder.format().value == value)
                .map(Self::codec),
        }
    }
}

pub struct Encoders(Vec<VideoEncoder>);

impl Encoders {
    pub fn probe(mut supported: impl FnMut(VideoEncoder) -> bool) -> Self {
        use strum::IntoEnumIterator;
        Self(
            VideoEncoder::iter()
                .filter(|encoder| {
                    matches!(
                        encoder,
                        VideoEncoder::X264 | VideoEncoder::X265 | VideoEncoder::Gif
                    ) || supported(*encoder)
                })
                .collect(),
        )
    }

    pub fn formats(&self) -> &[VideoEncoder] {
        &self.0
    }

    fn form_encoder(&self) -> VideoEncoder {
        self.0
            .iter()
            .copied()
            .find(|encoder| encoder.format().hardware || encoder.is_mediacodec())
            .unwrap_or(VideoEncoder::X265)
    }

    pub fn preferred(&self, codec: ExportVideoCodec) -> VideoEncoder {
        let mut matching = self
            .0
            .iter()
            .copied()
            .filter(|encoder| encoder.codec() == codec);
        matching
            .clone()
            .find(|encoder| encoder.format().hardware)
            .or_else(|| matching.next())
            .expect("built-in export encoder")
    }

    pub fn default_encoder(&self) -> VideoEncoder {
        self.preferred(ExportVideoCodec::H265)
    }

    pub fn resolve(&self, value: &str) -> Option<VideoEncoder> {
        match value {
            "h264" => Some(self.preferred(ExportVideoCodec::H264)),
            "h265" => Some(self.preferred(ExportVideoCodec::H265)),
            value => self
                .0
                .iter()
                .copied()
                .find(|encoder| encoder.format().value == value),
        }
    }
}

pub fn rate_controls(encoder: VideoEncoder) -> &'static [(ExportRateControl, &'static str)] {
    const MODES: &[(ExportRateControl, &str)] = &[
        (ExportRateControl::ConstantQp, "Constant QP"),
        (ExportRateControl::ConstantBitrate, "Constant Bitrate"),
        (ExportRateControl::VariableBitrate, "Variable Bitrate"),
        (
            ExportRateControl::VariableBitrateTargetQuality,
            "Variable Bitrate with Target Quality",
        ),
        (ExportRateControl::Lossless, "Lossless"),
    ];
    if encoder.is_nvenc() || matches!(encoder, VideoEncoder::X264 | VideoEncoder::X265) {
        MODES
    } else {
        &MODES[..3]
    }
}

pub fn validate_rate_control(settings: &ExportSettings) -> Result<(), String> {
    if settings.video_encoder != VideoEncoder::Gif
        && !rate_controls(settings.video_encoder)
            .iter()
            .any(|(mode, _)| *mode == settings.rate_control)
    {
        return Err("The selected rate control is not supported by this encoder".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportPreset {
    P1,
    P2,
    P3,
    P4,
    P5,
    P6,
    P7,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportTuning {
    UltraHighQuality,
    HighQuality,
    LowLatency,
    UltraLowLatency,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportMultipass {
    SinglePass,
    QuarterResolution,
    FullResolution,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportProfile {
    Main,
    Main10,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct EncoderOptions {
    pub max_bitrate_kbps: u32,
    pub target_quality: u32,
    pub preset: ExportPreset,
    pub tuning: ExportTuning,
    pub multipass: ExportMultipass,
    pub profile: ExportProfile,
    pub look_ahead: bool,
    pub adaptive_quantization: bool,
    pub b_frame_as_reference: bool,
}
impl Default for EncoderOptions {
    fn default() -> Self {
        Self {
            max_bitrate_kbps: DEFAULT_VIDEO_BITRATE_KBPS,
            target_quality: DEFAULT_CONSTANT_QP,
            preset: ExportPreset::P6,
            tuning: ExportTuning::HighQuality,
            multipass: ExportMultipass::QuarterResolution,
            profile: ExportProfile::Main,
            look_ahead: true,
            adaptive_quantization: true,
            b_frame_as_reference: false,
        }
    }
}
pub struct Configuration {
    pub common: ExportSettings,
    pub encoder: EncoderOptions,
}

#[derive(Default, Deserialize)]
struct Input {
    container: Option<ExportContainer>,
    fps: Option<String>,
    background_alpha: Option<u8>,
    rate_control: Option<ExportRateControl>,
    constant_qp: Option<u32>,
    bitrate_kbps: Option<u32>,
    keyframe_interval_seconds: Option<u32>,
    b_frames: Option<u32>,
    audio_encoder: Option<ExportAudioEncoder>,
    audio_sample_rate: Option<u32>,
    audio_bitrate_kbps: Option<u32>,
}

pub fn parse_configuration(
    project: &Project,
    path: PathBuf,
    request: &Value,
    encoders: &Encoders,
) -> Result<Configuration, String> {
    let (video_encoder, values) = form::normalize(project, encoders, request)?;
    let input: Input = serde_json::from_value(values.clone())
        .map_err(|error| format!("Invalid export settings: {error}"))?;
    let encoder: EncoderOptions = serde_json::from_value(values)
        .map_err(|error| format!("Invalid encoder settings: {error}"))?;
    let codec = video_encoder.codec();
    let settings = ExportSettings {
        path,
        overwrite: false,
        video_encoder,
        container: if codec == ExportVideoCodec::Gif {
            ExportContainer::Gif
        } else {
            input.container.unwrap_or(ExportContainer::Mp4)
        },
        fps: input.fps.map_or(Ok(project.fps), |value| {
            value.parse().map_err(|_| "Invalid exact export frame rate")
        })?,
        background_alpha: if codec == ExportVideoCodec::Gif {
            input.background_alpha.unwrap_or(0)
        } else {
            u8::MAX
        },
        rate_control: input.rate_control.unwrap_or(ExportRateControl::ConstantQp),
        constant_qp: input.constant_qp.unwrap_or(DEFAULT_CONSTANT_QP),
        bitrate_kbps: input.bitrate_kbps.unwrap_or(DEFAULT_VIDEO_BITRATE_KBPS),
        max_bitrate_kbps: if encoders.form_encoder().is_mediacodec() {
            0
        } else {
            encoder.max_bitrate_kbps
        },
        target_quality: encoder.target_quality,
        keyframe_interval_seconds: input.keyframe_interval_seconds.unwrap_or(0),
        b_frames: input.b_frames.unwrap_or(0),
        audio_encoder: input.audio_encoder.unwrap_or_default(),
        audio_sample_rate: input.audio_sample_rate.unwrap_or(DEFAULT_AUDIO_SAMPLE_RATE),
        audio_bitrate_kbps: input
            .audio_bitrate_kbps
            .unwrap_or(DEFAULT_AUDIO_BITRATE_KBPS),
    };
    super::validate_settings(project, &settings)?;
    if matches!(
        settings.rate_control,
        ExportRateControl::VariableBitrate | ExportRateControl::VariableBitrateTargetQuality
    ) && !encoders.form_encoder().is_mediacodec()
        && encoder.max_bitrate_kbps < settings.bitrate_kbps
    {
        return Err("Maximum video bitrate must not be below the target bitrate".into());
    }
    if settings.video_encoder.format().hardware
        && settings.video_encoder.codec() == ExportVideoCodec::H264
        && encoder.profile == ExportProfile::Main10
    {
        return Err("H.264 encoding does not support Main10; use H.265".into());
    }
    Ok(Configuration {
        common: settings,
        encoder,
    })
}
