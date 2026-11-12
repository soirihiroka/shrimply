//! Native event-driven dependency tracking. Watches are installed before reads.
use super::Command;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use sanim_lang::{FileSources, Sources};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};

pub(crate) struct WatchedSources {
    root: PathBuf,
    sources: FileSources,
    watcher: Mutex<RecommendedWatcher>,
    paths: Arc<Mutex<BTreeSet<PathBuf>>>,
    directories: Mutex<BTreeMap<PathBuf, DirectoryIdentity>>,
}
#[cfg(unix)]
type DirectoryIdentity = (u64, u64, i64, i64);
#[cfg(not(unix))]
type DirectoryIdentity = std::time::SystemTime;
fn directory_identity(path: &Path) -> Result<DirectoryIdentity, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok((
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        ))
    }
    #[cfg(not(unix))]
    {
        metadata.created().map_err(|e| e.to_string())
    }
}
impl WatchedSources {
    pub fn new(
        root: PathBuf,
        entry: &str,
        commands: mpsc::SyncSender<Command>,
    ) -> Result<Self, String> {
        let paths = Arc::new(Mutex::new(BTreeSet::from([root.join(entry)])));
        let dependencies = paths.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let command = match event {
                Ok(event) => {
                    if matches!(event.kind, notify::EventKind::Access(_)) {
                        return;
                    }
                    let paths = dependencies
                        .lock()
                        .expect("Sanim watch dependencies poisoned");
                    if !event.need_rescan() && !event.paths.iter().any(|path| affects(&paths, path))
                    {
                        return;
                    }
                    Command::Changed
                }
                Err(error) => Command::WatchError(error.to_string()),
            };
            let _ = commands.try_send(command);
        })
        .map_err(|e| format!("Sanim file watcher: {e}"))?;
        let sources = Self {
            sources: FileSources::new(&root)?,
            root,
            watcher: Mutex::new(watcher),
            paths,
            directories: Mutex::new(BTreeMap::new()),
        };
        sources.watch(entry)?;
        Ok(sources)
    }
    fn watch(&self, path: &str) -> Result<(), String> {
        let absolute = self.root.join(path);
        let mut paths = vec![absolute.clone()];
        if let Ok(canonical) = absolute.canonicalize() {
            if !canonical.starts_with(&self.root) {
                return Err(format!(
                    "resource '{path}' escapes the source root (including symlinks)"
                ));
            }
            paths.push(canonical);
        }
        let mut watched = self
            .directories
            .lock()
            .expect("Sanim watch directories poisoned");
        let mut dependencies = self
            .paths
            .lock()
            .expect("Sanim watch dependencies poisoned");
        dependencies.extend(paths.iter().cloned());
        drop(dependencies);
        let mut directories: BTreeSet<_> = paths
            .iter()
            .filter_map(|path| path.parent().map(Path::to_owned))
            .collect();
        // The parent keeps us informed if the source directory itself is replaced.
        if let Some(parent) = self.root.parent() {
            directories.insert(parent.to_owned());
        }
        for mut directory in directories {
            while !directory.exists() {
                if !directory.pop() {
                    return Err("Sanim dependency has no existing parent directory".into());
                }
            }
            let identity = directory_identity(&directory)?;
            if watched.get(&directory) != Some(&identity) {
                let mut watcher = self.watcher.lock().expect("Sanim watcher poisoned");
                if watched.contains_key(&directory) {
                    let _ = watcher.unwatch(&directory);
                }
                watcher
                    .watch(&directory, RecursiveMode::NonRecursive)
                    .map_err(|e| format!("Sanim cannot watch {}: {e}", directory.display()))?;
                watched.insert(directory, identity);
            }
        }
        Ok(())
    }
    pub fn retain(&self, dependencies: impl Iterator<Item = impl AsRef<str>>) {
        let mut paths = BTreeSet::new();
        for path in dependencies {
            let absolute = self.root.join(path.as_ref());
            if let Ok(canonical) = absolute.canonicalize() {
                paths.insert(canonical);
            }
            paths.insert(absolute);
        }
        // Keep native watches for directories still containing a dependency, including missing parents.
        let mut watched = self
            .directories
            .lock()
            .expect("Sanim watch directories poisoned");
        let obsolete: Vec<_> = watched
            .keys()
            .filter(|dir| !affects(&paths, dir))
            .cloned()
            .collect();
        for directory in obsolete {
            let _ = self
                .watcher
                .lock()
                .expect("Sanim watcher poisoned")
                .unwatch(&directory);
            watched.remove(&directory);
        }
        *self
            .paths
            .lock()
            .expect("Sanim watch dependencies poisoned") = paths;
    }
}
impl Sources for WatchedSources {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.watch(path)?;
        self.sources.read(path)
    }
}
fn affects(dependencies: &BTreeSet<PathBuf>, event: &Path) -> bool {
    dependencies.contains(event)
        || dependencies
            .range(event.to_owned()..)
            .next()
            .is_some_and(|path| path.starts_with(event))
}
