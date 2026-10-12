use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use crate::{AudioUpload, CancellationToken, queued_status};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MESSAGE_PACK: &str = "application/msgpack";
const MESSAGE_PACK_STREAM: &str = "application/x-msgpack-stream";
const STREAM_HEADER_BYTES: usize = 8;

#[derive(Clone, Debug, Deserialize)]
pub struct Model {
    pub name: String,
    #[serde(default)]
    pub metadata: ModelMetadata,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ModelMetadata {
    pub experiment_name: Option<String>,
    pub version: Option<String>,
    pub saved_at: Option<String>,
}

pub struct ConvertRequest<'a> {
    pub model: &'a str,
    pub input: &'a Path,
    pub pitch_offset: i32,
    pub f0_method: &'a str,
    pub speed: f32,
    pub maintain_pitch: bool,
}

#[derive(Deserialize)]
struct ModelsResponse {
    models: Vec<Model>,
}

#[derive(Serialize)]
struct WireConvertRequest<'a> {
    model: &'a str,
    audio: &'a AudioUpload,
    file_name: &'a str,
    pitch_offset: i32,
    f0_method: &'a str,
    speed: f32,
    maintain_pitch: bool,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ConversionEvent {
    Queued { position: usize },
    Progress {},
    Result { result: ConvertedAudio },
    Error { message: String },
}

#[derive(Deserialize)]
struct ConvertedAudio {
    #[serde(with = "serde_bytes")]
    wav: Vec<u8>,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ServerError,
}

#[derive(Deserialize)]
struct ServerError {
    message: String,
}

pub fn connect(server_url: &str) -> Result<(), String> {
    models(server_url).map(|_| ())
}

pub fn models(server_url: &str) -> Result<Vec<Model>, String> {
    let endpoint = format!("{}/pneuma/models", validate_url(server_url)?);
    tracing::info!(%endpoint, "Requesting Pneuma models");
    let response = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .get(&endpoint)
        .header(ACCEPT, MESSAGE_PACK)
        .send()
        .map_err(|error| error.to_string())?;
    tracing::info!(%endpoint, status = %response.status(), content_type = response_content_type(&response), "Received Pneuma models response");
    decode_message_pack::<ModelsResponse>(response).map(|response| response.models)
}

pub fn convert(
    request: ConvertRequest<'_>,
    server_url: &str,
    cancellation: &CancellationToken,
    mut keep_running: impl FnMut() -> bool,
) -> Result<Vec<u8>, String> {
    let audio = fs::read(request.input)
        .map_err(|error| format!("Could not read audio for Pneuma: {error}"))?;
    let audio_bytes = audio.len();
    let audio = AudioUpload::Encoded { data: audio };
    let file_name = request
        .input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("audio");
    let payload = rmp_serde::to_vec_named(&WireConvertRequest {
        model: request.model,
        audio: &audio,
        file_name,
        pitch_offset: request.pitch_offset,
        f0_method: request.f0_method,
        speed: request.speed,
        maintain_pitch: request.maintain_pitch,
    })
    .map_err(|error| format!("Could not encode Pneuma request: {error}"))?;
    let endpoint = format!("{}/pneuma/conversions", validate_url(server_url)?);
    tracing::info!(%endpoint, body_bytes = payload.len(), audio_bytes, "Sending Pneuma conversion request");
    let request = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .post(&endpoint)
        .header(CONTENT_TYPE, MESSAGE_PACK)
        .header(ACCEPT, MESSAGE_PACK_STREAM)
        .body(payload);
    let (request, _job) = cancellation.manage(request)?;
    let mut response = request
        .send()
        .map_err(|error| format!("Compute server connection failed: {error}"))?;
    let status = response.status();
    let content_type = response_content_type(&response);
    tracing::info!(%endpoint, %status, %content_type, "Received Pneuma response headers");
    if !status.is_success() {
        return Err(decode_server_error(response, status));
    }
    if !content_type.starts_with(MESSAGE_PACK_STREAM) {
        return Err(format!(
            "Server returned {content_type:?}, expected {MESSAGE_PACK_STREAM}"
        ));
    }
    loop {
        tracing::debug!(%endpoint, "Waiting for Pneuma event header");
        let mut header = [0_u8; STREAM_HEADER_BYTES];
        response
            .read_exact(&mut header)
            .map_err(|error| format!("Compute server connection failed: {error}"))?;
        let length = u64::from_le_bytes(header);
        tracing::debug!(%endpoint, event_bytes = length, "Reading Pneuma event payload");
        if length > crate::MAXIMUM_COMPUTE_EVENT_BYTES as u64 {
            return Err("Pneuma server event is too large".to_string());
        }
        let mut event = vec![
            0;
            usize::try_from(length).map_err(|_| {
                format!("Pneuma server event length {length} does not fit in memory")
            })?
        ];
        response
            .read_exact(&mut event)
            .map_err(|error| format!("Compute server connection failed: {error}"))?;
        let event = rmp_serde::from_slice::<ConversionEvent>(&event)
            .map_err(|error| format!("Invalid Pneuma server event: {error}"))?;
        if !keep_running() {
            cancellation.cancel();
            return Err("Pneuma conversion cancelled".to_string());
        }
        match event {
            ConversionEvent::Queued { position } => {
                tracing::info!(%endpoint, status = queued_status(position), "Pneuma conversion queued")
            }
            ConversionEvent::Progress {} => tracing::info!(%endpoint, "Pneuma conversion progress"),
            ConversionEvent::Result { result } => {
                if cancellation.is_cancelled() {
                    return Err("Pneuma conversion cancelled".to_string());
                }
                tracing::info!(%endpoint, wav_bytes = result.wav.len(), "Pneuma conversion completed");
                return Ok(result.wav);
            }
            ConversionEvent::Error { message } => {
                tracing::error!(%endpoint, %message, "Pneuma server returned an error");
                return Err(message);
            }
        }
    }
}

fn validate_url(server_url: &str) -> Result<&str, String> {
    let server_url = server_url.trim().trim_end_matches('/');
    if server_url.is_empty() {
        Err("Server URL is empty".to_string())
    } else {
        Ok(server_url)
    }
}

fn decode_message_pack<T: for<'de> Deserialize<'de>>(response: Response) -> Result<T, String> {
    let status = response.status();
    let content_type = response_content_type(&response);
    if !content_type.starts_with(MESSAGE_PACK) {
        return Err(format!(
            "Server returned {content_type:?}, expected {MESSAGE_PACK}"
        ));
    }
    let body = response.bytes().map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(server_error_message(&body, status));
    }
    rmp_serde::from_slice(&body).map_err(|error| format!("Invalid server response: {error}"))
}

fn response_content_type(response: &Response) -> &str {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

fn decode_server_error(response: Response, status: reqwest::StatusCode) -> String {
    response
        .bytes()
        .map(|body| server_error_message(&body, status))
        .unwrap_or_else(|_| format!("Server returned {status}"))
}

fn server_error_message(body: &[u8], status: reqwest::StatusCode) -> String {
    let message = rmp_serde::from_slice::<ErrorEnvelope>(body)
        .map(|error| error.error.message)
        .unwrap_or_else(|_| status.to_string());
    format!("Server returned {status}: {message}")
}
