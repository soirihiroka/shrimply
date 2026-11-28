use shrimply_editing_api::protocol::RenderProjectRequest;
use shrimply_export_core::video::settings::VideoEncoder;
use shrimply_export_core::video::{
    ExportAudioEncoder, ExportContainer, ExportVideoCodec, extension_for_container,
    video_frame_rate, video_time_base,
};
use shrimply_math_core::{Fraction, fraction_new};
use std::path::PathBuf;

const DEFAULT_VIDEO_BITRATE: u32 = 10_000;
const DEFAULT_AUDIO_BITRATE: u32 = 192;
const DEFAULT_AUDIO_SAMPLE_RATE: u32 = 48_000;

pub struct Options {
    pub project: PathBuf,
    pub output: PathBuf,
    pub format: String,
    pub container: ExportContainer,
    pub fps: Option<Fraction>,
    pub video_bitrate: u32,
    pub audio_encoder: ExportAudioEncoder,
    pub audio_bitrate: u32,
    pub audio_sample_rate: u32,
    pub overwrite: bool,
    pub if_revision: Option<String>,
}

pub fn from_request(
    project: PathBuf,
    request: RenderProjectRequest,
    if_revision: Option<String>,
) -> Result<Options, String> {
    if !project
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["shrimp", "sjson", "json"]
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
    {
        return Err("Render requires a .shrimp, .sjson, or .json project. Convert imported timelines in the editor first.".into());
    }
    if request.output.is_empty() {
        return Err("output path must not be empty".into());
    }
    let mut output = PathBuf::from(request.output);
    let explicit_container = request
        .container
        .as_deref()
        .map(parse_container)
        .transpose()?;
    let container = match output.extension() {
        Some(extension) => {
            let inferred = parse_container(&extension.to_string_lossy().to_ascii_lowercase())?;
            if explicit_container.is_some_and(|container| container != inferred) {
                return Err("output extension conflicts with container".into());
            }
            inferred
        }
        None => {
            let container = explicit_container
                .ok_or("use an .mp4, .mkv, or .gif output extension, or specify container")?;
            output.set_extension(extension_for_container(container));
            container
        }
    };
    let format = request.codec.unwrap_or_else(|| {
        if container == ExportContainer::Gif {
            "gif"
        } else {
            "h264"
        }
        .into()
    });
    let codec =
        VideoEncoder::request_codec(&format).ok_or("Unknown video format for this platform")?;
    if (codec == ExportVideoCodec::Gif) != (container == ExportContainer::Gif) {
        return Err(
            "GIF encoding requires the GIF container; video encoding requires MP4 or MKV".into(),
        );
    }
    let fps = request
        .fps
        .map(|fps| {
            if fps.numerator <= 0 || fps.denominator <= 0 {
                return Err("frame rate must be positive".to_string());
            }
            let rate = fraction_new(fps.numerator, fps.denominator);
            video_frame_rate(rate)?;
            video_time_base(rate)?;
            Ok(rate)
        })
        .transpose()?;
    let video_bitrate = request.video_bitrate.unwrap_or(DEFAULT_VIDEO_BITRATE);
    let audio_bitrate = request.audio_bitrate.unwrap_or(DEFAULT_AUDIO_BITRATE);
    let audio_sample_rate = request
        .audio_sample_rate
        .unwrap_or(DEFAULT_AUDIO_SAMPLE_RATE);
    for (name, value) in [
        ("video bitrate", video_bitrate),
        ("audio bitrate", audio_bitrate),
        ("audio sample rate", audio_sample_rate),
    ] {
        if value == 0 || value > i32::MAX as u32 {
            return Err(format!(
                "{name} must be positive and fit a signed 32-bit integer"
            ));
        }
    }
    let audio_encoder = match request.audio_encoder.as_deref() {
        None => ExportAudioEncoder::default(),
        Some("aac") => ExportAudioEncoder::Aac,
        Some("shrimply-aac") => ExportAudioEncoder::ShrimplyAac,
        Some("opus") => ExportAudioEncoder::Opus,
        #[cfg(target_os = "macos")]
        Some("audiotoolbox-aac") => ExportAudioEncoder::AudioToolboxAac,
        Some(name) => return Err(format!("unsupported audio encoder: {name}")),
    };
    Ok(Options {
        project,
        output,
        format,
        container,
        fps,
        video_bitrate,
        audio_encoder,
        audio_bitrate,
        audio_sample_rate,
        overwrite: request.overwrite,
        if_revision,
    })
}

fn parse_container(value: &str) -> Result<ExportContainer, String> {
    match value {
        "mp4" => Ok(ExportContainer::Mp4),
        "mkv" => Ok(ExportContainer::Mkv),
        "gif" => Ok(ExportContainer::Gif),
        _ => Err("container/output extension must be mp4, mkv, or gif".into()),
    }
}
