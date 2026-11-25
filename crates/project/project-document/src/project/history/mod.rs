mod recovery;
mod status;
mod storage;
mod view;

pub use status::{CommitStatus, connect_commit_status, poll as poll_commit_status};
pub use storage::{create_new_project_file, create_project_file, serialize_project_json};

use super::*;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use status::request_finish as request_finish_commit_status;
use status::{SaveStatus, begin as begin_commit_status, finish as finish_commit_status};
use status::{request_save as request_save_status_update, set_save as set_save_status};
use view::View;

const RECOVERY_FLUSH_INTERVAL: Duration = Duration::from_millis(150);
const ATTACHED_SAVE_DEBOUNCE: Duration = Duration::from_millis(20);
const ATTACHED_SAVE_RETRY: Duration = Duration::from_secs(1);
const HISTORY_SAVE_DEBOUNCE: Duration = Duration::from_secs(2);
const HISTORY_SAVE_INTERVAL: Duration = Duration::from_secs(30);
static NEXT_HISTORY_STEP: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static ACTIVE_SESSION: RefCell<Option<ActiveSession>> = const { RefCell::new(None) };
    static HISTORY_REVISION: Cell<u64> = const { Cell::new(0) };
    static PROVISIONAL_EDIT: Cell<bool> = const { Cell::new(false) };
}

struct ActiveSession {
    attached: bool,
    document: ProjectSession,
    sender: HistorySender,
    fingerprint: Arc<Mutex<Option<String>>>,
}

#[derive(Default)]
struct PendingUpdates {
    content: Option<(u64, Arc<Project>)>,
    view: Option<View>,
    count: usize,
}

#[derive(Clone)]
struct HistorySender {
    jobs: Sender<HistoryJob>,
    updates: Arc<Mutex<Option<PendingUpdates>>>,
    awaiting_publish: Arc<AtomicBool>,
}

impl HistorySender {
    fn send(&self, job: HistoryJob) -> Result<(), mpsc::SendError<HistoryJob>> {
        self.jobs.send(job)
    }

    fn update(&self, change: impl FnOnce(&mut PendingUpdates)) -> bool {
        let mut pending = self.updates.lock().expect("project updates lock poisoned");
        if pending.is_none() && self.jobs.send(HistoryJob::Update).is_err() {
            return false;
        }
        let pending = pending.get_or_insert_with(PendingUpdates::default);
        change(pending);
        if pending.content.is_some() {
            self.awaiting_publish.store(true, Ordering::Release);
        }
        true
    }
}

pub enum PublishError {
    Retry(String),
    Conflict(String),
}

type PublishAttached = dyn FnMut(&Project, &Project) -> Result<(), PublishError> + Send;

struct Attachment {
    published: Arc<Project>,
    published_at: Instant,
    recovery: PathBuf,
    conflict: Option<String>,
    staged: Option<Arc<Project>>,
    publish: Box<PublishAttached>,
}

pub fn session_is_attached() -> bool {
    ACTIVE_SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|session| session.attached)
    })
}

pub fn restore_editor_view(path: &Path, project: &mut Project) {
    match View::read(path) {
        Ok(Some(view)) => view.apply(project),
        Ok(None) => (),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "Could not restore the editor view")
        }
    }
}

pub fn activate_attached_project(
    project: &Project,
    path: &Path,
    publish: impl FnMut(&Project, &Project) -> Result<(), PublishError> + Send + 'static,
) {
    set_active_project_path(path);
    let project = Arc::new(project.clone());
    let project_key = shrimply_resource_pipeline::fingerprint::digest(
        normalized_project_path(path).as_os_str().as_encoded_bytes(),
    );
    let recovery = shrimply_path_core::config_directory()
        .join("attached-recovery")
        .join(project_key)
        .join(format!("{}.shrimp", Uuid::new_v4()));
    let attachment = Attachment {
        published: project.clone(),
        published_at: Instant::now(),
        recovery,
        conflict: None,
        staged: None,
        publish: Box::new(publish),
    };
    start_history_worker(project, path.to_path_buf(), None, false, Some(attachment));
}

/// An editor invalidation token that also changes when a session is replaced.
pub fn history_revision() -> u64 {
    HISTORY_REVISION.get()
}

fn advance_history_revision() {
    HISTORY_REVISION.set(
        history_revision()
            .checked_add(1)
            .expect("project history revision overflow"),
    );
}

pub fn committed_project() -> Option<Arc<Project>> {
    ACTIVE_SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|session| session.document.current().clone())
    })
}

pub fn history_destination(redo: bool) -> Option<Project> {
    ACTIVE_SESSION.with(|slot| {
        let slot = slot.borrow();
        Some((**slot.as_ref()?.document.history_destination(redo)?).clone())
    })
}

