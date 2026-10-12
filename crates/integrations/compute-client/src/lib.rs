use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize, ser::SerializeStruct};
use std::time::Duration;
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

mod managed;
pub mod pneuma;

pub use managed::{CancellationToken, ManagedJobGuard, queued_status};

pub const MAXIMUM_COMPUTE_EVENT_BYTES: usize = 256 * 1024 * 1024;

const MESSAGE_PACK: &str = "application/msgpack";
const MESSAGE_PACK_STREAM: &str = "application/x-msgpack-stream";
const PROTOCOL_MAJOR: u32 = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const COMPUTE_STREAM_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const COMPUTE_STREAM_ALLOWED_MISSED_KEEPALIVES: u32 = 2;
pub const COMPUTE_STREAM_READ_TIMEOUT: Duration =
    COMPUTE_STREAM_KEEPALIVE_INTERVAL.saturating_mul(COMPUTE_STREAM_ALLOWED_MISSED_KEEPALIVES + 1);
const STREAM_HEADER_BYTES: usize = 8;
const SAM2_CONTENT: &str = "application/x-shrimply-sam2-analysis";
const SAM2_ARCHIVE_MAGIC: &[u8; 8] = b"SHRMSA01";
const TRACKING_3D_CONTENT: &str = "application/x-shrimply-3dtracking-analysis";
const TRACKING_3D_ARCHIVE_MAGIC: &[u8; 8] = b"SHRM3D01";

#[derive(Deserialize)]
pub struct ServerStatus {
    pub protocol: Protocol,
    pub server: Server,
    pub status: String,
    pub capabilities: Vec<String>,
    pub torch: Torch,
    pub compute: ComputeStatus,
}

#[derive(Deserialize)]
pub struct ComputeStatus {
    pub queued_jobs: usize,
    pub active_jobs: usize,
    pub reserved_ram_bytes: u64,
    pub reserved_vram_bytes: u64,
    pub workers: Vec<ComputeWorkerStatus>,
}

#[derive(Deserialize)]
pub struct ComputeWorkerStatus {
    pub service: String,
    pub model: String,
    pub configuration: std::collections::BTreeMap<String, String>,
    pub state: String,
    pub copies: usize,
}

#[derive(Deserialize)]
pub struct Protocol {
    pub major: u32,
    pub minor: u32,
}

#[derive(Deserialize)]
pub struct Server {
    pub version: String,
    #[serde(default)]
    pub git_hash: String,
    #[serde(default)]
    pub git_short_hash: String,
}

#[derive(Deserialize)]
pub struct Torch {
    pub version: String,
    pub cuda_runtime: Option<String>,
    pub cuda_available: bool,
    pub devices: Vec<Device>,
    pub selected_device: String,
}

#[derive(Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub total_memory_bytes: Option<u64>,
}

#[derive(Deserialize)]
pub struct Transcription {
    pub segments: Vec<TranscriptionSegment>,
}

