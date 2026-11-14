mod memory;
mod status;
mod storage;

pub use memory::history_destination;
pub use status::{CommitStatus, connect_commit_status, poll as poll_commit_status};
pub use storage::{create_new_project_file, create_project_file, serialize_project_json};

use super::*;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use status::request_finish as request_finish_commit_status;
use status::{SaveStatus, begin as begin_commit_status, finish as finish_commit_status};
use status::{request_save as request_save_status_update, set_save as set_save_status};

const HISTORY_SAVE_DEBOUNCE: Duration = Duration::from_secs(2);
const HISTORY_SAVE_INTERVAL: Duration = Duration::from_secs(30);
const HISTORY_QUEUE_CAPACITY: usize = 1;
const SLOW_HISTORY_QUEUE_WAIT: Duration = Duration::from_millis(10);
static NEXT_HISTORY_STEP: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static HISTORY_WORKER: RefCell<Option<HistoryWorker>> = const { RefCell::new(None) };
}

struct HistoryWorker {
    sender: SyncSender<HistoryJob>,
    fingerprint: Arc<Mutex<Option<String>>>,
}

/// Exact bytes last read or durably written for the current document.
/// The receipt belongs to this history owner and is released with it.
pub fn saved_project_fingerprint() -> Option<String> {
    HISTORY_WORKER.with(|slot| {
        slot.borrow()
            .as_ref()?
            .fingerprint
            .lock()
            .expect("project receipt lock poisoned")
            .clone()
    })
}

pub struct PreparedProject {
    project: Option<Project>,
    fingerprint: String,
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
    let project = match outcome {
        ProjectValidationOutcome::Valid(project) => project,
        ProjectValidationOutcome::FrameGridRepair(project) => {
            release_project_lock(path);
            return Ok(ProjectPreparation::FrameGridRepair(project));
        }
    };
    tracing::info!(
        path = %path.display(),
        elapsed_ms = started.elapsed().as_millis(),
        "Prepared project"
    );
    Ok(ProjectPreparation::Ready(PreparedProject {
        project: Some(project),
        fingerprint,
        path: path.to_path_buf(),
    }))
}

pub fn activate_project(mut prepared: PreparedProject) -> Project {
    let project = prepared.project.take().expect("prepared project exists");
    set_active_project_path(&prepared.path);
    memory::seed(&project, 0);
    start_history_worker(
        project.clone(),
        prepared.path.clone(),
        Some(prepared.fingerprint.clone()),
    );
    tracing::info!(path = %prepared.path.display(), "Activated project");
    project
}

pub fn commit_edit(project: &Project, message: &str) -> bool {
    commit(project, message, None)
}

pub fn commit_edit_checked(project: &Project, message: &str) -> Result<(), String> {
    project
        .validate()
        .map_err(|error| format!("cannot commit invalid project: {error}"))?;
    if !queue_project(project, message) {
        return Err("project history worker rejected the edit".to_string());
    }
    memory::commit(project, None);
    Ok(())
}

/// Persist a prepared API edit before advancing the in-memory undo history.
pub fn commit_edit_saved(project: &Project, message: &str) -> Result<(), String> {
    project
        .validate()
        .map_err(|error| format!("cannot commit invalid project: {error}"))?;
    let sender = HISTORY_WORKER
        .with(|slot| slot.borrow().as_ref().map(|worker| worker.sender.clone()))
        .ok_or("project history is unavailable")?;
    let (response_tx, response_rx) = mpsc::channel();
    begin_commit_status(message);
    sender
        .send(HistoryJob::CommitSaved {
            project: Box::new(project.clone()),
            response_tx,
        })
        .map_err(|_| "project history worker stopped unexpectedly")?;
    let result = response_rx
        .recv()
        .map_err(|_| "project history worker dropped the commit response")?;
    finish_commit_status();
    result?;
    memory::commit(project, None);
    Ok(())
}

/// Adopt an already validated external save without writing over the file that
/// triggered it. Both autosave's baseline and undo history advance together.
pub fn adopt_project_snapshot(project: &Project, fingerprint: &str) -> Result<(), String> {
    project.validate()?;
    let sender = HISTORY_WORKER
        .with(|slot| slot.borrow().as_ref().map(|worker| worker.sender.clone()))
        .ok_or("project history is unavailable")?;
    let (response_tx, response_rx) = mpsc::channel();
    sender
        .send(HistoryJob::Adopt {
            project: Box::new(project.clone()),
            fingerprint: fingerprint.to_owned(),
            response_tx,
        })
        .map_err(|_| "project history worker stopped unexpectedly")?;
    response_rx
        .recv()
        .map_err(|_| "project history worker dropped the adopt response")?;
    memory::commit(project, None);
    Ok(())
}

