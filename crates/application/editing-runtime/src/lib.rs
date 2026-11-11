use shrimply_local_ipc::{Listener as UnixListener, Stream as UnixStream};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
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
    ManimClipResponse, ManimParameterValue as ProtocolManimParameterValue, PlayerSnapshot,
    ReloadManimSourceRequest, ReloadManimSourceResponse, ScopeRef, SetManimClipRequest,
    TranscribeAudioRequest, TtsInputValue, ViewFrameResponse,
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

mod execute;
mod frames;
mod imports;
mod manim;
mod speech;

use frames::*;
use manim::*;
use speech::*;

const SOCKET_READ_LIMIT: u64 = 16 * 1024 * 1024;
const SOCKET_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const EDIT_EXECUTION_TIMEOUT: Duration = Duration::from_secs(29 * 60);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const WORK_QUEUE_CAPACITY: usize = 32;
const HEADLESS_POLL_INTERVAL: Duration = Duration::from_millis(5);
const CONNECTION_INDICATOR_HOLD: Duration = Duration::from_secs(3);
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

#[derive(Default)]
struct ConnectionActivity {
    active: usize,
    last_finished: Option<Instant>,
}

pub struct Server {
    socket_path: PathBuf,
    stop: Arc<AtomicBool>,
    context: glib::MainContext,
    task: glib::JoinHandle<()>,
    connections: Arc<Mutex<ConnectionActivity>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.task.abort();
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

pub fn start(
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    preferences: SharedPreferences,
) -> Result<Server, String> {
    start_session(project, player_state, selection_state, preferences, false)
}

fn start_session(
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    preferences: SharedPreferences,
    headless: bool,
) -> Result<Server, String> {
    let context = glib::MainContext::new();
    runtime_directory()?;
    let socket_path = shrimply_editing_api::bridge::socket_path(std::process::id())
        .map_err(|error| error.to_string())?;
    if socket_path.exists() {
        #[cfg(unix)]
        let metadata = socket_path.metadata().map_err(|error| {
            format!(
                "could not inspect editing API socket {}: {error}",
                socket_path.display()
            )
        })?;
        #[cfg(unix)]
        if metadata.uid() != effective_uid() {
            return Err(format!(
                "refusing editing API socket owned by another user: {}",
                socket_path.display()
            ));
        }
        return Err(format!(
            "editing API socket already exists: {}",
            socket_path.display()
        ));
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

    let (sender, receiver) = async_channel::bounded::<Work>(WORK_QUEUE_CAPACITY);
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let connections = Arc::new(Mutex::new(ConnectionActivity::default()));
    let worker_connections = connections.clone();
    drop(
        thread::Builder::new()
            .name("shrimply-editing-bridge".to_string())
            .spawn(move || accept_loop(listener, sender, worker_stop, worker_connections))
            .map_err(|error| format!("could not start editing API bridge: {error}"))?,
    );

    let executor_stop = stop.clone();
    let task = context.spawn_local(async move {
        while let Ok(work) = receiver.recv().await {
            let project_path = shrimply_project_document::project::normalized_project_path(
                &shrimply_project_document::project::active_project_path(),
            );
            let project_path_text = project_path
                .to_str()
                .ok_or_else(|| "active project path is not valid UTF-8".to_string());
            let annotate = matches!(
                &work.request.command,
                BridgeCommand::Execute(_) | BridgeCommand::Snapshot
            );
            let result = if work.canceled.load(Ordering::Acquire) {
                Err("editing API client canceled the request".to_string())
            } else {
                match project_path_text.as_deref() {
                    Ok(path) if path == work.request.project_path => match work.request.command {
                        BridgeCommand::Execute(call) => {
                            execute::call(
                                execute::Context {
                                    project: &project,
                                    player: &player_state,
                                    selection: &selection_state,
                                    preferences: &preferences,
                                    headless,
                                },
                                call,
                                work.canceled.clone(),
                            )
                            .await
                        }
                        BridgeCommand::Handshake => {
                            Ok(json!({"ready":true,"mode":if headless {"headless"} else {"live"}}))
                        }
                        BridgeCommand::Version => serde_json::to_value(
                            shrimply_editing_api::protocol::BuildVersion::current(env!(
                                "CARGO_PKG_VERSION"
                            )),
                        )
                        .map_err(|error| error.to_string()),
                        BridgeCommand::Snapshot => {
                            snapshot(&project, &player_state, &selection_state).and_then(
                                |snapshot| {
                                    serde_json::to_value(snapshot)
                                        .map_err(|error| error.to_string())
                                },
                            )
                        }
                    },
                    Ok(_) => Err(format!(
                        "editing API request named {}, but this editor owns {}",
                        work.request.project_path,
                        project_path.display()
                    )),
                    Err(error) => Err(error.clone()),
                }
            };
            let result = if annotate {
                result.and_then(|value| {
                    shrimply_editing_api::preview::with_state(
                        value,
                        &project.borrow(),
                        if headless { "headless" } else { "live" },
                    )
                })
            } else {
                result
            };
            let response = BridgeResponse {
                project_path: project_path_text.unwrap_or_default().to_string(),
                result: result.as_ref().ok().cloned(),
                error: result.err(),
            };
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
        task,
        connections,
    })
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
    /// Process requests and report active or recently completed client connections.
    pub fn poll(&self) -> bool {
        self.context
            .with_thread_default(|| {
                while self.context.pending() {
                    self.context.iteration(false);
                }
            })
            .expect("editing context must remain on its owning thread");
        let connections = self.connections.lock().unwrap();
        connections.active > 0
            || connections
                .last_finished
                .is_some_and(|time| time.elapsed() < CONNECTION_INDICATOR_HOLD)
    }
}

fn accept_loop(
    listener: UnixListener,
    sender: async_channel::Sender<Work>,
    stop: Arc<AtomicBool>,
    connections: Arc<Mutex<ConnectionActivity>>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok(_) if stop.load(Ordering::Acquire) => break,
            Ok((stream, _)) => {
                let sender = sender.clone();
                let stop = stop.clone();
                let connections = connections.clone();
                connections.lock().unwrap().active += 1;
                thread::spawn(move || {
                    serve_connection(stream, &sender, &stop);
                    let mut connections = connections.lock().unwrap();
                    connections.active -= 1;
                    connections.last_finished = Some(Instant::now());
                });
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
        prepared.project.cursor_position = current.cursor_position;
        prepared.project.timeline_zoom = current.timeline_zoom;
        prepared.project.expanded_sequence_paths = current.expanded_sequence_paths.clone();
    }
    let duration = prepared.project.duration();
    let frame_rate = prepared.project.fps;
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
    let editor_selection =
        EditorSelection::capture(selection, &live.borrow()).reconcile(&prepared.project);
    prepared.promote()?;
    if canceled.load(Ordering::Acquire) {
        prepared.discard_promoted();
        return Err("editing API client canceled the edit before commit".to_string());
    }
    if let Err(error) =
        shrimply_project_document::project::commit_edit_saved(&prepared.project, &history_label)
    {
        prepared.discard_promoted();
        return Err(format!(
            "editing API edit could not be committed to project history: {error}"
        ));
    }
    *live.borrow_mut() = prepared.project;
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
    serde_json::to_value(EditResponse {
        operations: results,
        duration: duration_frame,
        revision: player_state::snapshot(player).revision,
    })
    .map_err(|error| format!("could not serialize editing API edit result: {error}"))
}

/// Run a project owner without a display server or audio playback device.
pub fn serve(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    shrimply_visual_cuda::gpu::configure_gtk_backend()?;
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
            true,
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

/// Execute queries requiring services or source discovery without owning the project.
pub fn query_saved(
    path: &std::path::Path,
    call: shrimply_editing_api::protocol::ApiCall,
) -> Result<Value, String> {
    if !shrimply_editing_api::commands::get(&call.name)?.read_only {
        return Err("saved-state queries require a read-only operation".into());
    }
    let project = shrimply_project_document::project::load_project_snapshot(path)?;
    shrimply_project_document::project::set_active_project_path(path);
    let player = player_state::new(project.duration(), project.fps);
    if let Some(position) = project.cursor_position {
        player_state::seek_time(&player, position);
    }
    let project = Rc::new(RefCell::new(project));
    let selection = selection_state::new();
    let preferences = preferences::open_with_defaults();
    let context = glib::MainContext::new();
    let result = context
        .with_thread_default(|| {
            context.block_on(execute::call(
                execute::Context {
                    project: &project,
                    player: &player,
                    selection: &selection,
                    preferences: &preferences,
                    headless: false,
                },
                call,
                Arc::new(AtomicBool::new(false)),
            ))
        })
        .map_err(|error| error.to_string())??;
    shrimply_editing_api::preview::with_state(result, &project.borrow(), "saved")
}