fn commit_session(project: Arc<Project>, coalesce_group: Option<&str>) {
    ACTIVE_SESSION.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .expect("project history is unavailable")
            .document
            .commit(project, coalesce_group);
    });
    advance_history_revision();
}

fn change_session_history(project: &mut Project, redo: bool) -> bool {
    let changed = ACTIVE_SESSION.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .is_some_and(|session| session.document.change_history(project, redo))
    });
    if changed {
        advance_history_revision();
    }
    changed
}

/// Exact bytes last read or durably written for the current document.
/// The receipt belongs to this history owner and is released with it.
pub fn saved_project_fingerprint() -> Option<String> {
    ACTIVE_SESSION.with(|slot| {
        slot.borrow()
            .as_ref()?
            .fingerprint
            .lock()
            .expect("project receipt lock poisoned")
            .clone()
    })
}

pub fn checkpoint_fingerprint(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let mut hash = shrimply_resource_pipeline::fingerprint::Writer::default();
    std::io::copy(&mut file, &mut hash)
        .map_err(|error| format!("could not read project: {error}"))?;
    Ok(hash.finish())
}

pub struct PreparedProject {
    project: Option<Project>,
    fingerprint: String,
    recovered: bool,
    path: PathBuf,
}

impl PreparedProject {
    pub fn trust_review(&self) -> Result<shrimply_trust_core::Review, String> {
        shrimply_trust_core::Review::new(
            self.project
                .as_ref()
                .expect("prepared project exists")
                .executable_sources(),
        )
    }
}

impl Drop for PreparedProject {
    fn drop(&mut self) {
        if self.project.is_some() {
            release_project_lock(&self.path);
        }
    }
}

pub fn prepare_project(path: &Path) -> Result<PreparedProject, ProjectLoadError> {
    match prepare_project_with_frame_grid_repair(path)? {
        ProjectPreparation::Ready(project) => Ok(project),
        ProjectPreparation::FrameGridRepair(_) => Err(ProjectLoadError::Other(
            "project is not aligned to the project frame grid".to_string(),
        )),
    }
}

pub enum ProjectPreparation {
    Ready(PreparedProject),
    FrameGridRepair(Project),
}

/// Read the saved project without acquiring an editor lock or starting autosave.
pub fn load_project_snapshot(path: &Path) -> Result<Project, String> {
    load_project_snapshot_with_fingerprint(path).map(|(project, _)| project)
}

/// Return the validated project and the identity of the very bytes decoded.
pub fn load_project_snapshot_with_fingerprint(path: &Path) -> Result<(Project, String), String> {
    let (outcome, fingerprint) = read_project_with_frame_grid_repair(path)?;
    match outcome {
        ProjectValidationOutcome::Valid(project) => Ok((project, fingerprint)),
        ProjectValidationOutcome::FrameGridRepair(_) => {
            Err("Project needs frame-grid repair. Open and save it in the editor first.".into())
        }
    }
}

/// Check for a new on-disk document without adopting a partially written checkpoint.
pub fn changed_checkpoint(
    path: &Path,
    observed_fingerprint: &str,
) -> Result<Option<(Project, String)>, String> {
    if saved_project_fingerprint().as_deref() == Some(observed_fingerprint) {
        return Ok(None);
    }
    let (project, fingerprint) = load_project_snapshot_with_fingerprint(path)?;
    if checkpoint_fingerprint(path)? != fingerprint {
        return Err("Project changed while loading. Wait for its next complete save.".into());
    }
    Ok(Some((project, fingerprint)))
}

fn read_project_with_frame_grid_repair(
    path: &Path,
) -> Result<(ProjectValidationOutcome, String), String> {
    if !is_project_path(path) {
        return Err(format!(
            "{} is not a supported project file",
            path.display()
        ));
    }
    let (project, fingerprint) = storage::read_project(path)?;
    let outcome = validate_with_frame_grid_repair(project)?;
    Ok((
        prepare_validation_outcome(outcome, path.parent().unwrap_or_else(|| Path::new("."))),
        fingerprint,
    ))
}

pub fn read_project_for_editor(path: &Path) -> Result<ProjectValidationOutcome, String> {
    read_project_with_frame_grid_repair(path).map(|(outcome, _)| outcome)
}