/// Persist a history destination before moving its cursor or exposing the project.
pub fn change_history_saved(project: &mut Project, redo: bool) -> Result<bool, String> {
    let Some(candidate) = memory::history_destination(redo) else {
        return Ok(false);
    };
    let mut candidate = candidate;
    candidate.cursor_position = project.cursor_position;
    candidate.timeline_zoom = project.timeline_zoom;
    candidate.expanded_sequence_paths = project.expanded_sequence_paths.clone();
    let sender = HISTORY_WORKER
        .with(|slot| slot.borrow().as_ref().map(|worker| worker.sender.clone()))
        .ok_or("project history is unavailable")?;
    let (response_tx, response_rx) = mpsc::channel();
    sender
        .send(HistoryJob::CommitSaved {
            project: Box::new(candidate),
            response_tx,
        })
        .map_err(|_| "project history worker stopped unexpectedly")?;
    response_rx
        .recv()
        .map_err(|_| "project history worker dropped the commit response")??;
    if redo {
        memory::redo(project);
    } else {
        memory::undo(project);
    }
    Ok(true)
}

pub fn commit_coalesced_edit(project: &Project, group: &str) -> bool {
    commit(project, group, Some(group))
}

pub fn finish_coalesced_edit() {
    memory::finish_coalesced_edit();
}

pub fn can_undo() -> bool {
    memory::can_undo()
}

pub fn can_redo() -> bool {
    memory::can_redo()
}

pub fn undo(project: &mut Project) -> bool {
    if memory::undo(project).is_none() {
        return false;
    }
    queue_project(project, "undo");
    true
}

pub fn redo(project: &mut Project) -> bool {
    if memory::redo(project).is_none() {
        return false;
    }
    queue_project(project, "redo");
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
    let sender = HISTORY_WORKER
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
        HISTORY_WORKER.with(|slot| slot.borrow().as_ref().map(|worker| worker.sender.clone()))
    else {
        memory::clear();
        return Ok(());
    };
    let (response_tx, response_rx) = mpsc::channel();
    sender
        .send(HistoryJob::Shutdown { response_tx })
        .map_err(|_| "project history worker stopped unexpectedly".to_string())?;
    response_rx
        .recv()
        .map_err(|_| "project history worker dropped the shutdown response".to_string())??;
    HISTORY_WORKER.with(|slot| slot.borrow_mut().take());
    memory::clear();
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
        let sender = HISTORY_WORKER
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
        release_project_lock(&current_path);
        set_active_project_path(path);
    } else if result.is_err() && path_changed {
        release_project_lock(path);
    }
    result
}

pub fn save_view_state(project: &Project) {
    let sender = history_worker(project);
    let started = Instant::now();
    if sender
        .send(HistoryJob::ViewState {
            cursor_position: project.cursor_position,
            timeline_zoom: project.timeline_zoom,
            expanded_sequence_paths: project.expanded_sequence_paths.clone(),
        })
        .is_err()
    {
        tracing::warn!("Could not queue project view state");
        set_save_status(SaveStatus::Failed(
            "project history worker stopped unexpectedly".to_string(),
        ));
    }
    let elapsed = started.elapsed();
    if elapsed >= SLOW_HISTORY_QUEUE_WAIT {
        tracing::warn!(
            wait_us = elapsed.as_micros(),
            "View-state send blocked waiting for history worker"
        );
    }
}

fn commit(project: &Project, message: &str, coalesce_group: Option<&str>) -> bool {
    project
        .validate()
        .unwrap_or_else(|error| panic!("cannot commit invalid project: {error}"));
    memory::commit(project, coalesce_group);
    queue_project(project, message)
}

