use crate::protocol::{BridgeCommand, BridgeRequest, BridgeResponse, BuildVersion};
use sha2::{Digest, Sha256};
use shrimply_local_ipc::Stream;
use shrimply_project_document::project::ProjectLockOwner;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
#[cfg(not(target_os = "android"))]
use std::process::{Child, Command, Stdio};
use std::time::Duration;
#[cfg(not(target_os = "android"))]
use std::time::Instant;

const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
#[cfg(not(target_os = "android"))]
const START_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(not(target_os = "android"))]
const START_POLL_INTERVAL: Duration = Duration::from_millis(100);
const FLATPAK_INSTANCE_HASH_LENGTH: usize = 16;

#[derive(Debug)]
pub enum BridgeError {
    Transport(String),
    Rejected(String),
}

#[cfg(target_os = "linux")]
fn spawn_flatpak(command: &Command) -> Result<u32, BridgeError> {
    use gio::glib::{
        VariantDict,
        variant::{Handle, ToVariant},
    };
    use std::collections::BTreeMap;

    let null = || {
        std::fs::File::options()
            .read(true)
            .write(true)
            .open("/dev/null")
            .map_err(|error| BridgeError::Transport(format!("could not open /dev/null: {error}")))
    };
    let fds = gio::UnixFDList::from_array([null()?, null()?, null()?]);
    let argv: Vec<_> = std::iter::once(command.get_program())
        .chain(command.get_args())
        .collect();
    let descriptors = BTreeMap::from([(0_u32, Handle(0)), (1, Handle(1)), (2, Handle(2))]);
    // The default flags keep the calling deployment and leave the daemon running after CLI exit.
    let parameters = (
        Path::new("/"),
        argv,
        descriptors,
        BTreeMap::<String, String>::new(),
        0_u32,
        VariantDict::default(),
    )
        .to_variant();
    let (reply, _) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .and_then(|bus| {
            bus.call_with_unix_fd_list_sync(
                Some("org.freedesktop.portal.Flatpak"),
                "/org/freedesktop/portal/Flatpak",
                "org.freedesktop.portal.Flatpak",
                "Spawn",
                Some(&parameters),
                None,
                gio::DBusCallFlags::NONE,
                WRITE_TIMEOUT.as_millis() as i32,
                Some(&fds),
                gio::Cancellable::NONE,
            )
        })
        .map_err(|error| {
            BridgeError::Transport(format!("could not start Flatpak session: {error}"))
        })?;
    reply
        .get::<(u32,)>()
        .map(|(pid,)| pid)
        .ok_or_else(|| BridgeError::Transport("Flatpak returned an invalid session PID".into()))
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) | Self::Rejected(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for BridgeError {}

pub fn runtime_directory() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(path).join("shrimply"));
    }
    Ok(shrimply_path_core::cache_directory()?.join("run"))
}