pub fn prepare_project_with_frame_grid_repair(
    path: &Path,
) -> Result<ProjectPreparation, ProjectLoadError> {
    if !is_project_path(path) {
        return Err(ProjectLoadError::Other(format!(
            "{} is not a supported project file",
            path.display()
        )));
    }
    acquire_project_lock(path).map_err(project_lock_error)?;
    let started = Instant::now();
    tracing::info!(path = %path.display(), "Loading project");
    let (outcome, fingerprint) = read_project_with_frame_grid_repair(path).map_err(|error| {
        release_project_lock(path);
        ProjectLoadError::Other(error)
    })?;
    let mut project = match outcome {
        ProjectValidationOutcome::Valid(project) => project,
        ProjectValidationOutcome::FrameGridRepair(project) => {
            release_project_lock(path);
            return Ok(ProjectPreparation::FrameGridRepair(project));
        }
    };
    let recovered = (|| {
        let Some(bytes) = recovery::load(&recovery::path(path), &fingerprint)? else {
            return Ok(false);
        };
        let encoding = ProjectEncoding::from_path(path).expect("validated project file extension");
        let outcome = validate_with_frame_grid_repair(encoding.decode(&bytes)?)?;
        let directory = path.parent().unwrap_or_else(|| Path::new("."));
        match prepare_validation_outcome(outcome, directory) {
            ProjectValidationOutcome::Valid(candidate) => {
                project = candidate;
                Ok(true)
            }
            ProjectValidationOutcome::FrameGridRepair(_) => {
                Err("project recovery data needs frame-grid repair".to_string())
            }
        }
    })()
    .map_err(|error| {
        release_project_lock(path);
        ProjectLoadError::Other(error)
    })?;
    tracing::info!(
        path = %path.display(),
        elapsed_ms = started.elapsed().as_millis(),
        "Prepared project"
    );
    Ok(ProjectPreparation::Ready(PreparedProject {
        project: Some(project),
        fingerprint,
        recovered,
        path: path.to_path_buf(),
    }))
}

pub fn activate_project(mut prepared: PreparedProject) -> Project {
    let project = prepared.project.take().expect("prepared project exists");
    set_active_project_path(&prepared.path);
    start_history_worker(
        Arc::new(project.clone()),
        prepared.path.clone(),
        Some(prepared.fingerprint.clone()),
        prepared.recovered,
        None,
    );
    tracing::info!(path = %prepared.path.display(), "Activated project");
    project
}

/// Preview a synchronous inspector mutation without advancing autosave or history.
/// Saved external/API writes deliberately do not participate in this scope.
pub fn with_provisional_edit<R>(edit: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            PROVISIONAL_EDIT.set(self.0);
        }
    }
    let previous = PROVISIONAL_EDIT.replace(true);
    let _restore = Restore(previous);
    edit()
}

pub fn commit_edit(project: &Project, message: &str) -> bool {
    commit(project, message, None)
}

pub fn commit_edit_checked(project: &Project, message: &str) -> Result<(), String> {
    project
        .validate()
        .map_err(|error| format!("cannot commit invalid project: {error}"))?;
    if PROVISIONAL_EDIT.get() {
        return Ok(());
    }
    queue_and_commit(project, message, None)
        .then_some(())
        .ok_or_else(|| "project history worker rejected the edit".to_string())
}

/// Persist a prepared API edit before advancing the in-memory undo history.
pub fn commit_edit_saved(project: &Project, message: &str) -> Result<(), String> {
    project
        .validate()
        .map_err(|error| format!("cannot commit invalid project: {error}"))?;
    let sender = ACTIVE_SESSION
        .with(|slot| slot.borrow().as_ref().map(|worker| worker.sender.clone()))
        .ok_or("project history is unavailable")?;
    let (response_tx, response_rx) = mpsc::channel();
    begin_commit_status(message);
    let project = Arc::new(project.clone());
    sender
        .send(HistoryJob::CommitSaved {
            project: project.clone(),
            response_tx,
        })
        .map_err(|_| "project history worker stopped unexpectedly")?;
    let result = response_rx
        .recv()
        .map_err(|_| "project history worker dropped the commit response")?;
    finish_commit_status();
    result?;
    commit_session(project, None);
    Ok(())
}

/// Adopt an already validated external save without writing over the file that
/// triggered it. Both autosave's baseline and undo history advance together.
pub fn adopt_project_snapshot(project: &Project, fingerprint: &str) -> Result<(), String> {
    adopt_snapshot(project, fingerprint, None)
}

pub fn adopt_attached_snapshot(project: &Project, read_started: Instant) -> Result<(), String> {
    let pending = ACTIVE_SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|session| session.sender.awaiting_publish.load(Ordering::Acquire))
    });
    if pending {
        return Err("local edits are waiting to be published".into());
    }
    adopt_snapshot(project, "", Some(read_started))
}

fn adopt_snapshot(
    project: &Project,
    fingerprint: &str,
    read_started: Option<Instant>,
) -> Result<(), String> {
    project.validate()?;
    let sender = ACTIVE_SESSION
        .with(|slot| slot.borrow().as_ref().map(|worker| worker.sender.clone()))
        .ok_or("project history is unavailable")?;
    let (response_tx, response_rx) = mpsc::channel();
    let project = Arc::new(project.clone());
    sender
        .send(HistoryJob::Adopt {
            project: project.clone(),
            fingerprint: fingerprint.to_owned(),
            read_started,
            response_tx,
        })
        .map_err(|_| "project history worker stopped unexpectedly")?;
    response_rx
        .recv()
        .map_err(|_| "project history worker dropped the adopt response")??;
    if session_is_attached() {
        adopt_peer(project);
    } else {
        commit_session(project, None);
    }
    Ok(())
}

