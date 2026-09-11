use serde::{Deserialize, Serialize};
use shrimply_math_core::Fraction;
use shrimply_trust_core::Child;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

const PROTOCOL_VERSION: u32 = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const MESSAGE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_FRAME_BYTES: usize = 1024 * 1024 * 1024;
const WORKER: &str = include_str!("worker.py");
static BLENDER_BINARY: LazyLock<RwLock<Option<PathBuf>>> = LazyLock::new(RwLock::default);
static METADATA_CACHE: LazyLock<Mutex<HashMap<DiscoveryKey, Metadata>>> =
    LazyLock::new(Mutex::default);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DiscoveryKey {
    binary: PathBuf,
    binary_modified: Option<SystemTime>,
    binary_size: u64,
    blend: PathBuf,
    blend_modified: Option<SystemTime>,
    blend_size: u64,
}

pub fn set_binary(binary: Option<PathBuf>) {
    *BLENDER_BINARY
        .write()
        .expect("Blender binary lock poisoned") = binary;
}

pub fn binary() -> Option<PathBuf> {
    BLENDER_BINARY
        .read()
        .expect("Blender binary lock poisoned")
        .clone()
}

pub fn file_duration(path: &Path) -> Result<Fraction, String> {
    let binary = binary().ok_or("Choose a compatible Blender binary in Preferences")?;
    let metadata = discover(&binary, path)?;
    let scene = metadata
        .scenes
        .first()
        .ok_or_else(|| format!("Blender file {} has no scene", path.display()))?;
    Ok(scene.duration())
}

