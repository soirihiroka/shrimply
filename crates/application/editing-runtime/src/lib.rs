use shrimply_local_ipc::{Listener as UnixListener, Stream as UnixStream};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use shrimply_asset::Asset;
use shrimply_editing_api::preview::revision as content_revision;
use shrimply_editing_api::protocol::{
    ActiveScopeSnapshot, AnalyzeTransparentFillRequest, AnalyzeTransparentFillResponse,
    BridgeCommand, BridgeRequest, BridgeResponse, CaptionCueInput, CollisionBehavior,
    EditOperationResult, EditRequest, EditResponse, GenerateTtsRequest, GetManimClipRequest,
    InsertCaptionsRequest, ListSttModelsResponse, ListTtsModelsResponse, LiveSnapshot,
    ManimClipResponse, ManimParameterValue as ProtocolManimParameterValue, MediaRewrites,
    PlayerSnapshot, ReloadManimSourceRequest, ReloadManimSourceResponse, ScopeRef, SessionState,
    SessionVersion, SetManimClipRequest, TranscribeAudioRequest, TtsInputValue, ViewFrameResponse,
};
use shrimply_editor_state::{
    player_state::{self, ProjectChange, SharedPlayerState},
    preferences::{self, SharedPreferences},
};
use shrimply_math_core::{Time, fraction_new, time_from_frame};
#[cfg(target_os = "macos")]
use shrimply_preview_render_metal::transparent_fill_analysis;
use shrimply_project_document::project::{
    AudioSource, ItemAddress, ManimParameter, ManimParameterControl, ManimParameterValue, Project,
    SequenceScopeId, TrackRef, VideoItemContent, caption_languages, fraction_denominator,
    fraction_numerator,
};
use shrimply_timeline_edit::selection_state::{self, SharedSelectionState};
use shrimply_visual_core::transparent_fill::analysis::{
    self as transparent_fill_jobs, Status as TransparentFillStatus,
};
#[cfg(not(target_os = "macos"))]
use shrimply_visual_cuda::transparent_fill_analysis;
use shrimply_visual_modifiers::{ModifierEffect, RasterModifierEffect};
use uuid::Uuid;

pub mod attached;
mod execute;
mod frames;
mod peers;
mod remote;
use shrimply_editing_api::imports;
mod manim;
mod sanim;
mod speech;

use frames::*;
use manim::*;
use speech::*;

const SOCKET_READ_LIMIT: u64 = 16 * 1024 * 1024;
const SOCKET_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const EDIT_EXECUTION_TIMEOUT: Duration = Duration::from_secs(29 * 60);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const WORK_QUEUE_CAPACITY: usize = 32;
const ATTACHED_RECEIPT_LIMIT: usize = 1024;
const HEADLESS_POLL_INTERVAL: Duration = Duration::from_millis(5);
const MANIM_ANTI_ALIASING_SAMPLES: [i64; 5] = [0, 2, 4, 8, 16];

struct Work {
    request: BridgeRequest,
    response: mpsc::Sender<BridgeResponse>,
    canceled: Arc<AtomicBool>,
}

struct EditorSelection {
    items: Vec<shrimply_project_document::project::ItemAddress>,
    focused_item: Option<shrimply_project_document::project::ItemAddress>,
    tracks: Vec<shrimply_project_document::project::TrackAddress>,
    focused_track: Option<shrimply_project_document::project::TrackAddress>,
    gap: Option<selection_state::TrackAddressGap>,
    active_scope: shrimply_project_document::project::SequenceScopeId,
}

impl EditorSelection {
    fn capture(selection: &SharedSelectionState, project: &Project) -> Self {
        Self {
            items: selection_state::selected_item_addresses(selection, project),
            focused_item: selection_state::focused_item_address(selection, project),
            tracks: selection_state::selected_track_addresses(selection, project),
            focused_track: selection_state::focused_track_address(selection, project),
            gap: selection_state::selected_gap_address(selection, project),
            active_scope: selection_state::active_scope(selection),
        }
    }