fn queue_project(project: &Project, action: &str) -> bool {
    let sender = history_worker(project);
    let step = NEXT_HISTORY_STEP
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    begin_commit_status(action);
    tracing::trace!(step, action, "Queueing project snapshot");
    if sender
        .send(HistoryJob::Update {
            step,
            project: Box::new(project.clone()),
        })
        .is_ok()
    {
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
        project: Box<Project>,
        fingerprint: String,
        response_tx: Sender<()>,
    },
    CommitSaved {
        project: Box<Project>,
        response_tx: Sender<Result<(), String>>,
    },
    Update {
        step: u64,
        project: Box<Project>,
    },
    ViewState {
        cursor_position: Option<Time>,
        timeline_zoom: Option<Time>,
        expanded_sequence_paths: Vec<Vec<Uuid>>,
    },
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

fn history_worker(project: &Project) -> SyncSender<HistoryJob> {
    HISTORY_WORKER.with(|slot| {
        if let Some(worker) = slot.borrow().as_ref() {
            return worker.sender.clone();
        }
        start_history_worker(project.clone(), active_project_path(), None)
    })
}

fn start_history_worker(
    mut project: Project,
    mut path: PathBuf,
    initial_fingerprint: Option<String>,
) -> SyncSender<HistoryJob> {
    let (sender, receiver) = mpsc::sync_channel::<HistoryJob>(HISTORY_QUEUE_CAPACITY);
    let fingerprint = Arc::new(Mutex::new(initial_fingerprint));
    let observed_fingerprint = fingerprint.clone();
    thread::spawn(move || {
        let mut pending_save = false;
        let mut pending_content = false;
        let mut last_update = Instant::now();
        let mut last_snapshot = Instant::now();
        loop {
            let job = if pending_save {
                let now = Instant::now();
                let idle_wait =
                    HISTORY_SAVE_DEBOUNCE.saturating_sub(now.duration_since(last_update));
                let periodic_wait =
                    HISTORY_SAVE_INTERVAL.saturating_sub(now.duration_since(last_snapshot));
                let wait = if pending_content {
                    idle_wait.min(periodic_wait)
                } else {
                    idle_wait
                };
                match receiver.recv_timeout(wait) {
                    Ok(job) => job,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let saving_content = pending_content;
                        let result = if saving_content {
                            save_project(&path, &project, &observed_fingerprint)
                        } else {
                            save_view_state_project(&path, &project, &observed_fingerprint)
                        };
                        pending_save = result.is_err();
                        if result.is_ok() {
                            pending_content = false;
                        }
                        if pending_save {
                            last_update = Instant::now();
                        }
                        if saving_content && last_snapshot.elapsed() >= HISTORY_SAVE_INTERVAL {
                            save_snapshot(&path, &project);
                            last_snapshot = Instant::now();
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        if pending_content {
                            let _ = save_project(&path, &project, &observed_fingerprint);
                        } else {
                            let _ = save_view_state_project(&path, &project, &observed_fingerprint);
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
                    response_tx,
                } => {
                    project = *updated;
                    *observed_fingerprint
                        .lock()
                        .expect("project receipt lock poisoned") = Some(loaded_fingerprint);
                    pending_save = false;
                    pending_content = false;
                    request_save_status_update(SaveStatus::Saved);
                    let _ = response_tx.send(());
                }
                HistoryJob::CommitSaved {
                    project: updated,
                    response_tx,
                } => {
                    let result = save_project(&path, &updated, &observed_fingerprint);
                    if result.is_ok() {
                        project = *updated;
                        pending_save = false;
                        pending_content = false;
                    }
                    let _ = response_tx.send(result);
                }
                HistoryJob::Update {
                    step,
                    project: updated,
                } => {
                    project = *updated;
                    pending_save = true;
                    pending_content = true;
                    last_update = Instant::now();
                    request_save_status_update(SaveStatus::Pending);
                    tracing::debug!(step, "Updated pending project snapshot");
                    request_finish_commit_status();
                }
                HistoryJob::ViewState {
                    cursor_position,
                    timeline_zoom,
                    expanded_sequence_paths,
                } => {
                    project.cursor_position = cursor_position;
                    project.timeline_zoom = timeline_zoom;
                    project.expanded_sequence_paths = expanded_sequence_paths;
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
                        save_view_state_project(&path, &project, &observed_fingerprint)
                    } else {
                        save_project(&path, &project, &observed_fingerprint)
                    };
                    pending_save = result.is_err();
                    if result.is_ok() {
                        pending_content = false;
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
                    let previous_path = std::mem::replace(&mut path, new_path);
                    let result = write_project(&path, &project, &observed_fingerprint);
                    if result.is_err() {
                        path = previous_path;
                    } else {
                        pending_save = false;
                        pending_content = false;
                    }
                    let _ = response_tx.send(result);
                }
                HistoryJob::Shutdown { response_tx } => {
                    let result = if !pending_save {
                        Ok(())
                    } else if pending_content {
                        save_project(&path, &project, &observed_fingerprint)
                    } else {
                        save_view_state_project(&path, &project, &observed_fingerprint)
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
    HISTORY_WORKER.with(|slot| {
        *slot.borrow_mut() = Some(HistoryWorker {
            sender: sender.clone(),
            fingerprint,
        })
    });
    sender
}

fn save_project(
    path: &Path,
    project: &Project,
    fingerprint: &Mutex<Option<String>>,
) -> Result<(), String> {
    let result = write_project(path, project, fingerprint);
    if let Err(error) = &result {
        tracing::warn!(path = %path.display(), "Could not save project: {error}");
    }
    result
}

fn save_view_state_project(
    path: &Path,
    project: &Project,
    fingerprint: &Mutex<Option<String>>,
) -> Result<(), String> {
    let started = Instant::now();
    let result = storage::write_project(path, project)
        .map(|saved| *fingerprint.lock().expect("project receipt lock poisoned") = Some(saved));
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
) -> Result<(), String> {
    request_save_status_update(SaveStatus::Saving);
    let started = Instant::now();
    tracing::info!(path = %path.display(), "Saving project");
    let result = storage::write_project(path, project)
        .map(|saved| *fingerprint.lock().expect("project receipt lock poisoned") = Some(saved));
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

fn is_project_path(path: &Path) -> bool {
    has_extension(path, "shrimp") || has_extension(path, "sjson") || has_extension(path, "json")
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
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
