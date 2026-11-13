//! Background preparation and dependency watching, independent of any editor.
mod watcher;
use watcher::WatchedSources;
mod notifications;
use sanim_core::{Diagnostic, Viewport};
pub use sanim_lang;
pub use sanim_lang::progress::{Phase, Progress, State};
use sanim_lang::{Limits, Prepared, Reload, Session};
pub use sanim_render;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

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
    /// Latest observed phase/counts; totals are phase-local and may be unknown.
    pub progress: Progress,
    /// A last-good scene is retained while a reload is running or has failed.
    pub reloading: bool,
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
    FontsChanged,
    WatchError(String),
    Inputs(u64, Configuration),
    Stop,
}
pub type Wake = Arc<dyn Fn() + Send + Sync>;
type Waker = Arc<Mutex<Option<Wake>>>;
type Shared = Arc<(Mutex<Snapshot>, Condvar)>;

/// Read-only published state with no worker ownership or blocking shutdown on drop.
#[derive(Clone)]
pub struct Observation {
    shared: Shared,
}
impl Observation {
    pub fn snapshot(&self) -> Snapshot {
        Self::read(&self.shared)
    }
    fn read(shared: &Shared) -> Snapshot {
        shared
            .0
            .lock()
            .expect("Sanim runtime state poisoned")
            .clone()
    }
}

