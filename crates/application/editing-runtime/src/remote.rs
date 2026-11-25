use std::{path::Path, sync::Mutex};

use shrimply_project_service::{
    MediaCache, Presence, ProjectClient, SharedWorkspace, WorkspaceUpdate,
};

use super::*;

const SYNC_INTERVAL: Duration = Duration::from_millis(200);
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

pub(super) struct Session {
    workspace: SharedWorkspace,
    worker: Option<SyncWorker>,
    media_client: ProjectClient,
    media_cache: MediaCache,
    project: Rc<RefCell<Project>>,
    player: SharedPlayerState,
    selection: SharedSelectionState,
    id: Uuid,
    name: String,
    pending: bool,
    next_sync: Instant,
    participants: Vec<Presence>,
    revision: Option<u64>,
    last_error: Option<String>,
}

struct SyncWorker {
    send: async_channel::Sender<Vec<Presence>>,
    receive: async_channel::Receiver<Result<WorkspaceUpdate, String>>,
}

impl SyncWorker {
    fn start(
        workspace: SharedWorkspace,
        uploads: SharedWorkspace,
        client: ProjectClient,
        cache: MediaCache,
    ) -> Self {
        let (wake_upload, upload_events) = async_channel::bounded(1);
        let upload_error = Arc::new(Mutex::new(None));
        let upload_error_thread = upload_error.clone();
        let upload_client = client.clone();
        thread::spawn(move || {
            while upload_events.recv_blocking().is_ok() {
                let result = uploads.upload_pending(&upload_client, &cache);
                *upload_error_thread
                    .lock()
                    .expect("upload status lock poisoned") = result.err();
            }
        });
        let (send, incoming) = async_channel::bounded(1);
        let (outgoing, receive) = async_channel::bounded(1);
        thread::spawn(move || {
            let mut workspace = workspace;
            while let Ok(presence) = incoming.recv_blocking() {
                if workspace.has_pending_uploads().unwrap_or_default() {
                    let _ = wake_upload.try_send(());
                }
                let result = workspace.synchronize(&client, presence).map(|mut update| {
                    if update.conflict.is_none() {
                        update.conflict = upload_error
                            .lock()
                            .expect("upload status lock poisoned")
                            .clone();
                    }
                    update
                });
                if outgoing.send_blocking(result).is_err() {
                    break;
                }
            }
        });
        Self { send, receive }
    }
}

impl Session {
    pub(super) fn open(
        project_path: &Path,
        project: Rc<RefCell<Project>>,
        player: SharedPlayerState,
        selection: SharedSelectionState,
    ) -> Result<Option<Rc<RefCell<Self>>>, String> {
        let Some(workspace) = SharedWorkspace::open(project_path)? else {
            return Ok(None);
        };
        let media_cache = workspace.media_cache()?;
        let (media_client, last_error) = workspace.media_client()?;
        let worker = if last_error.is_none() {
            let connection = || {
                SharedWorkspace::open(project_path)?
                    .ok_or_else(|| "shared project workspace was removed".to_string())
            };
            Some(SyncWorker::start(
                connection()?,
                connection()?,
                media_client.clone(),
                media_cache.clone(),
            ))
        } else {
            None
        };
        let name = peers::local_name();
        let revision = Some(workspace.confirmed_revision()?);
        let session = Self {
            workspace,
            worker,
            media_client,
            media_cache,
            project,
            player,
            selection,
            id: Uuid::new_v4(),
            name,
            pending: false,
            next_sync: Instant::now(),
            participants: Vec::new(),
            revision,
            last_error,
        };
        session.adopt_draft()?;
        Ok(Some(Rc::new(RefCell::new(session))))
    }

    pub(super) fn upload_directory(&self) -> PathBuf {
        self.workspace.upload_directory().to_path_buf()
    }

    pub(super) fn queue(
        &self,
        before: &Project,
        after: &Project,
        media: &[shrimply_project_service::MediaBlob],
        receipt: Option<(Uuid, &MediaRewrites)>,
    ) -> Result<(), String> {
        self.workspace.queue(before, after, media, receipt)
    }

    pub(super) fn attached_receipt(&self, id: Uuid) -> Result<Option<MediaRewrites>, String> {
        self.workspace.attached_receipt(id)
    }

    pub(super) fn bind_media(&self, project: &mut Project) -> Result<(), String> {
        self.workspace
            .bind_media(project, &self.media_client, &self.media_cache)
    }

    pub(super) fn participants(&self) -> &[Presence] {
        &self.participants
    }

    pub(super) fn sync_error(&self) -> Option<String> {
        self.last_error.clone()
    }

    pub(super) fn annotate(&self, response: &mut Value) {
        if let Some(response) = response.as_object_mut() {
            response.insert(
                "shared_project".into(),
                json!({
                    "id": self.workspace.link().project_id,
                    "server": self.workspace.link().server,
                    "revision": self.revision,
                    "participants": self.participants,
                    "error": self.last_error,
                }),
            );
        }
    }

    pub(super) fn poll(&mut self, mut local: Vec<Presence>) {
        if self.pending {
            let worker = self.worker.as_ref().expect("pending work has a worker");
            let result = match worker.receive.try_recv() {
                Ok(result) => result,
                Err(async_channel::TryRecvError::Empty) => return,
                Err(async_channel::TryRecvError::Closed) => {
                    Err("project synchronization worker stopped".into())
                }
            };
            self.pending = false;
            let result = result.and_then(|update| {
                if self.revision != Some(update.revision) || update.draft_changed {
                    self.adopt_draft()?;
                    self.revision = Some(update.revision);
                }
                self.participants = update
                    .participants
                    .into_iter()
                    .filter(|participant| participant.id != self.id)
                    .collect();
                Ok(update.conflict)
            });
            self.next_sync = Instant::now()
                + if result.is_ok() {
                    SYNC_INTERVAL
                } else {
                    RETRY_INTERVAL
                };
            self.last_error = match result {
                Ok(conflict) => conflict,
                Err(error) => Some(error),
            };
        }
        if Instant::now() < self.next_sync {
            return;
        }
        let Some(worker) = &self.worker else {
            return;
        };
        if local.is_empty() {
            local.push(peers::capture(
                self.id,
                &self.name,
                &self.player,
                &self.selection,
                &self.project.borrow(),
            ));
        }
        match worker.send.try_send(local) {
            Ok(()) => self.pending = true,
            Err(_) => {
                self.next_sync = Instant::now() + RETRY_INTERVAL;
                self.last_error = Some("project synchronization worker stopped".into());
            }
        }
    }

    fn adopt_draft(&self) -> Result<(), String> {
        let mut draft = self.workspace.draft()?;
        self.bind_media(&mut draft)?;
        let current = self.project.borrow();
        draft.copy_view_state_from(&current);
        if content_revision(&draft)? == content_revision(&current)? {
            return Ok(());
        }
        drop(current);
        shrimply_project_document::project::commit_peer_snapshot(&draft)?;
        replace_live_project(&self.project, &self.player, &self.selection, draft);
        Ok(())
    }
}
