#[cfg(not(target_os = "android"))]
use std::path::Path;
use std::{cmp::Ordering, fmt, path::PathBuf};

use shrimply_editing_api::protocol::BuildVersion;
#[cfg(not(target_os = "android"))]
use shrimply_editing_api::{
    bridge::{Bridge, BridgeError},
    protocol::BridgeCommand,
};
use shrimply_project_document::project::ProjectLockOwner;
use shrimply_project_service::{ProjectClient, SERVER_PROTOCOL, ServerVersion};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionMismatch {
    Local {
        owner: ProjectLockOwner,
        service: BuildVersion,
    },
    Remote {
        server: String,
        service: ServerVersion,
    },
}

#[derive(Clone, Debug)]
pub enum MatchVersion {
    RestartService(ProjectLockOwner),
    RestartEditor(PathBuf),
}

impl MatchVersion {
    pub fn label(&self) -> &'static str {
        match self {
            Self::RestartService(_) => "Restart Local Service",
            Self::RestartEditor(_) => "Restart Shrimply",
        }
    }

    /// Returns true when the current editor should retry opening the project.
    #[cfg(not(target_os = "android"))]
    pub fn apply(self, path: &Path) -> Result<bool, String> {
        match self {
            Self::RestartService(owner) => {
                let bridge = Bridge::connect(path).map_err(|error| error.to_string())?;
                if bridge.owner != owner {
                    return Err(
                        "The local service changed. Reopen the project to check its version."
                            .into(),
                    );
                }
                bridge.close(None).map_err(|error| error.to_string())?;
                Ok(true)
            }
            Self::RestartEditor(executable) => {
                std::process::Command::new(executable)
                    .arg(path)
                    .spawn()
                    .map_err(|error| error.to_string())?;
                Ok(false)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct VersionPrompt {
    pub mismatch: VersionMismatch,
    pub action: Option<MatchVersion>,
}

impl VersionPrompt {
    pub fn remote(server: &str) -> Option<Self> {
        let service = ProjectClient::configured(server).ok()?.version().ok()?;
        if service.matches(env!("CARGO_PKG_VERSION")) {
            return None;
        }
        Some(Self {
            mismatch: VersionMismatch::Remote {
                server: server.to_owned(),
                service,
            },
            action: None,
        })
    }
}

impl fmt::Display for VersionPrompt {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        let app = BuildVersion::current(env!("CARGO_PKG_VERSION"));
        match &self.mismatch {
            VersionMismatch::Local { service, .. } => {
                let guidance = match compare_builds(&app, service) {
                    Ordering::Less => "Update Shrimply or restart it with the matching build.",
                    Ordering::Greater => "Restart the local service with this build of Shrimply.",
                    Ordering::Equal => {
                        "Use the same build of Shrimply for the app and local service."
                    }
                };
                write!(
                    output,
                    "Shrimply is running build {} with editing protocol {}. The local service is running build {} with protocol {}.\n\n{guidance}",
                    build_label(&app),
                    app.protocol,
                    build_label(service),
                    service.protocol
                )
            }
            VersionMismatch::Remote { server, service } => {
                let client_order = SERVER_PROTOCOL.cmp(&service.protocol).then_with(|| {
                    service
                        .version
                        .as_deref()
                        .map_or(Ordering::Equal, |version| {
                            compare_releases(&app.version, version)
                        })
                });
                let guidance = match client_order {
                    Ordering::Less => "Update Shrimply to match this project server.",
                    Ordering::Greater => "Update the project server to match Shrimply.",
                    Ordering::Equal => "Use matching releases of Shrimply and the project server.",
                };
                write!(
                    output,
                    "Shrimply {} expects project server protocol {}. The server at {server} reports version {} with protocol {}.\n\n{guidance}",
                    app.version,
                    SERVER_PROTOCOL,
                    service.version.as_deref().unwrap_or("unknown"),
                    service.protocol
                )
            }
        }?;
        write!(
            output,
            "\n\nForce Open may fail if the versions are incompatible."
        )
    }
}

fn build_label(version: &BuildVersion) -> String {
    match version.git_hash.as_deref().and_then(|hash| hash.get(..7)) {
        Some(hash) => format!("{} ({hash})", version.version),
        None => version.version.clone(),
    }
}

fn compare_releases(client: &str, service: &str) -> Ordering {
    let parse = |version: &str| {
        version
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    };
    parse(client)
        .zip(parse(service))
        .map_or(Ordering::Equal, |(client, service)| client.cmp(&service))
}

fn compare_builds(client: &BuildVersion, service: &BuildVersion) -> Ordering {
    client
        .protocol
        .cmp(&service.protocol)
        .then_with(|| compare_releases(&client.version, &service.version))
        .then_with(|| {
            client
                .source_time
                .zip(service.source_time)
                .map_or(Ordering::Equal, |(client, service)| client.cmp(&service))
        })
}

#[cfg(not(target_os = "android"))]
type CheckedOwner = (ProjectLockOwner, Result<(), String>);

#[cfg(not(target_os = "android"))]
static CHECKED_OWNER: std::sync::Mutex<Option<CheckedOwner>> = std::sync::Mutex::new(None);

#[cfg(not(target_os = "android"))]
pub(super) fn checked_bridge(
    path: &Path,
    forced: &[VersionMismatch],
) -> Result<Bridge, super::OpenError> {
    check_bridge(path, Bridge::connect_or_launch(path)?, forced)
}

#[cfg(not(target_os = "android"))]
pub(super) fn reconnect(path: &Path) -> Result<Bridge, super::OpenError> {
    let bridge = Bridge::connect_or_launch(path)?;
    if let Some((owner, result)) = &*CHECKED_OWNER.lock().expect("checked service owner")
        && owner == &bridge.owner
    {
        result.clone()?;
        return Ok(bridge);
    }
    let owner = bridge.owner.clone();
    let result = check_bridge(path, bridge, &[]);
    if let Err(super::OpenError::Version(prompt)) = &result {
        *CHECKED_OWNER.lock().expect("checked service owner") =
            Some((owner, Err(prompt.to_string())));
    }
    result
}

#[cfg(not(target_os = "android"))]
fn check_bridge(
    path: &Path,
    bridge: Bridge,
    forced: &[VersionMismatch],
) -> Result<Bridge, super::OpenError> {
    let owner = bridge.owner.clone();
    let service = bridge.version()?;
    let app = BuildVersion::current(env!("CARGO_PKG_VERSION"));
    if !app.matches(&service) {
        let mismatch = VersionMismatch::Local {
            owner: owner.clone(),
            service: service.clone(),
        };
        if !forced.contains(&mismatch) {
            let action = match compare_builds(&app, &service) {
                Ordering::Greater => Bridge::connect(path)
                    .ok()
                    .filter(|bridge| bridge.owner == owner)
                    .and_then(|bridge| {
                        bridge
                            .request_with_timeout(BridgeCommand::Handshake, super::REQUEST_TIMEOUT)
                            .ok()
                    })
                    .filter(|reply| reply["mode"] == "headless")
                    .map(|_| MatchVersion::RestartService(owner.clone())),
                Ordering::Less => updated_editor(&service).map(MatchVersion::RestartEditor),
                Ordering::Equal => None,
            };
            return Err(super::OpenError::Version(Box::new(VersionPrompt {
                mismatch,
                action,
            })));
        }
    }
    let bridge = Bridge::connect(path)?;
    if bridge.owner != owner {
        return Err(BridgeError::Transport(
            "The local service changed while checking its version. Reopen the project to retry."
                .into(),
        )
        .into());
    }
    *CHECKED_OWNER.lock().expect("checked service owner") = Some((owner, Ok(())));
    Ok(bridge)
}

#[cfg(not(target_os = "android"))]
fn updated_editor(service: &BuildVersion) -> Option<PathBuf> {
    let executable = Bridge::executable().ok()?;
    let output = std::process::Command::new(&executable)
        .arg("version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let app: BuildVersion = serde_json::from_value(value.get("cli")?.clone()).ok()?;
    app.matches(service).then_some(executable)
}
