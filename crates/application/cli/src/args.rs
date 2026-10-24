use shrimply_export_core::video::{
    ExportAudioEncoder, ExportContainer, ExportVideoCodec, extension_for_container,
    video_frame_rate, video_time_base,
};
use shrimply_math_core::Fraction;
use std::{ffi::OsString, path::PathBuf};

pub struct Options {
    pub project: PathBuf,
    pub output: PathBuf,
    pub codec: ExportVideoCodec,
    pub container: ExportContainer,
    pub fps: Option<Fraction>,
    pub video_bitrate: u32,
    pub audio_encoder: ExportAudioEncoder,
    pub audio_bitrate: u32,
    pub audio_sample_rate: u32,
    pub overwrite: bool,
}

pub fn parse(arguments: Vec<OsString>) -> Result<Options, String> {
    let mut project = None;
    let mut output = None;
    let mut codec = None;
    let mut container = None;
    let mut fps = None;
    let mut video_bitrate = 10_000;
    let mut audio_encoder = ExportAudioEncoder::Aac;
    let mut audio_bitrate = 192;
    let mut audio_sample_rate = 48_000;
    let mut overwrite = false;
    let mut positional_only = false;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if !positional_only && argument == "--" {
            positional_only = true;
            continue;
        }
        if positional_only || !argument.as_encoded_bytes().starts_with(b"-") {
            if project.replace(PathBuf::from(&argument)).is_some() {
                return Err(format!("Unexpected argument: {}", argument.display()));
            }
            continue;
        }
        let argument = argument
            .to_str()
            .ok_or("Option names must be valid UTF-8")?;
        let (name, inline) = argument
            .split_once('=')
            .map_or((argument, None), |(name, value)| (name, Some(value)));
        if name == "--overwrite" && inline.is_none() {
            overwrite = true;
            continue;
        }
        if !matches!(
            name,
            "-o" | "--output"
                | "--codec"
                | "--container"
                | "--fps"
                | "--video-bitrate"
                | "--audio-encoder"
                | "--audio-bitrate"
                | "--audio-sample-rate"
        ) {
            return Err(format!("Unknown option: {argument}"));
        }
        let value = inline
            .map(OsString::from)
            .or_else(|| arguments.next())
            .ok_or_else(|| format!("Missing value for {name}"))?;
        if name == "-o" || name == "--output" {
            if value.is_empty() {
                return Err("Output path must not be empty".into());
            }
            output = Some(PathBuf::from(value));
            continue;
        }
        let value = value
            .to_str()
            .ok_or_else(|| format!("Invalid value for {name}"))?;
        match name {
            "--codec" => {
                codec = Some(match value {
                    "h264" => ExportVideoCodec::H264,
                    "h265" => ExportVideoCodec::H265,
                    "gif" => ExportVideoCodec::Gif,
                    _ => return Err("Codec must be h264, h265, or gif".into()),
                })
            }
            "--container" => container = Some(parse_container(value)?),
            "--fps" => {
                let rate = value
                    .parse::<Fraction>()
                    .map_err(|_| "Invalid frame rate")?;
                if !rate.is_finite() || rate <= Fraction::from(0_u8) {
                    return Err("Frame rate must be finite and larger than zero".into());
                }
                video_frame_rate(rate)?;
                video_time_base(rate)?;
                fps = Some(rate);
            }
            "--audio-encoder" => {
                audio_encoder = match value {
                    "aac" => ExportAudioEncoder::Aac,
                    "fdk-aac" => ExportAudioEncoder::FdkAac,
                    "opus" => ExportAudioEncoder::Opus,
                    #[cfg(target_os = "macos")]
                    "audiotoolbox-aac" => ExportAudioEncoder::AudioToolboxAac,
                    _ => return Err(format!("Unsupported audio encoder: {value}")),
                }
            }
            "--video-bitrate" => video_bitrate = positive_integer(name, value)?,
            "--audio-bitrate" => audio_bitrate = positive_integer(name, value)?,
            "--audio-sample-rate" => audio_sample_rate = positive_integer(name, value)?,
            _ => unreachable!("option was checked above"),
        }
    }
    let project = project.ok_or("A project path is required")?;
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
    let mut output = output.ok_or("An output path is required (-o PATH)")?;
    let container = match output.extension() {
        Some(extension) => {
            let inferred = parse_container(&extension.to_string_lossy().to_ascii_lowercase())?;
            if container.is_some_and(|container| container != inferred) {
                return Err("Output extension conflicts with --container".into());
            }
            inferred
        }
        None => {
            let container = container
                .ok_or("Use an .mp4, .mkv, or .gif output extension, or specify --container")?;
            output.set_extension(extension_for_container(container));
            container
        }
    };
    let codec = codec.unwrap_or(if container == ExportContainer::Gif {
        ExportVideoCodec::Gif
    } else {
        ExportVideoCodec::H264
    });
    if (codec == ExportVideoCodec::Gif) != (container == ExportContainer::Gif) {
        return Err(
            "GIF encoding requires the GIF container; video encoding requires MP4 or MKV".into(),
        );
    }
    Ok(Options {
        project,
        output,
        codec,
        container,
        fps,
        video_bitrate,
        audio_encoder,
        audio_bitrate,
        audio_sample_rate,
        overwrite,
    })
}

fn parse_container(value: &str) -> Result<ExportContainer, String> {
    match value {
        "mp4" => Ok(ExportContainer::Mp4),
        "mkv" => Ok(ExportContainer::Mkv),
        "gif" => Ok(ExportContainer::Gif),
        _ => Err("Container/output extension must be mp4, mkv, or gif".into()),
    }
}

fn positive_integer(name: &str, value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0 && *value <= i32::MAX as u32)
        .ok_or_else(|| {
            format!(
                "{name} must be a positive integer no larger than {}",
                i32::MAX
            )
        })
}
