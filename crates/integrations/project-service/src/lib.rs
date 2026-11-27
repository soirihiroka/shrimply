mod cache;
pub mod catalog;
mod client;
mod workspace;

pub use cache::{MediaCache, MediaReader};
pub use client::{ProjectClient, RequestError};
pub use shrimply_editing_api::protocol::Presence;
use shrimply_project_protocol as protocol;
pub use shrimply_project_protocol::{MediaBlob, SERVER_PROTOCOL, ServerVersion, blob_digest};
pub use workspace::{ProjectLink, SharedWorkspace, WorkspaceUpdate, prepare_uploads};

use serde::{Deserialize, Serialize};
use shrimply_project_document::project::{Project, ProjectChanges};
use uuid::Uuid;

pub type ProjectVersion = protocol::ProjectVersion<Project>;
type SyncRequest = protocol::SyncRequest<Presence>;
type SyncResponse = protocol::SyncResponse<Project, Presence>;

pub fn update_server(
    preferences: &shrimply_preferences_core::SharedPreferences,
    previous: Option<&str>,
    endpoint: Option<&str>,
) -> Result<(), String> {
    let endpoint = endpoint
        .map(shrimply_preferences_core::normalize_project_server_url)
        .transpose()?;
    if let Some(server) = &endpoint {
        ProjectClient::configured(server)?;
    }
    shrimply_preferences_core::update_project_server(preferences, previous, endpoint.as_deref())?;
    if let Some(previous) = previous
        && endpoint.is_none()
    {
        ProjectClient::forget(previous)?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct ChangeRequest {
    operation_id: Uuid,
    changes: ProjectChanges,
}
