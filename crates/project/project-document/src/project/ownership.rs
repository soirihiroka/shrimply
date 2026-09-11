use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;
use std::process::{self, Output};
use std::sync::{Mutex, OnceLock};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;

const LOCK_ACQUIRE_ATTEMPTS: usize = 8;
const PROCESS_STOP_WAIT_ATTEMPTS: usize = 20;
const PROCESS_STOP_WAIT_MILLIS: u64 = 25;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectLockOwner {
    System { pid: u32 },
    Flatpak { pid: u32, instance_id: String },
}

impl ProjectLockOwner {
    pub fn pid(&self) -> u32 {
        match self {
            Self::System { pid } | Self::Flatpak { pid, .. } => *pid,
        }
    }
}

#[derive(Debug)]
pub enum ProjectLoadError {
    LockedByOtherInstance { owner: ProjectLockOwner },
    Other(String),
}

#[derive(Debug)]
pub enum ProjectLockError {
    RegistryUnavailable,
    AlreadyLockedByThisInstance,
    AlreadyLockedByOtherInstance { owner: ProjectLockOwner },
    CouldNotInspect(String),
    CouldNotCreate(String),
}

static LOCKED_PROJECT_FILES: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

fn locked_project_files() -> &'static Mutex<HashSet<PathBuf>> {
    LOCKED_PROJECT_FILES.get_or_init(|| Mutex::new(HashSet::new()))
}

fn is_project_file(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("shrimp")
                || extension.eq_ignore_ascii_case("sjson")
                || extension.eq_ignore_ascii_case("json")
        })
}

pub fn normalized_project_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn project_lock_path(path: &Path) -> PathBuf {
    let extension = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("project");
    path.with_extension(format!("{extension}.lock"))
}

/// Returns the live process owning a project's lock, if any.
pub fn project_lock_owner(path: &Path) -> Result<Option<ProjectLockOwner>, String> {
    let path = normalized_project_path(path);
    let lock_path = project_lock_path(&path);
    if !lock_path.exists() {
        return Ok(None);
    }
    let owner = read_project_lock_owner(&lock_path).ok_or_else(|| {
        format!(
            "project lock {} does not contain a valid JSON owner",
            lock_path.display()
        )
    })?;
    if !project_lock_owner_is_running(&owner)? {
        return Err(format!(
            "project lock {} is stale (PID {} is not running)",
            lock_path.display(),
            owner.pid()
        ));
    }
    Ok(Some(owner))
}

pub fn acquire_project_lock(path: &Path) -> Result<(), ProjectLockError> {
    if !is_project_file(path) {
        return Ok(());
    }

    let canonical_path = normalized_project_path(path);
    let lock_path = project_lock_path(&canonical_path);
    let mut locks = locked_project_files()
        .lock()
        .map_err(|_| ProjectLockError::RegistryUnavailable)?;

    if locks.contains(&canonical_path) {
        return Err(ProjectLockError::AlreadyLockedByThisInstance);
    }

    let owner = current_project_lock_owner().map_err(ProjectLockError::CouldNotCreate)?;
    let contents = serde_json::to_vec(&owner)
        .map_err(|error| ProjectLockError::CouldNotCreate(error.to_string()))?;

    for _ in 0..LOCK_ACQUIRE_ATTEMPTS {
        if let Some(owner) = read_project_lock_owner(&lock_path) {
            if project_lock_owner_is_running(&owner).map_err(ProjectLockError::CouldNotInspect)? {
                return Err(ProjectLockError::AlreadyLockedByOtherInstance { owner });
            }
            let _ = fs::remove_file(&lock_path);
        } else if lock_path.exists() {
            let _ = fs::remove_file(&lock_path);
        }
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(&contents) {
                    let _ = fs::remove_file(&lock_path);
                    return Err(ProjectLockError::CouldNotCreate(error.to_string()));
                }
                locks.insert(canonical_path);
                return Ok(());
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(ProjectLockError::CouldNotCreate(error.to_string())),
        }
    }
    Err(ProjectLockError::CouldNotCreate(format!(
        "could not create lock for {}",
        canonical_path.display()
    )))
}

pub fn release_project_lock(path: &Path) {
    let canonical_path = normalized_project_path(path);
    let mut locks = match locked_project_files().lock() {
        Ok(locks) => locks,
        Err(_) => return,
    };
    if !locks.remove(&canonical_path) {
        return;
    }
    let _ = fs::remove_file(project_lock_path(&canonical_path));
}

pub fn clear_project_file_locks() {
    let mut locks = match locked_project_files().lock() {
        Ok(locks) => locks,
        Err(_) => return,
    };
    for path in locks.drain() {
        let _ = fs::remove_file(project_lock_path(&path));
    }
}