#[derive(Clone, Debug, Deserialize)]
pub struct Metadata {
    pub scenes: Vec<SceneMetadata>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SceneMetadata {
    pub name: String,
    pub view_layers: Vec<String>,
    pub cameras: Vec<String>,
    pub active_view_layer: String,
    pub active_camera: String,
    pub frame_start: i64,
    pub frame_end: i64,
    pub fps_numerator: u64,
    pub fps_denominator: u64,
}

impl SceneMetadata {
    pub fn duration(&self) -> Fraction {
        let frames = self
            .frame_end
            .saturating_sub(self.frame_start)
            .saturating_add(1);
        Fraction::new(
            (frames.max(1) as u64).saturating_mul(self.fps_denominator),
            self.fps_numerator.max(1),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMethod {
    Solid,
    MaterialPreview,
    SceneRenderer,
}

pub struct RenderedFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WireRequest<'a> {
    Render {
        scene: &'a str,
        view_layer: &'a str,
        camera: &'a str,
        method: RenderMethod,
        width: u32,
        height: u32,
        time_numerator: u64,
        time_denominator: u64,
    },
    Shutdown,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Response {
    Hello {
        protocol: u32,
    },
    Metadata {
        scenes: Vec<SceneMetadata>,
    },
    Frame {
        width: u32,
        height: u32,
        byte_len: u64,
        pixel_format: PixelFormat,
    },
    Error {
        message: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PixelFormat {
    Rgba,
    Bgra,
}

pub struct Session {
    child: Child,
    socket: UnixStream,
    _worker_file: tempfile::NamedTempFile,
    _stderr_file: tempfile::NamedTempFile,
    metadata: Metadata,
}

pub struct RenderRequest<'a> {
    pub scene: &'a str,
    pub view_layer: &'a str,
    pub camera: &'a str,
    pub method: RenderMethod,
    pub width: u32,
    pub height: u32,
    pub time: Fraction,
}

pub fn probe(binary: &Path) -> Result<(), String> {
    let mut session = Session::spawn(binary, None)?;
    session.shutdown()
}

pub fn discover(binary: &Path, blend: &Path) -> Result<Metadata, String> {
    let blend = shrimply_trust_core::require(blend)?;
    let blend = blend.as_path();
    let key = DiscoveryKey::new(binary, blend);
    if let Some(metadata) = METADATA_CACHE
        .lock()
        .expect("Blender metadata cache lock poisoned")
        .get(&key)
        .cloned()
    {
        return Ok(metadata);
    }
    let mut session = Session::spawn(binary, Some(blend))?;
    let metadata = session.metadata.clone();
    session.shutdown()?;
    let mut cache = METADATA_CACHE
        .lock()
        .expect("Blender metadata cache lock poisoned");
    cache.retain(|cached, _| cached.binary != binary || cached.blend != blend);
    cache.insert(key, metadata.clone());
    Ok(metadata)
}

pub fn invalidate_metadata(blend: &Path) {
    METADATA_CACHE
        .lock()
        .expect("Blender metadata cache lock poisoned")
        .retain(|key, _| key.blend != blend);
}

impl DiscoveryKey {
    fn new(binary: &Path, blend: &Path) -> Self {
        let binary_metadata = binary.metadata().ok();
        let blend_metadata = blend.metadata().ok();
        Self {
            binary: binary.to_path_buf(),
            binary_modified: binary_metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok()),
            binary_size: binary_metadata.as_ref().map_or(0, std::fs::Metadata::len),
            blend: blend.to_path_buf(),
            blend_modified: blend_metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok()),
            blend_size: blend_metadata.as_ref().map_or(0, std::fs::Metadata::len),
        }
    }
}

impl Session {
    pub fn open(binary: &Path, blend: &Path) -> Result<Self, String> {
        Self::spawn(binary, Some(blend))
    }

    fn spawn(binary: &Path, blend: Option<&Path>) -> Result<Self, String> {
        let blend = blend.map(shrimply_trust_core::require).transpose()?;
        let blend = blend.as_deref();
        if !binary.is_file() {
            return Err(format!(
                "Blender binary does not exist: {}",
                binary.display()
            ));
        }
        let socket_dir = tempfile::tempdir()
            .map_err(|error| format!("create Blender socket directory: {error}"))?;
        let socket_path = socket_dir.path().join("worker.sock");
        let listener = UnixListener::bind(&socket_path)
            .map_err(|error| format!("bind Blender worker socket: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("configure Blender worker listener: {error}"))?;
        let mut worker_file = tempfile::Builder::new()
            .prefix("shrimply-blender-")
            .suffix(".py")
            .tempfile()
            .map_err(|error| format!("create Blender worker script: {error}"))?;
        worker_file
            .write_all(WORKER.as_bytes())
            .map_err(|error| format!("write Blender worker script: {error}"))?;
        let stderr_file = tempfile::Builder::new()
            .prefix("shrimply-blender-stderr-")
            .tempfile()
            .map_err(|error| format!("create Blender stderr capture: {error}"))?;

        let mut command = Command::new(binary);
        command.arg("--factory-startup").arg("--background");
        if let Some(blend) = blend {
            command.arg("--enable-autoexec").arg(blend);
        }
        command
            .arg("--python")
            .arg(worker_file.path())
            .arg("--")
            .arg("--socket")
            .arg(&socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(
                stderr_file
                    .reopen()
                    .map_err(|error| format!("capture Blender stderr: {error}"))?,
            )
            .process_group(0);
        let mut child = Child::spawn(&mut command, blend)
            .map_err(|error| format!("start Blender worker: {error}"))?;
        let started = Instant::now();
        let socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if let Some(status) = child
                        .try_wait()
                        .map_err(|error| format!("poll Blender worker: {error}"))?
                    {
                        return Err(worker_exit_error(
                            "Blender worker exited before connecting",
                            status,
                            &stderr_file,
                        ));
                    }
                    if started.elapsed() >= CONNECT_TIMEOUT {
                        terminate(&mut child);
                        let output = worker_output(&stderr_file);
                        return Err(if output.is_empty() {
                            "Blender worker did not connect within 30 seconds".into()
                        } else {
                            format!("Blender worker did not connect within 30 seconds\n\n{output}")
                        });
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(format!("accept Blender worker connection: {error}")),
            }
        };
        drop(socket_dir);
        // macOS inherits the listener's nonblocking mode on accepted sockets.
        socket
            .set_nonblocking(false)
            .map_err(|error| format!("configure Blender worker socket: {error}"))?;
        socket
            .set_read_timeout(Some(MESSAGE_TIMEOUT))
            .map_err(|error| format!("configure Blender worker socket: {error}"))?;
        socket
            .set_write_timeout(Some(MESSAGE_TIMEOUT))
            .map_err(|error| format!("configure Blender worker socket: {error}"))?;
        let mut session = Self {
            child,
            socket,
            _worker_file: worker_file,
            _stderr_file: stderr_file,
            metadata: Metadata { scenes: Vec::new() },
        };
        match session.receive()? {
            Response::Hello { protocol } if protocol == PROTOCOL_VERSION => {}
            Response::Hello { protocol } => {
                return Err(format!("unsupported Blender worker protocol {protocol}"));
            }
            Response::Error { message } => return Err(message),
            _ => return Err("Blender worker did not send its protocol handshake".into()),
        }
        session.metadata = match session.receive()? {
            Response::Metadata { scenes } => Metadata { scenes },
            Response::Error { message } => return Err(message),
            _ => return Err("Blender worker did not send scene metadata".into()),
        };
        Ok(session)
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn render(&mut self, request: RenderRequest<'_>) -> Result<RenderedFrame, String> {
        self.child.check_trust()?;
        self.send(&WireRequest::Render {
            scene: request.scene,
            view_layer: request.view_layer,
            camera: request.camera,
            method: request.method,
            width: request.width,
            height: request.height,
            time_numerator: *request.time.numer().unwrap_or(&0),
            time_denominator: *request.time.denom().unwrap_or(&1),
        })?;
        match self.receive()? {
            Response::Frame {
                width,
                height,
                byte_len,
                pixel_format,
            } => {
                let expected = usize::try_from(width)
                    .ok()
                    .and_then(|width| usize::try_from(height).ok().map(|height| (width, height)))
                    .and_then(|(width, height)| width.checked_mul(height))
                    .and_then(|pixels| pixels.checked_mul(4))
                    .ok_or_else(|| "Blender frame dimensions overflow".to_string())?;
                let byte_len = usize::try_from(byte_len)
                    .map_err(|_| "Blender frame is too large for this system".to_string())?;
                if byte_len != expected || byte_len > MAX_FRAME_BYTES {
                    return Err(format!(
                        "Blender sent {byte_len} bytes for a {width}x{height} RGBA frame"
                    ));
                }
                let mut pixels = vec![0; byte_len];
                self.socket
                    .read_exact(&mut pixels)
                    .map_err(|error| format!("read Blender frame pixels: {error}"))?;
                if matches!(pixel_format, PixelFormat::Bgra) {
                    for pixel in pixels.chunks_exact_mut(4) {
                        pixel.swap(0, 2);
                    }
                }
                let row_bytes = width as usize * 4;
                for top in 0..height as usize / 2 {
                    let bottom = height as usize - top - 1;
                    let bottom_start = bottom * row_bytes;
                    let (before_bottom, bottom_and_after) = pixels.split_at_mut(bottom_start);
                    before_bottom[top * row_bytes..(top + 1) * row_bytes]
                        .swap_with_slice(&mut bottom_and_after[..row_bytes]);
                }
                Ok(RenderedFrame {
                    pixels,
                    width,
                    height,
                })
            }
            Response::Error { message } => Err(message),
            _ => Err("unexpected Blender worker response while rendering".into()),
        }
    }

    fn send(&mut self, request: &WireRequest<'_>) -> Result<(), String> {
        let bytes = serde_json::to_vec(request)
            .map_err(|error| format!("encode Blender worker request: {error}"))?;
        let length = u32::try_from(bytes.len())
            .map_err(|_| "Blender worker request is too large".to_string())?;
        self.socket
            .write_all(&length.to_be_bytes())
            .and_then(|()| self.socket.write_all(&bytes))
            .map_err(|error| format!("send Blender worker request: {error}"))
    }

    fn receive(&mut self) -> Result<Response, String> {
        let mut length = [0_u8; 4];
        self.socket
            .read_exact(&mut length)
            .map_err(|error| format!("receive Blender worker response: {error}"))?;
        let length = u32::from_be_bytes(length) as usize;
        if length > MAX_MESSAGE_BYTES {
            return Err("Blender worker response is too large".into());
        }
        let mut bytes = vec![0; length];
        self.socket
            .read_exact(&mut bytes)
            .map_err(|error| format!("read Blender worker response: {error}"))?;
        let response = serde_json::from_slice(&bytes)
            .map_err(|error| format!("decode Blender worker response: {error}"))?;
        Ok(response)
    }

    fn shutdown(&mut self) -> Result<(), String> {
        let _ = self.send(&WireRequest::Shutdown);
        match self.child.wait() {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(format!("Blender worker exited with {status}")),
            Err(error) => Err(format!("wait for Blender worker: {error}")),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            terminate(&mut self.child);
        }
    }
}

fn terminate(child: &mut Child) {
    child.terminate();
}

fn worker_exit_error(
    context: &str,
    status: std::process::ExitStatus,
    stderr: &tempfile::NamedTempFile,
) -> String {
    let output = worker_output(stderr);
    if output.is_empty() {
        format!("{context}: {status}")
    } else {
        format!("{context}: {status}\n\n{output}")
    }
}

fn worker_output(stderr: &tempfile::NamedTempFile) -> String {
    let Ok(bytes) = std::fs::read(stderr.path()) else {
        return String::new();
    };
    const MAX_CAPTURE_BYTES: usize = 16 * 1024;
    let start = bytes.len().saturating_sub(MAX_CAPTURE_BYTES);
    String::from_utf8_lossy(&bytes[start..]).trim().to_string()
}

pub fn binary_label(path: Option<&Path>) -> String {
    path.map_or_else(
        || "Not configured".to_string(),
        |path| path.display().to_string(),
    )
}

pub fn canonical_binary(path: &Path) -> Result<PathBuf, String> {
    File::open(path).map_err(|error| format!("open Blender binary: {error}"))?;
    path.canonicalize()
        .map_err(|error| format!("resolve Blender binary: {error}"))
}