pub fn commit_peer_snapshot(project: &Project) -> Result<(), String> {
    project.validate()?;
    let project = Arc::new(project.clone());
    if !queue_project(project.clone(), "Shared project update") {
        return Err("project history worker stopped unexpectedly".into());
    }
    adopt_peer(project);
    Ok(())
}

fn adopt_peer(project: Arc<Project>) {
    ACTIVE_SESSION.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .expect("project session exists")
            .document
            .adopt_peer(project);
    });
    advance_history_revision();
}

/// Persist a history destination before moving its cursor or exposing the project.
pub fn change_history_saved(project: &mut Project, redo: bool) -> Result<bool, String> {
    let Some(mut candidate) = history_destination(redo) else {
        return Ok(false);
    };
    candidate.copy_view_state_from(project);
    let sender = ACTIVE_SESSION
        .with(|slot| slot.borrow().as_ref().map(|worker| worker.sender.clone()))
        .ok_or("project history is unavailable")?;
    let (response_tx, response_rx) = mpsc::channel();
    sender
        .send(HistoryJob::CommitSaved {
            project: Arc::new(candidate),
            response_tx,
        })
        .map_err(|_| "project history worker stopped unexpectedly")?;
    response_rx
        .recv()
        .map_err(|_| "project history worker dropped the commit response")??;
    Ok(change_session_history(project, redo))
}

pub fn commit_coalesced_edit(project: &Project, group: &str) -> bool {
    commit(project, group, Some(group))
}

pub fn finish_coalesced_edit() {
    ACTIVE_SESSION.with(|slot| {
        if let Some(session) = slot.borrow_mut().as_mut() {
            session.document.finish_coalesced_edit();
        }
    });
}

pub fn can_undo() -> bool {
    can_change_history(false)
}

pub fn can_redo() -> bool {
    can_change_history(true)
}

fn can_change_history(redo: bool) -> bool {
    ACTIVE_SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|session| session.document.history_destination(redo).is_some())
    })
}

pub fn undo(project: &mut Project) -> bool {
    queue_history_change(project, false, "undo")
}

pub fn redo(project: &mut Project) -> bool {
    queue_history_change(project, true, "redo")
}

fn queue_history_change(project: &mut Project, redo: bool, action: &str) -> bool {
    if !change_session_history(project, redo) {
        return false;
    }
    queue_project(Arc::new(project.clone()), action);
    true
}

pub fn save() -> Result<(), String> {
    save_history(false)
}

/// Flush queued edits and view changes without rewriting an unchanged document.
pub fn flush_history() -> Result<(), String> {
    save_history(true)
}

fn save_history(only_pending: bool) -> Result<(), String> {
    let sender = ACTIVE_SESSION
        .with(|slot| slot.borrow().as_ref().map(|worker| worker.sender.clone()))
        .ok_or_else(|| "project history is unavailable".to_string())?;
    let (response_tx, response_rx) = mpsc::channel();
    sender
        .send(HistoryJob::Save {
            response_tx,
            only_pending,
        })
        .map_err(|_| "project history worker stopped unexpectedly".to_string())?;
    response_rx
        .recv()
        .map_err(|_| "project history worker dropped the save response".to_string())?
}

pub fn shutdown_history() -> Result<(), String> {
    let Some(sender) =
        ACTIVE_SESSION.with(|slot| slot.borrow().as_ref().map(|worker| worker.sender.clone()))
    else {
        advance_history_revision();
        return Ok(());
    };
    let (response_tx, response_rx) = mpsc::channel();
    sender
        .send(HistoryJob::Shutdown { response_tx })
        .map_err(|_| "project history worker stopped unexpectedly".to_string())?;
    response_rx
        .recv()
        .map_err(|_| "project history worker dropped the shutdown response".to_string())??;
    ACTIVE_SESSION.with(|slot| slot.borrow_mut().take());
    advance_history_revision();
    Ok(())
}

pub fn save_as(path: &Path) -> Result<(), String> {
    if !is_project_path(path) {
        return Err(
            "projects can only be saved as .shrimp, .sjson, or legacy .json files".to_string(),
        );
    }
    let current_path = active_project_path();
    let path_changed = current_path != path;
    if path_changed {
        acquire_project_lock(path).map_err(project_lock_message)?;
    }

    let result = (|| {
        let sender = ACTIVE_SESSION
            .with(|slot| slot.borrow().as_ref().map(|worker| worker.sender.clone()))
            .ok_or_else(|| "project history is unavailable".to_string())?;
        let (response_tx, response_rx) = mpsc::channel();
        sender
            .send(HistoryJob::SaveAs {
                path: path.to_path_buf(),
                response_tx,
            })
            .map_err(|_| "project history worker stopped unexpectedly".to_string())?;
        response_rx
            .recv()
            .map_err(|_| "project history worker dropped the save response".to_string())?
    })();

    if result.is_ok() && path_changed {
        ACTIVE_SESSION.with(|slot| {
            if let Some(session) = slot.borrow_mut().as_mut() {
                session.attached = false;
            }
        });
        release_project_lock(&current_path);
        set_active_project_path(path);
    } else if result.is_err() && path_changed {
        release_project_lock(path);
    }
    result
}