pub fn socket_path(owner: &ProjectLockOwner) -> Result<PathBuf, String> {
    let filename = match owner {
        ProjectLockOwner::System { pid } => format!("editing-{pid}.sock"),
        ProjectLockOwner::Flatpak { pid, instance_id } => {
            // Keep namespace identity stable across builds without exceeding Unix socket path limits.
            let instance_hash = Sha256::digest(instance_id.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            format!(
                "editing-{}-{pid}.sock",
                &instance_hash[..FLATPAK_INSTANCE_HASH_LENGTH]
            )
        }
    };
    Ok(runtime_directory()?.join(filename))
}

pub struct Bridge {
    project_path: String,
    socket_path: PathBuf,
    stream: Stream,
    pub owner: shrimply_project_document::project::ProjectLockOwner,
}

impl Bridge {
    pub fn connect(path: &Path) -> Result<Self, BridgeError> {
        let path = shrimply_project_document::project::normalized_project_path(path);
        let owner = shrimply_project_document::project::project_lock_record(&path)
            .map_err(BridgeError::Transport)?
            .ok_or_else(|| {
                BridgeError::Transport(format!("{} has no project session", path.display()))
            })?;
        let socket_path = socket_path(&owner).map_err(BridgeError::Transport)?;
        let stream = Stream::connect(&socket_path).map_err(|error| {
            BridgeError::Transport(format!(
                "project session at {}: {error}; inspect state before retrying",
                socket_path.display()
            ))
        })?;
        Ok(Self {
            project_path: path
                .to_str()
                .ok_or_else(|| BridgeError::Transport("project path must be UTF-8".into()))?
                .into(),
            socket_path,
            stream,
            owner,
        })
    }

    /// Connect to the active owner, or start the local editing service.
    #[cfg(target_os = "android")]
    pub fn connect_or_launch(path: &Path) -> Result<Self, BridgeError> {
        Self::connect(path)
    }

    #[cfg(not(target_os = "android"))]
    pub fn connect_or_launch(path: &Path) -> Result<Self, BridgeError> {
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            let error = match Self::connect(path) {
                Ok(bridge) => return Ok(bridge),
                Err(error) => error,
            };
            // A working socket is sufficient for editing. Inspect the owner before
            // replacing an unreachable session, which can require a host Flatpak call.
            if shrimply_project_document::project::project_lock_owner(path)
                .map_err(BridgeError::Transport)?
                .is_none()
            {
                return Self::launch(path);
            }
            if Instant::now() >= deadline {
                return Err(error);
            }
            std::thread::sleep(START_POLL_INTERVAL);
        }
    }

    #[cfg(not(target_os = "android"))]
    pub fn executable() -> Result<PathBuf, BridgeError> {
        std::env::current_exe().map_err(|error| BridgeError::Transport(error.to_string()))
    }

    /// Start the same binary as a detached project owner; concurrent callers reuse the winner.
    #[cfg(not(target_os = "android"))]
    fn launch(path: &Path) -> Result<Self, BridgeError> {
        let project_path =
            std::path::absolute(path).map_err(|error| BridgeError::Transport(error.to_string()))?;
        let mut command = Command::new(Self::executable()?);
        command
            .args(["project", "serve", "--project"])
            .arg(project_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "linux")]
        if Path::new("/.flatpak-info").is_file() {
            // Bubblewrap keeps the caller's stdio open until every sandbox child exits.
            return Self::wait_for_owner(path, spawn_flatpak(&command)?, None);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
            command.creation_flags(CREATE_NEW_PROCESS_GROUP.0 | CREATE_NO_WINDOW.0);
        }
        let child = command.spawn().map_err(|error| {
            BridgeError::Transport(format!("could not start headless session: {error}"))
        })?;
        Self::wait_for_owner(path, child.id(), Some(child))
    }

    #[cfg(not(target_os = "android"))]
    fn wait_for_owner(
        path: &Path,
        pid: u32,
        mut child: Option<Child>,
    ) -> Result<Self, BridgeError> {
        let deadline = Instant::now() + START_TIMEOUT;
        let result = loop {
            if let Ok(bridge) = Self::connect(path) {
                break Ok(bridge);
            }
            if let Some(child) = &mut child
                && let Some(status) = child
                    .try_wait()
                    .map_err(|error| BridgeError::Transport(error.to_string()))?
            {
                let other_owner = shrimply_project_document::project::project_lock_owner(path)
                    .ok()
                    .flatten()
                    .is_some();
                if !other_owner || Instant::now() >= deadline {
                    let detail = format!(
                        "headless session exited with {status}; run shrimply project serve --project {} to inspect startup",
                        path.display()
                    );
                    break Err(BridgeError::Transport(detail));
                }
            }
            if Instant::now() >= deadline {
                break Err(BridgeError::Transport(format!(
                    "headless session PID {pid} did not become ready within {} seconds; inspect its project state before retrying",
                    START_TIMEOUT.as_secs()
                )));
            }
            std::thread::sleep(START_POLL_INTERVAL);
        };
        if let Some(mut child) = child {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        result
    }

    /// Send one request on this connection, then close it, including on failure.
    /// The request validates project ownership; no separate queued handshake is needed.
    pub fn request(self, command: BridgeCommand) -> Result<serde_json::Value, BridgeError> {
        self.request_with_timeout(command, RESPONSE_TIMEOUT)
    }

    pub fn version(self) -> Result<BuildVersion, BridgeError> {
        let value = self.request_with_timeout(BridgeCommand::Version, WRITE_TIMEOUT)?;
        serde_json::from_value(value).map_err(|error| BridgeError::Transport(error.to_string()))
    }

    #[cfg(not(target_os = "android"))]
    pub fn close(self, if_revision: Option<String>) -> Result<serde_json::Value, BridgeError> {
        let path = PathBuf::from(&self.project_path);
        let owner = self.owner.clone();
        let mut result = self.request(BridgeCommand::Execute(crate::protocol::ApiCall {
            name: "project_close".into(),
            arguments: serde_json::json!({}),
            dry_run: false,
            if_revision,
        }))?;
        if result["closing"] != true {
            return Err(BridgeError::Rejected(
                "The local service did not confirm it saved and shut down.".into(),
            ));
        }
        let deadline = Instant::now() + START_TIMEOUT;
        while shrimply_project_document::project::project_lock_owner(&path)
            .map_err(BridgeError::Transport)?
            .as_ref()
            == Some(&owner)
        {
            if Instant::now() >= deadline {
                return Err(BridgeError::Transport(
                    "project session accepted close but has not released the project within 30 seconds".into(),
                ));
            }
            std::thread::sleep(START_POLL_INTERVAL);
        }
        result
            .as_object_mut()
            .expect("close response is an object")
            .remove("closing");
        result["closed"] = serde_json::json!(true);
        Ok(result)
    }

    /// Interactive clients can bound how long an active owner may take to reply.
    pub fn request_with_timeout(
        mut self,
        command: BridgeCommand,
        timeout: Duration,
    ) -> Result<serde_json::Value, BridgeError> {
        let transport = |error: std::io::Error| {
            BridgeError::Transport(format!(
                "project session at {}: {error}; inspect state before retrying",
                self.socket_path.display()
            ))
        };
        let stream = &mut self.stream;
        stream
            .set_write_timeout(Some(WRITE_TIMEOUT.min(timeout)))
            .map_err(transport)?;
        stream.set_read_timeout(Some(timeout)).map_err(transport)?;
        let request = BridgeRequest {
            project_path: self.project_path.clone(),
            command,
        };
        let mut encoded = serde_json::to_vec(&request)
            .map_err(|error| BridgeError::Transport(error.to_string()))?;
        encoded.push(b'\n');
        stream.write_all(&encoded).map_err(transport)?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).map_err(|error| {
            if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) {
                BridgeError::Transport(format!("The project session did not respond within {} seconds. Try opening it again.", timeout.as_secs()))
            } else {
                transport(error)
            }
        })?;
        let response: BridgeResponse = serde_json::from_str(&line).map_err(|error| {
            BridgeError::Transport(format!("invalid project session response: {error}"))
        })?;
        if response.project_path.is_empty()
            && let Some(error) = response.error
        {
            return Err(BridgeError::Transport(error));
        }
        if response.project_path != self.project_path {
            return Err(BridgeError::Rejected(
                "session owns a different project".into(),
            ));
        }
        match (response.result, response.error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => Err(BridgeError::Rejected(error)),
            _ => Err(BridgeError::Transport(
                "invalid project session response envelope".into(),
            )),
        }
    }
}
