use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use shrimply_asset::Asset;
use shrimply_compute_client::AudioUpload;
pub use shrimply_math_core::Fraction;
use shrimply_math_core::{deserialize_fraction, fraction_is_finite, serialize_fraction};

const MESSAGE_PACK: &str = "application/msgpack";
const MESSAGE_PACK_STREAM: &str = "application/x-msgpack-stream";
const STREAM_HEADER_BYTES: usize = 8;
const REFERENCE_AUDIO_KEY: &str = "reference_audio";
const LEGACY_REFERENCE_AUDIO_KEY: &str = "voice";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TtsSettings {
    pub model: Option<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, TtsValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TtsValue {
    Text {
        value: String,
    },
    Select {
        value: String,
    },
    Audio {
        value: Asset,
    },
    Toggle {
        value: bool,
    },
    Number {
        #[serde(
            deserialize_with = "deserialize_fraction",
            serialize_with = "serialize_fraction"
        )]
        value: Fraction,
    },
    Table {
        rows: Vec<BTreeMap<String, String>>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TtsModel {
    pub id: String,
    pub label: String,
    pub inputs: Vec<InputDefinition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputDefinition {
    Text {
        key: String,
        label: String,
        default: String,
        required: bool,
        multiline: bool,
        max_length: usize,
        purpose: Option<InputPurpose>,
        visible_when: Option<VisibleWhen>,
    },
    Select {
        key: String,
        label: String,
        options: Vec<SelectOption>,
        default: String,
        visible_when: Option<VisibleWhen>,
    },
    Audio {
        key: String,
        label: String,
        required: bool,
        visible_when: Option<VisibleWhen>,
    },
    Toggle {
        key: String,
        label: String,
        default: bool,
        visible_when: Option<VisibleWhen>,
    },
    Number {
        key: String,
        label: String,
        #[serde(
            deserialize_with = "deserialize_fraction",
            serialize_with = "serialize_fraction"
        )]
        default: Fraction,
        #[serde(
            deserialize_with = "deserialize_fraction",
            serialize_with = "serialize_fraction"
        )]
        minimum: Fraction,
        #[serde(
            deserialize_with = "deserialize_fraction",
            serialize_with = "serialize_fraction"
        )]
        maximum: Fraction,
        #[serde(
            deserialize_with = "deserialize_fraction",
            serialize_with = "serialize_fraction"
        )]
        step: Fraction,
        presentation: NumberPresentation,
        purpose: Option<InputPurpose>,
        visible_when: Option<VisibleWhen>,
    },
    Table {
        key: String,
        label: String,
        columns: Vec<TableColumn>,
        visible_when: Option<VisibleWhen>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VisibleWhen {
    pub input: String,
    pub values: Vec<ConditionValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ConditionValue {
    Text(String),
    Toggle(bool),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
    pub purpose: Option<InputPurpose>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputPurpose {
    Text,
    Duration,
    SpeedFactor,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberPresentation {
    Number,
    Slider,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TableColumn {
    pub key: String,
    pub label: String,
    pub required: bool,
    pub max_length: usize,
}

#[derive(Serialize)]
pub struct SpeechRequest {
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
    Audio {
        value: AudioUpload,
    },
    Toggle {
        value: bool,
    },
    Number {
        #[serde(serialize_with = "serialize_fraction")]
        value: Fraction,
    },
    Table {
        rows: Vec<BTreeMap<String, String>>,
    },
}

#[derive(Deserialize)]
pub struct Speech {
    #[serde(with = "serde_bytes")]
    pub wav: Vec<u8>,
    #[serde(deserialize_with = "deserialize_fraction")]
    pub speed_factor: Fraction,
}

#[derive(Deserialize)]
struct ModelsResponse {
    models: Vec<TtsModel>,
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
enum SpeechEvent {
    Queued { position: usize },
    Progress { message: String },
    Result { result: Speech },
    Error { message: String },
}

impl InputDefinition {
    pub fn key(&self) -> &str {
        match self {
            Self::Text { key, .. }
            | Self::Select { key, .. }
            | Self::Audio { key, .. }
            | Self::Toggle { key, .. }
            | Self::Number { key, .. }
            | Self::Table { key, .. } => key,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Text { label, .. }
            | Self::Select { label, .. }
            | Self::Audio { label, .. }
            | Self::Toggle { label, .. }
            | Self::Number { label, .. }
            | Self::Table { label, .. } => label,
        }
    }

    pub fn visible_when(&self) -> Option<&VisibleWhen> {
        match self {
            Self::Text { visible_when, .. }
            | Self::Select { visible_when, .. }
            | Self::Audio { visible_when, .. }
            | Self::Toggle { visible_when, .. }
            | Self::Number { visible_when, .. }
            | Self::Table { visible_when, .. } => visible_when.as_ref(),
        }
    }

    pub fn purpose(&self) -> Option<InputPurpose> {
        match self {
            Self::Text { purpose, .. } | Self::Number { purpose, .. } => *purpose,
            _ => None,
        }
    }

    fn default_value(&self) -> Option<TtsValue> {
        match self {
            Self::Text { default, .. } => Some(TtsValue::Text {
                value: default.clone(),
            }),
            Self::Select { default, .. } => Some(TtsValue::Select {
                value: default.clone(),
            }),
            Self::Audio { .. } => None,
            Self::Toggle { default, .. } => Some(TtsValue::Toggle { value: *default }),
            Self::Number { default, .. } => Some(TtsValue::Number { value: *default }),
            Self::Table { .. } => Some(TtsValue::Table { rows: Vec::new() }),
        }
    }

    fn accepts(&self, value: &TtsValue) -> bool {
        match (self, value) {
            (Self::Text { .. }, TtsValue::Text { .. })
            | (Self::Audio { .. }, TtsValue::Audio { .. })
            | (Self::Toggle { .. }, TtsValue::Toggle { .. })
            | (Self::Number { .. }, TtsValue::Number { .. })
            | (Self::Table { .. }, TtsValue::Table { .. }) => true,
            (Self::Select { options, .. }, TtsValue::Select { value }) => {
                options.iter().any(|option| option.value == *value)
            }
            _ => false,
        }
    }
}

pub fn sync_settings(settings: &mut TtsSettings, model: &TtsModel) {
    settings.model = Some(model.id.clone());
    if model
        .inputs
        .iter()
        .any(|input| input.key() == REFERENCE_AUDIO_KEY)
        && let Some(value) = settings.inputs.remove(LEGACY_REFERENCE_AUDIO_KEY)
    {
        settings
            .inputs
            .entry(REFERENCE_AUDIO_KEY.to_string())
            .or_insert(value);
    }
    for input in &model.inputs {
        let valid = settings
            .inputs
            .get(input.key())
            .is_some_and(|value| input.accepts(value));
        if !valid {
            if let Some(value) = input.default_value() {
                settings.inputs.insert(input.key().to_string(), value);
            } else {
                settings.inputs.remove(input.key());
            }
        }
    }
}

pub fn is_visible(input: &InputDefinition, values: &BTreeMap<String, TtsValue>) -> bool {
    let Some(condition) = input.visible_when() else {
        return true;
    };
    let Some(value) = values.get(&condition.input) else {
        return false;
    };
    condition
        .values
        .iter()
        .any(|expected| match (expected, value) {
            (ConditionValue::Text(expected), TtsValue::Select { value }) => expected == value,
            (ConditionValue::Toggle(expected), TtsValue::Toggle { value }) => expected == value,
            _ => false,
        })
}

pub fn set_text(settings: &mut TtsSettings, model: &TtsModel, text: String) {
    if let Some(input) = model
        .inputs
        .iter()
        .find(|input| input.purpose() == Some(InputPurpose::Text))
    {
        settings
            .inputs
            .insert(input.key().to_string(), TtsValue::Text { value: text });
    }
}

pub fn set_duration(settings: &mut TtsSettings, model: &TtsModel, duration: Fraction) {
    set_timing_mode(settings, model, InputPurpose::Duration);
    if let Some(input) = model
        .inputs
        .iter()
        .find(|input| input.purpose() == Some(InputPurpose::Duration))
    {
        settings.inputs.insert(
            input.key().to_string(),
            TtsValue::Number { value: duration },
        );
    }
}

pub fn apply_speed_factor(settings: &mut TtsSettings, model: &TtsModel, speed: Fraction) {
    set_timing_mode(settings, model, InputPurpose::SpeedFactor);
    if let Some(input) = model
        .inputs
        .iter()
        .find(|input| input.purpose() == Some(InputPurpose::SpeedFactor))
    {
        settings
            .inputs
            .insert(input.key().to_string(), TtsValue::Number { value: speed });
    }
}

fn set_timing_mode(settings: &mut TtsSettings, model: &TtsModel, purpose: InputPurpose) {
    if let Some((input, option)) = model.inputs.iter().find_map(|input| {
        let InputDefinition::Select { options, .. } = input else {
            return None;
        };
        options
            .iter()
            .find(|option| option.purpose == Some(purpose))
            .map(|option| (input, option))
    }) {
        settings.inputs.insert(
            input.key().to_string(),
            TtsValue::Select {
                value: option.value.clone(),
            },
        );
    }
}

pub fn speech_request(model: &TtsModel, settings: &TtsSettings) -> Result<SpeechRequest, String> {
    tracing::info!(model = %model.id, "Preparing text-to-speech request");
    if settings.model.as_deref() != Some(model.id.as_str()) {
        return Err("Text-to-speech settings do not match the selected model".to_string());
    }
    let mut inputs = BTreeMap::new();
    for definition in model
        .inputs
        .iter()
        .filter(|input| is_visible(input, &settings.inputs))
    {
        let value = settings
            .inputs
            .get(definition.key())
            .ok_or_else(|| format!("{} is required", definition.label()))?;
        inputs.insert(
            definition.key().to_string(),
            encode_value(definition, value)?,
        );
    }
    tracing::info!(model = %model.id, inputs = inputs.len(), "Prepared text-to-speech request");
    Ok(SpeechRequest {
        model: model.id.clone(),
        inputs,
    })
}

fn encode_value(definition: &InputDefinition, value: &TtsValue) -> Result<InputValue, String> {
    match (definition, value) {
        (
            InputDefinition::Text {
                required,
                max_length,
                ..
            },
            TtsValue::Text { value },
        ) => {
            if *required && value.trim().is_empty() {
                return Err(format!("{} is required", definition.label()));
            }
            if value.chars().count() > *max_length {
                return Err(format!("{} is too long", definition.label()));
            }
            Ok(InputValue::Text {
                value: value.clone(),
            })
        }
        (InputDefinition::Select { options, .. }, TtsValue::Select { value }) => {
            if !options.iter().any(|option| option.value == *value) {
                return Err(format!("{} has an invalid value", definition.label()));
            }
            Ok(InputValue::Select {
                value: value.clone(),
            })
        }
        (InputDefinition::Audio { .. }, TtsValue::Audio { value }) => {
            let path = value.path();
            let data = value.read()?;
            tracing::info!(input = definition.key(), path = %path.display(), audio_bytes = data.len(), "Read text-to-speech reference audio");
            Ok(InputValue::Audio {
                value: AudioUpload::Encoded { data },
            })
        }
        (InputDefinition::Toggle { .. }, TtsValue::Toggle { value }) => {
            Ok(InputValue::Toggle { value: *value })
        }
        (
            InputDefinition::Number {
                minimum, maximum, ..
            },
            TtsValue::Number { value },
        ) => {
            if !fraction_is_finite(*value) {
                return Err(format!("{} has an invalid denominator", definition.label()));
            }
            if value < minimum || value > maximum {
                return Err(format!(
                    "{} is outside its allowed range",
                    definition.label()
                ));
            }
            Ok(InputValue::Number { value: *value })
        }
        (InputDefinition::Table { .. }, TtsValue::Table { rows }) => {
            Ok(InputValue::Table { rows: rows.clone() })
        }
        _ => Err(format!("{} has the wrong value type", definition.label())),
    }
}

pub fn models(server_url: &str) -> Result<Vec<TtsModel>, String> {
    let server_url = server_url.trim();
    if server_url.is_empty() {
        return Err("Server URL is empty".to_string());
    }
    let endpoint = format!("{}/tts/models", server_url.trim_end_matches('/'));
    tracing::info!(%endpoint, "Requesting text-to-speech models");
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?
        .get(&endpoint)
        .header(ACCEPT, MESSAGE_PACK)
        .send()
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    tracing::info!(%endpoint, %status, %content_type, "Received text-to-speech models response");
    if !status.is_success() {
        let body = response.bytes().map_err(|error| error.to_string())?;
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
    rmp_serde::from_slice::<ModelsResponse>(&response.bytes().map_err(|error| error.to_string())?)
        .map(|response| response.models)
        .map_err(|error| format!("Invalid server response: {error}"))
}

pub fn synthesize(
    server_url: &str,
    cancellation: &shrimply_compute_client::CancellationToken,
    request: &SpeechRequest,
    on_progress: impl FnMut(&str) -> bool,
) -> Result<Speech, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Could not start speech network runtime: {error}"))?
        .block_on(synthesize_async(
            server_url,
            cancellation,
            request,
            on_progress,
        ))
}

async fn synthesize_async(
    server_url: &str,
    cancellation: &shrimply_compute_client::CancellationToken,
    request: &SpeechRequest,
    mut on_progress: impl FnMut(&str) -> bool,
) -> Result<Speech, String> {
    let server_url = server_url.trim();
    if server_url.is_empty() {
        return Err("Server URL is empty".to_string());
    }
    tracing::info!("Encoding text-to-speech request");
    let body = rmp_serde::to_vec_named(request)
        .map_err(|error| format!("Could not encode speech request: {error}"))?;
    let endpoint = format!("{}/speech", server_url.trim_end_matches('/'));
    tracing::info!(%endpoint, body_bytes = body.len(), "Sending text-to-speech request");
    let request = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(shrimply_compute_client::COMPUTE_STREAM_READ_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .post(&endpoint)
        .header(ACCEPT, MESSAGE_PACK_STREAM)
        .header(CONTENT_TYPE, MESSAGE_PACK)
        .body(body);
    let (request, _job) = cancellation.manage_async(request)?;
    let mut response = request.send().await.map_err(|error| {
        tracing::error!(%endpoint, %error, "Text-to-speech request failed before response headers");
        format!("Compute server connection failed: {error}")
    })?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    tracing::info!(%endpoint, %status, %content_type, "Received text-to-speech response headers");
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
                return Err("Speech generation cancelled".into());
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
                .expect("speech event header is complete"),
        ))
        .map_err(|_| "Server event is too large".to_string())?;
        if length > shrimply_compute_client::MAXIMUM_COMPUTE_EVENT_BYTES {
            return Err("Server event is too large".to_string());
        }
        let event_bytes = STREAM_HEADER_BYTES
            .checked_add(length)
            .ok_or("Server event is too large")?;
        while buffered.len() < event_bytes {
            if cancellation.is_cancelled() {
                return Err("Speech generation cancelled".into());
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
        tracing::debug!(%endpoint, event_bytes = length, "Reading text-to-speech event payload");
        match rmp_serde::from_slice::<SpeechEvent>(&payload)
            .map_err(|error| format!("Invalid server event: {error}"))?
        {
            SpeechEvent::Queued { position } => {
                let message = shrimply_compute_client::queued_status(position);
                if !on_progress(&message) {
                    cancellation.cancel();
                    return Err("Speech generation cancelled".to_string());
                }
            }
            SpeechEvent::Progress { message } => {
                tracing::info!(%endpoint, %message, "Text-to-speech progress");
                if !on_progress(&message) {
                    tracing::info!(%endpoint, "Cancelling text-to-speech response");
                    cancellation.cancel();
                    return Err("Speech generation cancelled".to_string());
                }
            }
            SpeechEvent::Result { result } => {
                if cancellation.is_cancelled() {
                    return Err("Speech generation cancelled".to_string());
                }
                tracing::info!(%endpoint, wav_bytes = result.wav.len(), "Text-to-speech request completed");
                return Ok(result);
            }
            SpeechEvent::Error { message } => {
                tracing::error!(%endpoint, %message, "Text-to-speech server returned an error");
                return Err(message);
            }
        }
    }
}