    fn reconcile(mut self, project: &Project) -> Self {
        self.items.retain(|address| project.item(address).is_some());
        self.focused_item = self
            .focused_item
            .filter(|address| self.items.contains(address));
        self.tracks
            .retain(|address| project.track(address).is_some());
        self.focused_track = self
            .focused_track
            .filter(|address| self.tracks.contains(address));
        self.gap = self.gap.filter(|gap| project.track(&gap.track).is_some());
        if project.sequence_id_for_scope(&self.active_scope).is_none() {
            self.active_scope = shrimply_project_document::project::SequenceScopeId::root();
        }
        self
    }

    fn restore(self, selection: &SharedSelectionState, project: &Project) {
        let items = self.items;
        let focused_item = self.focused_item;
        let tracks = self.tracks;
        let focused_track = self.focused_track;
        if !items.is_empty() {
            selection_state::set_selected_item_addresses(selection, project, items, focused_item);
        } else if !tracks.is_empty() {
            selection_state::set_selected_track_addresses(
                selection,
                project,
                tracks,
                focused_track,
            );
        } else if self.gap.is_some() {
            selection_state::set_selected_gap_address(selection, project, self.gap);
        } else {
            selection_state::set_selected_items(selection, Vec::new(), None);
            selection_state::set_active_scope(selection, self.active_scope);
        }
    }
}

struct Server {
    socket_path: PathBuf,
    stop: Arc<AtomicBool>,
    context: glib::MainContext,
    tasks: [glib::JoinHandle<()>; 2],
    remote: Option<Rc<RefCell<remote::Session>>>,
    peers: Rc<RefCell<peers::Peers>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for task in &self.tasks {
            task.abort();
        }
        let _ = UnixStream::connect(&self.socket_path);
        let _ = fs::remove_file(&self.socket_path);
    }
}

struct BoundSocket(PathBuf);

impl BoundSocket {
    fn keep(mut self) -> PathBuf {
        std::mem::take(&mut self.0)
    }
}

impl Drop for BoundSocket {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.0);
        }
    }
}

