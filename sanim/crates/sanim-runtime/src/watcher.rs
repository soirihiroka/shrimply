//! Native event-driven dependency tracking. Watches are installed before reads.
use super::Command;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use sanim_core::cache::Cache;
use sanim_lang::{FileSources, Sources};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
};

pub(crate) struct WatchedSources {
    root: PathBuf,
    sources: OnceLock<FileSources>,
    watcher: Mutex<RecommendedWatcher>,
    state: Arc<Mutex<WatchState>>,
    directories: Mutex<BTreeMap<PathBuf, DirectoryIdentity>>,
    #[cfg(test)]
    reads: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    hashed_bytes: std::sync::atomic::AtomicUsize,
}
const MAX_RESOURCE_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_RESOURCE_CACHE_ENTRIES: usize = 4096;
const MAX_WATCHED_RESOURCES: usize = 16_384;
type ResourceSnapshot = Result<(Arc<Vec<u8>>, String), String>;
struct Dependency {
    paths: BTreeSet<PathBuf>,
    present: bool,
    version: u64,
}
struct WatchState {
    dependencies: BTreeMap<String, Dependency>,
    aliases: BTreeMap<PathBuf, BTreeSet<String>>,
    cache: Cache<String, ResourceSnapshot>,
    next_version: u64,
    remote: BTreeSet<String>,
}
impl WatchState {
    fn new() -> Self {
        Self {
            dependencies: BTreeMap::new(),
            aliases: BTreeMap::new(),
            cache: Cache::new(MAX_RESOURCE_CACHE_BYTES, MAX_RESOURCE_CACHE_ENTRIES),
            next_version: 0,
            remote: BTreeSet::new(),
        }
    }
    fn version(&mut self) -> u64 {
        self.next_version = self
            .next_version
            .checked_add(1)
            .expect("Sanim resource generation overflow");
        self.next_version
    }
    fn register(
        &mut self,
        path: &str,
        paths: BTreeSet<PathBuf>,
        present: bool,
    ) -> Result<(), String> {
        if self
            .dependencies
            .get(path)
            .is_some_and(|dependency| dependency.paths == paths && dependency.present == present)
        {
            return Ok(());
        }
        if !self.dependencies.contains_key(path)
            && self.dependencies.len() + self.remote.len() >= MAX_WATCHED_RESOURCES
        {
            return Err(format!(
                "Sanim dependency watch budget exceeds {MAX_WATCHED_RESOURCES} resources"
            ));
        }
        if let Some(old) = self.dependencies.remove(path) {
            self.remove_aliases(path, &old.paths);
        }
        self.cache.remove(&path.to_owned());
        for alias in &paths {
            self.aliases
                .entry(alias.clone())
                .or_default()
                .insert(path.to_owned());
        }
        let version = self.version();
        self.dependencies.insert(
            path.to_owned(),
            Dependency {
                paths,
                present,
                version,
            },
        );
        Ok(())
    }
    fn remove_aliases(&mut self, path: &str, aliases: &BTreeSet<PathBuf>) {
        for alias in aliases {
            if let Some(names) = self.aliases.get_mut(alias) {
                names.remove(path);
                if names.is_empty() {
                    self.aliases.remove(alias);
                }
            }
        }
    }
    fn affected(&self, event: &Path) -> BTreeSet<String> {
        self.aliases
            .range(event.to_owned()..)
            .take_while(|(path, _)| path.starts_with(event))
            .flat_map(|(_, names)| names.iter().cloned())
            .collect()
    }
    fn invalidate(&mut self, paths: impl IntoIterator<Item = String>) {
        for path in paths {
            self.cache.remove(&path);
            let version = self.version();
            if let Some(dependency) = self.dependencies.get_mut(&path) {
                dependency.version = version;
            }
        }
    }
    fn clear(&mut self) {
        let paths = self.dependencies.keys().cloned().collect::<Vec<_>>();
        self.invalidate(paths);
    }
    fn retain(&mut self, active: &BTreeSet<String>) {
        self.remote.retain(|path| active.contains(path));
        let removed = self
            .dependencies
            .keys()
            .filter(|path| !active.contains(*path))
            .cloned()
            .collect::<Vec<_>>();
        for path in removed {
            if let Some(old) = self.dependencies.remove(&path) {
                self.remove_aliases(&path, &old.paths);
            }
            self.cache.remove(&path);
        }
    }
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
        let state = Arc::new(Mutex::new(WatchState::new()));
        state.lock().expect("Sanim watch state poisoned").register(
            entry,
            BTreeSet::from([root.join(entry)]),
            root.join(entry).exists(),
        )?;
        let dependencies = state.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let command = match event {
                Ok(event) => {
                    if matches!(event.kind, notify::EventKind::Access(_)) {
                        return;
                    }
                    let mut state = dependencies
                        .lock()
                        .expect("Sanim watch dependencies poisoned");
                    if event.need_rescan() {
                        state.clear();
                    } else {
                        let metadata = matches!(
                            event.kind,
                            notify::EventKind::Modify(notify::event::ModifyKind::Metadata(_))
                        );
                        let mut recheck = false;
                        let affected = event
                            .paths
                            .iter()
                            .filter(|path| {
                                if metadata && path.is_dir() {
                                    // A child rename changes its parent's ctime.
                                    // watch() checks object identity before cache
                                    // hits; ordinary metadata must not evict all
                                    // unrelated data files in the directory.
                                    recheck |= !state.affected(path).is_empty();
                                    false
                                } else {
                                    true
                                }
                            })
                            .flat_map(|path| state.affected(path))
                            .collect::<BTreeSet<_>>();
                        if affected.is_empty() && !recheck {
                            return;
                        }
                        state.invalidate(affected);
                    }
                    Command::Changed
                }
                Err(error) => {
                    dependencies
                        .lock()
                        .expect("Sanim watch dependencies poisoned")
                        .clear();
                    Command::WatchError(error.to_string())
                }
            };
            let _ = commands.try_send(command);
        })
        .map_err(|e| format!("Sanim file watcher: {e}"))?;
        let sources = Self {
            sources: OnceLock::new(),
            root,
            watcher: Mutex::new(watcher),
            state,
            directories: Mutex::new(BTreeMap::new()),
            #[cfg(test)]
            reads: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            hashed_bytes: std::sync::atomic::AtomicUsize::new(0),
        };
        sources.watch(entry)?;
        Ok(sources)
    }
    fn sources(&self) -> Result<&FileSources, String> {
        if self.sources.get().is_none() {
            let sources = FileSources::new(&self.root).map_err(|error| {
                format!("Sanim source directory {}: {error}", self.root.display())
            })?;
            let _ = self.sources.set(sources);
        }
        Ok(self.sources.get().expect("initialized Sanim file sources"))
    }
    fn watch(&self, path: &str) -> Result<(), String> {
        if sanim_lang::source::is_remote(path) {
            return Ok(());
        }
        let absolute = self.root.join(path);
        let mut paths = BTreeSet::from([absolute.clone()]);
        let canonical = absolute.canonicalize();
        let present = canonical.is_ok();
        if let Ok(canonical) = canonical {
            if !canonical.starts_with(self.sources()?.root()) {
                return Err(format!(
                    "resource '{path}' escapes the source root (including symlinks)"
                ));
            }
            paths.insert(canonical);
        }
        let mut watched = self
            .directories
            .lock()
            .expect("Sanim watch directories poisoned");
        self.state
            .lock()
            .expect("Sanim watch dependencies poisoned")
            .register(path, paths.clone(), present)?;
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
                if watched
                    .get(&directory)
                    .is_some_and(|old| directory_replaced(old, &identity))
                {
                    let mut state = self
                        .state
                        .lock()
                        .expect("Sanim watch dependencies poisoned");
                    let affected = state.affected(&directory);
                    state.invalidate(affected);
                }
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
    pub fn invalidate_all(&self) {
        let remote = {
            let mut state = self
                .state
                .lock()
                .expect("Sanim watch dependencies poisoned");
            state.clear();
            state.remote.iter().cloned().collect::<Vec<_>>()
        };
        for url in remote {
            // Invalid URLs have no cached response; their normal read reports
            // the source diagnostic during the next preparation.
            let _ = sanim_lang::source::invalidate_remote(&url);
        }
    }
    pub fn retain(&self, dependencies: impl Iterator<Item = impl AsRef<str>>) {
        let active = dependencies
            .map(|path| path.as_ref().to_owned())
            .collect::<BTreeSet<_>>();
        let mut state = self
            .state
            .lock()
            .expect("Sanim watch dependencies poisoned");
        state.retain(&active);
        let paths = state.aliases.keys().cloned().collect::<BTreeSet<_>>();
        drop(state);
        // Keep native watches for directories containing an active dependency.
        let mut watched = self
            .directories
            .lock()
            .expect("Sanim watch directories poisoned");
        let obsolete = watched
            .keys()
            .filter(|dir| !affects(&paths, dir))
            .cloned()
            .collect::<Vec<_>>();
        for directory in obsolete {
            let _ = self
                .watcher
                .lock()
                .expect("Sanim watcher poisoned")
                .unwatch(&directory);
            watched.remove(&directory);
        }
    }
}
impl Sources for WatchedSources {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.read_snapshot(path)
            .map(|(bytes, _)| bytes.as_ref().clone())
    }
    fn read_snapshot(&self, path: &str) -> Result<(Arc<Vec<u8>>, String), String> {
        if sanim_lang::source::is_remote(path) {
            {
                let mut state = self
                    .state
                    .lock()
                    .expect("Sanim watch dependencies poisoned");
                if !state.remote.contains(path)
                    && state.remote.len() + state.dependencies.len() >= MAX_WATCHED_RESOURCES
                {
                    return Err(format!(
                        "Sanim dependency budget exceeds {MAX_WATCHED_RESOURCES} resources"
                    ));
                }
                state.remote.insert(path.to_owned());
            }
            return self.sources()?.read_snapshot(path);
        }

        // Security and directory replacement checks happen even on cache hits.
        self.watch(path)?;
        let key = path.to_owned();
        let version = {
            let mut state = self
                .state
                .lock()
                .expect("Sanim watch dependencies poisoned");
            if let Some(snapshot) = state.cache.get(&key) {
                return snapshot.as_ref().clone();
            }
            state.dependencies[&key].version
        };
        #[cfg(test)]
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let snapshot = self
            .sources()
            .and_then(|sources| sources.read(path))
            .map(|bytes| {
                #[cfg(test)]
                self.hashed_bytes
                    .fetch_add(bytes.len(), std::sync::atomic::Ordering::Relaxed);
                let fingerprint = sanim_core::digest(&bytes);
                (Arc::new(bytes), fingerprint)
            });
        let bytes = std::mem::size_of::<ResourceSnapshot>()
            + std::mem::size_of::<String>()
            + key.len()
            + match &snapshot {
                Ok((bytes, fingerprint)) => bytes.len() + fingerprint.len(),
                Err(error) => error.len(),
            };
        let mut state = self
            .state
            .lock()
            .expect("Sanim watch dependencies poisoned");
        // A native event arriving during I/O must not repopulate stale bytes.
        if state
            .dependencies
            .get(&key)
            .is_some_and(|dependency| dependency.version == version)
        {
            state.cache.insert(key, Arc::new(snapshot.clone()), bytes);
        }
        snapshot
    }
}

