//! Native project-directory events; idle projects perform no polling or I/O.
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

pub(super) const PROJECT: u8 = 1;
pub(super) const ASSETS: u8 = 2;

pub(super) struct Watch {
    _watcher: RecommendedWatcher,
    pending: Arc<AtomicU8>,
}
impl Watch {
    pub fn new(path: &Path, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        let pending = Arc::new(AtomicU8::new(0));
        let changes = pending.clone();
        let project = path.to_owned();
        let root = path
            .parent()
            .ok_or("project requires a directory")?
            .to_owned();
        let watched_root = root.clone();
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            let flags = match result {
                Ok(event)
                    if matches!(
                        event.kind,
                        EventKind::Create(_)
                            | EventKind::Modify(_)
                            | EventKind::Remove(_)
                            | EventKind::Any
                            | EventKind::Other
                    ) =>
                {
                    event
                        .paths
                        .iter()
                        .filter(|path| path.starts_with(&watched_root))
                        .fold(0, |flags, path| {
                            if path
                                .strip_prefix(&watched_root)
                                .expect("filtered project path")
                                .components()
                                .any(|part| {
                                    let name = part.as_os_str().to_string_lossy();
                                    matches!(name.as_ref(), ".snapshots" | ".cache")
                                        || internal_file(
                                            &name,
                                            project
                                                .file_name()
                                                .and_then(|name| name.to_str())
                                                .expect("project filename"),
                                        )
                                })
                            {
                                flags
                            } else {
                                flags | if path == &project { PROJECT } else { ASSETS }
                            }
                        })
                }
                Ok(_) => 0,
                Err(_) => PROJECT | ASSETS,
            };
            if flags != 0 && changes.fetch_or(flags, Ordering::AcqRel) == 0 {
                wake();
            }
        })
        .map_err(|error| format!("could not create project watcher: {error}"))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| format!("could not watch {}: {error}", root.display()))?;
        Ok(Self {
            _watcher: watcher,
            pending,
        })
    }
    pub fn take(&self) -> u8 {
        self.pending.swap(0, Ordering::AcqRel)
    }
}

fn internal_file(name: &str, project_name: &str) -> bool {
    name.strip_suffix(".lock").is_some_and(|name| {
        [".shrimp", ".sjson", ".json"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
    }) || name
        .strip_prefix(".incoming-")
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        || name
            .strip_prefix(project_name)
            .and_then(|name| name.strip_prefix(".tmp-"))
            .is_some_and(|generation| {
                let mut parts = generation.split('-');
                let numeric =
                    |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
                parts.next().is_some_and(numeric)
                    && parts.next().is_none_or(numeric)
                    && parts.next().is_none()
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_adapter_owned_files_are_ignored() {
        for name in [
            "actual.shrimp.lock",
            "copy.json.lock",
            "actual.shrimp.tmp-123",
            "actual.shrimp.tmp-123-456",
            ".incoming-35f29d59-cdcc-4a91-8244-31e242b17833",
        ] {
            assert!(internal_file(name, "actual.shrimp"), "{name}");
        }
        for name in [
            "Cargo.lock",
            "clip.tmp-final.mp4",
            "actual.shrimp.tmp-final",
            ".incoming-data.csv",
            "actual.shrimp.tmp-1-2-3",
        ] {
            assert!(!internal_file(name, "actual.shrimp"), "{name}");
        }
    }
}