fn start_session(
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    preferences: SharedPreferences,
) -> Result<Server, String> {
    let context = glib::MainContext::new();
    let remote = remote::Session::open(
        &shrimply_project_document::project::active_project_path(),
        project.clone(),
        player_state.clone(),
        selection_state.clone(),
    )?;
    runtime_directory()?;
    let owner = shrimply_project_document::project::current_project_lock_owner()?;
    let socket_path = shrimply_editing_api::bridge::socket_path(&owner)?;
    let existing = match fs::symlink_metadata(&socket_path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "could not inspect editing API socket {}: {error}",
                socket_path.display()
            ));
        }
    };
    if let Some(_metadata) = existing {
        #[cfg(unix)]
        if _metadata.uid() != effective_uid() || !_metadata.file_type().is_socket() {
            return Err(format!(
                "refusing to replace an editing API path that is not a socket owned by this user: {}",
                socket_path.display()
            ));
        }
        match UnixStream::connect(&socket_path) {
            Ok(_) => {
                return Err(format!(
                    "editing API socket is already active: {}",
                    socket_path.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                fs::remove_file(&socket_path).map_err(|error| {
                    format!(
                        "could not remove stale editing API socket {}: {error}",
                        socket_path.display()
                    )
                })?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "could not check editing API socket {}: {error}",
                    socket_path.display()
                ));
            }
        }
    }
    let listener = UnixListener::bind(&socket_path).map_err(|error| {
        format!(
            "could not bind editing API socket {}: {error}",
            socket_path.display()
        )
    })?;
    let socket = BoundSocket(socket_path);
    #[cfg(unix)]
    fs::set_permissions(&socket.0, fs::Permissions::from_mode(0o600)).map_err(|error| {
        format!(
            "could not secure editing API socket {}: {error}",
            socket.0.display()
        )
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("could not configure editing API socket: {error}"))?;

    let project_path = shrimply_project_document::project::normalized_project_path(
        &shrimply_project_document::project::active_project_path(),
    );
    let project_path = project_path
        .to_str()
        .ok_or("active project path is not valid UTF-8")?
        .to_owned();
    let (presence_sender, presence_receiver) = async_channel::bounded::<Work>(WORK_QUEUE_CAPACITY);
    let (sender, receiver) = async_channel::bounded::<Work>(WORK_QUEUE_CAPACITY);
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    drop(
        thread::Builder::new()
            .name("shrimply-editing-bridge".to_string())
            .spawn(move || accept_loop(listener, sender, presence_sender, worker_stop))
            .map_err(|error| format!("could not start editing API bridge: {error}"))?,
    );

    let executor_stop = stop.clone();
    let remote_actor = remote.clone();
    let peers = Rc::new(RefCell::new(peers::Peers::default()));
    let actor_peers = peers.clone();
    let presence_project = project.clone();
    let presence_remote = remote.clone();
    let presence_path = project_path.clone();
    let presence_task = context.spawn_local(async move {
        let epoch = Uuid::new_v4();
        while let Ok(work) = presence_receiver.recv().await {
            let BridgeCommand::SessionUpdates(request) = work.request.command else {
                unreachable!("the presence queue only accepts session updates");
            };
            let result = if work.request.project_path != presence_path {
                Err(format!("editing service is open for {presence_path}"))
            } else if work.canceled.load(Ordering::Acquire) {
                Err("editing API client canceled the request".into())
            } else {
                let caller = request.presence.id;
                let mut peers = actor_peers.borrow_mut();
                peers.update(request.presence);
                let mut participants = peers.active();
                let remote = presence_remote.as_ref().map(|remote| remote.borrow());
                if let Some(remote) = &remote {
                    participants.extend_from_slice(remote.participants());
                }
                participants.retain(|participant| participant.id != caller);
                participants.sort_by_key(|participant| participant.id);
                participants.dedup_by_key(|participant| participant.id);
                let version = SessionVersion {
                    epoch,
                    revision: shrimply_project_document::project::history_revision(),
                };
                serde_json::to_value(SessionState {
                    version,
                    project: (request.version != Some(version))
                        .then(|| presence_project.borrow().clone()),
                    participants,
                    sync_error: remote.and_then(|remote| remote.sync_error()),
                })
                .map_err(|error| error.to_string())
            };
            let _ = work.response.send(bridge_response(&presence_path, result));
        }
    });
    let task = context.spawn_local(async move {
        let mut receipts = HashMap::<_, Value>::new();
        let mut receipt_order = VecDeque::new();
        while let Ok(work) = receiver.recv().await {
            let annotate = matches!(
                &work.request.command,
                BridgeCommand::Execute(_) | BridgeCommand::Snapshot
            );
            let result = if work.canceled.load(Ordering::Acquire) {
                Err("editing API client canceled the request".to_string())
            } else if project_path == work.request.project_path {
                match work.request.command {
                    BridgeCommand::Execute(call) => {
                        execute::call(
                            execute::Context {
                                project: &project,
                                player: &player_state,
                                selection: &selection_state,
                                preferences: &preferences,
                                remote: remote_actor.as_ref(),
                            },
                            call,
                            work.canceled.clone(),
                        )
                        .await
                    }
                    BridgeCommand::ApplyChanges(request) => {
                        let id = request.id;
                        if let Some(receipt) = receipts.get(&id) {
                            Ok(receipt.clone())
                        } else {
                            let result = apply_attached_changes(
                                &project,
                                &player_state,
                                &selection_state,
                                remote_actor.as_ref(),
                                request,
                                &work.canceled,
                            )
                            .await;
                            if let Ok(receipt) = &result {
                                receipts.insert(id, receipt.clone());
                                receipt_order.push_back(id);
                                if receipt_order.len() > ATTACHED_RECEIPT_LIMIT
                                    && let Some(expired) = receipt_order.pop_front()
                                {
                                    receipts.remove(&expired);
                                }
                            }
                            result
                        }
                    }
                    BridgeCommand::SessionUpdates(_) => {
                        unreachable!("presence requests have their own queue")
                    }
                    BridgeCommand::Handshake => Ok(json!({"ready":true,"mode":"headless"})),
                    BridgeCommand::Version => {
                        serde_json::to_value(shrimply_editing_api::protocol::BuildVersion::current(
                            env!("CARGO_PKG_VERSION"),
                        ))
                        .map_err(|error| error.to_string())
                    }
                    BridgeCommand::Snapshot => snapshot(&project, &player_state, &selection_state)
                        .and_then(|snapshot| {
                            serde_json::to_value(snapshot).map_err(|error| error.to_string())
                        }),
                }
            } else {
                Err(format!("editing service is open for {project_path}"))
            };
            let result = if annotate {
                result.and_then(|value| {
                    shrimply_editing_api::preview::with_state(value, &project.borrow(), "headless")
                })
            } else {
                result
            };
            let mut result = result;
            if annotate
                && let Some(remote) = &remote_actor
                && let Ok(value) = &mut result
            {
                remote.borrow().annotate(value);
            }
            let response = bridge_response(&project_path, result);
            let closing = response
                .result
                .as_ref()
                .and_then(|value| value.get("closing"))
                .and_then(Value::as_bool)
                == Some(true);
            if work.response.send(response).is_err() && closing {
                executor_stop.store(true, Ordering::Release);
            }
            if closing {
                // Reject queued work once close succeeds; the transport finishes the reply.
                break;
            }
        }
    });

    let socket_path = socket.keep();
    Ok(Server {
        socket_path,
        stop,
        context,
        tasks: [task, presence_task],
        remote,
        peers,
    })
}