fn directory_replaced(old: &DirectoryIdentity, current: &DirectoryIdentity) -> bool {
    #[cfg(unix)]
    {
        old.0 != current.0 || old.1 != current.1
    }
    #[cfg(not(unix))]
    {
        old != current
    }
}

fn affects(dependencies: &BTreeSet<PathBuf>, event: &Path) -> bool {
    dependencies.contains(event)
        || dependencies
            .range(event.to_owned()..)
            .next()
            .is_some_and(|path| path.starts_with(event))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };
    struct Files(PathBuf);
    impl Files {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "sanim-watch-snapshots-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Files {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn source_only_atomic_edit_keeps_large_resource_bytes_and_hashes() {
        let files = Files::new();
        std::fs::write(files.0.join("main.sanim"), "wait(1s)").unwrap();
        const BYTES: usize = 32 * 1024 * 1024;
        std::fs::write(files.0.join("payload.txt"), vec![b'x'; BYTES]).unwrap();
        let (sender, receiver) = mpsc::sync_channel(128);
        let sources = WatchedSources::new(files.0.clone(), "main.sanim", sender).unwrap();
        sources.read_snapshot("main.sanim").unwrap();
        let first = sources.read_snapshot("payload.txt").unwrap();
        let before = sources.hashed_bytes.load(Ordering::Relaxed);
        let reads = sources.reads.load(Ordering::Relaxed);
        for _ in 0..10 {
            let unchanged = sources.read_snapshot("payload.txt").unwrap();
            assert!(Arc::ptr_eq(&first.0, &unchanged.0));
            assert_eq!(first.1, unchanged.1);
        }
        assert_eq!(sources.reads.load(Ordering::Relaxed), reads);
        assert_eq!(sources.hashed_bytes.load(Ordering::Relaxed), before);
        let version = sources.state.lock().unwrap().dependencies["main.sanim"].version;
        std::fs::write(files.0.join("pending.sanim"), "wait(2s)").unwrap();
        std::fs::rename(files.0.join("pending.sanim"), files.0.join("main.sanim")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while sources.state.lock().unwrap().dependencies["main.sanim"].version == version {
            receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("source replacement native notification");
        }
        let changed = sources.read_snapshot("main.sanim").unwrap();
        assert_eq!(changed.0.as_slice(), b"wait(2s)");
        for _ in 0..10 {
            let retained = sources.read_snapshot("payload.txt").unwrap();
            assert!(
                Arc::ptr_eq(&first.0, &retained.0),
                "ordinary parent-directory ctime changes must not invalidate the unrelated payload"
            );
        }
        assert_eq!(
            sources.hashed_bytes.load(Ordering::Relaxed) - before,
            b"wait(2s)".len()
        );
        sources.invalidate_all();
        let refreshed = sources.read_snapshot("payload.txt").unwrap();
        assert!(
            !Arc::ptr_eq(&first.0, &refreshed.0),
            "manual refresh must bypass cached filesystem bytes"
        );
    }

    #[test]
    fn per_resource_generations_and_alias_pruning_release_old_bytes_without_gc() {
        let mut state = WatchState::new();
        let mut previous = None;
        for index in 0..2000 {
            let name = format!("data-{index}.txt");
            let path = PathBuf::from("/source").join(&name);
            state
                .register(
                    &name,
                    BTreeSet::from([path.clone(), PathBuf::from("/source/link").join(&name)]),
                    true,
                )
                .unwrap();
            let bytes = Arc::new(vec![index as u8; 1024]);
            let weak = Arc::downgrade(&bytes);
            state
                .cache
                .insert(name.clone(), Arc::new(Ok((bytes, "hash".into()))), 1024);
            state.retain(&BTreeSet::from([name.clone()]));
            if let Some(old) = previous.take() {
                let old: std::sync::Weak<Vec<u8>> = old;
                assert!(
                    old.upgrade().is_none(),
                    "pruned resource bytes release deterministically after the last Arc"
                );
            }
            assert_eq!(state.dependencies.len(), 1);
            assert_eq!(state.aliases.len(), 2);
            assert!(weak.upgrade().is_some());
            previous = Some(weak);
        }
        let name = "data-1999.txt".to_owned();
        let before = state.dependencies[&name].version;
        let affected = state.affected(&PathBuf::from("/source/data-1999.txt"));
        state.invalidate(affected);
        assert!(state.dependencies[&name].version > before);
        assert!(state.cache.get(&name).is_none());
        assert!(previous.unwrap().upgrade().is_none());
        state.retain(&BTreeSet::new());
        assert!(state.dependencies.is_empty());
        assert!(state.aliases.is_empty());
    }
}