pub fn save_project_copy(path: &Path, project: &Project) -> Result<(), String> {
    let path = normalized_project_path(path);
    acquire_project_lock(&path).map_err(project_lock_message)?;
    let result = storage::write_project(&path, project).map(|_| ());
    release_project_lock(&path);
    result
}

pub fn save_view_state(project: &Project) {
    let sender = history_worker(|| Arc::new(project.clone()));
    if !sender.update(|pending| pending.view = Some(View::capture(project))) {
        tracing::warn!("Could not queue project view state");
        set_save_status(SaveStatus::Failed(
            "project history worker stopped unexpectedly".to_string(),
        ));
    }
}

fn commit(project: &Project, message: &str, coalesce_group: Option<&str>) -> bool {
    project
        .validate()
        .unwrap_or_else(|error| panic!("cannot commit invalid project: {error}"));
    if PROVISIONAL_EDIT.get() {
        return true;
    }
    queue_and_commit(project, message, coalesce_group)
}

fn queue_and_commit(project: &Project, message: &str, coalesce_group: Option<&str>) -> bool {
    let project = Arc::new(project.clone());
    if !queue_project(project.clone(), message) {
        return false;
    }
    commit_session(project, coalesce_group);
    true
}

fn queue_project(project: Arc<Project>, action: &str) -> bool {
    let sender = history_worker(|| project.clone());
    let step = NEXT_HISTORY_STEP
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    begin_commit_status(action);
    tracing::trace!(step, action, "Queueing project snapshot");
    if sender.update(|pending| {
        pending.content = Some((step, project));
        pending.view = None;
        pending.count += 1;
    }) {
        true
    } else {
        tracing::warn!("Could not queue project snapshot");
        set_save_status(SaveStatus::Failed(
            "project history worker stopped unexpectedly".to_string(),
        ));
        finish_commit_status();
        false
    }
}

enum HistoryJob {
    Adopt {
        project: Arc<Project>,
        fingerprint: String,
        read_started: Option<Instant>,
        response_tx: Sender<Result<(), String>>,
    },
    CommitSaved {
        project: Arc<Project>,
        response_tx: Sender<Result<(), String>>,
    },
    Update,
    Save {
        response_tx: Sender<Result<(), String>>,
        only_pending: bool,
    },
    SaveAs {
        path: PathBuf,
        response_tx: Sender<Result<(), String>>,
    },
    Shutdown {
        response_tx: Sender<Result<(), String>>,
    },
}

fn history_worker(project: impl FnOnce() -> Arc<Project>) -> HistorySender {
    let sender =
        ACTIVE_SESSION.with(|slot| slot.borrow().as_ref().map(|session| session.sender.clone()));
    sender.unwrap_or_else(|| {
        start_history_worker(project(), active_project_path(), None, false, None)
    })
}