fn bridge_response(path: &str, result: Result<Value, String>) -> BridgeResponse {
    let (result, error) = match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    };
    BridgeResponse {
        project_path: path.to_owned(),
        result,
        error,
    }
}

fn runtime_directory() -> Result<PathBuf, String> {
    let directory = shrimply_editing_api::bridge::runtime_directory()?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        if directory
            .metadata()
            .map_err(|error| error.to_string())?
            .uid()
            != effective_uid()
        {
            return Err("editing runtime directory is owned by another user".into());
        }
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(directory)
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    // SAFETY: geteuid has no arguments or memory safety preconditions.
    unsafe { libc::geteuid() }
}

impl Server {
    fn poll(&self) {
        if let Some(remote) = &self.remote {
            remote.borrow_mut().poll(self.peers.borrow_mut().active());
        }
        self.context
            .with_thread_default(|| {
                while self.context.pending() {
                    self.context.iteration(false);
                }
            })
            .expect("editing context must remain on its owning thread");
    }
}

fn accept_loop(
    listener: UnixListener,
    sender: async_channel::Sender<Work>,
    presence_sender: async_channel::Sender<Work>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok(_) if stop.load(Ordering::Acquire) => break,
            Ok((stream, _)) => {
                let sender = sender.clone();
                let presence_sender = presence_sender.clone();
                let stop = stop.clone();
                thread::spawn(move || serve_connection(stream, &sender, &presence_sender, &stop));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("editing API bridge accept failed: {error}"),
        }
    }
}

