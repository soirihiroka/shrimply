use crate::protocol::{BridgeCommand, BridgeRequest, BridgeResponse};
use sha2::{Digest, Sha256};
use shrimply_local_ipc::Stream;
use shrimply_project_document::project::ProjectLockOwner;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const START_TIMEOUT: Duration = Duration::from_secs(30);
const START_POLL_INTERVAL: Duration = Duration::from_millis(100);
const FLATPAK_INSTANCE_HASH_LENGTH: usize = 16;

#[derive(Debug)]
pub enum BridgeError {
    Transport(String),
    Rejected(String),
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
        let owner = shrimply_project_document::project::project_lock_owner(&path)
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

    /// Start the same binary as a detached project owner; concurrent callers reuse the winner.
    pub fn launch(path: &Path) -> Result<Self, BridgeError> {
        let executable =
            std::env::current_exe().map_err(|error| BridgeError::Transport(error.to_string()))?;
        let mut command = Command::new(executable);
        command
            .args(["project", "serve", "--project"])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
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
        let mut child = command.spawn().map_err(|error| {
            BridgeError::Transport(format!("could not start headless session: {error}"))
        })?;
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            if let Ok(bridge) = Self::connect(path) {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(bridge);
            }
            if let Some(status) = child
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
                    return Err(BridgeError::Transport(detail));
                }
            }
            if Instant::now() >= deadline {
                let pid = child.id();
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Err(BridgeError::Transport(format!(
                    "headless session PID {pid} did not become ready within {} seconds; inspect its project state before retrying",
                    START_TIMEOUT.as_secs()
                )));
            }
            std::thread::sleep(START_POLL_INTERVAL);
        }
    }

    /// Send one request on this connection, then close it, including on failure.
    /// The request validates project ownership; no separate queued handshake is needed.
    pub fn request(self, command: BridgeCommand) -> Result<serde_json::Value, BridgeError> {
        self.request_with_timeout(command, RESPONSE_TIMEOUT)
    }

    /// Interactive clients can bound how long an active owner may take to reply.
    pub fn request_with_timeout(mut self, command: BridgeCommand, timeout: Duration) -> Result<serde_json::Value, BridgeError> {
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
        stream
            .set_read_timeout(Some(timeout))
            .map_err(transport)?;
        let request = BridgeRequest {
            project_path: self.project_path.clone(),
            command,
        };
        let mut encoded = serde_json::to_vec(&request)
            .map_err(|error| BridgeError::Transport(error.to_string()))?;
        encoded.push(b'\n');
        stream.write_all(&encoded).map_err(transport)?;
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .map_err(transport)?;
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
