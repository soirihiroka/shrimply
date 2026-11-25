mod cache;
mod client;
mod workspace;

pub use cache::{MediaCache, MediaReader};
pub use client::{ProjectClient, RequestError};
pub use shrimply_editing_api::protocol::Presence;
use shrimply_project_protocol as protocol;
pub use shrimply_project_protocol::{MediaBlob, blob_digest};
pub use workspace::{ProjectLink, SharedWorkspace, WorkspaceUpdate, prepare_uploads};

use serde::{Deserialize, Serialize};
use shrimply_project_document::project::{Project, ProjectChanges};
use uuid::Uuid;

pub type ProjectVersion = protocol::ProjectVersion<Project>;
type SyncRequest = protocol::SyncRequest<Presence>;
type SyncResponse = protocol::SyncResponse<Project, Presence>;

#[derive(Serialize, Deserialize)]
struct ChangeRequest {
    operation_id: Uuid,
    changes: ProjectChanges,
}