fn serve_connection(
    mut stream: UnixStream,
    sender: &async_channel::Sender<Work>,
    presence_sender: &async_channel::Sender<Work>,
    stop: &AtomicBool,
) {
    stream
        .set_read_timeout(Some(SOCKET_REQUEST_TIMEOUT))
        .expect("editing API request timeout must be configurable");
    stream
        .set_write_timeout(Some(SOCKET_REQUEST_TIMEOUT))
        .expect("editing API response timeout must be configurable");
    let response = (|| {
        let mut line = String::new();
        BufReader::new(&mut stream)
            .take(SOCKET_READ_LIMIT)
            .read_line(&mut line)
            .map_err(|error| format!("could not read editing API bridge request: {error}"))?;
        let request: BridgeRequest = serde_json::from_str(&line)
            .map_err(|error| format!("malformed editing API bridge request: {error}"))?;
        let sender = if matches!(request.command, BridgeCommand::SessionUpdates(_)) {
            presence_sender
        } else {
            sender
        };
        let (response, receiver) = mpsc::channel();
        let canceled = Arc::new(AtomicBool::new(false));
        sender
            .send_blocking(Work {
                request,
                response,
                canceled: canceled.clone(),
            })
            .map_err(|_| "editing API executor stopped; the session may be closing".to_string())?;
        stream
            .set_nonblocking(true)
            .map_err(|error| format!("could not monitor editing API client: {error}"))?;
        let deadline = Instant::now() + EDIT_EXECUTION_TIMEOUT;
        loop {
            if stop.load(Ordering::Acquire) {
                canceled.store(true, Ordering::Release);
                break Err("editor editing API bridge is shutting down".to_string());
            }
            match receiver.recv_timeout(CANCELLATION_POLL_INTERVAL) {
                Ok(response) => break Ok(response),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    canceled.store(true, Ordering::Release);
                    break Err("editor editing API executor dropped the response".to_string());
                }
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                    canceled.store(true, Ordering::Release);
                    break Err("editor editing API response timed out".to_string());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let mut byte = [0];
                    match stream.read(&mut byte) {
                        Ok(0) => {
                            canceled.store(true, Ordering::Release);
                            break Err("editing API client disconnected".to_string());
                        }
                        Ok(_) => {
                            canceled.store(true, Ordering::Release);
                            break Err("editing API client sent more than one request".to_string());
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(error) => {
                            canceled.store(true, Ordering::Release);
                            break Err(format!("could not monitor editing API client: {error}"));
                        }
                    }
                }
            }
        }
    })()
    .unwrap_or_else(|error| BridgeResponse {
        project_path: String::new(),
        result: None,
        error: Some(error),
    });
    let _ = stream.set_nonblocking(false);
    if serde_json::to_writer(&mut stream, &response).is_ok() {
        let _ = stream.write_all(b"\n");
    }
    if response
        .result
        .as_ref()
        .and_then(|value| value.get("closing"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        stop.store(true, Ordering::Release);
    }
}

pub fn snapshot(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    selection: &SharedSelectionState,
) -> Result<LiveSnapshot, String> {
    let project = project.borrow();
    let player_snapshot = player_state::snapshot(player);
    let active_scope = selection_state::active_scope(selection);
    let asset_revisions = project
        .assets()
        .into_iter()
        .filter_map(|asset| {
            asset.snapshot().ok().map(|snapshot| {
                (
                    asset.path().to_string_lossy().into_owned(),
                    snapshot.revision(),
                )
            })
        })
        .collect();
    Ok(LiveSnapshot {
        project_path: shrimply_project_document::project::normalized_project_path(
            &shrimply_project_document::project::active_project_path(),
        )
        .to_str()
        .ok_or_else(|| "active project path is not valid UTF-8".to_string())?
        .to_string(),
        project: project.clone(),
        player: PlayerSnapshot {
            position: player_snapshot.position,
            duration: player_snapshot.duration,
            playing: player_snapshot.playing,
            revision: player_snapshot.revision,
        },
        active_scope: ActiveScopeSnapshot {
            instance_path: active_scope
                .instance_ids()
                .iter()
                .map(Uuid::to_string)
                .collect(),
            video_paths: project
                .sequence_paths_for_scope(
                    shrimply_project_document::project::ItemKind::Video,
                    &active_scope,
                )
                .into_iter()
                .map(|path| ScopeRef {
                    sequence_path: path.iter().map(Uuid::to_string).collect(),
                })
                .collect(),
            audio_paths: project
                .sequence_paths_for_scope(
                    shrimply_project_document::project::ItemKind::Audio,
                    &active_scope,
                )
                .into_iter()
                .map(|path| ScopeRef {
                    sequence_path: path.iter().map(Uuid::to_string).collect(),
                })
                .collect(),
        },
        focused_item: selection_state::focused_item_address(selection, &project)
            .as_ref()
            .map(shrimply_editing_api::query::protocol_item_address),
        selected_items: selection_state::selected_item_addresses(selection, &project)
            .iter()
            .map(shrimply_editing_api::query::protocol_item_address)
            .collect(),
        focused_track: selection_state::focused_track_address(selection, &project)
            .as_ref()
            .map(shrimply_editing_api::query::protocol_track_address),
        selected_tracks: selection_state::selected_track_addresses(selection, &project)
            .iter()
            .map(shrimply_editing_api::query::protocol_track_address)
            .collect(),
        asset_revisions,
    })
}