pub fn terminate_project_process(owner: &ProjectLockOwner) -> bool {
    if checked_pid(owner.pid()).is_none() {
        return false;
    }
    #[cfg(unix)]
    {
        match project_lock_owner_is_running(owner) {
            Ok(false) => return true,
            Ok(true) => {}
            Err(_) => return false,
        }
        match owner {
            ProjectLockOwner::System { pid } => {
                if !send_signal_to_system_process(*pid, libc::SIGTERM) {
                    return false;
                }
                if wait_for_process_to_stop(owner) {
                    return true;
                }
                if !send_signal_to_system_process(*pid, libc::SIGKILL) {
                    return false;
                }
                wait_for_process_to_stop(owner)
            }
            ProjectLockOwner::Flatpak { instance_id, .. } => {
                flatpak_command(&["kill", instance_id]).is_ok_and(|output| output.status.success())
                    && wait_for_process_to_stop(owner)
            }
        }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(unix)]
fn send_signal_to_system_process(pid: u32, signal: libc::c_int) -> bool {
    #[cfg(target_os = "linux")]
    if matches!(current_flatpak_instance_id(), Ok(Some(_))) {
        let signal = match signal {
            libc::SIGTERM => "-TERM",
            libc::SIGKILL => "-KILL",
            _ => return false,
        };
        return host_command("kill", &[signal, &pid.to_string()])
            .is_ok_and(|output| output.status.success());
    }

    let Some(pid) = checked_pid(pid) else {
        return false;
    };
    // SAFETY: `libc::kill` is called with a plain PID and standard signals only.
    // This mirrors existing libc usage in the codebase and does not involve pointers.
    let result = unsafe { libc::kill(pid, signal) };
    if result == 0 {
        return true;
    }
    let Some(code) = std::io::Error::last_os_error().raw_os_error() else {
        return false;
    };
    code == libc::ESRCH
}

#[cfg(unix)]
fn wait_for_process_to_stop(owner: &ProjectLockOwner) -> bool {
    for _ in 0..PROCESS_STOP_WAIT_ATTEMPTS {
        if matches!(project_lock_owner_is_running(owner), Ok(false)) {
            return true;
        }
        thread::sleep(Duration::from_millis(PROCESS_STOP_WAIT_MILLIS));
    }
    matches!(project_lock_owner_is_running(owner), Ok(false))
}

fn read_project_lock_owner(lock_path: &Path) -> Option<ProjectLockOwner> {
    fs::read(lock_path)
        .ok()
        .and_then(|contents| serde_json::from_slice(&contents).ok())
        .filter(|owner: &ProjectLockOwner| {
            checked_pid(owner.pid()).is_some()
                && match owner {
                    ProjectLockOwner::System { .. } => true,
                    ProjectLockOwner::Flatpak { instance_id, .. } => !instance_id.is_empty(),
                }
        })
}

fn current_project_lock_owner() -> Result<ProjectLockOwner, String> {
    let pid = process::id();
    #[cfg(target_os = "linux")]
    if let Some(instance_id) = current_flatpak_instance_id()? {
        return Ok(ProjectLockOwner::Flatpak { pid, instance_id });
    }
    Ok(ProjectLockOwner::System { pid })
}

fn project_lock_owner_is_running(owner: &ProjectLockOwner) -> Result<bool, String> {
    match owner {
        ProjectLockOwner::System { pid } => {
            #[cfg(target_os = "linux")]
            if current_flatpak_instance_id()?.is_some() {
                let output = host_command("test", &["-d", &format!("/proc/{pid}")])?;
                return Ok(output.status.success());
            }
            Ok(process_is_running(*pid))
        }
        ProjectLockOwner::Flatpak { instance_id, .. } => {
            let output = flatpak_command(&["ps", "--columns=instance"])?;
            if !output.status.success() {
                return Err(command_error("flatpak ps", &output));
            }
            let instances = String::from_utf8(output.stdout)
                .map_err(|error| format!("flatpak ps returned invalid UTF-8: {error}"))?;
            Ok(instances.lines().any(|instance| instance == instance_id))
        }
    }
}

#[cfg(target_os = "linux")]
fn current_flatpak_instance_id() -> Result<Option<String>, String> {
    let path = Path::new("/.flatpak-info");
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let mut instance_section = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            instance_section = line == "[Instance]";
        } else if instance_section
            && let Some(instance_id) = line.strip_prefix("instance-id=")
            && !instance_id.is_empty()
        {
            return Ok(Some(instance_id.to_string()));
        }
    }
    Err("/.flatpak-info does not contain an Instance instance-id".to_string())
}

#[cfg(target_os = "linux")]
fn host_command(program: &str, arguments: &[&str]) -> Result<Output, String> {
    let mut command = if current_flatpak_instance_id()?.is_some() {
        let mut command = Command::new("flatpak-spawn");
        command.args(["--host", "--env=LC_ALL=C", program]);
        command
    } else {
        let mut command = Command::new(program);
        command.env("LC_ALL", "C");
        command
    };
    command
        .args(arguments)
        .output()
        .map_err(|error| format!("could not run {program}: {error}"))
}

#[cfg(target_os = "linux")]
fn flatpak_command(arguments: &[&str]) -> Result<Output, String> {
    host_command("flatpak", arguments)
}

#[cfg(not(target_os = "linux"))]
fn flatpak_command(_arguments: &[&str]) -> Result<Output, String> {
    Err("Flatpak project owners can only be inspected on Linux".to_string())
}

fn command_error(command: &str, output: &Output) -> String {
    let error = String::from_utf8_lossy(&output.stderr);
    format!("{command} failed: {}", error.trim())
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    let Some(pid) = checked_pid(pid) else {
        return false;
    };
    match unsafe { libc::kill(pid, 0) } {
        0 => true,
        _ => {
            matches!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EPERM) | Some(libc::EACCES)
            )
        }
    }
}

fn checked_pid(pid: u32) -> Option<i32> {
    i32::try_from(pid).ok().filter(|pid| *pid > 0)
}

#[cfg(not(unix))]
fn process_is_running(_pid: u32) -> bool {
    false
}
