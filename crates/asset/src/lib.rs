use std::collections::{HashMap, HashSet, hash_map::DefaultHasher};
use std::fmt;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak, mpsc};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use async_channel::{Receiver, Sender, TrySendError};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const WATCH_EVENT_COALESCE_WINDOW: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub struct Asset {
    path: PathBuf,
    entry: Arc<OnceLock<Arc<Entry>>>,
}

#[derive(Clone)]
pub struct AssetSnapshot {
    asset: Asset,
    version: Version,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Version {
    revision: u64,
    fingerprint: Fingerprint,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Fingerprint {
    Missing,
    Present {
        len: u64,
        modified_ns: i128,
        #[cfg(unix)]
        device: u64,
        #[cfg(unix)]
        inode: u64,
        #[cfg(unix)]
        identity_seconds: i64,
        #[cfg(unix)]
        identity_nanoseconds: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetChange {
    pub path: PathBuf,
    pub revision: u64,
}

struct Entry {
    id: u64,
    path: PathBuf,
    watched_directory: PathBuf,
    version: RwLock<Version>,
    commands: mpsc::Sender<Command>,
}

struct Registry {
    watcher: RecommendedWatcher,
    entries: HashMap<PathBuf, Weak<Entry>>,
    watched_directories: HashMap<PathBuf, usize>,
    next_id: u64,
}

struct Manager {
    commands: mpsc::Sender<Command>,
}

enum Command {
    Register {
        path: PathBuf,
        response: mpsc::Sender<Result<Arc<Entry>, String>>,
    },
    Unregister {
        path: PathBuf,
        watched_directory: PathBuf,
        id: u64,
    },
    Event(notify::Result<Event>),
}

static MANAGER: OnceLock<Result<Manager, String>> = OnceLock::new();
static SUBSCRIBERS: OnceLock<Mutex<Vec<Sender<AssetChange>>>> = OnceLock::new();
static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);

impl Asset {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            entry: Arc::new(OnceLock::new()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn resolve_against(&self, directory: &Path) -> Self {
        if self.path.is_relative() {
            Self::new(directory.join(&self.path))
        } else {
            self.clone()
        }
    }

    pub fn watch(&self) -> Result<(), String> {
        self.entry()?;
        Ok(())
    }

    pub fn snapshot(&self) -> Result<AssetSnapshot, String> {
        let entry = self.entry()?;
        let version = entry
            .version
            .read()
            .map_err(|_| format!("asset version lock poisoned for {}", self.path.display()))?
            .clone();
        if version.fingerprint == Fingerprint::Missing {
            return Err(format!("asset does not exist: {}", self.path.display()));
        }
        Ok(AssetSnapshot {
            asset: self.clone(),
            version,
        })
    }

    pub fn read(&self) -> Result<Vec<u8>, String> {
        self.snapshot()?.read()
    }

    pub fn read_to_string(&self) -> Result<String, String> {
        self.snapshot()?.read_to_string()
    }

    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        let entry = self.entry()?;
        let directory = self.path.parent().unwrap_or_else(|| Path::new("."));
        let suffix = NEXT_WRITE.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let mut temporary = tempfile::Builder::new()
            .prefix(".shrimply-asset-")
            .suffix(&format!("-{suffix}"))
            .tempfile_in(directory)
            .map_err(|error| {
                format!(
                    "could not create temporary file for {}: {error}",
                    self.path.display()
                )
            })?;
        temporary
            .write_all(bytes)
            .and_then(|()| temporary.flush())
            .map_err(|error| format!("could not write {}: {error}", self.path.display()))?;
        temporary.persist(&self.path).map_err(|error| {
            format!("could not replace {}: {}", self.path.display(), error.error)
        })?;
        entry.refresh_from_disk(false)?;
        Ok(())
    }

    pub fn mark_dirty(&self) -> Result<(), String> {
        self.entry()?.refresh_from_disk(true)?;
        Ok(())
    }

    fn entry(&self) -> Result<Arc<Entry>, String> {
        if let Some(entry) = self.entry.get() {
            return Ok(Arc::clone(entry));
        }
        let entry = manager()?.register(&self.path)?;
        let _ = self.entry.set(Arc::clone(&entry));
        Ok(self.entry.get().map_or(entry, Arc::clone))
    }
}

impl AssetSnapshot {
    pub fn path(&self) -> &Path {
        self.asset.path()
    }

    pub fn asset(&self) -> &Asset {
        &self.asset
    }

    pub fn revision(&self) -> u64 {
        self.version.revision
    }

    pub fn len(&self) -> u64 {
        match self.version.fingerprint {
            Fingerprint::Present { len, .. } => len,
            Fingerprint::Missing => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn modified_ns(&self) -> i128 {
        match self.version.fingerprint {
            Fingerprint::Present { modified_ns, .. } => modified_ns,
            Fingerprint::Missing => 0,
        }
    }

    pub fn read(&self) -> Result<Vec<u8>, String> {
        let bytes = fs::read(self.path())
            .map_err(|error| format!("could not read {}: {error}", self.path().display()))?;
        self.verify_current()?;
        Ok(bytes)
    }

    pub fn read_to_string(&self) -> Result<String, String> {
        let contents = fs::read_to_string(self.path())
            .map_err(|error| format!("could not read {}: {error}", self.path().display()))?;
        self.verify_current()?;
        Ok(contents)
    }

    pub fn cache_key(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    pub fn is_current(&self) -> bool {
        let Ok(entry) = self.asset.entry() else {
            return false;
        };
        entry
            .version
            .read()
            .is_ok_and(|version| *version == self.version)
    }

    pub fn ensure_current(&self) -> Result<(), String> {
        if self.is_current() {
            Ok(())
        } else {
            Err(format!(
                "asset changed while it was in use: {}",
                self.path().display()
            ))
        }
    }

    pub fn verify_current(&self) -> Result<(), String> {
        self.asset.entry()?.refresh_from_disk(false)?;
        self.ensure_current()
    }
}

impl Entry {
    fn refresh_from_disk(&self, force: bool) -> Result<bool, String> {
        let fingerprint = fingerprint(&self.path)?;
        let revision = {
            let mut version = self
                .version
                .write()
                .map_err(|_| format!("asset version lock poisoned for {}", self.path.display()))?;
            if !force && version.fingerprint == fingerprint {
                return Ok(false);
            }
            version.revision = version.revision.wrapping_add(1);
            version.fingerprint = fingerprint;
            version.revision
        };
        broadcast(AssetChange {
            path: self.path.clone(),
            revision,
        });
        Ok(true)
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Unregister {
            path: self.path.clone(),
            watched_directory: self.watched_directory.clone(),
            id: self.id,
        });
    }
}

impl Manager {
    fn start() -> Result<Self, String> {
        let (commands, receiver) = mpsc::channel();
        let event_commands = commands.clone();
        let watcher = notify::recommended_watcher(move |event| {
            let _ = event_commands.send(Command::Event(event));
        })
        .map_err(|error| format!("could not start native asset watcher: {error}"))?;
        thread::Builder::new()
            .name("asset-watcher".to_string())
            .spawn(move || Registry::new(watcher).run(receiver))
            .map_err(|error| format!("could not start asset watcher thread: {error}"))?;
        Ok(Self { commands })
    }

    fn register(&self, path: &Path) -> Result<Arc<Entry>, String> {
        let path = normalize_path(path)?;
        let (response, receiver) = mpsc::channel();
        self.commands
            .send(Command::Register { path, response })
            .map_err(|_| "asset watcher stopped unexpectedly".to_string())?;
        receiver
            .recv()
            .map_err(|_| "asset watcher dropped a registration response".to_string())?
    }
}

impl Registry {
    fn new(watcher: RecommendedWatcher) -> Self {
        Self {
            watcher,
            entries: HashMap::new(),
            watched_directories: HashMap::new(),
            next_id: 0,
        }
    }

    fn run(mut self, receiver: mpsc::Receiver<Command>) {
        while let Ok(command) = receiver.recv() {
            match command {
                Command::Event(event) => self.coalesce_events(event, &receiver),
                command => self.handle(command),
            }
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Register { path, response } => {
                let _ = response.send(self.register(path));
            }
            Command::Unregister {
                path,
                watched_directory,
                id,
            } => self.unregister(&path, &watched_directory, id),
            Command::Event(_) => unreachable!("events are handled by the watcher loop"),
        }
    }

    fn register(&mut self, path: PathBuf) -> Result<Arc<Entry>, String> {
        if let Some(entry) = self.entries.get(&path).and_then(Weak::upgrade) {
            return Ok(entry);
        }
        let fingerprint = fingerprint(&path)?;
        let containing_directory = path
            .parent()
            .ok_or_else(|| format!("asset has no containing directory: {}", path.display()))?
            .to_path_buf();
        let directory = containing_directory
            .ancestors()
            .find(|directory| directory.is_dir())
            .ok_or_else(|| {
                format!(
                    "asset has no existing ancestor directory: {}",
                    path.display()
                )
            })?
            .to_path_buf();
        if !self.watched_directories.contains_key(&directory) {
            self.watcher
                .watch(&directory, RecursiveMode::Recursive)
                .map_err(|error| {
                    format!(
                        "could not watch asset directory {}: {error}",
                        directory.display()
                    )
                })?;
        }
        *self
            .watched_directories
            .entry(directory.clone())
            .or_default() += 1;
        self.next_id = self.next_id.wrapping_add(1);
        let entry = Arc::new(Entry {
            id: self.next_id,
            version: RwLock::new(Version {
                revision: 0,
                fingerprint,
            }),
            path: path.clone(),
            watched_directory: directory,
            commands: manager()?.commands.clone(),
        });
        self.entries.insert(path, Arc::downgrade(&entry));
        Ok(entry)
    }

    fn unregister(&mut self, path: &Path, watched_directory: &Path, id: u64) {
        let remove = self
            .entries
            .get(path)
            .is_some_and(|entry| entry.upgrade().is_none_or(|entry| entry.id == id));
        if !remove {
            return;
        }
        self.entries.remove(path);
        let Some(count) = self.watched_directories.get_mut(watched_directory) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.watched_directories.remove(watched_directory);
            if let Err(error) = self.watcher.unwatch(watched_directory) {
                tracing::warn!(path = %watched_directory.display(), %error, "could not stop watching asset directory");
            }
        }
    }

    fn coalesce_events(
        &mut self,
        first: notify::Result<Event>,
        receiver: &mpsc::Receiver<Command>,
    ) {
        let mut paths = HashSet::new();
        let mut refresh_all = false;
        collect_event(first, &mut paths, &mut refresh_all);
        loop {
            match receiver.recv_timeout(WATCH_EVENT_COALESCE_WINDOW) {
                Ok(Command::Event(event)) => collect_event(event, &mut paths, &mut refresh_all),
                Ok(command) => self.handle(command),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        self.refresh(paths, refresh_all);
    }

    fn refresh(&mut self, paths: HashSet<PathBuf>, refresh_all: bool) {
        for (path, entry) in &self.entries {
            let affected = refresh_all
                || paths.iter().any(|changed| {
                    path == changed || path.starts_with(changed) || changed.starts_with(path)
                });
            if !affected {
                continue;
            }
            if let Some(entry) = entry.upgrade()
                && let Err(error) = entry.refresh_from_disk(false)
            {
                tracing::warn!(path = %path.display(), %error, "could not refresh asset state");
            }
        }
    }
}

pub fn subscribe() -> Receiver<AssetChange> {
    let (sender, receiver) = async_channel::bounded(1);
    subscribers()
        .lock()
        .expect("asset subscriber lock poisoned")
        .push(sender);
    receiver
}

fn manager() -> Result<&'static Manager, String> {
    MANAGER
        .get_or_init(Manager::start)
        .as_ref()
        .map_err(Clone::clone)
}

fn subscribers() -> &'static Mutex<Vec<Sender<AssetChange>>> {
    SUBSCRIBERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn broadcast(change: AssetChange) {
    subscribers()
        .lock()
        .expect("asset subscriber lock poisoned")
        .retain(|subscriber| match subscriber.try_send(change.clone()) {
            Ok(()) | Err(TrySendError::Full(_)) => true,
            Err(TrySendError::Closed(_)) => false,
        });
}

fn collect_event(
    event: notify::Result<Event>,
    paths: &mut HashSet<PathBuf>,
    refresh_all: &mut bool,
) {
    match event {
        Ok(event) => {
            *refresh_all |= event.need_rescan();
            paths.extend(
                event
                    .paths
                    .into_iter()
                    .filter_map(|path| normalize_path(&path).ok()),
            );
        }
        Err(error) => {
            tracing::warn!(%error, "native asset watcher reported an error");
            *refresh_all = true;
        }
    }
}

fn normalize_path(path: &Path) -> Result<PathBuf, String> {
    std::path::absolute(path)
        .map_err(|error| format!("could not resolve asset path {}: {error}", path.display()))
}

fn fingerprint(path: &Path) -> Result<Fingerprint, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Fingerprint::Missing);
        }
        Err(error) => {
            return Err(format!(
                "could not inspect asset {}: {error}",
                path.display()
            ));
        }
    };
    if !metadata.is_file() {
        return Err(format!("asset is not a regular file: {}", path.display()));
    }
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i128::MAX as u128) as i128)
        .unwrap_or_default();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        #[cfg(target_os = "macos")]
        let (identity_seconds, identity_nanoseconds) = (
            std::os::macos::fs::MetadataExt::st_birthtime(&metadata),
            std::os::macos::fs::MetadataExt::st_birthtime_nsec(&metadata),
        );
        #[cfg(not(target_os = "macos"))]
        let (identity_seconds, identity_nanoseconds) =
            (metadata.ctime(), metadata.ctime_nsec());
        Ok(Fingerprint::Present {
            len: metadata.len(),
            modified_ns,
            device: metadata.dev(),
            inode: metadata.ino(),
            // macOS ctime changes when File Provider or Powerbox updates access metadata.
            // Birth time still identifies replacement files without restarting unchanged media.
            identity_seconds,
            identity_nanoseconds,
        })
    }
    #[cfg(not(unix))]
    {
        Ok(Fingerprint::Present {
            len: metadata.len(),
            modified_ns,
        })
    }
}