fn start_history_worker(
    mut project: Arc<Project>,
    mut path: PathBuf,
    initial_fingerprint: Option<String>,
    recovered: bool,
    mut attachment: Option<Attachment>,
) -> HistorySender {
    let (jobs, receiver) = mpsc::channel();
    let updates = Arc::new(Mutex::new(None::<PendingUpdates>));
    let awaiting_publish = Arc::new(AtomicBool::new(false));
    let sender = HistorySender {
        jobs,
        updates: updates.clone(),
        awaiting_publish: awaiting_publish.clone(),
    };
    let fingerprint = Arc::new(Mutex::new(initial_fingerprint));
    let observed_fingerprint = fingerprint.clone();
    let attached = attachment.is_some();
    let document = if attached {
        ProjectSession::at_revision(project.clone(), 0, session::SHARED_HISTORY_LIMIT)
    } else {
        ProjectSession::new(project.clone())
    };
    thread::spawn(move || {
        let finish_publish = || {
            let pending = updates.lock().expect("project updates lock poisoned");
            if pending
                .as_ref()
                .is_none_or(|pending| pending.content.is_none())
            {
                awaiting_publish.store(false, Ordering::Release);
            }
        };
        let mut recovery_path = recovery::path(&path);
        let mut pending_save = recovered;
        let mut pending_content = recovered;
        let mut last_update = Instant::now();
        let mut last_snapshot = Instant::now();
        let mut recovery_due: Option<Instant> = None;
        let mut save_failed = false;
        loop {
            let now = Instant::now();
            if recovery_due.is_some_and(|due| due <= now) {
                recovery_due = None;
                let receipt = observed_fingerprint
                    .lock()
                    .expect("project receipt lock poisoned");
                if let Some(checkpoint) = receipt.as_deref()
                    && let Err(error) = storage::encode_project(&path, &project)
                        .and_then(|bytes| recovery::write(&recovery_path, checkpoint, &bytes))
                {
                    request_save_status_update(SaveStatus::Failed(error));
                }
            }
            let job = if pending_save {
                let debounce = if attachment.is_some() && pending_content {
                    if save_failed {
                        ATTACHED_SAVE_RETRY
                    } else {
                        ATTACHED_SAVE_DEBOUNCE
                    }
                } else {
                    HISTORY_SAVE_DEBOUNCE
                };
                let idle_wait = debounce.saturating_sub(now.duration_since(last_update));
                let periodic_wait =
                    HISTORY_SAVE_INTERVAL.saturating_sub(now.duration_since(last_snapshot));
                let wait = if pending_content && attachment.is_none() {
                    idle_wait.min(periodic_wait)
                } else {
                    idle_wait
                };
                let wait = recovery_due.map_or(wait, |due| wait.min(due.duration_since(now)));
                match receiver.recv_timeout(wait) {
                    Ok(job) => job,
                    Err(mpsc::RecvTimeoutError::Timeout)
                        if recovery_due.is_some_and(|due| due <= Instant::now()) =>
                    {
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let saving_content = pending_content;
                        let result = if saving_content {
                            save_project(
                                &path,
                                &project,
                                &observed_fingerprint,
                                attachment.as_mut(),
                            )
                        } else {
                            save_view_state_project(
                                &path,
                                &project,
                                &observed_fingerprint,
                                attachment.is_some(),
                            )
                        };
                        pending_save = result.is_err();
                        save_failed = pending_save;
                        if result.is_ok() {
                            finish_publish();
                            pending_content = false;
                            recovery_due = None;
                        }
                        if pending_save {
                            last_update = Instant::now();
                        }
                        if saving_content
                            && attachment.is_none()
                            && last_snapshot.elapsed() >= HISTORY_SAVE_INTERVAL
                        {
                            save_snapshot(&path, &project);
                            last_snapshot = Instant::now();
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        if pending_content {
                            let _ = save_project(
                                &path,
                                &project,
                                &observed_fingerprint,
                                attachment.as_mut(),
                            );
                        } else {
                            let _ = save_view_state_project(
                                &path,
                                &project,
                                &observed_fingerprint,
                                attachment.is_some(),
                            );
                        }
                        break;
                    }
                }
            } else {
                match receiver.recv() {
                    Ok(job) => job,
                    Err(_) => break,
                }
            };

            match job {
                HistoryJob::Adopt {
                    project: updated,
                    fingerprint: loaded_fingerprint,
                    read_started,
                    response_tx,
                } => {
                    if let Some(attachment) = &attachment
                        && read_started.is_some_and(|started| started < attachment.published_at)
                    {
                        let _ = response_tx.send(Err("a newer local edit was published".into()));
                        continue;
                    }
                    if pending_content {
                        let _ = response_tx.send(Err(
                            "project changed outside Shrimply while it had unsaved edits".into(),
                        ));
                        continue;
                    }
                    project = updated;
                    if let Some(attachment) = &mut attachment {
                        attachment.published = project.clone();
                    }
                    if read_started.is_none() {
                        *observed_fingerprint
                            .lock()
                            .expect("project receipt lock poisoned") = Some(loaded_fingerprint);
                    }
                    recovery::discard(&recovery_path);
                    pending_save &= attachment.is_some();
                    pending_content = false;
                    if attachment
                        .as_ref()
                        .is_none_or(|attachment| attachment.conflict.is_none())
                    {
                        request_save_status_update(SaveStatus::Saved);
                    }
                    let _ = response_tx.send(Ok(()));
                }
                HistoryJob::CommitSaved {
                    project: updated,
                    response_tx,
                } => {
                    if pending_content {
                        if let Err(error) = save_project(
                            &path,
                            &project,
                            &observed_fingerprint,
                            attachment.as_mut(),
                        ) {
                            let _ = response_tx.send(Err(error));
                            continue;
                        }
                        finish_publish();
                        pending_content = false;
                        pending_save = false;
                        recovery_due = None;
                    }
                    let result =
                        save_project(&path, &updated, &observed_fingerprint, attachment.as_mut());
                    if result.is_ok() {
                        finish_publish();
                        project = updated;
                        pending_save = false;
                        pending_content = false;
                    }
                    let _ = response_tx.send(result);
                }
                HistoryJob::Update => {
                    let pending = updates
                        .lock()
                        .expect("project updates lock poisoned")
                        .take()
                        .expect("an update signal has pending edits or view state");
                    if let Some((step, updated)) = pending.content {
                        project = updated;
                        pending_content = true;
                        save_failed = false;
                        if attachment.is_none() {
                            recovery_due.get_or_insert(Instant::now() + RECOVERY_FLUSH_INTERVAL);
                        }
                        request_save_status_update(SaveStatus::Pending);
                        tracing::debug!(step, "Updated pending project snapshot");
                        for _ in 0..pending.count {
                            request_finish_commit_status();
                        }
                    }
                    if let Some(view) = pending.view {
                        view.apply(Arc::make_mut(&mut project));
                    }
                    pending_save = true;
                    last_update = Instant::now();
                }
                HistoryJob::Save {
                    response_tx,
                    only_pending,
                } => {
                    let result = if only_pending && !pending_save {
                        Ok(())
                    } else if only_pending && !pending_content {
                        save_view_state_project(
                            &path,
                            &project,
                            &observed_fingerprint,
                            attachment.is_some(),
                        )
                    } else {
                        save_project(&path, &project, &observed_fingerprint, attachment.as_mut())
                    };
                    pending_save = result.is_err();
                    save_failed = pending_save;
                    if result.is_ok() {
                        finish_publish();
                        pending_content = false;
                        recovery_due = None;
                    }
                    if pending_save {
                        last_update = Instant::now();
                    }
                    let _ = response_tx.send(result);
                }
                HistoryJob::SaveAs {
                    path: new_path,
                    response_tx,
                } => {
                    if attachment.is_some() {
                        let result = save_project(
                            &path,
                            &project,
                            &observed_fingerprint,
                            attachment.as_mut(),
                        );
                        if result.is_err() || new_path == path {
                            let _ = response_tx.send(result);
                            continue;
                        }
                    }
                    let previous_path = std::mem::replace(&mut path, new_path);
                    let result = write_project(
                        &path,
                        &project,
                        &observed_fingerprint,
                        path == previous_path,
                    );
                    if result.is_err() {
                        path = previous_path;
                    } else {
                        recovery::discard(&recovery_path);
                        recovery_path = recovery::path(&path);
                        attachment = None;
                        recovery_due = None;
                        pending_save = false;
                        pending_content = false;
                    }
                    let _ = response_tx.send(result);
                }
                HistoryJob::Shutdown { response_tx } => {
                    let result = if !pending_save {
                        Ok(())
                    } else if pending_content {
                        save_project(&path, &project, &observed_fingerprint, attachment.as_mut())
                    } else {
                        save_view_state_project(
                            &path,
                            &project,
                            &observed_fingerprint,
                            attachment.is_some(),
                        )
                    };
                    let saved = result.is_ok();
                    let _ = response_tx.send(result);
                    if saved {
                        break;
                    }
                    // Failed shutdown retains its owner, pending document and
                    // undo history so the host can repair storage and retry.
                    last_update = Instant::now();
                }
            }
        }
    });
    ACTIVE_SESSION.with(|slot| {
        *slot.borrow_mut() = Some(ActiveSession {
            attached,
            document,
            sender: sender.clone(),
            fingerprint,
        })
    });
    advance_history_revision();
    sender
}

fn save_project(
    path: &Path,
    project: &Arc<Project>,
    fingerprint: &Mutex<Option<String>>,
    attachment: Option<&mut Attachment>,
) -> Result<(), String> {
    let result = if let Some(attachment) = attachment {
        let result = publish_attached(path, project, attachment);
        request_save_status_update(match &result {
            Ok(()) => attachment
                .conflict
                .clone()
                .map_or(SaveStatus::Saved, SaveStatus::Failed),
            Err(error) => SaveStatus::Failed(error.clone()),
        });
        result
    } else {
        write_project(path, project, fingerprint, true)
    };
    if let Err(error) = &result {
        tracing::warn!(path = %path.display(), "Could not save project: {error}");
    }
    result
}

fn publish_attached(
    path: &Path,
    project: &Arc<Project>,
    attachment: &mut Attachment,
) -> Result<(), String> {
    if !Arc::ptr_eq(&attachment.published, project)
        && attachment
            .staged
            .as_ref()
            .is_none_or(|staged| !Arc::ptr_eq(staged, project))
    {
        let backup = &attachment.recovery;
        fs::create_dir_all(backup.parent().expect("recovery path has a parent"))
            .map_err(|error| error.to_string())
            .and_then(|()| storage::write_project(backup, project))
            .map_err(|error| format!("Could not preserve the pending edit: {error}"))?;
        attachment.staged = Some(project.clone());
    }
    match (attachment.publish)(&attachment.published, project) {
        Ok(()) => {
            recovery::discard(&attachment.recovery);
            if let Err(error) = View::capture(project).write(path) {
                tracing::warn!(path = %path.display(), %error, "Could not save the editor view");
            }
        }
        Err(PublishError::Retry(error)) => {
            return Err(format!(
                "{error}. Unpublished edits have been saved at {}",
                attachment.recovery.display()
            ));
        }
        Err(PublishError::Conflict(error)) => {
            let backup = attachment
                .recovery
                .with_file_name(format!("{}.shrimp", Uuid::new_v4()));
            fs::rename(&attachment.recovery, &backup).map_err(|rename_error| {
                format!(
                    "{error}. Pending edits remain at {} ({rename_error})",
                    attachment.recovery.display()
                )
            })?;
            attachment.conflict = Some(format!(
                "{error}. Conflicting edits were preserved in {}",
                backup.display()
            ));
        }
    }
    attachment.published = project.clone();
    attachment.published_at = Instant::now();
    attachment.staged = None;
    Ok(())
}

fn save_view_state_project(
    path: &Path,
    project: &Project,
    fingerprint: &Mutex<Option<String>>,
    attached: bool,
) -> Result<(), String> {
    if attached {
        return View::capture(project).write(path);
    }
    let started = Instant::now();
    let result = {
        let mut receipt = fingerprint.lock().expect("project receipt lock poisoned");
        write_local_checkpoint(path, project, &mut receipt, true)
    };
    tracing::info!(
        elapsed_us = started.elapsed().as_micros(),
        success = result.is_ok(),
        "View-state disk write completed"
    );
    if let Err(error) = &result {
        tracing::warn!(path = %path.display(), "Could not save project view state: {error}");
    }
    result
}

fn save_snapshot(path: &Path, project: &Project) {
    let started = Instant::now();
    match storage::write_snapshot(path, project) {
        Ok(snapshot) => tracing::info!(
            path = %snapshot.display(),
            elapsed_ms = started.elapsed().as_millis(),
            "Saved periodic project snapshot"
        ),
        Err(error) => tracing::warn!(
            path = %path.display(),
            "Could not save periodic project snapshot: {error}"
        ),
    }
}

fn write_project(
    path: &Path,
    project: &Project,
    fingerprint: &Mutex<Option<String>>,
    compare_checkpoint: bool,
) -> Result<(), String> {
    request_save_status_update(SaveStatus::Saving);
    let started = Instant::now();
    tracing::info!(path = %path.display(), "Saving project");
    let result = {
        // A watcher must not observe our replacement file with the previous receipt.
        let mut receipt = fingerprint.lock().expect("project receipt lock poisoned");
        write_local_checkpoint(path, project, &mut receipt, compare_checkpoint)
    };
    request_save_status_update(match &result {
        Ok(()) => SaveStatus::Saved,
        Err(error) => SaveStatus::Failed(error.clone()),
    });
    if result.is_ok() {
        tracing::info!(
            path = %path.display(),
            elapsed_ms = started.elapsed().as_millis(),
            "Saved project"
        );
    }
    result
}

fn write_local_checkpoint(
    path: &Path,
    project: &Project,
    receipt: &mut Option<String>,
    compare_checkpoint: bool,
) -> Result<(), String> {
    let bytes = storage::encode_project(path, project)?;
    let recovery_path = recovery::path(path);
    if compare_checkpoint && let Some(checkpoint) = receipt.as_deref() {
        let disk = fs::read(path)
            .map_err(|error| format!("could not check project file {}: {error}", path.display()))?;
        if disk != bytes && shrimply_resource_pipeline::fingerprint::digest(&disk) != checkpoint {
            return Err(format!(
                "project file changed outside Shrimply: {}",
                path.display()
            ));
        }
        if recovery_path.is_file() {
            recovery::write(&recovery_path, checkpoint, &bytes)?;
        }
    }
    *receipt = Some(storage::write_checkpoint(path, &bytes)?);
    if compare_checkpoint {
        recovery::discard(&recovery_path);
    }
    Ok(())
}

fn is_project_path(path: &Path) -> bool {
    ProjectEncoding::from_path(path).is_some()
}

fn project_lock_error(error: ProjectLockError) -> ProjectLoadError {
    match error {
        ProjectLockError::AlreadyLockedByThisInstance => {
            ProjectLoadError::Other("project is already open in this instance".to_string())
        }
        ProjectLockError::AlreadyLockedByOtherInstance { owner } => {
            ProjectLoadError::LockedByOtherInstance { owner }
        }
        ProjectLockError::CouldNotInspect(error) => ProjectLoadError::Other(error),
        ProjectLockError::CouldNotCreate(error) => ProjectLoadError::Other(error),
        ProjectLockError::RegistryUnavailable => {
            ProjectLoadError::Other("could not acquire the project lock registry".to_string())
        }
    }
}

fn project_lock_message(error: ProjectLockError) -> String {
    match project_lock_error(error) {
        ProjectLoadError::LockedByOtherInstance { owner } => {
            format!("project is open by process {}", owner.pid())
        }
        ProjectLoadError::Other(error) => error,
    }
}