#[derive(Deserialize)]
pub struct TranscriptionSegment {
    pub start_frame: u64,
    pub end_frame: u64,
    pub text: String,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioUpload {
    Pcm {
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
        sample_rate: u32,
        channels: u8,
    },
    Encoded {
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
}

#[derive(Serialize)]
struct TranscriptionRequest<'a> {
    model: &'a str,
    audio: &'a AudioUpload,
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
enum TranscriptionEvent {
    Queued { position: usize },
    Progress { message: String },
    Result { result: Transcription },
    Error { message: String },
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sam2Model {
    #[serde(rename = "facebook/sam2.1-hiera-tiny")]
    Tiny,
    #[serde(rename = "facebook/sam2.1-hiera-small")]
    Small,
    #[serde(rename = "facebook/sam2.1-hiera-base-plus")]
    BasePlus,
    #[serde(rename = "facebook/sam2.1-hiera-large")]
    Large,
}

pub struct Sam2Point {
    pub position: glam::Vec2,
    pub label: u8,
}

impl Serialize for Sam2Point {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut point = serializer.serialize_struct("Sam2Point", 3)?;
        point.serialize_field("x", &self.position.x)?;
        point.serialize_field("y", &self.position.y)?;
        point.serialize_field("label", &self.label)?;
        point.end()
    }
}

#[derive(Serialize)]
pub struct Sam2Box {
    pub minimum: glam::Vec2,
    pub maximum: glam::Vec2,
}

#[derive(Serialize)]
pub struct Sam2AnalysisRequest {
    version: u8,
    pub model: Sam2Model,
    pub frame_count: u64,
    pub seed_frame: u64,
    pub points: Vec<Sam2Point>,
    #[serde(rename = "box")]
    pub box_prompt: Option<Sam2Box>,
}

impl Sam2AnalysisRequest {
    pub fn new(
        model: Sam2Model,
        frame_count: u64,
        seed_frame: u64,
        points: Vec<Sam2Point>,
        box_prompt: Option<Sam2Box>,
    ) -> Self {
        Self {
            version: 1,
            model,
            frame_count,
            seed_frame,
            points,
            box_prompt,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Sam2Event {
    Queued {
        position: usize,
    },
    Progress {
        message: String,
        completed_frames: u64,
        total_frames: u64,
    },
    Mask {
        frame_index: u64,
        #[serde(with = "serde_bytes")]
        mask: Vec<u8>,
    },
    Result {
        completed_frames: u64,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Serialize)]
pub struct Tracking3dAnalysisRequest {
    version: u8,
    pub model: String,
    pub frame_count: u64,
    pub quality: Option<String>,
    pub camera_model: Option<String>,
}

impl Tracking3dAnalysisRequest {
    pub fn new(
        model: String,
        frame_count: u64,
        quality: Option<String>,
        camera_model: Option<String>,
    ) -> Self {
        Self {
            version: 1,
            model,
            frame_count,
            quality,
            camera_model,
        }
    }
}

#[derive(Deserialize)]
pub struct Tracking3dCamera {
    pub frame_index: u64,
    #[serde(flatten)]
    pub pose: shrimply_math_geometry::CameraPose,
    pub projection: String,
    pub image_width: u32,
    pub image_height: u32,
    pub focal_y: Option<f64>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tracking3dEvent {
    Queued {
        position: usize,
    },
    Progress {
        message: String,
        completed_frames: u64,
        total_frames: u64,
    },
    Camera(Tracking3dCamera),
    Result {
        camera_count: u64,
    },
    Error {
        code: String,
        message: String,
    },
}

pub fn write_tracking_3d_archive_header(
    output: &mut impl Write,
    request: &Tracking3dAnalysisRequest,
) -> Result<(), String> {
    let header = rmp_serde::to_vec_named(request)
        .map_err(|error| format!("Could not encode 3D tracking request: {error}"))?;
    output
        .write_all(TRACKING_3D_ARCHIVE_MAGIC)
        .and_then(|()| output.write_all(&(header.len() as u64).to_le_bytes()))
        .and_then(|()| output.write_all(&header))
        .map_err(|error| format!("Could not write 3D tracking archive: {error}"))
}

pub fn write_tracking_3d_archive_frame(
    output: &mut impl Write,
    frame_index: u64,
    jpeg: &[u8],
) -> Result<(), String> {
    output
        .write_all(&frame_index.to_le_bytes())
        .and_then(|()| output.write_all(&(jpeg.len() as u64).to_le_bytes()))
        .and_then(|()| output.write_all(jpeg))
        .map_err(|error| format!("Could not write 3D tracking frame: {error}"))
}

pub fn analyze_tracking_3d(
    server_url: &str,
    cancellation: &CancellationToken,
    archive: &Path,
    mut on_event: impl FnMut(Tracking3dEvent) -> bool,
) -> Result<(), String> {
    let server_url = server_url.trim();
    if server_url.is_empty() {
        return Err("Server URL is empty".to_string());
    }
    let file = File::open(archive).map_err(|error| format!("Open 3D tracking archive: {error}"))?;
    let length = file
        .metadata()
        .map_err(|error| format!("Inspect 3D tracking archive: {error}"))?
        .len();
    let endpoint = format!("{}/3dtracking/analyses", server_url.trim_end_matches('/'));
    tracing::info!(%endpoint, job_id = %cancellation.job_id(), archive = %archive.display(), body_bytes = length, "Sending 3D tracking request");
    let request = reqwest::blocking::Client::builder()
        .connect_timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .post(&endpoint)
        .header(ACCEPT, MESSAGE_PACK_STREAM)
        .header(CONTENT_TYPE, TRACKING_3D_CONTENT)
        .body(reqwest::blocking::Body::sized(file, length));
    let (request, _job) = cancellation.manage(request)?;
    let mut response = request
        .send()
        .map_err(|error| format!("Compute server connection failed: {error}"))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    tracing::info!(%endpoint, job_id = %cancellation.job_id(), %status, %content_type, "Received 3D tracking response headers");
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
    loop {
        tracing::debug!(%endpoint, job_id = %cancellation.job_id(), "Waiting for 3D tracking event header");
        let mut header = [0; STREAM_HEADER_BYTES];
        response
            .read_exact(&mut header)
            .map_err(|error| format!("Compute server connection failed: {error}"))?;
        let length = usize::try_from(u64::from_le_bytes(header))
            .map_err(|_| "3D tracking server event is too large".to_string())?;
        if length > MAXIMUM_COMPUTE_EVENT_BYTES {
            return Err("3D tracking server event is too large".to_string());
        }
        let mut payload = vec![0; length];
        tracing::debug!(%endpoint, job_id = %cancellation.job_id(), event_bytes = payload.len(), "Reading 3D tracking event payload");
        response
            .read_exact(&mut payload)
            .map_err(|error| format!("Compute server connection failed: {error}"))?;
        let event = rmp_serde::from_slice::<Tracking3dEvent>(&payload)
            .map_err(|error| format!("Invalid 3D tracking server event: {error}"))?;
        let done = matches!(
            &event,
            Tracking3dEvent::Result { .. } | Tracking3dEvent::Error { .. }
        );
        let event_kind = match &event {
            Tracking3dEvent::Queued { .. } => "queued",
            Tracking3dEvent::Progress { .. } => "progress",
            Tracking3dEvent::Camera(_) => "camera",
            Tracking3dEvent::Result { .. } => "result",
            Tracking3dEvent::Error { .. } => "error",
        };
        tracing::debug!(%endpoint, job_id = %cancellation.job_id(), event_kind, "Received 3D tracking event");
        if !on_event(event) || done {
            if !done {
                cancellation.cancel();
            }
            tracing::info!(%endpoint, job_id = %cancellation.job_id(), event_kind, "3D tracking response finished");
            return Ok(());
        }
    }
}

pub fn write_sam2_archive_header(
    output: &mut impl Write,
    request: &Sam2AnalysisRequest,
) -> Result<(), String> {
    let header = rmp_serde::to_vec_named(request)
        .map_err(|error| format!("Could not encode SAM2 request: {error}"))?;
    output
        .write_all(SAM2_ARCHIVE_MAGIC)
        .and_then(|()| output.write_all(&(header.len() as u64).to_le_bytes()))
        .and_then(|()| output.write_all(&header))
        .map_err(|error| format!("Could not write SAM2 proxy archive: {error}"))
}

pub fn write_sam2_archive_frame(output: &mut impl Write, jpeg: &[u8]) -> Result<(), String> {
    output
        .write_all(&(jpeg.len() as u64).to_le_bytes())
        .and_then(|()| output.write_all(jpeg))
        .map_err(|error| format!("Could not write SAM2 proxy frame: {error}"))
}

pub fn analyze_sam2(
    server_url: &str,
    cancellation: &CancellationToken,
    archive: &Path,
    mut on_event: impl FnMut(Sam2Event) -> bool,
) -> Result<(), String> {
    let server_url = server_url.trim();
    if server_url.is_empty() {
        return Err("Server URL is empty".to_string());
    }
    let file = File::open(archive).map_err(|error| format!("Open SAM2 proxy archive: {error}"))?;
    let length = file
        .metadata()
        .map_err(|error| format!("Inspect SAM2 proxy archive: {error}"))?
        .len();
    let endpoint = format!("{}/sam2/analyses", server_url.trim_end_matches('/'));
    tracing::info!(%endpoint, archive = %archive.display(), body_bytes = length, "Sending SAM2 request");
    let request = reqwest::blocking::Client::builder()
        .connect_timeout(REQUEST_TIMEOUT)
        .timeout(COMPUTE_STREAM_READ_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .post(&endpoint)
        .header(ACCEPT, MESSAGE_PACK_STREAM)
        .header(CONTENT_TYPE, SAM2_CONTENT)
        .body(reqwest::blocking::Body::sized(file, length));
    let (request, _job) = cancellation.manage(request)?;
    let mut response = request
        .send()
        .map_err(|error| format!("Compute server connection failed: {error}"))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    tracing::info!(%endpoint, %status, %content_type, "Received SAM2 response headers");
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
    loop {
        tracing::debug!(%endpoint, "Waiting for SAM2 event header");
        let mut header = [0; STREAM_HEADER_BYTES];
        read_sam2_stream(&mut response, &mut header, cancellation)?;
        let length = usize::try_from(u64::from_le_bytes(header))
            .map_err(|_| "SAM2 server event is too large".to_string())?;
        if length > MAXIMUM_COMPUTE_EVENT_BYTES {
            return Err("SAM2 server event is too large".to_string());
        }
        let mut payload = vec![0; length];
        tracing::debug!(%endpoint, event_bytes = payload.len(), "Reading SAM2 event payload");
        read_sam2_stream(&mut response, &mut payload, cancellation)?;
        let event = rmp_serde::from_slice::<Sam2Event>(&payload)
            .map_err(|error| format!("Invalid SAM2 server event: {error}"))?;
        let done = matches!(&event, Sam2Event::Result { .. });
        let event_kind = match &event {
            Sam2Event::Queued { .. } => "queued",
            Sam2Event::Progress { .. } => "progress",
            Sam2Event::Mask { .. } => "mask",
            Sam2Event::Result { .. } => "result",
            Sam2Event::Error { .. } => "error",
        };
        tracing::debug!(%endpoint, event_kind, "Received SAM2 event");
        if !on_event(event) || done {
            if !done {
                cancellation.cancel();
            }
            tracing::info!(%endpoint, event_kind, "SAM2 response finished");
            return Ok(());
        }
    }
}

fn read_sam2_stream(
    response: &mut reqwest::blocking::Response,
    mut output: &mut [u8],
    cancellation: &CancellationToken,
) -> Result<(), String> {
    while !output.is_empty() {
        if cancellation.is_cancelled() {
            return Err("Compute job cancelled".to_string());
        }
        match response.read(output) {
            Ok(0) => {
                return Err("Compute server connection failed: response ended early".to_string());
            }
            Ok(read) => output = &mut output[read..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) if cancellation.is_cancelled() => {
                return Err("Compute job cancelled".to_string());
            }
            Err(error) => return Err(format!("Compute server connection failed: {error}")),
        }
    }
    Ok(())
}

pub fn server_status(server_url: &str) -> Result<ServerStatus, String> {
    let server_url = server_url.trim();
    if server_url.is_empty() {
        return Err("Server URL is empty".to_string());
    }
    let endpoint = format!("{}/", server_url.trim_end_matches('/'));
    tracing::info!(%endpoint, "Requesting server status");
    let response = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .get(&endpoint)
        .header(ACCEPT, MESSAGE_PACK)
        .send()
        .map_err(|error| error.to_string())?;
    tracing::info!(%endpoint, status = %response.status(), "Received server status response");

    decode_server_status(response)
}

pub fn set_compute_device(server_url: &str, device: &str) -> Result<ServerStatus, String> {
    let server_url = server_url.trim();
    if server_url.is_empty() {
        return Err("Server URL is empty".to_string());
    }
    let endpoint = format!("{}/compute/device", server_url.trim_end_matches('/'));
    tracing::info!(%endpoint, %device, "Selecting compute device");
    let response = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .put(&endpoint)
        .query(&[("device", device)])
        .header(ACCEPT, MESSAGE_PACK)
        .send()
        .map_err(|error| error.to_string())?;
    tracing::info!(%endpoint, %device, status = %response.status(), "Received compute-device response");

    decode_server_status(response)
}

fn decode_server_status(response: reqwest::blocking::Response) -> Result<ServerStatus, String> {
    let status_code = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type.starts_with(MESSAGE_PACK) {
        return Err(format!(
            "Server returned {content_type:?}, expected {MESSAGE_PACK}"
        ));
    }
    let body = response.bytes().map_err(|error| error.to_string())?;
    if !status_code.is_success() {
        let message = rmp_serde::from_slice::<ErrorEnvelope>(&body)
            .map(|error| error.error.message)
            .unwrap_or_else(|_| status_code.to_string());
        return Err(format!("Server returned {status_code}: {message}"));
    }
    let status: ServerStatus = rmp_serde::from_slice(&body)
        .map_err(|error| format!("Invalid server response: {error}"))?;
    if status.protocol.major != PROTOCOL_MAJOR {
        return Err(format!(
            "Unsupported protocol {}.{}",
            status.protocol.major, status.protocol.minor
        ));
    }
    Ok(status)
}

pub fn transcribe(
    server_url: &str,
    cancellation: &CancellationToken,
    model: &str,
    audio: &AudioUpload,
    on_progress: impl FnMut(&str),
) -> Result<Transcription, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Could not start transcription network runtime: {error}"))?
        .block_on(transcribe_async(
            server_url,
            cancellation,
            model,
            audio,
            on_progress,
        ))
}

async fn transcribe_async(
    server_url: &str,
    cancellation: &CancellationToken,
    model: &str,
    audio: &AudioUpload,
    mut on_progress: impl FnMut(&str),
) -> Result<Transcription, String> {
    let server_url = server_url.trim();
    if server_url.is_empty() {
        return Err("Server URL is empty".to_string());
    }
    let endpoint = format!("{}/transcriptions", server_url.trim_end_matches('/'));
    let payload = rmp_serde::to_vec_named(&TranscriptionRequest { model, audio })
        .map_err(|error| format!("Could not encode transcription request: {error}"))?;
    tracing::info!(%endpoint, %model, body_bytes = payload.len(), "Sending transcription request");
    let request = reqwest::Client::builder()
        .connect_timeout(REQUEST_TIMEOUT)
        .read_timeout(COMPUTE_STREAM_READ_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .post(&endpoint)
        .header(ACCEPT, MESSAGE_PACK_STREAM)
        .header(CONTENT_TYPE, MESSAGE_PACK)
        .body(payload);
    let (request, _job) = cancellation.manage_async(request)?;
    let mut response = request
        .send()
        .await
        .map_err(|error| format!("Compute server connection failed: {error}"))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    tracing::info!(%endpoint, %model, %status, %content_type, "Received transcription response headers");
    if !status.is_success() {
        let body = response.bytes().await.map_err(|error| error.to_string())?;
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
    let mut buffered = Vec::new();
    loop {
        while buffered.len() < STREAM_HEADER_BYTES {
            if cancellation.is_cancelled() {
                return Err("Transcription cancelled".to_string());
            }
            let chunk = response
                .chunk()
                .await
                .map_err(|error| format!("Compute server connection failed: {error}"))?
                .ok_or("Compute server connection failed: response ended early")?;
            buffered.extend_from_slice(&chunk);
        }
        let length = usize::try_from(u64::from_le_bytes(
            buffered[..STREAM_HEADER_BYTES]
                .try_into()
                .expect("transcription event header is complete"),
        ))
        .map_err(|_| "Transcription server event is too large".to_string())?;
        if length > MAXIMUM_COMPUTE_EVENT_BYTES {
            return Err("Transcription server event is too large".to_string());
        }
        let event_bytes = STREAM_HEADER_BYTES
            .checked_add(length)
            .ok_or("Transcription server event is too large")?;
        while buffered.len() < event_bytes {
            if cancellation.is_cancelled() {
                return Err("Transcription cancelled".to_string());
            }
            let chunk = response
                .chunk()
                .await
                .map_err(|error| format!("Compute server connection failed: {error}"))?
                .ok_or("Compute server connection failed: response ended early")?;
            buffered.extend_from_slice(&chunk);
        }
        let payload = buffered[STREAM_HEADER_BYTES..event_bytes].to_vec();
        buffered.drain(..event_bytes);
        tracing::debug!(%endpoint, %model, event_bytes = payload.len(), "Reading transcription event payload");
        match rmp_serde::from_slice::<TranscriptionEvent>(&payload)
            .map_err(|error| format!("Invalid server event: {error}"))?
        {
            TranscriptionEvent::Queued { position } => {
                on_progress(&queued_status(position));
            }
            TranscriptionEvent::Progress { message } => {
                tracing::info!(%endpoint, %model, %message, "Transcription progress");
                on_progress(&message);
            }
            TranscriptionEvent::Result { result } => {
                if cancellation.is_cancelled() {
                    return Err("Transcription cancelled".to_string());
                }
                tracing::info!(%endpoint, %model, segments = result.segments.len(), "Transcription request completed");
                return Ok(result);
            }
            TranscriptionEvent::Error { message } => {
                tracing::error!(%endpoint, %model, %message, "Transcription server returned an error");
                return Err(message);
            }
        }
    }
}
