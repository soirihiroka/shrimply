use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const UPLOAD_CHUNK_BYTES: usize = 4 * 1024 * 1024;
pub const LEGACY_CHECKPOINT_FIELD: &str = "$shrimp";
pub const SERVER_PROTOCOL: u32 = 3;
pub const PROJECT_SERVER_NAME_HEADER: &str = "x-shrimply-project-server";

#[derive(Serialize, Deserialize)]
pub struct SignIn {
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerVersion {
    pub protocol: u32,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ServerVersion {
    pub fn matches(&self, release: &str) -> bool {
        self.protocol == SERVER_PROTOCOL
            && self
                .version
                .as_deref()
                .is_none_or(|version| version == release)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub opened_at: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct ProjectVersion<T> {
    pub revision: u64,
    pub project: Arc<T>,
}

#[derive(Serialize, Deserialize)]
pub struct ProjectVersionReceipt {
    pub revision: u64,
}

#[derive(Serialize, Deserialize)]
pub struct PublishRequest<T> {
    pub operation_id: Uuid,
    pub operation_digest: String,
    pub base_revision: u64,
    pub project: T,
    pub media: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(bound(deserialize = "P: Deserialize<'de>"))]
pub struct SyncRequest<P> {
    pub revision: u64,
    pub presence: Option<P>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<P>,
}

#[derive(Serialize, Deserialize)]
pub struct SyncResponse<T, P> {
    pub version: Option<ProjectVersion<T>>,
    pub participants: Vec<P>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<ServerVersion>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaBlob {
    pub digest: String,
    pub length: u64,
}

impl MediaBlob {
    pub fn reference_path(&self, source: &Path) -> PathBuf {
        let mut name = self.digest.clone();
        if let Some(extension) = source.extension().and_then(|value| value.to_str())
            && valid_extension(extension)
        {
            name.push('.');
            name.push_str(&extension.to_ascii_lowercase());
        }
        PathBuf::from(format!("blobs/{name}"))
    }
}

fn valid_extension(extension: &str) -> bool {
    !extension.is_empty()
        && extension.len() <= 12
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub fn blob_digest(path: &Path) -> Option<&str> {
    let reference = path.to_str()?;
    let filename = reference
        .strip_prefix("blobs/")
        .or_else(|| reference.strip_prefix("blobs\\"))?;
    let digest = match filename.split_once('.') {
        Some((digest, extension)) if valid_extension(extension) => digest,
        Some(_) => return None,
        None => filename,
    };
    valid_digest(digest).then_some(digest)
}

#[derive(Serialize, Deserialize)]
pub struct UploadReceipt {
    pub offset: u64,
    pub complete: bool,
}

#[derive(Serialize, Deserialize)]
pub struct ErrorResponse {
    pub message: String,
    pub revision: Option<u64>,
}

pub fn valid_digest(digest: &str) -> bool {
    digest.len() == shrimply_resource_pipeline::fingerprint::HEX_DIGEST_BYTES
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
