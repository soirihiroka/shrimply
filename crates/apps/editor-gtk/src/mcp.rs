use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use gtk::{gdk, glib, prelude::*};
use serde_json::{Value, json};
use shrimply_asset::Asset;
use shrimply_math_core::{Time, fraction_new, time_from_frame};
use shrimply_mcp::protocol::{
    ActiveScopeSnapshot, AnalyzeTransparentFillRequest, AnalyzeTransparentFillResponse,
    BridgeCommand, BridgeRequest, BridgeResponse, CaptionCueInput, CollisionBehavior,
    EditOperationResult, EditRequest, EditResponse, GenerateTtsRequest, GetManimClipRequest,
    InsertCaptionsRequest, ListSttModelsResponse, ListTtsModelsResponse, LiveSnapshot,
    ManimClipResponse, ManimParameterValue as ProtocolManimParameterValue, PlayerSnapshot,
    ReloadManimSourceRequest, ReloadManimSourceResponse, ScopeRef, SetManimClipRequest,
    TranscribeAudioRequest, TtsInputValue, ViewFrameResponse,
};
use shrimply_preview_gtk::video::compositor::{EXPORT_ASSETS_LOADING, VideoExportRenderer};
use shrimply_project::project::{
    AudioSource, ItemAddress, ManimParameter, ManimParameterControl, ManimParameterValue, Project,
    SequenceScopeId, TrackRef, VideoItemContent, caption_languages, fraction_denominator,
    fraction_numerator,
};
use shrimply_state::{
    player_state::{self, ProjectChange, SharedPlayerState},
    preferences::{self, SharedPreferences},
};
use shrimply_timeline::selection_state::{self, SharedSelectionState};
use shrimply_video_cuda::transparent_fill_analysis::Status as TransparentFillStatus;
use shrimply_video_modifiers::{ModifierEffect, RasterModifierEffect};
use uuid::Uuid;

mod imports;

const SOCKET_READ_LIMIT: u64 = 16 * 1024 * 1024;
const SOCKET_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const EDIT_EXECUTION_TIMEOUT: Duration = Duration::from_secs(29 * 60);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const WORK_QUEUE_CAPACITY: usize = 32;
const FRAME_RENDER_AUDIO_SAMPLE_RATE: u32 = 48_000;
const MANIM_ANTI_ALIASING_SAMPLES: [i64; 5] = [0, 2, 4, 8, 16];

struct Work {
    request: BridgeRequest,
    response: mpsc::Sender<BridgeResponse>,
    canceled: Arc<AtomicBool>,
}

struct EditorSelection {
    items: Vec<shrimply_project::project::ItemAddress>,
    focused_item: Option<shrimply_project::project::ItemAddress>,
    tracks: Vec<shrimply_project::project::TrackAddress>,
    focused_track: Option<shrimply_project::project::TrackAddress>,
    gap: Option<selection_state::TrackAddressGap>,
    active_scope: shrimply_project::project::SequenceScopeId,
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
            self.active_scope = shrimply_project::project::SequenceScopeId::root();
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

pub struct Server {
    socket_path: PathBuf,
    stop: Arc<AtomicBool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
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
    runtime_directory()?;
    let socket_path =
        shrimply_mcp::bridge::socket_path(std::process::id()).map_err(|error| error.to_string())?;
    if socket_path.exists() {
        let metadata = socket_path.metadata().map_err(|error| {
            format!(
                "could not inspect MCP socket {}: {error}",
                socket_path.display()
            )
        })?;
        if metadata.uid() != effective_uid() {
            return Err(format!(
                "refusing MCP socket owned by another user: {}",
                socket_path.display()
            ));
        }
        return Err(format!(
            "MCP socket already exists: {}",
            socket_path.display()
        ));
    }
    let listener = UnixListener::bind(&socket_path).map_err(|error| {
        format!(
            "could not bind MCP socket {}: {error}",
            socket_path.display()
        )
    })?;
    let socket = BoundSocket(socket_path);
    fs::set_permissions(&socket.0, fs::Permissions::from_mode(0o600)).map_err(|error| {
        format!(
            "could not secure MCP socket {}: {error}",
            socket.0.display()
        )
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("could not configure MCP socket: {error}"))?;

    let (sender, receiver) = async_channel::bounded::<Work>(WORK_QUEUE_CAPACITY);
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    drop(
        thread::Builder::new()
            .name("shrimply-mcp-bridge".to_string())
            .spawn(move || accept_loop(listener, sender, worker_stop))
            .map_err(|error| format!("could not start MCP bridge: {error}"))?,
    );

    glib::spawn_future_local(async move {
        while let Ok(work) = receiver.recv().await {
            let project_path = shrimply_project::project::normalized_project_path(
                &shrimply_project::project::active_project_path(),
            );
            let project_path_text = project_path
                .to_str()
                .ok_or_else(|| "active project path is not valid UTF-8".to_string());
            let result = if work.canceled.load(Ordering::Acquire) {
                Err("MCP client canceled the request".to_string())
            } else {
                match project_path_text.as_deref() {
                    Ok(path) if path == work.request.project_path => match work.request.command {
                        BridgeCommand::Apply(request) => {
                            apply_edit(
                                &project,
                                &player_state,
                                &selection_state,
                                preferences::snapshot(&preferences).default_visual_duration,
                                request,
                                work.canceled.clone(),
                            )
                            .await
                        }
                        BridgeCommand::GetManimClip(request) => {
                            get_manim_clip(&project, request, work.canceled.clone()).await
                        }
                        BridgeCommand::SetManimClip(request) => {
                            set_manim_clip(&project, &player_state, request, work.canceled.clone())
                                .await
                        }
                        BridgeCommand::ListTtsModels => {
                            list_tts_models(&preferences, work.canceled.clone()).await
                        }
                        BridgeCommand::ListSttModels => {
                            list_stt_models(&preferences, work.canceled.clone()).await
                        }
                        BridgeCommand::TranscribeAudio(request) => {
                            transcribe_audio(
                                &project,
                                &player_state,
                                &selection_state,
                                &preferences,
                                request,
                                work.canceled.clone(),
                            )
                            .await
                        }
                        BridgeCommand::GenerateTts(request) => {
                            generate_tts(
                                &project,
                                &player_state,
                                &selection_state,
                                &preferences,
                                request,
                                work.canceled.clone(),
                            )
                            .await
                        }
                        BridgeCommand::ViewFrame { frame } => {
                            view_frame(&project, frame, work.canceled.clone()).await
                        }
                        BridgeCommand::AnalyzeTransparentFill(request) => {
                            analyze_transparent_fill(
                                &project,
                                &player_state,
                                request,
                                work.canceled.clone(),
                            )
                            .await
                        }
                        command => handle_command(
                            &project,
                            &player_state,
                            &selection_state,
                            command,
                            &work.canceled,
                        ),
                    },
                    Ok(_) => Err(format!(
                        "MCP request named {}, but this editor owns {}",
                        work.request.project_path,
                        project_path.display()
                    )),
                    Err(error) => Err(error.clone()),
                }
            };
            let response = BridgeResponse {
                project_path: project_path_text.unwrap_or_default().to_string(),
                result: result.as_ref().ok().cloned(),
                error: result.err(),
            };
            let _ = work.response.send(response);
        }
    });

    let socket_path = socket.keep();
    Ok(Server { socket_path, stop })
}

fn runtime_directory() -> Result<PathBuf, String> {
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| {
            "XDG_RUNTIME_DIR is not set; cannot expose the live MCP bridge".to_string()
        })?;
    let directory = root.join("shrimply");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let metadata = directory
        .metadata()
        .map_err(|error| format!("could not inspect {}: {error}", directory.display()))?;
    if metadata.uid() != effective_uid() {
        return Err(format!(
            "runtime directory is owned by another user: {}",
            directory.display()
        ));
    }
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("could not secure {}: {error}", directory.display()))?;
    Ok(directory)
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no arguments and no memory safety preconditions.
    unsafe { libc::geteuid() }
}

