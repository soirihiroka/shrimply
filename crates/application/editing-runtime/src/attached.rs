use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    fs,
    path::Path,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use shrimply_editing_api::{
    bridge::{Bridge, BridgeError},
    protocol::{
        ApplyChanges, BridgeCommand, LiveSnapshot, MediaRewrites, SessionState, SessionUpdates,
        SessionVersion,
    },
};
use shrimply_project_document::project::{self, Project, ProjectChanges, PublishError};
use shrimply_project_service::{MediaCache, ProjectClient, SharedWorkspace};
use uuid::Uuid;

use super::{
    SharedPlayerState, SharedSelectionState, content_revision, player_state, replace_live_project,
};

mod version;
pub use shrimply_editing_api::bridge::local_service_status;
pub use version::{MatchVersion, VersionMismatch, VersionPrompt};

const UPDATE_INTERVAL: Duration = Duration::from_millis(100);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum OpenError {
    Version(Box<VersionPrompt>),
    Failed(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version(prompt) => write!(formatter, "{prompt}"),
            Self::Failed(error) => formatter.write_str(error),
        }
    }
}

impl From<String> for OpenError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl From<BridgeError> for OpenError {
    fn from(error: BridgeError) -> Self {
        Self::Failed(error.to_string())
    }
}

#[cfg(not(target_os = "android"))]
pub fn open(
    path: &Path,
    forced: &[VersionMismatch],
    action: Option<MatchVersion>,
) -> Result<Option<Project>, OpenError> {
    if let Some(action) = action
        && !action.apply(path)?
    {
        return Ok(None);
    }
    Ok(Some(load_snapshot(path, connect_project(path, forced)?)?))
}

#[cfg(not(target_os = "android"))]
pub fn connect_project(path: &Path, forced: &[VersionMismatch]) -> Result<Bridge, OpenError> {
    if let Some(workspace) = SharedWorkspace::open(path)?
        && let Some(prompt) = VersionPrompt::remote(&workspace.link().server)
        && !forced.contains(&prompt.mismatch)
    {
        return Err(OpenError::Version(Box::new(prompt)));
    }
    version::checked_bridge(path, forced)
}

pub fn load(path: &Path) -> Result<Project, String> {
    load_snapshot(
        path,
        Bridge::connect_or_launch(path).map_err(|error| error.to_string())?,
    )
}

