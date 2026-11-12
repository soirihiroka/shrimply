//! Background preparation and dependency watching, independent of any editor.
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use sanim_core::{Diagnostic, Viewport};
pub use sanim_lang;
use sanim_lang::{FileSources, Limits, Prepared, Reload, Session};
pub use sanim_render;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

const WATCH_RECOVERY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
pub struct Snapshot {
    pub generation: u64,
    pub revision: u64,
    /// Timing-only result for the pending source revision; prepared remains the last valid scene.
    pub planned_duration: Option<sanim_core::Fraction>,
    pub prepared: Option<Arc<Prepared>>,
    pub diagnostic: Option<Diagnostic>,
    pub changed_elements: Arc<[String]>,
    pub preparing: bool,
    pub requested_inputs: u64,
    pub applied_inputs: u64,
}
#[derive(Clone, Default)]
struct Configuration {
    inputs: BTreeMap<String, serde_json::Value>,
    viewport: Viewport,
}
enum Command {
    Changed,
    Inputs(u64, Configuration),
    Stop,
}
pub type Wake = Arc<dyn Fn() + Send + Sync>;
type Waker = Arc<Mutex<Option<Wake>>>;
type Shared = Arc<(Mutex<Snapshot>, Condvar)>;

pub struct Runtime {
    commands: mpsc::SyncSender<Command>,
    shared: Shared,
    worker: Option<thread::JoinHandle<()>>,
    waker: Waker,
    configuration: Mutex<Configuration>,
}
impl Runtime {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        Self::with_limits(path, Limits::default())
    }
    pub fn with_limits(path: impl AsRef<Path>, limits: Limits) -> Result<Self, String> {
        Self::with_viewport(path, Viewport::default(), limits)
    }
    pub fn with_viewport(
        path: impl AsRef<Path>,
        viewport: Viewport,
        limits: Limits,
    ) -> Result<Self, String> {
        viewport.validate()?;
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let root = parent
            .canonicalize()
            .map_err(|e| format!("Sanim source directory: {e}"))?;
        let entry = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Sanim filename must be UTF-8")?
            .to_owned();
        let sources = FileSources::new(&root)?;
        let shared = Arc::new((
            Mutex::new(Snapshot {
                preparing: true,
                ..Snapshot::default()
            }),
            Condvar::new(),
        ));
        let (commands, receiver) = mpsc::sync_channel(32);
        let published = shared.clone();
        let waker = Waker::default();
        let worker_waker = waker.clone();
        let changed = commands.clone();
        let configuration = Configuration {
            viewport: viewport.clone(),
            ..Configuration::default()
        };
        let worker = thread::Builder::new()
            .name("sanim-runtime".into())
            .spawn(move || {
                worker(
                    root,
                    entry,
                    sources,
                    (limits, viewport),
                    receiver,
                    changed,
                    (published, worker_waker),
                )
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            commands,
            shared,
            worker: Some(worker),
            waker,
            configuration: Mutex::new(configuration),
        })
    }
    /// Register a host notification. Called without runtime locks, including once on registration.
    pub fn set_waker(&self, wake: Option<Wake>) {
        *self.waker.lock().expect("Sanim runtime waker poisoned") = wake;
        wake_host(&self.waker);
    }
    /// A cheap immutable snapshot: no source reads or preparation on this thread.
    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .0
            .lock()
            .expect("Sanim runtime state poisoned")
            .clone()
    }
    pub fn reload(&self) -> Result<(), String> {
        self.commands
            .send(Command::Changed)
            .map_err(|_| "Sanim runtime stopped".into())
    }
    /// Replace the complete reflected override snapshot. Missing keys reset to defaults.
    pub fn set_inputs(&self, inputs: BTreeMap<String, serde_json::Value>) -> Result<(), String> {
        self.configure(|configuration| {
            configuration.inputs = inputs;
        })
    }
    pub fn set_viewport(&self, viewport: Viewport) -> Result<(), String> {
        viewport.validate()?;
        self.configure(|configuration| {
            configuration.viewport = viewport;
        })
    }
    fn configure(&self, change: impl FnOnce(&mut Configuration)) -> Result<(), String> {
        let mut configuration = self
            .configuration
            .lock()
            .expect("Sanim configuration poisoned");
        let previous = configuration.viewport.clone();
        let previous_inputs = configuration.inputs.clone();
        change(&mut configuration);
        if configuration.viewport == previous && configuration.inputs == previous_inputs {
            return Ok(());
        }
        let serial = {
            let mut state = self.shared.0.lock().expect("Sanim runtime state poisoned");
            state.requested_inputs += 1;
            state.preparing = true;
            state.requested_inputs
        };
        let command = Command::Inputs(serial, configuration.clone());
        drop(configuration);
        self.commands
            .send(command)
            .map_err(|_| "Sanim runtime stopped".into())
    }
    /// Useful for headless hosts; UI callers should read snapshot() without waiting.
    pub fn wait_for_change(&self, generation: u64, timeout: Duration) -> Snapshot {
        let state = self.shared.0.lock().expect("Sanim runtime state poisoned");
        self.shared
            .1
            .wait_timeout_while(state, timeout, |s| s.generation <= generation)
            .expect("Sanim runtime state poisoned")
            .0
            .clone()
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("Sanim runtime worker panicked");
        }
    }
}
fn publish(shared: &Shared, waker: &Waker, update: impl FnOnce(&mut Snapshot)) {
    let mut state = shared.0.lock().expect("Sanim runtime state poisoned");
    update(&mut state);
    state.generation += 1;
    shared.1.notify_all();
    drop(state);
    wake_host(waker);
}
fn wake_host(waker: &Waker) {
    let wake = waker.lock().expect("Sanim runtime waker poisoned").clone();
    if let Some(wake) = wake {
        wake();
    }
}
fn worker(
    root: PathBuf,
    entry: String,
    sources: FileSources,
    (limits, viewport): (Limits, Viewport),
    receiver: mpsc::Receiver<Command>,
    changed: mpsc::SyncSender<Command>,
    (shared, waker): (Shared, Waker),
) {
    let mut session = Session::new(&entry);
    session.engine.limits = limits;
    session.engine.viewport = viewport.clone();
    let mut watcher: Option<RecommendedWatcher> =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event
                .as_ref()
                .is_ok_and(|e| matches!(e.kind, notify::EventKind::Access(_)))
            {
                return;
            }
            let _ = changed.try_send(Command::Changed);
        })
        .ok();
    let mut watched = BTreeSet::new();
    if let Some(watcher) = &mut watcher
        && watcher.watch(&root, RecursiveMode::NonRecursive).is_ok()
    {
        watched.insert(root.clone());
    }
    let mut raw = Configuration {
        viewport,
        ..Configuration::default()
    };
    let mut input_serial = 0;
    let mut dirty = true;
    let mut input_dirty = false;
    let mut last_poll = Instant::now();
    loop {
        if dirty {
            {
                let mut state = shared.0.lock().expect("Sanim runtime state poisoned");
                state.preparing = true;
            }
            if input_dirty {
                session.engine.viewport = raw.viewport.clone();
                session.set_json_inputs(raw.inputs.clone());
            }
            let outcome = session.poll_with_duration(&sources, |duration| {
                let changed = shared
                    .0
                    .lock()
                    .expect("Sanim runtime state poisoned")
                    .planned_duration
                    != Some(duration);
                if changed {
                    publish(&shared, &waker, |state| {
                        state.planned_duration = Some(duration)
                    });
                }
            });
            match outcome {
                Reload::Updated {
                    revision,
                    prepared,
                    changed_elements,
                } => publish(&shared, &waker, |s| {
                    s.planned_duration = Some(prepared.animation.duration);
                    s.revision = revision;
                    s.prepared = Some(prepared);
                    s.diagnostic = None;
                    s.changed_elements = changed_elements.into();
                    s.applied_inputs = input_serial;
                    s.preparing = s.applied_inputs < s.requested_inputs;
                }),
                Reload::Failed { mut diagnostic, .. } => {
                    if diagnostic.source.is_empty() {
                        diagnostic.source = entry.clone();
                    }
                    publish(&shared, &waker, |s| {
                        s.diagnostic = Some(diagnostic);
                        s.applied_inputs = input_serial;
                        s.preparing = s.applied_inputs < s.requested_inputs;
                    });
                }
                Reload::Unchanged => {
                    let mut state = shared.0.lock().expect("Sanim runtime state poisoned");
                    let diagnostic = session.last_error().cloned();
                    let duration = if diagnostic.is_none() {
                        session
                            .current()
                            .map(|prepared| prepared.animation.duration)
                    } else {
                        state.planned_duration
                    };
                    let changed = state.diagnostic != diagnostic
                        || state.applied_inputs != input_serial
                        || state.planned_duration != duration;
                    if changed {
                        state.generation += 1;
                        shared.1.notify_all();
                    }
                    state.diagnostic = diagnostic;
                    state.planned_duration = duration;
                    if state.diagnostic.is_none() {
                        state.prepared = session.current().cloned();
                    }
                    state.applied_inputs = input_serial;
                    state.preparing = state.applied_inputs < state.requested_inputs;
                    state.changed_elements = Arc::from([]);
                    drop(state);
                    if changed {
                        wake_host(&waker);
                    }
                }
            }
            if let Some(prepared) = session.current() {
                let mut directories = BTreeSet::from([root.clone()]);
                for path in prepared.dependencies.keys() {
                    let absolute = root.join(path);
                    if let Some(parent) = absolute.parent() {
                        let mut directory = parent.to_owned();
                        while !directory.exists() {
                            if !directory.pop() {
                                break;
                            }
                        }
                        directories.insert(directory);
                    }
                }
                if let Some(watcher) = &mut watcher {
                    for path in directories.difference(&watched) {
                        let _ = watcher.watch(path, RecursiveMode::NonRecursive);
                    }
                    for path in watched.difference(&directories) {
                        let _ = watcher.unwatch(path);
                    }
                    watched = directories;
                }
            }
            last_poll = Instant::now();
            input_dirty = false;
        }
        match receiver.recv_timeout(WATCH_RECOVERY_INTERVAL.saturating_sub(last_poll.elapsed())) {
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Command::Inputs(serial, values)) => {
                if serial > input_serial {
                    raw = values;
                    input_serial = serial;
                }
                input_dirty = true;
                dirty = true;
            }
            Ok(Command::Changed) | Err(mpsc::RecvTimeoutError::Timeout) => dirty = true,
        }
        // Coalesce already-queued writes and parameter updates into one revision.
        while let Ok(command) = receiver.try_recv() {
            match command {
                Command::Stop => return,
                Command::Changed => dirty = true,
                Command::Inputs(serial, values) => {
                    if serial > input_serial {
                        raw = values;
                        input_serial = serial;
                    }
                    input_dirty = true;
                    dirty = true;
                }
            }
        }
        if dirty && !input_dirty {
            // Editors commonly truncate then rewrite or replace a file. Wait
            // briefly for the write burst to settle before accepting a revision.
            const QUIET: Duration = Duration::from_millis(25);
            const MAX_DEBOUNCE: Duration = Duration::from_millis(100);
            let limit = Instant::now() + MAX_DEBOUNCE;
            loop {
                let timeout = QUIET.min(limit.saturating_duration_since(Instant::now()));
                if timeout.is_zero() {
                    break;
                }
                match receiver.recv_timeout(timeout) {
                    Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Ok(Command::Changed) => {}
                    Ok(Command::Inputs(serial, values)) => {
                        if serial > input_serial {
                            raw = values;
                            input_serial = serial;
                        }
                        input_dirty = true;
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                }
            }
        }
    }
}