fn accept_loop(listener: UnixListener, sender: async_channel::Sender<Work>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok(_) if stop.load(Ordering::Acquire) => break,
            Ok((stream, _)) => {
                let sender = sender.clone();
                let stop = stop.clone();
                thread::spawn(move || serve_connection(stream, &sender, &stop));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("MCP bridge accept failed: {error}"),
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
        .expect("MCP request timeout must be configurable");
    stream
        .set_write_timeout(Some(SOCKET_REQUEST_TIMEOUT))
        .expect("MCP response timeout must be configurable");
    let response = (|| {
        let mut line = String::new();
        BufReader::new(&stream)
            .take(SOCKET_READ_LIMIT)
            .read_line(&mut line)
            .map_err(|error| format!("could not read MCP bridge request: {error}"))?;
        let request: BridgeRequest = serde_json::from_str(&line)
            .map_err(|error| format!("malformed MCP bridge request: {error}"))?;
        let (response, receiver) = mpsc::channel();
        let canceled = Arc::new(AtomicBool::new(false));
        sender
            .send_blocking(Work {
                request,
                response,
                canceled: canceled.clone(),
            })
            .map_err(|_| "GTK MCP executor stopped".to_string())?;
        stream
            .set_nonblocking(true)
            .map_err(|error| format!("could not monitor MCP client: {error}"))?;
        let deadline = Instant::now() + EDIT_EXECUTION_TIMEOUT;
        loop {
            if stop.load(Ordering::Acquire) {
                canceled.store(true, Ordering::Release);
                break Err("editor MCP bridge is shutting down".to_string());
            }
            match receiver.recv_timeout(CANCELLATION_POLL_INTERVAL) {
                Ok(response) => break Ok(response),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    canceled.store(true, Ordering::Release);
                    break Err("editor MCP executor dropped the response".to_string());
                }
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                    canceled.store(true, Ordering::Release);
                    break Err("editor MCP response timed out".to_string());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let mut byte = [0];
                    match stream.read(&mut byte) {
                        Ok(0) => {
                            canceled.store(true, Ordering::Release);
                            break Err("MCP client disconnected".to_string());
                        }
                        Ok(_) => {
                            canceled.store(true, Ordering::Release);
                            break Err("MCP client sent more than one request".to_string());
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(error) => {
                            canceled.store(true, Ordering::Release);
                            break Err(format!("could not monitor MCP client: {error}"));
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
}

fn handle_command(
    project: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    selection: &SharedSelectionState,
    command: BridgeCommand,
    canceled: &AtomicBool,
) -> Result<Value, String> {
    if canceled.load(Ordering::Acquire) {
        return Err("MCP client canceled the request".to_string());
    }
    match command {
        BridgeCommand::Handshake => Ok(json!({ "connected": true })),
        BridgeCommand::Snapshot => serde_json::to_value(snapshot(project, player, selection)?)
            .map_err(|error| format!("could not serialize live snapshot: {error}")),
        BridgeCommand::Seek { frame } => {
            let project = project.borrow();
            let position = time_from_frame(frame, project.fps)
                .ok_or_else(|| "frame exceeds the supported exact range".to_string())?;
            player_state::seek_time(player, position);
            serde_json::to_value(shrimply_mcp::query::frame_time(frame, project.fps)?)
                .map_err(|error| format!("could not serialize playhead: {error}"))
        }
        BridgeCommand::ViewFrame { .. } => {
            unreachable!("frame rendering is prepared asynchronously")
        }
        BridgeCommand::AnalyzeTransparentFill(_) => {
            unreachable!("modifier analysis is prepared asynchronously")
        }
        BridgeCommand::GetManimClip(_) => {
            unreachable!("Manim inspection is prepared asynchronously")
        }
        BridgeCommand::SetManimClip(_) => {
            unreachable!("Manim edits are prepared asynchronously")
        }
        BridgeCommand::ReloadManimSource(request) => reload_manim_source(project, request),
        BridgeCommand::ListTtsModels => {
            unreachable!("TTS model discovery is prepared asynchronously")
        }
        BridgeCommand::ListSttModels => {
            unreachable!("STT model discovery is prepared asynchronously")
        }
        BridgeCommand::TranscribeAudio(_) => {
            unreachable!("transcription is prepared asynchronously")
        }
        BridgeCommand::GenerateTts(_) => {
            unreachable!("TTS generation is prepared asynchronously")
        }
        BridgeCommand::Apply(_) => unreachable!("edit commands are prepared asynchronously"),
    }
}

async fn get_manim_clip(
    live: &Rc<RefCell<Project>>,
    request: GetManimClipRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let address = shrimply_mcp::query::model_item_address(&request.address)?;
    let (source, source_revision, scene, input_parameters, parameters, error) = {
        let project = live.borrow();
        let item = project
            .video_item(&address)
            .ok_or_else(|| "get_manim_clip requires a video clip address".to_string())?;
        let VideoItemContent::Manim(manim) = &item.content else {
            return Err("addressed clip is not a Manim clip".to_string());
        };
        let source_revision = item.file.snapshot()?.revision();
        (
            item.file.clone(),
            source_revision,
            manim.scene.clone(),
            manim.parameters.clone(),
            shrimply_state::manim_status::parameters(
                item.id,
                source_revision,
                &manim.scene,
                &manim.parameters,
            ),
            shrimply_state::manim_status::error(
                item.id,
                source_revision,
                &manim.scene,
                &manim.parameters,
            ),
        )
    };
    let scenes = discover_manim_scenes(source.clone(), canceled.clone()).await?;
    if canceled.load(Ordering::Acquire) {
        return Err("MCP client canceled Manim inspection".to_string());
    }
    {
        let project = live.borrow();
        let item = project
            .video_item(&address)
            .ok_or_else(|| "Manim clip changed while it was being inspected".to_string())?;
        let VideoItemContent::Manim(manim) = &item.content else {
            return Err("clip stopped being a Manim clip while it was being inspected".to_string());
        };
        if item.file.snapshot()?.revision() != source_revision
            || manim.scene != scene
            || manim.parameters != input_parameters
        {
            return Err("Manim clip changed while it was being inspected; retry".to_string());
        }
    }
    let parameters_ready = parameters.is_some();
    let parameters = parameters
        .unwrap_or_default()
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not serialize reflected Manim parameter: {error}"))?;
    serde_json::to_value(ManimClipResponse {
        address: request.address,
        source: source.path().to_string_lossy().into_owned(),
        source_revision,
        scene,
        scenes,
        parameters_ready,
        parameters,
        error,
    })
    .map_err(|error| format!("could not serialize Manim clip metadata: {error}"))
}

async fn set_manim_clip(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    request: SetManimClipRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    if request.scene.is_none() && request.parameters.is_empty() {
        return Err("set_manim_clip requires a scene or parameter change".to_string());
    }
    let address = shrimply_mcp::query::model_item_address(&request.address)?;
    let (original, source, source_revision, current_scene, definitions) = {
        let project = live.borrow();
        let item = project
            .video_item(&address)
            .ok_or_else(|| "set_manim_clip requires a video clip address".to_string())?;
        let VideoItemContent::Manim(manim) = &item.content else {
            return Err("addressed clip is not a Manim clip".to_string());
        };
        let source_revision = item.file.snapshot()?.revision();
        (
            project_content_fingerprint(&project)?,
            item.file.clone(),
            source_revision,
            manim.scene.clone(),
            shrimply_state::manim_status::parameters(
                item.id,
                source_revision,
                &manim.scene,
                &manim.parameters,
            ),
        )
    };
    if let Some(scene) = &request.scene {
        let scenes = discover_manim_scenes(source, canceled.clone()).await?;
        if !scenes.contains(scene) {
            return Err(format!("Manim scene {scene:?} was not found in the source"));
        }
        if scene != &current_scene && !request.parameters.is_empty() {
            return Err(
                "set the Manim scene first, render it, then set its reflected parameters"
                    .to_string(),
            );
        }
    }
    if canceled.load(Ordering::Acquire) {
        return Err("MCP client canceled the Manim edit".to_string());
    }

    let parameter_values = if request.parameters.is_empty() {
        Vec::new()
    } else {
        let definitions = definitions.ok_or_else(|| {
            "Manim parameters are not ready; render the current scene with view_frame, then retry"
                .to_string()
        })?;
        request
            .parameters
            .iter()
            .map(|(key, value)| {
                let definition = definitions
                    .iter()
                    .find(|parameter| &parameter.key == key)
                    .ok_or_else(|| format!("Manim parameter {key:?} was not reflected"))?;
                value
                    .as_ref()
                    .map(|value| manim_parameter_value(definition, value))
                    .transpose()
                    .map(|value| (key.clone(), value))
            })
            .collect::<Result<Vec<_>, String>>()?
    };

    let mut project = live.borrow().clone();
    if project_content_fingerprint(&project)? != original {
        return Err("project changed while the Manim edit was being prepared; retry".to_string());
    }
    let item = project
        .video_item_mut(&address)
        .expect("validated Manim clip must still exist in an unchanged project");
    if item.file.snapshot()?.revision() != source_revision {
        return Err("Manim source changed while the edit was being prepared; retry".to_string());
    }
    let VideoItemContent::Manim(manim) = &mut item.content else {
        unreachable!("validated Manim clip must keep its content kind");
    };
    let mut changed = false;
    if let Some(scene) = request.scene
        && manim.scene != scene
    {
        manim.scene = scene;
        manim.parameters.clear();
        changed = true;
    }
    for (key, value) in parameter_values {
        match value {
            Some(value) if manim.parameters.get(&key) != Some(&value) => {
                manim.parameters.insert(key, value);
                changed = true;
            }
            None if manim.parameters.remove(&key).is_some() => changed = true,
            Some(_) | None => {}
        }
    }
    if !changed {
        return Err("requested Manim values are already set".to_string());
    }

    let item_id = address.item_id();
    shrimply_project::project::commit_edit_checked(&project, "MCP set Manim clip")?;
    *live.borrow_mut() = project;
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
    let project = live.borrow();
    let presentations =
        shrimply_mcp::query::presentations_affected_by_items(&project, &HashSet::from([item_id]))?;
    let response = EditResponse {
        operations: vec![EditOperationResult {
            index: 0,
            operation: "set_manim_clip".to_string(),
            changed_addresses: presentations
                .iter()
                .map(|clip| clip.address.clone())
                .collect(),
            deleted_addresses: Vec::new(),
            changed_tracks: Vec::new(),
            presentations,
        }],
        duration: shrimply_mcp::query::frame_time_from_time(project.duration(), project.fps, true),
        revision: player_state::snapshot(player).revision,
    };
    serde_json::to_value(response)
        .map_err(|error| format!("could not serialize Manim edit result: {error}"))
}

fn reload_manim_source(
    live: &Rc<RefCell<Project>>,
    request: ReloadManimSourceRequest,
) -> Result<Value, String> {
    let address = shrimply_mcp::query::model_item_address(&request.address)?;
    let source = {
        let project = live.borrow();
        let item = project
            .video_item(&address)
            .ok_or_else(|| "reload_manim_source requires a video clip address".to_string())?;
        if !matches!(item.content, VideoItemContent::Manim(_)) {
            return Err("addressed clip is not a Manim clip".to_string());
        }
        item.file.clone()
    };
    shrimply_manim_parser::invalidate_ir_cache(&source)?;
    source.mark_dirty()?;
    serde_json::to_value(ReloadManimSourceResponse {
        address: request.address,
        source_revision: source.snapshot()?.revision(),
    })
    .map_err(|error| format!("could not serialize Manim reload result: {error}"))
}

async fn discover_manim_scenes(
    source: Asset,
    canceled: Arc<AtomicBool>,
) -> Result<Vec<String>, String> {
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-mcp-manim-scenes".to_string())
        .spawn(move || {
            let result = if canceled.load(Ordering::Acquire) {
                Err("MCP client canceled Manim scene discovery".to_string())
            } else {
                shrimply_manim_parser::discover_scenes(&source)
            };
            let _ = sender.send_blocking(result);
        })
        .map_err(|error| format!("could not start Manim scene discovery: {error}"))?;
    receiver
        .recv()
        .await
        .map_err(|_| "Manim scene discovery stopped without a result".to_string())?
}

fn manim_parameter_value(
    parameter: &ManimParameter,
    value: &ProtocolManimParameterValue,
) -> Result<ManimParameterValue, String> {
    let invalid = || {
        format!(
            "value for Manim parameter {:?} does not match its reflected control",
            parameter.key
        )
    };
    match (&parameter.control, value) {
        (ManimParameterControl::AntiAliasing, ProtocolManimParameterValue::Integer(value))
            if MANIM_ANTI_ALIASING_SAMPLES.contains(value) =>
        {
            Ok(ManimParameterValue::Integer(*value))
        }
        (
            ManimParameterControl::Integer {
                minimum, maximum, ..
            },
            ProtocolManimParameterValue::Integer(value),
        ) if minimum.is_none_or(|minimum| *value >= minimum)
            && maximum.is_none_or(|maximum| *value <= maximum) =>
        {
            Ok(ManimParameterValue::Integer(*value))
        }
        (
            ManimParameterControl::Float {
                minimum, maximum, ..
            },
            ProtocolManimParameterValue::Float(value),
        ) if value.is_finite()
            && minimum.is_none_or(|minimum| *value >= minimum)
            && maximum.is_none_or(|maximum| *value <= maximum) =>
        {
            Ok(ManimParameterValue::Float(*value))
        }
        (ManimParameterControl::Fraction, ProtocolManimParameterValue::Fraction(value))
            if value.denominator != 0 =>
        {
            let value = fraction_new(value.numerator, value.denominator);
            Ok(ManimParameterValue::Fraction {
                numerator: fraction_numerator(value),
                denominator: fraction_denominator(value),
            })
        }
        (ManimParameterControl::Color, ProtocolManimParameterValue::Color(value)) => {
            Ok(ManimParameterValue::Color(
                shrimply_project::project::Color::new(value.r, value.g, value.b, u8::MAX),
            ))
        }
        (ManimParameterControl::Option { options }, ProtocolManimParameterValue::Option(value))
            if options.contains(value) =>
        {
            Ok(ManimParameterValue::Option(value.clone()))
        }
        (ManimParameterControl::Boolean, ProtocolManimParameterValue::Boolean(value)) => {
            Ok(ManimParameterValue::Boolean(*value))
        }
        (ManimParameterControl::String, ProtocolManimParameterValue::String(value)) => {
            Ok(ManimParameterValue::String(value.clone()))
        }
        _ => Err(invalid()),
    }
}

async fn analyze_transparent_fill(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    request: AnalyzeTransparentFillRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let address = shrimply_mcp::query::model_item_address(&request.address)?;
    let modifier_id = Uuid::parse_str(&request.modifier_id)
        .map_err(|error| format!("invalid modifier_id {:?}: {error}", request.modifier_id))?;
    let mut project = live.borrow().clone();
    let fill = project
        .video_item_mut(&address)
        .ok_or_else(|| "Transparent Fill analysis requires a video clip address".to_string())?
        .modifiers
        .iter_mut()
        .find(|modifier| modifier.id == modifier_id)
        .ok_or_else(|| format!("modifier {modifier_id} does not exist on the addressed clip"))
        .and_then(|modifier| match &mut modifier.effect {
            ModifierEffect::Raster(effect) => match &mut **effect {
                RasterModifierEffect::TransparentFill(fill) => Ok(fill),
                _ => Err(format!("modifier {modifier_id} is not Transparent Fill")),
            },
            _ => Err(format!("modifier {modifier_id} is not Transparent Fill")),
        })?;
    if fill.points.is_empty() {
        return Err("add at least one transparent fill point before analyzing".to_string());
    }
    fill.analysis_generation = fill.analysis_generation.wrapping_add(1).max(1);
    let generation = fill.analysis_generation;
    shrimply_project::project::commit_edit_checked(&project, "MCP analyze Transparent Fill")?;
    *live.borrow_mut() = project.clone();
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
    let run_id =
        shrimply_video_cuda::transparent_fill_analysis::analyze(project, &address, modifier_id)?;

    loop {
        if canceled.load(Ordering::Acquire) {
            shrimply_video_cuda::transparent_fill_analysis::cancel(run_id);
            return Err("MCP client canceled Transparent Fill analysis".to_string());
        }
        let status = shrimply_video_cuda::transparent_fill_analysis::status_for_run(run_id);
        match status {
            TransparentFillStatus::Running { .. } => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            TransparentFillStatus::Complete => break,
            TransparentFillStatus::Failed(error) => {
                return Err(format!("Transparent Fill analysis failed: {error}"));
            }
            TransparentFillStatus::Cancelled => {
                return Err("Transparent Fill analysis was canceled".to_string());
            }
            TransparentFillStatus::Missing => {
                shrimply_video_cuda::transparent_fill_analysis::cancel(run_id);
                return Err(
                    "Transparent Fill inputs changed while analysis was running; retry analysis"
                        .to_string(),
                );
            }
        }
    }

    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
    serde_json::to_value(AnalyzeTransparentFillResponse {
        address: request.address,
        modifier_id: request.modifier_id,
        analysis_generation: generation,
        revision: player_state::snapshot(player).revision,
    })
    .map_err(|error| format!("could not serialize Transparent Fill analysis result: {error}"))
}

async fn view_frame(
    live: &Rc<RefCell<Project>>,
    frame: u64,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let project = live.borrow().clone();
    let position = time_from_frame(frame, project.fps)
        .ok_or_else(|| "frame exceeds the supported exact range".to_string())?;
    let fps = project.fps;
    let canvas = project.canvas_size;
    let (sender, receiver) = async_channel::bounded(1);
    let render_canceled = canceled.clone();
    thread::Builder::new()
        .name("shrimply-mcp-frame".to_string())
        .spawn(move || {
            let result = (|| {
                let mut renderer = VideoExportRenderer::new(FRAME_RENDER_AUDIO_SAMPLE_RATE)?;
                let rendered = loop {
                    if render_canceled.load(Ordering::Acquire) {
                        return Err("MCP client canceled the frame render".to_string());
                    }
                    match renderer.render(&project, position, 0) {
                        Ok(frame) => break frame,
                        Err(error) if error == EXPORT_ASSETS_LOADING => thread::yield_now(),
                        Err(error) => return Err(error),
                    }
                };
                let mut rgba = ffmpeg_next::frame::Video::new(
                    ffmpeg_next::format::Pixel::RGBA,
                    canvas.width,
                    canvas.height,
                );
                renderer.copy_to_rgba_frame(rendered, &mut rgba)?;
                let row_bytes = canvas.width as usize * std::mem::size_of::<u32>();
                let mut pixels = Vec::with_capacity(row_bytes * canvas.height as usize);
                for row in rgba
                    .data(0)
                    .chunks_exact(rgba.stride(0))
                    .take(canvas.height as usize)
                {
                    pixels.extend_from_slice(&row[..row_bytes]);
                }
                Ok::<_, String>(pixels)
            })();
            let _ = sender.send_blocking(result);
        })
        .map_err(|error| format!("could not start MCP frame renderer: {error}"))?;

    let pixels = loop {
        if canceled.load(Ordering::Acquire) {
            return Err("MCP client canceled the frame render".to_string());
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("MCP frame renderer stopped without a result".to_string());
            }
        }
    };
    let width = i32::try_from(canvas.width).map_err(|_| "canvas width is too large")?;
    let height = i32::try_from(canvas.height).map_err(|_| "canvas height is too large")?;
    let texture = gdk::MemoryTexture::new(
        width,
        height,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(pixels),
        canvas.width as usize * std::mem::size_of::<u32>(),
    );
    let png = glib::base64_encode(texture.save_to_png_bytes().as_ref()).to_string();
    serde_json::to_value(ViewFrameResponse {
        frame: shrimply_mcp::query::frame_time(frame, fps)?,
        png,
    })
    .map_err(|error| format!("could not serialize rendered frame: {error}"))
}

fn snapshot(
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
        project_path: shrimply_project::project::normalized_project_path(
            &shrimply_project::project::active_project_path(),
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
                .sequence_paths_for_scope(shrimply_project::project::ItemKind::Video, &active_scope)
                .into_iter()
                .map(|path| ScopeRef {
                    sequence_path: path.iter().map(Uuid::to_string).collect(),
                })
                .collect(),
            audio_paths: project
                .sequence_paths_for_scope(shrimply_project::project::ItemKind::Audio, &active_scope)
                .into_iter()
                .map(|path| ScopeRef {
                    sequence_path: path.iter().map(Uuid::to_string).collect(),
                })
                .collect(),
        },
        focused_item: selection_state::focused_item_address(selection, &project)
            .as_ref()
            .map(shrimply_mcp::query::protocol_item_address),
        selected_items: selection_state::selected_item_addresses(selection, &project)
            .iter()
            .map(shrimply_mcp::query::protocol_item_address)
            .collect(),
        focused_track: selection_state::focused_track_address(selection, &project)
            .as_ref()
            .map(shrimply_mcp::query::protocol_track_address),
        selected_tracks: selection_state::selected_track_addresses(selection, &project)
            .iter()
            .map(shrimply_mcp::query::protocol_track_address)
            .collect(),
        asset_revisions,
    })
}

async fn list_stt_models(
    preferences: &SharedPreferences,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let preferences = preferences::snapshot(preferences);
    let server_url = preferences.compute_server_url;
    let preferred = preferences.last_stt_model;
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-mcp-stt-models".to_string())
        .spawn(move || {
            let _ = sender.send_blocking(stt_models(&server_url));
        })
        .map_err(|error| format!("could not start STT model discovery: {error}"))?;
    let models = loop {
        if canceled.load(Ordering::Acquire) {
            return Err("MCP client canceled STT model discovery".to_string());
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("STT model discovery worker stopped without a result".to_string());
            }
        }
    };
    let default_model = models
        .iter()
        .find(|model| **model == preferred)
        .or_else(|| models.first())
        .cloned();
    serde_json::to_value(ListSttModelsResponse {
        models,
        default_model,
    })
    .map_err(|error| format!("could not serialize STT model response: {error}"))
}

fn stt_models(server_url: &str) -> Result<Vec<String>, String> {
    let mut models = shrimply_server_client::server_status(server_url)?
        .capabilities
        .into_iter()
        .filter_map(|capability| capability.strip_prefix("stt:").map(str::to_string))
        .filter(|model| !model.is_empty())
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    Ok(models)
}

struct CompletedTranscription {
    model: String,
    segments: Vec<shrimply_transcription::TranscribedSegment>,
}

async fn transcribe_audio(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    selection: &SharedSelectionState,
    preferences: &SharedPreferences,
    request: TranscribeAudioRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    if let Some(language) = &request.language
        && !caption_languages().contains(language)
    {
        return Err(format!("{language} is not a supported caption language"));
    }
    let project_path = shrimply_project::project::normalized_project_path(
        &shrimply_project::project::active_project_path(),
    );
    let mut project = live.borrow().clone();
    let original = project_content_fingerprint(&project)?;
    let (transcription_project, ranges) = transcription_source(&project, &request)?;
    let preference = preferences::snapshot(preferences);
    let active_job = Arc::new(Mutex::new(None));
    let worker_job = active_job.clone();
    let worker_canceled = canceled.clone();
    let requested_model = request.model.clone();
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-mcp-transcribe".to_string())
        .spawn(move || {
            let result = run_transcription(
                transcription_project,
                ranges,
                preference.compute_server_url,
                preference.last_stt_model,
                requested_model,
                worker_canceled,
                worker_job,
            );
            let _ = sender.send_blocking(result);
        })
        .map_err(|error| format!("could not start transcription worker: {error}"))?;

    let mut transcription = loop {
        if canceled.load(Ordering::Acquire)
            && let Some(job) = active_job
                .lock()
                .expect("MCP transcription active job lock was poisoned")
                .as_ref()
        {
            job.cancel();
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("transcription worker stopped without a result".to_string());
            }
        }
    };
    if canceled.load(Ordering::Acquire) {
        return Err("MCP client canceled transcription".to_string());
    }
    let current_path = shrimply_project::project::normalized_project_path(
        &shrimply_project::project::active_project_path(),
    );
    if current_path != project_path {
        return Err("project path changed while audio was being transcribed".to_string());
    }
    if project_content_fingerprint(&live.borrow())? != original {
        return Err(
            "project changed while audio was being transcribed; retry the request".to_string(),
        );
    }

    let overlap_count = shrimply_transcription::sanitize_transcribed_segments(
        &mut transcription.segments,
        project.frame_step(),
    );
    if overlap_count > 0 {
        tracing::warn!(
            overlap_count,
            segment_count = transcription.segments.len(),
            "resolved overlapping MCP transcription segments"
        );
    }
    if transcription.segments.is_empty() {
        return Err("the selected audio produced no transcript".to_string());
    }
    let captions = transcription
        .segments
        .into_iter()
        .map(|segment| CaptionCueInput {
            start_frame: segment.start.as_frame(project.fps),
            end_frame: segment.end.as_frame(project.fps),
            text: segment.text,
            copy_style_from: None,
        })
        .collect();
    let operation = shrimply_mcp::protocol::EditOperation::InsertCaptions(InsertCaptionsRequest {
        track: None,
        captions,
        language: request.language,
        enabled: None,
        collision: CollisionBehavior::Reject,
    });
    let mutation = shrimply_mcp::edit::apply_non_import(
        &mut project,
        &operation,
        0,
        &SequenceScopeId::root(),
    )?;
    project
        .validate()
        .map_err(|error| format!("transcription edit is invalid: {error}"))?;
    let changed_presentations = shrimply_mcp::query::presentations_affected_by_items(
        &project,
        &mutation.changed_item_ids.iter().copied().collect(),
    )?;
    let result = EditOperationResult {
        index: 0,
        operation: "transcribe_audio".to_string(),
        changed_addresses: changed_presentations
            .iter()
            .map(|clip| clip.address.clone())
            .collect(),
        deleted_addresses: mutation.deleted_addresses,
        changed_tracks: mutation.changed_tracks,
        presentations: changed_presentations,
    };
    let duration = project.duration();
    let frame_rate = project.fps;
    let duration_frame = shrimply_mcp::query::frame_time_from_time(duration, frame_rate, true);
    if canceled.load(Ordering::Acquire) {
        return Err("MCP client canceled transcription before commit".to_string());
    }
    let editor_selection = EditorSelection::capture(selection, &live.borrow());
    shrimply_project::project::commit_edit_checked(&project, "MCP transcribe audio")
        .map_err(|error| format!("MCP transcription edit could not be committed: {error}"))?;
    *live.borrow_mut() = project;
    editor_selection.restore(selection, &live.borrow());
    preferences::set_last_stt_model(preferences, &transcription.model);
    player_state::refresh_project(
        player,
        ProjectChange {
            duration: Some(duration),
            frame_rate: Some(frame_rate),
            captions: true,
            inspector: true,
            ..Default::default()
        },
    );
    serde_json::to_value(EditResponse {
        operations: vec![result],
        duration: duration_frame,
        revision: player_state::snapshot(player).revision,
    })
    .map_err(|error| format!("could not serialize transcription edit result: {error}"))
}

fn run_transcription(
    project: Project,
    ranges: Vec<(Time, Time)>,
    server_url: String,
    preferred_model: String,
    requested_model: Option<String>,
    canceled: Arc<AtomicBool>,
    active_job: Arc<Mutex<Option<shrimply_server_client::CancellationToken>>>,
) -> Result<CompletedTranscription, String> {
    let models = stt_models(&server_url)?;
    let model = requested_model
        .as_ref()
        .and_then(|requested| models.iter().find(|model| *model == requested))
        .or_else(|| models.iter().find(|model| **model == preferred_model))
        .or_else(|| models.first())
        .cloned()
        .ok_or_else(|| "the compute server provided no STT models".to_string())?;
    if let Some(requested) = &requested_model
        && requested != &model
    {
        return Err(format!("STT model {requested:?} is not available"));
    }
    let chunks = shrimply_transcription::prepare_transcription_chunks(&project, &ranges)?;
    let mut output = Vec::new();
    for (index, chunk) in chunks.into_iter().enumerate() {
        if canceled.load(Ordering::Acquire) {
            return Err("MCP client canceled transcription".to_string());
        }
        let cancellation = shrimply_server_client::CancellationToken::new(&server_url)?;
        *active_job
            .lock()
            .expect("MCP transcription active job lock was poisoned") = Some(cancellation.clone());
        let result = shrimply_server_client::transcribe(
            &server_url,
            &cancellation,
            &model,
            &chunk.samples,
            |message| tracing::info!(chunk = index + 1, %message, "MCP transcription progress"),
        );
        active_job
            .lock()
            .expect("MCP transcription active job lock was poisoned")
            .take();
        let transcription = result?;
        if canceled.load(Ordering::Acquire) {
            return Err("MCP client canceled transcription".to_string());
        }
        let duration = chunk.end.saturating_sub(chunk.start);
        let mut segments = transcription
            .segments
            .into_iter()
            .filter_map(|segment| {
                let text = segment.text.trim();
                if text.is_empty() {
                    return None;
                }
                let start = Time::from_fraction(
                    segment.start_frame.min(i64::MAX as u64) as i64,
                    i64::from(shrimply_transcription::SAMPLE_RATE),
                )
                .min(duration);
                let mut end = Time::from_fraction(
                    segment.end_frame.min(i64::MAX as u64) as i64,
                    i64::from(shrimply_transcription::SAMPLE_RATE),
                )
                .min(duration);
                if end <= start {
                    end = start.saturating_add(Time::from_fraction(
                        1,
                        i64::from(shrimply_transcription::SAMPLE_RATE),
                    ));
                }
                Some(shrimply_transcription::TranscribedSegment {
                    start: chunk.start.saturating_add(start),
                    end: chunk.start.saturating_add(end).min(chunk.end),
                    text: text.to_string(),
                })
            })
            .collect::<Vec<_>>();
        if let Some(first) = segments.first_mut() {
            first.start = chunk.start;
        }
        if let Some(last) = segments.last_mut() {
            last.end = chunk.end;
        }
        output.extend(segments);
    }
    Ok(CompletedTranscription {
        model,
        segments: output,
    })
}

fn transcription_source(
    project: &Project,
    request: &TranscribeAudioRequest,
) -> Result<(Project, Vec<(Time, Time)>), String> {
    let addresses = match (&request.track, request.clips.is_empty()) {
        (Some(_), false) => {
            return Err("transcribe_audio requires exactly one of track or clips".to_string());
        }
        (None, true) => {
            return Err(
                "transcribe_audio requires an audio track or at least one audio clip".to_string(),
            );
        }
        (Some(track), true) => {
            let address = shrimply_mcp::query::model_track_address(track)?;
            let TrackRef::Audio(source) = project
                .track(&address)
                .ok_or_else(|| "transcription track was not found".to_string())?
            else {
                return Err("transcription requires an audio track".to_string());
            };
            source
                .items
                .iter()
                .map(|item| address.item(item.id))
                .collect::<Vec<_>>()
        }
        (None, false) => request
            .clips
            .iter()
            .map(|clip| {
                let address = shrimply_mcp::query::model_item_address(clip)?;
                project
                    .audio_item(&address)
                    .ok_or_else(|| format!("audio clip {} was not found", clip.item_id))?;
                Ok(address)
            })
            .collect::<Result<Vec<_>, String>>()?,
    };
    let addresses = addresses.into_iter().collect::<HashSet<_>>();
    if addresses.is_empty() {
        return Err("the transcription source contains no audio clips".to_string());
    }

    let mut allowed = HashMap::<Option<Uuid>, HashSet<Uuid>>::new();
    let mut included_sequences = HashSet::new();
    let mut ranges = Vec::with_capacity(addresses.len());
    for address in &addresses {
        let ItemAddress::Audio {
            sequence_path,
            track_id,
            item_id,
        } = address
        else {
            return Err("transcription requires audio clips".to_string());
        };
        let mut tracks = project.audio_tracks.as_slice();
        let mut sequence_id = None;
        for host_id in sequence_path {
            let host = tracks
                .iter()
                .flat_map(|track| &track.items)
                .find(|item| item.id == *host_id)
                .ok_or_else(|| "audio clip sequence path was not found".to_string())?;
            allowed.entry(sequence_id).or_default().insert(*host_id);
            let AudioSource::FoldedSequence(reference) = &host.source else {
                return Err("audio clip sequence path contains a non-sequence clip".to_string());
            };
            sequence_id = Some(reference.sequence_id);
            tracks = &project
                .folded_sequence(reference.sequence_id)
                .ok_or_else(|| "audio clip sequence definition was not found".to_string())?
                .audio_tracks;
        }
        let item = tracks
            .iter()
            .find(|track| track.id == *track_id)
            .and_then(|track| track.items.iter().find(|item| item.id == *item_id))
            .ok_or_else(|| "audio clip was not found".to_string())?;
        allowed.entry(sequence_id).or_default().insert(*item_id);
        if let AudioSource::FoldedSequence(reference) = &item.source {
            include_sequence_audio(
                project,
                reference.sequence_id,
                &mut allowed,
                &mut included_sequences,
            )?;
        }
        ranges.push(
            project
                .projected_item_times(address)
                .ok_or_else(|| "audio clip has no visible projected range".to_string())?,
        );
    }

    let mut selected = project.clone();
    selected.caption_tracks.clear();
    selected.video_tracks.clear();
    let root = allowed.get(&None);
    for track in &mut selected.audio_tracks {
        track
            .items
            .retain(|item| root.is_some_and(|items| items.contains(&item.id)));
    }
    for sequence in &mut selected.folded_sequences {
        sequence.video_tracks.clear();
        let items = allowed.get(&Some(sequence.id));
        for track in &mut sequence.audio_tracks {
            track
                .items
                .retain(|item| items.is_some_and(|items| items.contains(&item.id)));
        }
    }

    ranges.sort_by_key(|range| *range);
    let mut chunks: Vec<(Time, Time)> = Vec::new();
    for (start, end) in ranges {
        let Some((_, last_end)) = chunks.last_mut() else {
            chunks.push((start, end));
            continue;
        };
        if start < *last_end {
            *last_end = (*last_end).max(end);
        } else {
            chunks.push((start, end));
        }
    }
    Ok((selected, chunks))
}

fn include_sequence_audio(
    project: &Project,
    sequence_id: Uuid,
    allowed: &mut HashMap<Option<Uuid>, HashSet<Uuid>>,
    included: &mut HashSet<Uuid>,
) -> Result<(), String> {
    if !included.insert(sequence_id) {
        return Ok(());
    }
    let sequence = project
        .folded_sequence(sequence_id)
        .ok_or_else(|| format!("audio sequence {sequence_id} was not found"))?;
    for item in sequence.audio_tracks.iter().flat_map(|track| &track.items) {
        allowed
            .entry(Some(sequence_id))
            .or_default()
            .insert(item.id);
        if let AudioSource::FoldedSequence(reference) = &item.source {
            include_sequence_audio(project, reference.sequence_id, allowed, included)?;
        }
    }
    Ok(())
}

async fn list_tts_models(
    preferences: &SharedPreferences,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let preferences = preferences::snapshot(preferences);
    let server_url = preferences.compute_server_url;
    let preferred = preferences.last_tts_model;
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-mcp-tts-models".to_string())
        .spawn(move || {
            let _ = sender.send_blocking(shrimply_tts::models(&server_url));
        })
        .map_err(|error| format!("could not start TTS model discovery: {error}"))?;
    let models = loop {
        if canceled.load(Ordering::Acquire) {
            return Err("MCP client canceled TTS model discovery".to_string());
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("TTS model discovery worker stopped without a result".to_string());
            }
        }
    };
    let default_model = models
        .iter()
        .find(|model| model.id == preferred)
        .or_else(|| models.first())
        .map(|model| model.id.clone());
    let models = models
        .into_iter()
        .map(|model| {
            serde_json::to_value(model)
                .map_err(|error| format!("could not serialize TTS model: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_value(ListTtsModelsResponse {
        models,
        default_model,
    })
    .map_err(|error| format!("could not serialize TTS model response: {error}"))
}

struct StagedSpeech {
    staging: PathBuf,
    final_path: PathBuf,
    promoted: bool,
}

impl StagedSpeech {
    fn promote(&mut self) -> Result<(), String> {
        fs::rename(&self.staging, &self.final_path).map_err(|error| {
            format!(
                "could not promote generated speech {}: {error}",
                self.final_path.display()
            )
        })?;
        self.promoted = true;
        Ok(())
    }

    fn rollback(&mut self) {
        fs::rename(&self.final_path, &self.staging).unwrap_or_else(|error| {
            panic!(
                "could not roll back generated speech {}: {error}",
                self.final_path.display()
            )
        });
        self.promoted = false;
    }
}

impl Drop for StagedSpeech {
    fn drop(&mut self) {
        if !self.promoted && self.staging.exists() {
            fs::remove_file(&self.staging).unwrap_or_else(|error| {
                panic!(
                    "could not clean staged generated speech {}: {error}",
                    self.staging.display()
                )
            });
        }
    }
}

struct GeneratedTts {
    speech: StagedSpeech,
    duration: Time,
    settings: shrimply_tts::TtsSettings,
}

async fn generate_tts(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    selection: &SharedSelectionState,
    preferences: &SharedPreferences,
    request: GenerateTtsRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    if request.text.trim().is_empty() {
        return Err("text must not be empty".to_string());
    }
    let project_path = shrimply_project::project::normalized_project_path(
        &shrimply_project::project::active_project_path(),
    );
    let mut project = live.borrow().clone();
    let original = project_content_fingerprint(&project)?;
    let playhead_frame = player_state::current_time(player).as_frame(project.fps);
    let active_scope = selection_state::active_scope(selection);
    let preferences = preferences::snapshot(preferences);
    let cancellation =
        shrimply_server_client::CancellationToken::new(&preferences.compute_server_url)?;
    let worker_cancellation = cancellation.clone();
    let worker_path = project_path.clone();
    let worker_request = request.clone();
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-mcp-generate-tts".to_string())
        .spawn(move || {
            let result = prepare_generated_tts(
                worker_path,
                preferences.compute_server_url,
                preferences.last_tts_model,
                worker_request,
                worker_cancellation,
            );
            let _ = sender.send_blocking(result);
        })
        .map_err(|error| format!("could not start TTS generation worker: {error}"))?;
    let mut cancellation_sent = false;
    let mut generated = loop {
        if canceled.load(Ordering::Acquire) && !cancellation_sent {
            cancellation.cancel();
            cancellation_sent = true;
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("TTS generation worker stopped without a result".to_string());
            }
        }
    };
    if canceled.load(Ordering::Acquire) {
        return Err("MCP client canceled TTS generation".to_string());
    }
    let current_path = shrimply_project::project::normalized_project_path(
        &shrimply_project::project::active_project_path(),
    );
    if current_path != project_path {
        return Err("project path changed while TTS was being generated".to_string());
    }
    if project_content_fingerprint(&live.borrow())? != original {
        return Err("project changed while TTS was being generated; retry the request".to_string());
    }
    let editor_selection = EditorSelection::capture(selection, &project);
    let mutation = imports::insert_generated_tts(
        &mut project,
        &request,
        playhead_frame,
        active_scope,
        generated.duration,
        generated.settings,
        generated.speech.final_path.clone(),
    )?;
    project
        .validate()
        .map_err(|error| format!("generated TTS edit is invalid: {error}"))?;
    let changed_presentations = shrimply_mcp::query::presentations_affected_by_items(
        &project,
        &mutation.changed_item_ids.iter().copied().collect(),
    )?;
    let mut presentations = changed_presentations.clone();
    presentations.extend(mutation.deleted_presentations.clone());
    let result = EditOperationResult {
        index: 0,
        operation: "generate_tts".to_string(),
        changed_addresses: changed_presentations
            .iter()
            .map(|clip| clip.address.clone())
            .collect(),
        deleted_addresses: mutation.deleted_addresses,
        changed_tracks: mutation.changed_tracks,
        presentations,
    };
    let duration = project.duration();
    let frame_rate = project.fps;
    let duration_frame = shrimply_mcp::query::frame_time_from_time(duration, frame_rate, true);
    if canceled.load(Ordering::Acquire) {
        return Err("MCP client canceled TTS generation before commit".to_string());
    }
    generated.speech.promote()?;
    if canceled.load(Ordering::Acquire) {
        generated.speech.rollback();
        return Err("MCP client canceled TTS generation before commit".to_string());
    }
    if let Err(error) = shrimply_project::project::commit_edit_checked(&project, "MCP generate TTS")
    {
        generated.speech.rollback();
        return Err(format!("MCP TTS edit could not be committed: {error}"));
    }
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
            inspector: true,
            ..Default::default()
        },
    );
    serde_json::to_value(EditResponse {
        operations: vec![result],
        duration: duration_frame,
        revision: player_state::snapshot(player).revision,
    })
    .map_err(|error| format!("could not serialize TTS edit result: {error}"))
}

