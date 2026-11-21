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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Nvenc,
    Vaapi,
    MediaCodec,
}

pub fn rate_controls(backend: Backend) -> &'static [(ExportRateControl, &'static str)] {
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
    if backend == Backend::Nvenc {
        MODES
    } else {
        &MODES[..3]
    }
}

pub fn validate_backend(settings: &ExportSettings, backend: Backend) -> Result<(), String> {
    if settings.video_codec != ExportVideoCodec::Gif
        && !rate_controls(backend)
            .iter()
            .any(|(mode, _)| *mode == settings.rate_control)
    {
        return Err("The selected rate control is not supported by this hardware encoder".into());
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
    video_codec: Option<ExportVideoCodec>,
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

pub fn parse(project: &Project, path: PathBuf, request: &Value) -> Result<ExportSettings, String> {
    Ok(parse_configuration(project, path, request, Backend::MediaCodec)?.common)
}

pub fn parse_configuration(
    project: &Project,
    path: PathBuf,
    request: &Value,
    backend: Backend,
) -> Result<Configuration, String> {
    let values = form::normalize(project, backend, request)?;
    let input: Input = serde_json::from_value(values.clone())
        .map_err(|error| format!("Invalid export settings: {error}"))?;
    let encoder: EncoderOptions = serde_json::from_value(values)
        .map_err(|error| format!("Invalid encoder settings: {error}"))?;
    let codec = input.video_codec.unwrap_or(ExportVideoCodec::H264);
    let settings = ExportSettings {
        path,
        overwrite: false,
        video_codec: codec,
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
    ) && backend != Backend::MediaCodec
        && encoder.max_bitrate_kbps < settings.bitrate_kbps
    {
        return Err("Maximum video bitrate must not be below the target bitrate".into());
    }
    if settings.video_codec == ExportVideoCodec::H264 && encoder.profile == ExportProfile::Main10 {
        return Err("H.264 encoding does not support Main10; use H.265".into());
    }
    Ok(Configuration {
        common: settings,
        encoder,
    })
}

pub fn schema(project: &Project) -> Value {
    schema_for(project, Backend::MediaCodec, None)
}
