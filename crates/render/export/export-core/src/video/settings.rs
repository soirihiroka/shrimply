//! Common native-host settings, exact frame-rate parsing, and field choices.
use super::{
    ExportAudioEncoder, ExportContainer, ExportRateControl, ExportSettings, ExportVideoCodec,
};
use serde::Deserialize;
use serde_json::{Value, json};
use shrimply_math_core::Fraction;
use shrimply_project_document::project::{self, Project};
use std::path::PathBuf;

pub const DEFAULT_VIDEO_BITRATE_KBPS: u32 = 10_000;
pub const DEFAULT_AUDIO_BITRATE_KBPS: u32 = 192;
pub const DEFAULT_AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const DEFAULT_CONSTANT_QP: u32 = 20;
pub const MAX_CONSTANT_QP: u32 = 51;

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
    let input: Input = serde_json::from_value(request.clone())
        .map_err(|error| format!("Invalid export settings: {error}"))?;
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
        background_alpha: input.background_alpha.unwrap_or(0),
        rate_control: input.rate_control.unwrap_or(ExportRateControl::ConstantQp),
        constant_qp: input.constant_qp.unwrap_or(DEFAULT_CONSTANT_QP),
        bitrate_kbps: input.bitrate_kbps.unwrap_or(DEFAULT_VIDEO_BITRATE_KBPS),
        keyframe_interval_seconds: input.keyframe_interval_seconds.unwrap_or(0),
        b_frames: input.b_frames.unwrap_or(0),
        audio_encoder: input.audio_encoder.unwrap_or(ExportAudioEncoder::Aac),
        audio_sample_rate: input.audio_sample_rate.unwrap_or(DEFAULT_AUDIO_SAMPLE_RATE),
        audio_bitrate_kbps: input
            .audio_bitrate_kbps
            .unwrap_or(DEFAULT_AUDIO_BITRATE_KBPS),
    };
    super::validate_settings(project, &settings)?;
    Ok(settings)
}

pub fn frame_rates(current: Fraction) -> Vec<(Fraction, String)> {
    let mut rates = project::COMMON_FRAME_RATES
        .iter()
        .map(|rate| (rate.value, rate.label.to_string()))
        .collect::<Vec<_>>();
    if !rates.iter().any(|(rate, _)| *rate == current) {
        rates.push((current, project::fraction_as_label(current)));
    }
    rates
}

pub fn schema(project: &Project) -> Value {
    json!({
        "filename":crate::output::default_filename(project, "mp4"),
        "defaults":{"video_codec":"h264","container":"mp4","fps":project.fps.to_string(),
            "rate_control":"constant_qp","constant_qp":DEFAULT_CONSTANT_QP,
            "bitrate_kbps":DEFAULT_VIDEO_BITRATE_KBPS,"keyframe_interval_seconds":0,"background_alpha":0,
            "audio_encoder":"aac","audio_sample_rate":DEFAULT_AUDIO_SAMPLE_RATE,"audio_bitrate_kbps":DEFAULT_AUDIO_BITRATE_KBPS},
        "video_codecs":[{"value":"h264","label":"H.264"},{"value":"h265","label":"H.265"},{"value":"gif","label":"GIF"}],
        "containers":[{"value":"mp4","label":"MP4"},{"value":"mkv","label":"Matroska"}],
        "frame_rates":frame_rates(project.fps).into_iter().map(|(value,label)| json!({"value":value.to_string(),"label":label})).collect::<Vec<_>>(),
        "rate_controls":[{"value":"constant_qp","label":"Constant QP"},{"value":"constant_bitrate","label":"Constant Bitrate"},{"value":"variable_bitrate","label":"Variable Bitrate"}],
        "audio_encoders":[{"value":"aac","label":"AAC"},{"value":"opus","label":"Opus"}],
        "audio_sample_rates":[44100,48000,96000]
    })
}