pub struct Runtime {
    commands: mpsc::SyncSender<Command>,
    shared: Shared,
    worker: Option<thread::JoinHandle<()>>,
    progress_worker: Option<thread::JoinHandle<()>>,
    waker: Waker,
    configuration: Mutex<Configuration>,
    stopping: Arc<AtomicBool>,
    _font_changes: sanim_data::font::FontChange,
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
        let shared = Arc::new((
            Mutex::new(Snapshot {
                preparing: true,
                progress: Progress {
                    resource: Some(entry.clone()),
                    ..Progress::default()
                },
                ..Snapshot::default()
            }),
            Condvar::new(),
        ));
        let (commands, receiver) = mpsc::sync_channel(32);
        let font_changes = sanim_data::font::subscribe(Arc::new({
            let commands = commands.clone();
            move || {
                // A full queue already guarantees another preparation. The
                // session checks the latest font epoch after coalescing it.
                let _ = commands.try_send(Command::FontsChanged);
            }
        }));
        let sources = WatchedSources::new(root, &entry, commands.clone())?;
        let published = shared.clone();
        let waker = Waker::default();
        let worker_waker = waker.clone();
        let (progress_changes, progress_receiver) = mpsc::sync_channel(1);
        let progress_worker = thread::Builder::new()
            .name("sanim-progress".into())
            .spawn({
                let shared = shared.clone();
                let waker = waker.clone();
                move || notifications::run(shared, waker, progress_receiver)
            })
            .map_err(|error| error.to_string())?;
        let configuration = Configuration {
            viewport: viewport.clone(),
            ..Configuration::default()
        };
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let worker = thread::Builder::new()
            .name("sanim-runtime".into())
            .spawn(move || {
                worker(
                    entry,
                    sources,
                    (limits, viewport, worker_stopping),
                    receiver,
                    (published, worker_waker, progress_changes),
                )
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            commands,
            shared,
            worker: Some(worker),
            progress_worker: Some(progress_worker),
            waker,
            configuration: Mutex::new(configuration),
            stopping,
            _font_changes: font_changes,
        })
    }
    /// Register a host notification. Called without runtime locks, including once on registration.
    pub fn set_waker(&self, wake: Option<Wake>) {
        *self.waker.lock().expect("Sanim runtime waker poisoned") = wake;
        wake_host(&self.waker);
    }
    /// A cheap immutable snapshot: no source reads or preparation on this thread.
    pub fn snapshot(&self) -> Snapshot {
        Observation::read(&self.shared)
    }
    pub fn observe(&self) -> Observation {
        Observation {
            shared: self.shared.clone(),
        }
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
            state.progress.state = State::Running;
            state.reloading = state.prepared.is_some();
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
        self.stopping.store(true, Ordering::Release);
        // A full queue already wakes the worker; the stop flag prevents another job.
        let _ = self.commands.try_send(Command::Stop);
        let workers = [self.worker.take(), self.progress_worker.take()];
        let reentrant = workers
            .iter()
            .flatten()
            .any(|worker| worker.thread().id() == thread::current().id());
        if !reentrant {
            for worker in workers.into_iter().flatten() {
                worker.join().expect("Sanim runtime worker panicked");
            }
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
    entry: String,
    sources: WatchedSources,
    (limits, viewport, stopping): (Limits, Viewport, Arc<AtomicBool>),
    receiver: mpsc::Receiver<Command>,
    (shared, waker, progress_changes): (Shared, Waker, mpsc::SyncSender<()>),
) {
    let mut session = Session::new(&entry);
    session.engine.preparation = sanim_lang::Preparation::Demand;
    session.engine.limits = limits;
    session.engine.viewport = viewport.clone();
    let mut raw = Configuration {
        viewport,
        ..Configuration::default()
    };
    let mut input_serial = 0;
    let mut dirty = true;
    let mut input_dirty = false;
    let mut watch_error = None;
    loop {
        if stopping.load(Ordering::Acquire) {
            return;
        }
        if let Some(error) = watch_error.take() {
            publish(&shared, &waker, |state| {
                state.preparing = false;
                state.progress.state = State::Failed;
                state.diagnostic = Some(Diagnostic::new(
                    "S405",
                    format!("file watcher failed: {error}; reopen the source to restore watching"),
                    &entry,
                    Default::default(),
                ));
            });
            return;
        }
        if dirty {
            let started = Instant::now();
            let (previous_progress, previous_reloading, previous_generation) = {
                let state = shared.0.lock().expect("Sanim runtime state poisoned");
                (state.progress.clone(), state.reloading, state.generation)
            };
            let engine_progress = Arc::new(AtomicBool::new(false));
            session.engine.set_progress_observer(Some(Arc::new({
                let shared = shared.clone();
                let progress_changes = progress_changes.clone();
                let engine_progress = engine_progress.clone();
                let stopping = stopping.clone();
                move |mut progress: Progress| {
                    if stopping.load(Ordering::Acquire) {
                        return;
                    }
                    engine_progress.store(true, Ordering::Relaxed);
                    // Individual timing/full passes do not complete the whole published reload.
                    progress.state = State::Running;
                    progress.elapsed_micros =
                        started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
                    {
                        let mut state = shared.0.lock().expect("Sanim runtime state poisoned");
                        state.preparing = true;
                        state.progress = progress;
                        state.reloading = state.prepared.is_some();
                    }
                    let _ = progress_changes.try_send(());
                }
            })));
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
            if stopping.load(Ordering::Acquire) {
                return;
            }
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
                    s.progress.state = if s.preparing {
                        State::Running
                    } else {
                        State::Ready
                    };
                    s.progress.elapsed_micros =
                        started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
                    s.reloading = s.preparing;
                }),
                Reload::Failed { mut diagnostic, .. } => {
                    if diagnostic.source.is_empty() {
                        diagnostic.source = entry.clone();
                    }
                    publish(&shared, &waker, |s| {
                        s.diagnostic = Some(diagnostic);
                        s.applied_inputs = input_serial;
                        s.preparing = s.applied_inputs < s.requested_inputs;
                        s.progress.state = if s.preparing {
                            State::Running
                        } else {
                            State::Failed
                        };
                        s.progress.elapsed_micros =
                            started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
                        s.reloading = s.prepared.is_some();
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
                    let observed = engine_progress.load(Ordering::Relaxed);
                    let changed = state.diagnostic != diagnostic
                        || state.applied_inputs != input_serial
                        || state.planned_duration != duration
                        || state.generation != previous_generation
                        || observed;
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
                    if observed {
                        state.progress.state = if state.preparing {
                            State::Running
                        } else if state.diagnostic.is_some() {
                            State::Failed
                        } else {
                            State::Ready
                        };
                        state.progress.elapsed_micros =
                            started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
                        state.reloading = state.prepared.is_some()
                            && (state.preparing || state.diagnostic.is_some());
                    } else {
                        state.progress = previous_progress;
                        state.reloading = previous_reloading;
                    }
                    drop(state);
                    if changed {
                        wake_host(&waker);
                    }
                }
            }
            sources.retain(session.dependencies());
            input_dirty = false;
            dirty = false;
        }
        match receiver.recv() {
            Ok(Command::Stop) | Err(_) => break,
            Ok(Command::WatchError(error)) => {
                watch_error = Some(error);
            }
            Ok(Command::Inputs(serial, values)) => {
                if serial > input_serial {
                    raw = values;
                    input_serial = serial;
                }
                input_dirty = true;
                dirty = true;
            }
            Ok(Command::Changed) => dirty = true,
            Ok(Command::FontsChanged) => dirty |= session.font_environment_changed(),
        }
        // Coalesce already-queued writes and parameter updates into one revision.
        while let Ok(command) = receiver.try_recv() {
            match command {
                Command::Stop => return,
                Command::Changed => dirty = true,
                Command::FontsChanged => dirty |= session.font_environment_changed(),
                Command::WatchError(error) => {
                    watch_error = Some(error);
                }
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
        if watch_error.is_some() {
            continue;
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
                    Ok(Command::FontsChanged) => dirty |= session.font_environment_changed(),
                    Ok(Command::WatchError(error)) => {
                        watch_error = Some(error);
                        break;
                    }
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
