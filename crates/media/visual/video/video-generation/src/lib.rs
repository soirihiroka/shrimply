use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use shrimply_asset::Asset;
use shrimply_compute_client::AudioUpload;
pub use shrimply_math_core::Fraction;
use shrimply_math_core::{deserialize_fraction, serialize_fraction};

const MESSAGE_PACK: &str = "application/msgpack";
const MESSAGE_PACK_STREAM: &str = "application/x-msgpack-stream";
const MAXIMUM_MEDIA_BYTES: usize = 256 * 1024 * 1024;
const MAXIMUM_REQUEST_MEDIA_BYTES: usize = 512 * 1024 * 1024;
const MAXIMUM_EVENT_BYTES: usize = 2 * 1024 * 1024;
const WAN_MODEL_PREFIX: &str = "Wan-AI/Wan";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct VideoGenerationSettings {
    pub model: Option<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, VideoGenerationValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VideoGenerationValue {
    Text {
        value: String,
    },
    Select {
        value: String,
    },
    Number {
        #[serde(
            deserialize_with = "deserialize_fraction",
            serialize_with = "serialize_fraction"
        )]
        value: Fraction,
    },
    Media {
        items: Vec<MediaAsset>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MediaAsset {
    pub kind: MediaKind,
    pub value: Asset,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct VideoGenerationModel {
    pub id: String,
    pub label: String,
    pub inputs: Vec<InputDefinition>,
    pub output: ModelOutput,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub struct ModelOutput {
    pub video: bool,
    pub audio: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputDefinition {
    Text {
        key: String,
        label: String,
        default: String,
        required: bool,
        multiline: bool,
        max_length: usize,
        visible_when: Option<VisibleWhen>,
    },
    Select {
        key: String,
        label: String,
        options: Vec<SelectOption>,
        default: String,
        visible_when: Option<VisibleWhen>,
    },
    Number {
        key: String,
        label: String,
        #[serde(deserialize_with = "deserialize_fraction")]
        default: Fraction,
        #[serde(deserialize_with = "deserialize_fraction")]
        minimum: Fraction,
        #[serde(deserialize_with = "deserialize_fraction")]
        maximum: Fraction,
        #[serde(deserialize_with = "deserialize_fraction")]
        step: Fraction,
        presentation: NumberPresentation,
        visible_when: Option<VisibleWhen>,
    },
    Media {
        key: String,
        label: String,
        accepted: Vec<MediaKind>,
        minimum_items: usize,
        maximum_items: usize,
        ordered: bool,
        visible_when: Option<VisibleWhen>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct VisibleWhen {
    pub input: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NumberPresentation {
    Number,
    Slider,
}

impl InputDefinition {
    pub fn key(&self) -> &str {
        match self {
            Self::Text { key, .. }
            | Self::Select { key, .. }
            | Self::Number { key, .. }
            | Self::Media { key, .. } => key,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Text { label, .. }
            | Self::Select { label, .. }
            | Self::Number { label, .. }
            | Self::Media { label, .. } => label,
        }
    }

    pub fn visible_when(&self) -> Option<&VisibleWhen> {
        match self {
            Self::Text { visible_when, .. }
            | Self::Select { visible_when, .. }
            | Self::Number { visible_when, .. }
            | Self::Media { visible_when, .. } => visible_when.as_ref(),
        }
    }

    fn default_value(&self) -> VideoGenerationValue {
        match self {
            Self::Text { default, .. } => VideoGenerationValue::Text {
                value: default.clone(),
            },
            Self::Select { default, .. } => VideoGenerationValue::Select {
                value: default.clone(),
            },
            Self::Number { default, .. } => VideoGenerationValue::Number { value: *default },
            Self::Media { .. } => VideoGenerationValue::Media { items: Vec::new() },
        }
    }

    fn accepts(&self, value: &VideoGenerationValue) -> bool {
        match (self, value) {
            (Self::Text { .. }, VideoGenerationValue::Text { .. })
            | (Self::Number { .. }, VideoGenerationValue::Number { .. }) => true,
            (Self::Select { options, .. }, VideoGenerationValue::Select { value }) => {
                options.iter().any(|option| option.value == *value)
            }
            (
                Self::Media {
                    accepted,
                    maximum_items,
                    ..
                },
                VideoGenerationValue::Media { items },
            ) => {
                items.len() <= *maximum_items
                    && items.iter().all(|item| accepted.contains(&item.kind))
            }
            _ => false,
        }
    }
}

pub fn sync_settings(settings: &mut VideoGenerationSettings, model: &VideoGenerationModel) {
    settings.model = Some(model.id.clone());
    settings.inputs.retain(|key, value| {
        model
            .inputs
            .iter()
            .find(|input| input.key() == key)
            .is_some_and(|input| input.accepts(value))
    });
    for input in &model.inputs {
        settings
            .inputs
            .entry(input.key().to_string())
            .or_insert_with(|| input.default_value());
    }
}

pub fn is_visible(
    input: &InputDefinition,
    values: &BTreeMap<String, VideoGenerationValue>,
) -> bool {
    let Some(condition) = input.visible_when() else {
        return true;
    };
    let Some(VideoGenerationValue::Select { value }) = values.get(&condition.input) else {
        return false;
    };
    condition.values.contains(value)
}

#[derive(Serialize)]
pub struct GenerationRequest {
    model: String,
    inputs: BTreeMap<String, InputValue>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum InputValue {
    Text {
        value: String,
    },
    Select {
        value: String,
    },
    Number {
        #[serde(serialize_with = "serialize_fraction")]
        value: Fraction,
    },
    Media {
        items: Vec<EncodedMedia>,
    },
}

#[derive(Serialize)]
struct EncodedMedia {
    kind: MediaKind,
    filename: String,
    data: MediaData,
}

#[derive(Serialize)]
#[serde(untagged)]
enum MediaData {
    Bytes(#[serde(with = "serde_bytes")] Vec<u8>),
    Audio(AudioUpload),
}

pub fn generation_request(
    model: &VideoGenerationModel,
    settings: &VideoGenerationSettings,
) -> Result<GenerationRequest, String> {
    if settings.model.as_deref() != Some(&model.id) {
        return Err("Video-generation settings do not match the selected model".to_string());
    }
    let active = model
        .inputs
        .iter()
        .filter(|input| is_visible(input, &settings.inputs))
        .collect::<Vec<_>>();
    let mut inputs = BTreeMap::new();
    let mut total_media_bytes = 0_usize;
    for definition in active {
        let value = settings
            .inputs
            .get(definition.key())
            .ok_or_else(|| format!("Missing input {}", definition.label()))?;
        let encoded = match (definition, value) {
            (
                InputDefinition::Text {
                    required,
                    max_length,
                    ..
                },
                VideoGenerationValue::Text { value },
            ) => {
                if *required && value.trim().is_empty() {
                    return Err(format!("{} is required", definition.label()));
                }
                if value.chars().count() > *max_length {
                    return Err(format!("{} is too long", definition.label()));
                }
                InputValue::Text {
                    value: value.clone(),
                }
            }
            (InputDefinition::Select { options, .. }, VideoGenerationValue::Select { value })
                if options.iter().any(|option| option.value == *value) =>
            {
                InputValue::Select {
                    value: value.clone(),
                }
            }
            (InputDefinition::Number { .. }, VideoGenerationValue::Number { value }) => {
                InputValue::Number { value: *value }
            }
            (
                InputDefinition::Media {
                    accepted,
                    minimum_items,
                    maximum_items,
                    ..
                },
                VideoGenerationValue::Media { items },
            ) => {
                if !(*minimum_items..=*maximum_items).contains(&items.len()) {
                    return Err(format!(
                        "{} has the wrong number of files",
                        definition.label()
                    ));
                }
                let items = items
                    .iter()
                    .map(|item| {
                        if !accepted.contains(&item.kind) {
                            return Err(format!(
                                "{} contains an unsupported file",
                                definition.label()
                            ));
                        }
                        let data = item.value.read()?;
                        if data.len() > MAXIMUM_MEDIA_BYTES {
                            return Err(format!(
                                "{} contains a file larger than 256 MiB",
                                definition.label()
                            ));
                        }
                        total_media_bytes = total_media_bytes.saturating_add(data.len());
                        Ok(EncodedMedia {
                            kind: item.kind,
                            filename: item
                                .value
                                .path()
                                .file_name()
                                .and_then(|value| value.to_str())
                                .unwrap_or("media")
                                .to_string(),
                            data: if item.kind == MediaKind::Audio {
                                MediaData::Audio(AudioUpload::Encoded { data })
                            } else {
                                MediaData::Bytes(data)
                            },
                        })
                    })
                    .collect::<Result<_, String>>()?;
                InputValue::Media { items }
            }
            _ => return Err(format!("{} has an invalid value", definition.label())),
        };
        inputs.insert(definition.key().to_string(), encoded);
    }
    if total_media_bytes > MAXIMUM_REQUEST_MEDIA_BYTES {
        return Err("Video-generation media exceeds 512 MiB".to_string());
    }
    Ok(GenerationRequest {
        model: model.id.clone(),
        inputs,
    })
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct GenerationResult {
    #[serde(deserialize_with = "deserialize_fraction")]
    pub duration: Fraction,
    #[serde(deserialize_with = "deserialize_fraction")]
    pub frame_rate: Fraction,
    pub width: u32,
    pub height: u32,
    pub video_streams: u32,
    pub audio_streams: u32,
}

#[derive(Deserialize)]
struct ModelsResponse {
    models: Vec<VideoGenerationModel>,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ServerError,
}

#[derive(Deserialize)]
struct ServerError {
    message: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum GenerationEvent {
    Queued {
        position: usize,
    },
    Progress {
        message: String,
    },
    OutputStart {
        content_type: String,
        bytes: u64,
    },
    OutputChunk {
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    Result {
        result: GenerationResult,
    },
    Error {
        message: String,
    },
}

pub fn models(server_url: &str) -> Result<Vec<VideoGenerationModel>, String> {
    let server_url = server_url.trim();
    if server_url.is_empty() {
        return Err("Server URL is empty".to_string());
    }
    let endpoint = format!(
        "{}/video-generation/models",
        server_url.trim_end_matches('/')
    );
    tracing::info!(%endpoint, "Requesting video-generation models");
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?
        .get(&endpoint)
        .header(ACCEPT, MESSAGE_PACK)
        .send()
        .map_err(|error| error.to_string())?;
    tracing::info!(%endpoint, status = %response.status(), "Received video-generation models response");
    decode_models(response)
}

fn decode_models(
    response: reqwest::blocking::Response,
) -> Result<Vec<VideoGenerationModel>, String> {
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = response.bytes().map_err(|error| error.to_string())?;
    if !status.is_success() {
        let message = rmp_serde::from_slice::<ErrorEnvelope>(&body)
            .map(|error| error.error.message)
            .unwrap_or_else(|_| status.to_string());
        return Err(format!("Server returned {status}: {message}"));
    }
    if !content_type.starts_with(MESSAGE_PACK) {
        return Err(format!(
            "Server returned {content_type:?}, expected {MESSAGE_PACK}"
        ));
    }
    rmp_serde::from_slice::<ModelsResponse>(&body)
        .map(|response| response.models)
        .map_err(|error| format!("Invalid server response: {error}"))
}

pub fn generate(
    server_url: &str,
    cancellation: &shrimply_compute_client::CancellationToken,
    request: &GenerationRequest,
    destination: &Path,
    mut on_progress: impl FnMut(&str) -> bool,
) -> Result<GenerationResult, String> {
    let expected_audio_streams = u32::from(!request.model.starts_with(WAN_MODEL_PREFIX));
    let body = rmp_serde::to_vec_named(request)
        .map_err(|error| format!("Could not encode video-generation request: {error}"))?;
    let endpoint = format!("{}/video-generations", server_url.trim_end_matches('/'));
    tracing::info!(%endpoint, body_bytes = body.len(), destination = %destination.display(), "Sending video-generation request");
    let request = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?
        .post(&endpoint)
        .header(ACCEPT, MESSAGE_PACK_STREAM)
        .header(CONTENT_TYPE, MESSAGE_PACK)
        .body(body);
    let (request, _job) = cancellation.manage(request)?;
    let mut response = request.send().map_err(|error| {
        tracing::error!(%endpoint, %error, "Video-generation compute connection failed");
        format!("Compute server connection failed: {error}")
    })?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    tracing::info!(%endpoint, %status, %content_type, "Received video-generation response headers");
    if !status.is_success() {
        let body = response.bytes().map_err(|error| error.to_string())?;
        let message = rmp_serde::from_slice::<ErrorEnvelope>(&body)
            .map(|error| error.error.message)
            .unwrap_or_else(|_| status.to_string());
        return Err(format!("Server returned {status}: {message}"));
    }
    if !content_type.starts_with(MESSAGE_PACK_STREAM) {
        return Err(format!(
            "Server returned {content_type:?}, expected {MESSAGE_PACK_STREAM}"
        ));
    }
    let directory = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
    let mut output: Option<tempfile::NamedTempFile> = None;
    let mut expected_bytes = None;
    let mut received_bytes = 0_u64;
    loop {
        tracing::debug!(%endpoint, "Waiting for video-generation event header");
        let mut header = [0; 8];
        response
            .read_exact(&mut header)
            .map_err(|error| format!("Compute server connection failed: {error}"))?;
        let length = usize::try_from(u64::from_le_bytes(header))
            .map_err(|_| "Server event is too large".to_string())?;
        tracing::debug!(%endpoint, event_bytes = length, "Reading video-generation event payload");
        if length > MAXIMUM_EVENT_BYTES {
            return Err("Server event is too large".to_string());
        }
        let mut payload = vec![0; length];
        response
            .read_exact(&mut payload)
            .map_err(|error| format!("Compute server connection failed: {error}"))?;
        match rmp_serde::from_slice::<GenerationEvent>(&payload)
            .map_err(|error| format!("Invalid server event: {error}"))?
        {
            GenerationEvent::Queued { position } => {
                let message = shrimply_compute_client::queued_status(position);
                if !on_progress(&message) {
                    cancellation.cancel();
                    return Err("Video generation cancelled".to_string());
                }
            }
            GenerationEvent::Progress { message } => {
                tracing::info!(%endpoint, %message, "Video-generation progress");
                if !on_progress(&message) {
                    tracing::info!(%endpoint, "Cancelling video-generation response");
                    cancellation.cancel();
                    return Err("Video generation cancelled".to_string());
                }
            }
            GenerationEvent::OutputStart {
                content_type,
                bytes,
            } => {
                tracing::info!(%endpoint, %content_type, bytes, "Video-generation output started");
                if content_type != "video/mp4" || bytes == 0 || output.is_some() {
                    return Err("Server returned invalid output metadata".to_string());
                }
                expected_bytes = Some(bytes);
                output = Some(
                    tempfile::Builder::new()
                        .prefix(".shrimply-video-generation-")
                        .suffix(".mp4")
                        .tempfile_in(directory)
                        .map_err(|error| format!("Could not create output file: {error}"))?,
                );
            }
            GenerationEvent::OutputChunk { data } => {
                if cancellation.is_cancelled() {
                    return Err("Video generation cancelled".to_string());
                }
                tracing::debug!(%endpoint, chunk_bytes = data.len(), "Received video-generation output chunk");
                let Some(output) = output.as_mut() else {
                    return Err("Server sent output data before metadata".to_string());
                };
                output
                    .write_all(&data)
                    .map_err(|error| format!("Could not save generated video: {error}"))?;
                received_bytes = received_bytes.saturating_add(data.len() as u64);
                if expected_bytes.is_some_and(|expected| received_bytes > expected) {
                    return Err("Server sent more output than advertised".to_string());
                }
            }
            GenerationEvent::Result { result } => {
                if cancellation.is_cancelled() {
                    return Err("Video generation cancelled".to_string());
                }
                if expected_bytes != Some(received_bytes) {
                    return Err("Generated output ended at the wrong size".to_string());
                }
                if result.video_streams != 1
                    || result.audio_streams != expected_audio_streams
                    || result.width == 0
                    || result.height == 0
                {
                    return Err("Server returned invalid audio-video metadata".to_string());
                }
                let mut output = output.ok_or_else(|| "Server returned no output".to_string())?;
                output
                    .flush()
                    .map_err(|error| format!("Could not flush generated video: {error}"))?;
                output.persist(destination).map_err(|error| {
                    format!("Could not finalize generated video: {}", error.error)
                })?;
                tracing::info!(%endpoint, received_bytes, "Video-generation request completed");
                return Ok(result);
            }
            GenerationEvent::Error { message } => {
                tracing::error!(%endpoint, %message, "Video-generation server returned an error");
                return Err(message);
            }
        }
    }
}
