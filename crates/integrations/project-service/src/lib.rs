mod cache;
mod client;
#[cfg(feature = "server")]
mod media;
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "server")]
mod store;
mod workspace;

const UPLOAD_CHUNK_BYTES: usize = 4 * 1024 * 1024;

pub use cache::{MediaCache, MediaReader};
pub use client::{ProjectClient, RequestError};
#[cfg(feature = "server")]
pub use server::serve;
pub use workspace::{ProjectLink, SharedWorkspace, WorkspaceUpdate, prepare_uploads};

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use shrimply_editing_api::protocol::EditRequest;
pub use shrimply_editing_api::protocol::Presence;
use shrimply_project_document::project::{Project, ProjectChanges};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct ProjectVersion {
    pub revision: u64,
    pub project: Arc<Project>,
}

#[derive(Serialize, Deserialize)]
pub struct ProjectVersionReceipt {
    pub revision: u64,
}

#[derive(Serialize, Deserialize)]
pub struct CommitRequest {
    pub operation_id: Uuid,
    pub base_revision: u64,
    pub edit: EditRequest,
}

#[derive(Serialize, Deserialize)]
pub struct ChangeRequest {
    pub operation_id: Uuid,
    pub base_revision: u64,
    pub changes: ProjectChanges,
}

#[derive(Serialize, Deserialize)]
pub struct SyncRequest {
    pub revision: u64,
    pub presence: Option<Presence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<Presence>,
}

#[derive(Serialize, Deserialize)]
pub struct SyncResponse {
    pub version: Option<ProjectVersion>,
    pub participants: Vec<Presence>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaBlob {
    pub digest: String,
    pub length: u64,
}

impl MediaBlob {
    fn reference_path(&self, source: &Path) -> PathBuf {
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
struct ErrorResponse {
    message: String,
    revision: Option<u64>,
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == shrimply_resource_pipeline::fingerprint::HEX_DIGEST_BYTES
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