impl Default for Asset {
    fn default() -> Self {
        Self::new(PathBuf::new())
    }
}

impl From<PathBuf> for Asset {
    fn from(path: PathBuf) -> Self {
        Self::new(path)
    }
}

impl From<&Path> for Asset {
    fn from(path: &Path) -> Self {
        Self::new(path)
    }
}

impl From<&PathBuf> for Asset {
    fn from(path: &PathBuf) -> Self {
        Self::new(path)
    }
}

impl From<&Asset> for Asset {
    fn from(asset: &Asset) -> Self {
        asset.clone()
    }
}

impl AsRef<Path> for Asset {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

impl Deref for Asset {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.path()
    }
}

impl AsRef<Path> for AssetSnapshot {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

impl fmt::Debug for Asset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Asset").field(&self.path).finish()
    }
}

impl fmt::Debug for AssetSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AssetSnapshot")
            .field("path", &self.path())
            .field("revision", &self.revision())
            .finish()
    }
}

impl PartialEq for Asset {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for Asset {}

impl Hash for Asset {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl PartialEq for AssetSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.asset == other.asset && self.version == other.version
    }
}

impl Eq for AssetSnapshot {}

impl Hash for AssetSnapshot {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.asset.hash(state);
        self.version.hash(state);
    }
}

impl Serialize for Asset {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.path.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Asset {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        PathBuf::deserialize(deserializer).map(Self::new)
    }
}