#[cfg(not(target_os = "android"))]
pub fn stop_local_service(path: &Path) -> Result<(), String> {
    if let Some(owner) = project::project_lock_owner(path)? {
        MatchVersion::RestartService(owner).apply(path)?;
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
pub fn restart_local_service(path: &Path) -> Result<(), String> {
    stop_local_service(path)?;
    Bridge::connect_or_launch(path)
        .and_then(Bridge::version)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn load_snapshot(path: &Path, bridge: Bridge) -> Result<Project, String> {
    let mut snapshot: LiveSnapshot = serde_json::from_value(
        bridge
            .request_with_timeout(BridgeCommand::Snapshot, OPEN_TIMEOUT)
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if let Some(media) = Media::open(path)? {
        media.bind(&mut snapshot.project)?;
    }
    project::restore_editor_view(path, &mut snapshot.project);
    Ok(snapshot.project)
}

pub fn flush_checkpoint(path: &Path) -> Result<(), String> {
    request(path, BridgeCommand::FlushCheckpoint).map(|_| ())
}

pub fn reload_checkpoint(path: &Path) -> Result<Option<Project>, String> {
    let mut response = request(path, BridgeCommand::ReloadCheckpoint)?;
    serde_json::from_value(response["project"].take()).map_err(|error| error.to_string())
}

pub fn save_copy(path: &Path, project: &Project) -> Result<(), String> {
    if SharedWorkspace::database(path).is_file() {
        return Err("Choose a destination outside other shared project workspaces".into());
    }
    if !project
        .assets()
        .iter()
        .any(|asset| shrimply_project_service::blob_digest(asset.path()).is_some())
    {
        return project::save_project_copy(path, project);
    }
    let path = project::normalized_project_path(path);
    let directory = tempfile::Builder::new()
        .prefix("shrimply-media-")
        .tempdir_in(
            path.parent()
                .ok_or("The project copy needs a parent directory")?,
        )
        .map_err(|error| error.to_string())?;
    let mut project = project.clone();
    let mut copied = HashSet::new();
    let mut result = Ok(());
    project.visit_assets_mut(|asset| {
        let Some(digest) = shrimply_project_service::blob_digest(asset.path()).map(str::to_owned)
        else {
            return;
        };
        if result.is_err() {
            return;
        }
        let destination = directory
            .path()
            .join(asset.path().file_name().expect("blob filename"));
        result = (|| {
            if copied.insert(destination.clone()) {
                let snapshot = asset.snapshot()?;
                let mut reader = snapshot
                    .seekable_reader()
                    .map_err(|error| error.to_string())?
                    .ok_or("The shared asset has no media reader")?;
                let mut file = fs::File::create(&destination).map_err(|error| error.to_string())?;
                let (copied, checksum) =
                    shrimply_resource_pipeline::fingerprint::copy(&mut reader, &mut file)
                        .map_err(|error| error.to_string())?;
                if copied != snapshot.len() || checksum != digest {
                    return Err(format!("The shared asset {digest} failed verification"));
                }
                file.sync_all().map_err(|error| error.to_string())?;
            }
            *asset = shrimply_asset::Asset::new(destination);
            Ok(())
        })();
    });
    result?;
    project::save_project_copy(&path, &project)?;
    let _ = directory.keep();
    Ok(())
}

pub fn activate(path: &Path, project: &Project) {
    let target = path.to_path_buf();
    let mut in_flight: Option<(ApplyChanges, Arc<Project>)> = None;
    let mut canonical_media = MediaRewrites::new();
    project::activate_attached_project(project, path, move |before, after| {
        let mut acknowledged = None;
        if let Some((operation, sent)) = &mut in_flight {
            let sent = sent.clone();
            let result = publish(&target, operation);
            if matches!(&result, Err(PublishError::Conflict(_))) {
                in_flight = None;
            }
            canonical_media.extend(result?);
            acknowledged = Some(sent);
            in_flight = None;
        }
        let before = acknowledged.as_deref().unwrap_or(before);
        if !canonical_media.is_empty() {
            let paths = before.assets();
            canonical_media.retain(|source, _| paths.iter().any(|asset| asset.path() == source));
        }
        let changes = if canonical_media.is_empty() {
            ProjectChanges::between(before, after)
        } else {
            let mut canonical_before = before.clone();
            let mut canonical_after = after.clone();
            for project in [&mut canonical_before, &mut canonical_after] {
                project.visit_assets_mut(|asset| {
                    if let Some(reference) = canonical_media.get(asset.path()) {
                        *asset = reference.clone().into();
                    }
                });
            }
            ProjectChanges::between(&canonical_before, &canonical_after)
        }
        .map_err(|error| PublishError::Retry(error.to_string()))?;
        if !changes.is_empty() {
            let mut operation = ApplyChanges {
                id: Uuid::new_v4(),
                label: "Edit project".into(),
                changes,
                replay: false,
            };
            let result = publish(&target, &mut operation);
            in_flight = Some((operation, Arc::new(after.clone())));
            if !matches!(&result, Err(PublishError::Retry(_))) {
                in_flight = None;
            }
            canonical_media.extend(result?);
        }
        Ok(())
    });
}

fn publish(target: &Path, operation: &mut ApplyChanges) -> Result<MediaRewrites, PublishError> {
    let bridge = reconnect(target).map_err(PublishError::Retry)?;
    let receipt = bridge
        .request(BridgeCommand::ApplyChanges(operation.clone()))
        .map_err(|error| match error {
            BridgeError::Rejected(message)
                if message.starts_with("revision conflict:")
                    || message.starts_with("project changed at ")
                    || message.starts_with("Shared source projects require ") =>
            {
                PublishError::Conflict(message)
            }
            BridgeError::Transport(message) => {
                operation.replay = true;
                PublishError::Retry(message)
            }
            error => PublishError::Retry(error.to_string()),
        })?;
    serde_json::from_value(receipt).map_err(|error| {
        operation.replay = true;
        PublishError::Retry(error.to_string())
    })
}

struct Media {
    workspace: SharedWorkspace,
    client: ProjectClient,
    cache: MediaCache,
}

impl Media {
    fn open(path: &Path) -> Result<Option<Self>, String> {
        let Some(workspace) = SharedWorkspace::open(path)? else {
            return Ok(None);
        };
        let (client, _) = workspace.media_client()?;
        let cache = workspace.media_cache()?;
        Ok(Some(Self {
            workspace,
            client,
            cache,
        }))
    }

    fn bind(&self, project: &mut Project) -> Result<(), String> {
        self.workspace
            .bind_media(project, &self.client, &self.cache)
    }
}

pub struct Editor {
    project: Rc<RefCell<Project>>,
    player: SharedPlayerState,
    selection: SharedSelectionState,
    presence_id: Uuid,
    name: String,
    version: Cell<Option<SessionVersion>>,
    media: Option<Media>,
    next_update: Cell<Instant>,
    sync_error: RefCell<Option<String>>,
    send: async_channel::Sender<SessionUpdates>,
    receive: async_channel::Receiver<(Instant, Result<SessionState, String>)>,
}

impl Editor {
    pub fn new(
        target: &Path,
        project: Rc<RefCell<Project>>,
        player: SharedPlayerState,
        selection: SharedSelectionState,
    ) -> Result<Self, String> {
        let media = Media::open(target)?;
        let target = target.to_path_buf();
        let name = super::peers::local_name();
        let (send, requests) = async_channel::bounded(1);
        let (responses, receive) = async_channel::bounded(1);
        std::thread::spawn(move || {
            while let Ok(update) = requests.recv_blocking() {
                let started = Instant::now();
                let result =
                    request(&target, BridgeCommand::SessionUpdates(update)).and_then(|value| {
                        serde_json::from_value(value).map_err(|error| error.to_string())
                    });
                if responses.send_blocking((started, result)).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            project,
            player,
            selection,
            presence_id: Uuid::new_v4(),
            name,
            version: Cell::new(None),
            media,
            next_update: Cell::new(Instant::now()),
            sync_error: RefCell::new(None),
            send,
            receive,
        })
    }

    pub fn poll(&self) {
        if let Ok((started, result)) = self.receive.try_recv() {
            match result {
                Err(error) => {
                    self.sync_error.replace(Some(error));
                    player_state::set_collaborators(&self.player, Vec::new());
                }
                Ok(state) => {
                    self.sync_error.replace(state.sync_error.clone());
                    if let Err(error) = self.sync_snapshot(started, state) {
                        tracing::debug!(%error, "Waiting to catch up with the editing service");
                    }
                }
            }
        }
        if Instant::now() < self.next_update.get() {
            return;
        }
        let presence = super::peers::capture(
            self.presence_id,
            &self.name,
            &self.player,
            &self.selection,
            &self.project.borrow(),
        );
        self.next_update.set(Instant::now() + UPDATE_INTERVAL);
        let _ = self.send.try_send(SessionUpdates {
            version: self.version.get(),
            presence,
        });
    }

    pub fn sync_error(&self) -> Option<String> {
        self.sync_error.borrow().clone()
    }

    fn sync_snapshot(&self, started: Instant, state: SessionState) -> Result<(), String> {
        let collaborators = state
            .participants
            .into_iter()
            .map(|peer| player_state::Collaborator {
                id: peer.id.as_u128(),
                name: peer.name,
                position: peer.position,
                playing: peer.playing,
                selected: peer.clips,
                selected_tracks: peer.tracks,
            })
            .collect();
        player_state::set_collaborators(&self.player, collaborators);
        if let Some(mut candidate) = state.project {
            let project = self.project.borrow();
            let committed = project::committed_project().ok_or("editor session is closed")?;
            let revision = content_revision(&project)?;
            if content_revision(&committed)? != revision {
                return Err("an interactive edit is in progress".into());
            }
            candidate.copy_view_state_from(&project);
            drop(project);
            let candidate_revision = content_revision(&candidate)?;
            if candidate_revision != revision {
                if let Some(media) = &self.media {
                    media.bind(&mut candidate)?;
                }
                project::adopt_attached_snapshot(&candidate, started)?;
                replace_live_project(&self.project, &self.player, &self.selection, candidate);
            }
        }
        self.version.set(Some(state.version));
        Ok(())
    }
}

fn request(target: &Path, command: BridgeCommand) -> Result<serde_json::Value, String> {
    reconnect(target)?
        .request_with_timeout(command, REQUEST_TIMEOUT)
        .map_err(|error| error.to_string())
}

fn reconnect(target: &Path) -> Result<Bridge, String> {
    #[cfg(not(target_os = "android"))]
    let bridge = version::reconnect(target).map_err(|error| error.to_string());
    #[cfg(target_os = "android")]
    let bridge = Bridge::connect_or_launch(target).map_err(|error| error.to_string());
    bridge
}
