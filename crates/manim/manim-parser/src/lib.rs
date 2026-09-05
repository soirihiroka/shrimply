use hashbrown::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
mod macos_source_access;

use shrimply_asset::{Asset, AssetSnapshot};
pub use shrimply_manim_ir::{CompiledAnimation, PacketBody, Progress, ProgressStage};
use shrimply_math_core::Fraction;
use shrimply_project::project::{ManimParameter, ManimParameterValue};

mod cache;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);
const COMPILE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const PROGRESS_LOG_INTERVAL: Duration = Duration::from_secs(1);
const PROGRESS_LOG_FRAME_INTERVAL: u64 = 30;
const MAX_IR_PACKET_BYTES: usize = 256 * 1024 * 1024;
static NEXT_SOCKET: AtomicU64 = AtomicU64::new(1);
type SceneCache = HashMap<AssetSnapshot, Result<Vec<String>, String>>;
static SCENE_CACHE: OnceLock<Mutex<SceneCache>> = OnceLock::new();
static COMPILE_GATE: Mutex<()> = Mutex::new(());

struct WorkerHandle {
    child: Child,
    active: bool,
    description: WorkerDescription,
}

struct WorkerDescription {
    source: PathBuf,
    scene: String,
}

struct SocketPath(PathBuf);

impl Drop for SocketPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn check_compile_state(cancelled: &AtomicBool, started: Instant) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("Manim compilation was cancelled".to_string());
    }
    if started.elapsed() >= COMPILE_TIMEOUT {
        return Err(format!(
            "Manim compilation exceeded {} minutes",
            COMPILE_TIMEOUT.as_secs() / 60
        ));
    }
    Ok(())
}

fn read_exact_cancelled(
    socket: &mut UnixStream,
    bytes: &mut [u8],
    cancelled: &AtomicBool,
    started: Instant,
    description: &str,
) -> Result<(), String> {
    let mut offset = 0;
    while offset < bytes.len() {
        check_compile_state(cancelled, started)?;
        match socket.read(&mut bytes[offset..]) {
            Ok(0) => {
                return Err("Manim IR worker disconnected before finishing".to_string());
            }
            Ok(read) => offset += read,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(format!("{description}: {error}")),
        }
    }
    Ok(())
}

fn write_all_cancelled(
    socket: &mut UnixStream,
    bytes: &[u8],
    cancelled: &AtomicBool,
    started: Instant,
) -> Result<(), String> {
    let mut offset = 0;
    while offset < bytes.len() {
        check_compile_state(cancelled, started)?;
        match socket.write(&bytes[offset..]) {
            Ok(0) => return Err("Manim IR worker disconnected while receiving parameters".into()),
            Ok(written) => offset += written,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(format!("send Manim parameter overrides: {error}")),
        }
    }
    Ok(())
}

fn accept_worker(
    listener: &UnixListener,
    child: &mut WorkerHandle,
    cancelled: &AtomicBool,
    started: Instant,
) -> Result<UnixStream, String> {
    let socket = loop {
        check_compile_state(cancelled, started)?;
        match listener.accept() {
            Ok((socket, _)) => break socket,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if let Some(status) = child.exit_code()? {
                    return Err(format!(
                        "Manim worker exited before connecting with status {status}"
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("accept Manim worker connection: {error}")),
        }
    };
    socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| format!("configure Manim worker socket: {error}"))?;
    socket
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| format!("configure Manim worker socket: {error}"))?;
    Ok(socket)
}

fn read_packet(
    socket: &mut UnixStream,
    cancelled: &AtomicBool,
    started: Instant,
) -> Result<shrimply_manim_ir::Packet, String> {
    let mut length = [0_u8; 4];
    read_exact_cancelled(
        socket,
        &mut length,
        cancelled,
        started,
        "read Manim IR packet length",
    )?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_IR_PACKET_BYTES {
        return Err("Manim IR packet exceeds the maximum size".to_string());
    }
    let mut bytes = vec![0; length];
    read_exact_cancelled(
        socket,
        &mut bytes,
        cancelled,
        started,
        "read Manim IR packet",
    )?;
    shrimply_manim_ir::decode_packet(&bytes)
        .map_err(|error| format!("decode Manim compiler packet: {error}"))
}