fn prepare_generated_tts(
    project_path: PathBuf,
    server_url: String,
    preferred_model: String,
    request: GenerateTtsRequest,
    cancellation: shrimply_server_client::CancellationToken,
) -> Result<GeneratedTts, String> {
    let models = shrimply_tts::models(&server_url)?;
    let model = request
        .model
        .as_ref()
        .and_then(|id| models.iter().find(|model| &model.id == id))
        .or_else(|| models.iter().find(|model| model.id == preferred_model))
        .or_else(|| models.first())
        .cloned()
        .ok_or_else(|| "the compute server provided no TTS models".to_string())?;
    if let Some(requested) = &request.model
        && requested != &model.id
    {
        return Err(format!("TTS model {requested:?} is not available"));
    }
    let mut settings = shrimply_tts::TtsSettings::default();
    shrimply_tts::sync_settings(&mut settings, &model);
    for (key, input) in request.inputs {
        let definition = model
            .inputs
            .iter()
            .find(|definition| definition.key() == key)
            .ok_or_else(|| format!("TTS model {} has no input {key:?}", model.id))?;
        if definition.purpose() == Some(shrimply_tts::InputPurpose::Text) {
            return Err(format!(
                "TTS input {key:?} is controlled by the top-level text field"
            ));
        }
        let value = match (definition, input) {
            (shrimply_tts::InputDefinition::Text { .. }, TtsInputValue::Text { value }) => {
                shrimply_tts::TtsValue::Text { value }
            }
            (shrimply_tts::InputDefinition::Select { .. }, TtsInputValue::Select { value }) => {
                shrimply_tts::TtsValue::Select { value }
            }
            (shrimply_tts::InputDefinition::Audio { .. }, TtsInputValue::Audio { path }) => {
                let path = PathBuf::from(&path).canonicalize().map_err(|error| {
                    format!("could not resolve TTS audio input {path:?}: {error}")
                })?;
                if !path.is_file() {
                    return Err(format!(
                        "TTS audio input is not a regular file: {}",
                        path.display()
                    ));
                }
                shrimply_tts::TtsValue::Audio {
                    value: Asset::new(path),
                }
            }
            (shrimply_tts::InputDefinition::Toggle { .. }, TtsInputValue::Toggle { value }) => {
                shrimply_tts::TtsValue::Toggle { value }
            }
            (shrimply_tts::InputDefinition::Number { .. }, TtsInputValue::Number { value }) => {
                if value.denominator <= 0 {
                    return Err(format!(
                        "TTS numeric input {key:?} requires a positive denominator"
                    ));
                }
                shrimply_tts::TtsValue::Number {
                    value: fraction_new(value.numerator, value.denominator),
                }
            }
            (shrimply_tts::InputDefinition::Table { .. }, TtsInputValue::Table { rows }) => {
                shrimply_tts::TtsValue::Table { rows }
            }
            _ => return Err(format!("TTS input {key:?} has the wrong value type")),
        };
        settings.inputs.insert(key, value);
    }
    shrimply_tts::set_text(&mut settings, &model, request.text);
    let speech_request = shrimply_tts::speech_request(
        &model,
        &settings,
        shrimply_audio::recording::transcode_to_wav,
    )?;
    let speech = shrimply_tts::synthesize(&server_url, &cancellation, &speech_request, |_| {
        !cancellation.is_cancelled()
    })?;
    let directory = project_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("media/tts");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let id = Uuid::new_v4();
    let staging = directory.join(format!(".{id}.staging.opus"));
    let final_path = directory.join(format!("{id}.opus"));
    if staging.exists() || final_path.exists() {
        return Err("generated TTS destination already exists".to_string());
    }
    let duration = shrimply_audio::recording::save_wav_as_opus(&speech.wav, &staging)?;
    shrimply_tts::apply_speed_factor(&mut settings, &model, speech.speed_factor);
    Ok(GeneratedTts {
        speech: StagedSpeech {
            staging,
            final_path,
            promoted: false,
        },
        duration,
        settings,
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
        return Err("MCP client canceled the edit".to_string());
    }
    let project_path = shrimply_project::project::normalized_project_path(
        &shrimply_project::project::active_project_path(),
    );
    let project = live.borrow().clone();
    let original = project_content_fingerprint(&project)?;
    let playhead_frame = player_state::current_time(player).as_frame(project.fps);
    let active_scope = selection_state::active_scope(selection);
    let history_label = request.history_label.clone();
    let worker_path = project_path.clone();
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-mcp-edit".to_string())
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
        .map_err(|error| format!("could not start MCP edit worker: {error}"))?;

    let mut prepared = loop {
        if canceled.load(Ordering::Acquire) {
            return Err("MCP client canceled the edit".to_string());
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("MCP edit worker stopped without a result".to_string());
            }
        }
    };
    let current_path = shrimply_project::project::normalized_project_path(
        &shrimply_project::project::active_project_path(),
    );
    if current_path != project_path {
        return Err("project path changed while the MCP edit was being prepared".to_string());
    }
    {
        let current = live.borrow();
        if project_content_fingerprint(&current)? != original {
            return Err(
                "project changed while the MCP edit was being prepared; retry the edit".to_string(),
            );
        }
        prepared.project.cursor_position = current.cursor_position;
        prepared.project.timeline_zoom = current.timeline_zoom;
        prepared.project.expanded_sequence_paths = current.expanded_sequence_paths.clone();
    }
    let duration = prepared.project.duration();
    let frame_rate = prepared.project.fps;
    let duration_frame =
        shrimply_mcp::query::frame_time_from_time(duration, prepared.project.fps, true);
    let results = prepared.results()?;
    prepared.ensure_linked_sources_current()?;
    if canceled.load(Ordering::Acquire) {
        return Err("MCP client canceled the edit before commit".to_string());
    }
    let editor_selection =
        EditorSelection::capture(selection, &live.borrow()).reconcile(&prepared.project);
    prepared.promote()?;
    if canceled.load(Ordering::Acquire) {
        prepared.discard_promoted();
        return Err("MCP client canceled the edit before commit".to_string());
    }
    if let Err(error) =
        shrimply_project::project::commit_edit_checked(&prepared.project, &history_label)
    {
        prepared.discard_promoted();
        return Err(format!(
            "MCP edit could not be committed to project history: {error}"
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
    .map_err(|error| format!("could not serialize MCP edit result: {error}"))
}

fn project_content_fingerprint(project: &Project) -> Result<Vec<u8>, String> {
    let mut content = project.clone();
    content.cursor_position = None;
    content.timeline_zoom = None;
    content.expanded_sequence_paths.clear();
    serde_json::to_vec(&content)
        .map_err(|error| format!("could not fingerprint live project: {error}"))
}