async fn apply_edit(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    selection: &SharedSelectionState,
    default_visual_duration: Time,
    remote: Option<&Rc<RefCell<remote::Session>>>,
    request: EditRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    if canceled.load(Ordering::Acquire) {
        return Err("editing API client canceled the edit".to_string());
    }
    let project_path = shrimply_project_document::project::normalized_project_path(
        &shrimply_project_document::project::active_project_path(),
    );
    let project = live.borrow().clone();
    let original = content_revision(&project)?;
    let playhead_frame = player_state::current_time(player).as_frame(project.fps);
    let active_scope = selection_state::active_scope(selection);
    let history_label = request.history_label.clone();
    let worker_path = project_path.clone();
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-editing-edit".to_string())
        .spawn(move || {
            let result = imports::prepare(
                project,
                &request,
                playhead_frame,
                active_scope,
                default_visual_duration,
                &worker_path,
            );
            let _ = sender.send_blocking(result);
        })
        .map_err(|error| format!("could not start editing API edit worker: {error}"))?;

    let mut prepared = loop {
        if canceled.load(Ordering::Acquire) {
            return Err("editing API client canceled the edit".to_string());
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("editing API edit worker stopped without a result".to_string());
            }
        }
    };
    let current_path = shrimply_project_document::project::normalized_project_path(
        &shrimply_project_document::project::active_project_path(),
    );
    if current_path != project_path {
        return Err(
            "project path changed while the editing API edit was being prepared".to_string(),
        );
    }
    {
        let current = live.borrow();
        if content_revision(&current)? != original {
            return Err(
                "project changed while the editing API edit was being prepared; retry the edit"
                    .to_string(),
            );
        }
        prepared.project.copy_view_state_from(&current);
    }
    let duration = prepared.project.duration();
    let duration_frame =
        shrimply_editing_api::query::frame_time_from_time(duration, prepared.project.fps, true);
    let results = prepared.results()?;
    prepared.ensure_linked_sources_current()?;
    if content_revision(&prepared.project)? == original {
        return serde_json::to_value(EditResponse {
            operations: results,
            duration: duration_frame,
            revision: player_state::snapshot(player).revision,
        })
        .map_err(|error| error.to_string());
    }
    if canceled.load(Ordering::Acquire) {
        return Err("editing API client canceled the edit before commit".to_string());
    }
    prepared.promote()?;
    if canceled.load(Ordering::Acquire) {
        prepared.discard_promoted();
        return Err("editing API client canceled the edit before commit".to_string());
    }
    let result = commit_candidate(
        live,
        remote,
        prepared.project.clone(),
        &original,
        &history_label,
        None,
        &canceled,
    )
    .await;
    match result {
        Ok((candidate, _)) => prepared.project = candidate,
        Err(error) => {
            prepared.discard_promoted();
            return Err(error);
        }
    }
    replace_live_project(live, player, selection, prepared.project);
    serde_json::to_value(EditResponse {
        operations: results,
        duration: duration_frame,
        revision: player_state::snapshot(player).revision,
    })
    .map_err(|error| format!("could not serialize editing API edit result: {error}"))
}

async fn apply_attached_changes(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    selection: &SharedSelectionState,
    remote: Option<&Rc<RefCell<remote::Session>>>,
    request: shrimply_editing_api::protocol::ApplyChanges,
    canceled: &AtomicBool,
) -> Result<Value, String> {
    let before = live.borrow().clone();
    if let Some(remote) = remote
        && let Some(rewrites) = remote.borrow().attached_receipt(request.id)?
    {
        return serde_json::to_value(rewrites).map_err(|error| error.to_string());
    }
    let candidate = if request.replay {
        request.changes.confirm(&before)
    } else {
        request.changes.apply(&before)
    }
    .map_err(|error| format!("revision conflict: {error}"))?;
    if canceled.load(Ordering::Acquire) {
        return Err("project edit was canceled".into());
    }
    let original = content_revision(&before)?;
    if original == content_revision(&candidate)? {
        return Ok(json!({}));
    }
    let (candidate, rewrites) = commit_candidate(
        live,
        remote,
        candidate,
        &original,
        &request.label,
        Some(request.id),
        canceled,
    )
    .await?;
    replace_live_project(live, player, selection, candidate);
    serde_json::to_value(rewrites).map_err(|error| error.to_string())
}