fn send_parameters(
    socket: &mut UnixStream,
    parameters: &HashMap<String, ManimParameterValue>,
    cancelled: &AtomicBool,
    started: Instant,
) -> Result<(), String> {
    let parameters = rmp_serde::to_vec_named(parameters)
        .map_err(|error| format!("encode Manim parameter overrides: {error}"))?;
    let parameter_bytes = u32::try_from(parameters.len())
        .map_err(|_| "Manim parameter overrides exceed the maximum size".to_string())?;
    write_all_cancelled(socket, &parameter_bytes.to_be_bytes(), cancelled, started)?;
    write_all_cancelled(socket, &parameters, cancelled, started)
}

impl WorkerHandle {
    fn spawn(settings: &Settings, worker_socket: &Path, source: &Path) -> Result<Self, String> {
        let python_project = Path::new(env!("CARGO_MANIFEST_DIR")).join("python");
        let child = Command::new(std::env::var_os("UV").unwrap_or_else(|| "uv".into()))
            .arg("run")
            .arg("--python")
            .arg("3.14")
            .arg("--project")
            .arg(&python_project)
            .arg("python")
            .arg(python_project.join("shrimply_manim/ir_worker.py"))
            .arg("--socket")
            .arg(worker_socket)
            .arg("--source")
            .arg(source)
            .arg("--scene")
            .arg(&settings.scene)
            .arg("--width")
            .arg(settings.width.to_string())
            .arg("--height")
            .arg(settings.height.to_string())
            .arg("--fps")
            .arg(settings.fps.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .process_group(0)
            .spawn()
            .map_err(|error| format!("start one-shot Manim compiler with uv: {error}"))?;
        let id = child.id();
        tracing::info!(
            worker_id = id,
            source = %source.display(),
            scene = %settings.scene,
            width = settings.width,
            height = settings.height,
            fps = %settings.fps,
            "spawned Manim compiler",
        );
        Ok(Self {
            child,
            active: true,
            description: WorkerDescription {
                source: source.to_path_buf(),
                scene: settings.scene.clone(),
            },
        })
    }

    fn exit_code(&mut self) -> Result<Option<i32>, String> {
        match self
            .child
            .try_wait()
            .map_err(|error| format!("poll Manim compiler: {error}"))?
        {
            Some(status) => Ok(Some(status.code().unwrap_or(1))),
            None => Ok(None),
        }
    }

    fn stop(&mut self) {
        if self.active {
            let id = self.child.id();
            let process_group = i32::try_from(id).expect("Manim compiler PID exceeds i32");
            let killed = unsafe { libc::kill(-process_group, libc::SIGKILL) };
            let error = std::io::Error::last_os_error();
            if killed != 0 && error.raw_os_error() != Some(libc::ESRCH) {
                tracing::error!(
                    worker_id = id,
                    source = %self.description.source.display(),
                    scene = %self.description.scene,
                    %error,
                    "could not stop Manim compiler process group",
                );
                std::process::abort();
            }
            let result = self.child.wait();
            self.active = false;
            match result {
                Ok(_) => tracing::debug!(
                    worker_id = id,
                    source = %self.description.source.display(),
                    scene = %self.description.scene,
                    "stopped Manim compiler",
                ),
                Err(error) => tracing::warn!(
                    worker_id = id,
                    source = %self.description.source.display(),
                    scene = %self.description.scene,
                    %error,
                    "could not reap Manim compiler",
                ),
            }
        }
    }

    fn finish(&mut self, cancelled: &AtomicBool) -> Result<(), String> {
        let started = Instant::now();
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err("Manim compilation was cancelled".to_string());
            }
            if let Some(exit_code) = self.exit_code()? {
                self.active = false;
                return if exit_code == 0 {
                    tracing::debug!(
                        worker_id = self.child.id(),
                        source = %self.description.source.display(),
                        scene = %self.description.scene,
                        "Manim compiler exited",
                    );
                    Ok(())
                } else {
                    Err(format!("Manim compiler exited with status {exit_code}"))
                };
            }
            if started.elapsed() >= CONNECT_TIMEOUT {
                return Err("Manim compiler did not exit after finishing".to_string());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub source: Asset,
    pub scene: String,
    pub width: u32,
    pub height: u32,
    pub fps: Fraction,
    pub parameters: HashMap<String, ManimParameterValue>,
}

pub fn discover_scenes(source: &Asset) -> Result<Vec<String>, String> {
    let key = source.snapshot()?;
    let cache = SCENE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let mut cache = cache.lock().expect("Manim scene cache poisoned");
        cache.retain(|snapshot, _| snapshot.asset() != source || snapshot == &key);
        if let Some(result) = cache.get(&key) {
            return result.clone();
        }
    }

    #[cfg(target_os = "macos")]
    let _source_access = macos_source_access::RelatedSourceAccess::new(source.path())?;
    let python_project = Path::new(env!("CARGO_MANIFEST_DIR")).join("python");
    let output = Command::new(std::env::var_os("UV").unwrap_or_else(|| "uv".into()))
        .arg("run")
        .arg("--python")
        .arg("3.14")
        .arg("--project")
        .arg(&python_project)
        .arg("python")
        .arg(python_project.join("shrimply_manim/scene_discovery.py"))
        .arg(source.path())
        .output()
        .map_err(|error| format!("inspect Manim scenes with uv: {error}"));
    let result = output.and_then(|output| {
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        let scenes: Vec<String> = rmp_serde::from_slice(&output.stdout)
            .map_err(|error| format!("decode Manim scene list: {error}"))?;
        if scenes.is_empty() {
            return Err(format!("{} defines no Manim Scene", source.display()));
        }
        Ok(scenes)
    });
    key.verify_current()?;
    cache
        .lock()
        .expect("Manim scene cache poisoned")
        .insert(key, result.clone());
    result
}

pub fn invalidate_ir_cache(source: &Asset) -> Result<(), String> {
    if let Some(cache) = SCENE_CACHE.get() {
        cache
            .lock()
            .expect("Manim scene cache poisoned")
            .retain(|snapshot, _| snapshot.asset() != source);
    }
    cache::invalidate(source);
    Ok(())
}

pub fn compile(
    settings: &Settings,
    cancelled: &AtomicBool,
    mut on_progress: impl FnMut(Progress),
) -> Result<Arc<CompiledAnimation>, String> {
    if settings.width == 0 || settings.height == 0 {
        return Err("Manim render dimensions must be positive".to_string());
    }
    let source = settings.source.snapshot()?;
    #[cfg(target_os = "macos")]
    let _source_access = macos_source_access::RelatedSourceAccess::new(source.path())?;
    let cache_key = cache::key(settings, &source)?;
    if let Some(animation) = cache::get(&cache_key)? {
        return Ok(animation);
    }
    let waiting_for_compiler = Instant::now();
    let _compile_guard = loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Manim compilation was cancelled".to_string());
        }
        match COMPILE_GATE.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(10)),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err("Manim compiler lock is poisoned".to_string());
            }
        }
    };
    if let Some(animation) = cache::get(&cache_key)? {
        return Ok(animation);
    }
    let socket_path = std::env::temp_dir().join(format!(
        "shrimply-manim-ir-{}-{}.sock",
        std::process::id(),
        NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path)
        .map_err(|error| format!("create Manim IR worker socket: {error}"))?;
    let _socket_path = SocketPath(socket_path.clone());
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure Manim IR listener: {error}"))?;
    let started = Instant::now();
    tracing::info!(
        source = %source.path().display(),
        scene = %settings.scene,
        parameters = settings.parameters.len(),
        compiler_queue_ms = waiting_for_compiler.elapsed().as_millis(),
        "Manim compilation started",
    );
    let mut child = WorkerHandle::spawn(settings, &socket_path, source.path())?;
    let mut socket = accept_worker(&listener, &mut child, cancelled, started)?;
    send_parameters(&mut socket, &settings.parameters, cancelled, started)?;
    let mut builder = shrimply_manim_ir::CompiledAnimationBuilder::new();
    let mut progress_stage = None;
    let mut last_progress_log = Instant::now();
    let mut last_logged_frame = 0;
    let mut streamed_frames = 0;
    let mut last_progress_at = started;
    loop {
        check_compile_state(cancelled, started)?;
        let packet = read_packet(&mut socket, cancelled, started)?;
        let finished = matches!(&packet.body, PacketBody::Finished);
        if let PacketBody::Progress(progress) = &packet.body {
            if progress_stage != Some(progress.stage) {
                progress_stage = Some(progress.stage);
                tracing::info!(
                    source = %source.path().display(),
                    scene = %settings.scene,
                    stage = ?progress.stage,
                    frames = progress.completed,
                    elapsed_ms = started.elapsed().as_millis(),
                    "Manim compiler entered stage",
                );
                last_progress_log = Instant::now();
                last_logged_frame = progress.completed;
            } else if progress.stage == ProgressStage::StreamingFrames
                && (progress.completed.saturating_sub(last_logged_frame)
                    >= PROGRESS_LOG_FRAME_INTERVAL
                    || last_progress_log.elapsed() >= PROGRESS_LOG_INTERVAL)
            {
                tracing::info!(
                    source = %source.path().display(),
                    scene = %settings.scene,
                    frames = progress.completed,
                    elapsed_ms = started.elapsed().as_millis(),
                    "Manim frames streaming",
                );
                last_progress_log = Instant::now();
                last_logged_frame = progress.completed;
            }
            streamed_frames = progress.completed;
            last_progress_at = Instant::now();
            on_progress(*progress);
        } else {
            builder.ingest(packet).map_err(|error| error.to_string())?;
        }
        if finished {
            break;
        }
    }
    let final_packet_at = Instant::now();
    child.finish(cancelled)?;
    let worker_exit_at = Instant::now();
    let animation = builder
        .finish()
        .map(Arc::new)
        .map_err(|error| error.to_string())?;
    let ir_finalized_at = Instant::now();
    source.verify_current()?;
    let verified_at = Instant::now();
    let animation = cache::store(cache_key, animation);
    let cached_at = Instant::now();
    tracing::info!(
        source = %source.path().display(),
        scene = %settings.scene,
        frames = streamed_frames,
        render_is_current = animation.scene().render_is_current,
        final_packet_ms = final_packet_at.duration_since(last_progress_at).as_millis(),
        worker_exit_ms = worker_exit_at.duration_since(final_packet_at).as_millis(),
        ir_finalize_ms = ir_finalized_at.duration_since(worker_exit_at).as_millis(),
        source_verify_ms = verified_at.duration_since(ir_finalized_at).as_millis(),
        cache_store_ms = cached_at.duration_since(verified_at).as_millis(),
        elapsed_ms = started.elapsed().as_millis(),
        "Manim compilation finished",
    );
    Ok(animation)
}

pub fn compile_uncancelled(
    settings: &Settings,
    on_progress: impl FnMut(Progress),
) -> Result<Arc<CompiledAnimation>, String> {
    compile(settings, &AtomicBool::new(false), on_progress)
}

pub fn reflected_parameters(animation: &CompiledAnimation) -> Result<Vec<ManimParameter>, String> {
    rmp_serde::from_slice(&animation.scene().parameters)
        .map_err(|error| format!("decode reflected Manim parameters: {error}"))
}