async fn commit_candidate(
    live: &Rc<RefCell<Project>>,
    remote: Option<&Rc<RefCell<remote::Session>>>,
    mut candidate: Project,
    original: &str,
    label: &str,
    operation_id: Option<Uuid>,
    canceled: &AtomicBool,
) -> Result<(Project, MediaRewrites), String> {
    let (media, rewrites) = if let Some(remote) = remote
        && candidate
            .assets()
            .iter()
            .any(|asset| shrimply_project_service::blob_digest(asset.path()).is_none())
    {
        let upload_directory = remote.borrow().upload_directory();
        let (sender, receiver) = async_channel::bounded(1);
        thread::spawn(move || {
            let result =
                shrimply_project_service::prepare_uploads(&upload_directory, &mut candidate)
                    .map(|media| (candidate, media));
            let _ = sender.send_blocking(result);
        });
        let (next, media) = receiver.recv().await.map_err(|error| error.to_string())??;
        candidate = next;
        media
    } else {
        (Vec::new(), MediaRewrites::new())
    };
    if canceled.load(Ordering::Acquire) {
        return Err("project edit was canceled before commit".into());
    }
    if content_revision(&live.borrow())? != original {
        return Err("revision conflict: project changed while preparing the edit".into());
    }

    if let Some(remote) = remote {
        let remote = remote.borrow();
        remote.queue(
            &live.borrow(),
            &candidate,
            &media,
            operation_id.map(|id| (id, &rewrites)),
        )?;
        if let Err(error) = remote.bind_media(&mut candidate) {
            tracing::warn!(%error, "Shared edit is queued, but media could not be opened");
        }
        if let Err(error) =
            shrimply_project_document::project::commit_edit_checked(&candidate, label)
        {
            tracing::warn!(%error, "Shared edit remains in the local recovery queue");
        }
    } else if operation_id.is_some() {
        shrimply_project_document::project::commit_edit_checked(&candidate, label)?;
    } else {
        shrimply_project_document::project::commit_edit_saved(&candidate, label)?;
    }
    Ok((candidate, rewrites))
}

fn replace_live_project(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    selection: &SharedSelectionState,
    project: Project,
) {
    let editor_selection = EditorSelection::capture(selection, &live.borrow()).reconcile(&project);
    let duration = project.duration();
    let frame_rate = project.fps;
    *live.borrow_mut() = project;
    editor_selection.restore(selection, &live.borrow());
    player_state::refresh_project(
        player,
        ProjectChange {
            duration: Some(duration),
            frame_rate: Some(frame_rate),
            audio: true,
            audio_beats: true,
            audio_waveforms: true,
            video: true,
            live_preview: true,
            captions: true,
            inspector: true,
        },
    );
}

/// Run a project owner without a display server or audio playback device.
pub fn serve(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if let Err(error) = shrimply_visual_cuda::gpu::configure_gtk_backend() {
        tracing::warn!(%error, "The editing service could not configure its render backend");
    }
    ffmpeg_next::init().map_err(|error| error.to_string())?;
    let prepared = shrimply_project_document::project::prepare_project(path)
        .map_err(|error| format!("could not acquire project: {error:?}"))?;
    let project = shrimply_project_document::project::activate_project(prepared);
    let result = (|| {
        let preferences = preferences::open_with_defaults();
        let preference = preferences::snapshot(&preferences);
        shrimply_blender_core::set_binary(preference.blender_binary);
        shrimply_audio_engine::pneuma::set_server_url(&preference.compute_server_url);
        let player = player_state::new(project.duration(), project.fps);
        if let Some(position) = project.cursor_position {
            player_state::seek_time(&player, position);
        }
        let server = start_session(
            Rc::new(RefCell::new(project)),
            player,
            selection_state::new(),
            preferences,
        )?;
        while !server.stop.load(Ordering::Acquire) {
            server.poll();
            shrimply_project_document::project::poll_commit_status();
            thread::sleep(HEADLESS_POLL_INTERVAL);
        }
        Ok(())
    })();
    let saved = shrimply_project_document::project::shutdown_history();
    shrimply_project_document::project::clear_project_file_locks();
    result.and(saved)
}
