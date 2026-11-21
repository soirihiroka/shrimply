//! Background preparation and dependency watching, independent of any editor.
mod watcher;
use watcher::WatchedSources;
mod notifications;
mod pending;
use pending::Pending;
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
    /// Last valid source has an intrinsic indefinite extent, independent of its host crop.
    pub indefinite: bool,
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
    inputs: Arc<BTreeMap<String, serde_json::Value>>,
    viewport: Viewport,
    available_duration: Option<sanim_core::Fraction>,
}
enum Command {
    Changed,
    Refresh,
    FontsChanged,
    WatchError(String),
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
    pending: Arc<Pending>,
    shared: Shared,
    worker: Option<thread::JoinHandle<()>>,
    progress_worker: Option<thread::JoinHandle<()>>,
    waker: Waker,
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
        let root = match parent.canonicalize() {
            Ok(root) => root,
            // A missing clip directory is a recoverable source diagnostic, not a
            // failure to construct the editor's runtime or open its project.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::path::absolute(parent)
                    .map_err(|error| format!("Sanim source directory: {error}"))?
            }
            Err(error) => return Err(format!("Sanim source directory: {error}")),
        };
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
        let pending = Arc::new(Pending::new(Configuration {
            viewport: viewport.clone(),
            ..Default::default()
        }));
        let font_changes = sanim_data::font::subscribe(Arc::new({
            let pending = pending.clone();
            move || pending.event(Command::FontsChanged)
        }));
        let sources = WatchedSources::new(root, &entry, {
            let pending = pending.clone();
            move |event| pending.event(event)
        })?;
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
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let worker_pending = pending.clone();
        let worker = thread::Builder::new()
            .name("sanim-runtime".into())
            .spawn(move || {
                worker(
                    entry,
                    sources,
                    (limits, viewport, worker_stopping),
                    worker_pending,
                    (published, worker_waker, progress_changes),
                )
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            pending,
            shared,
            worker: Some(worker),
            progress_worker: Some(progress_worker),
            waker,
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
        if self.stopping.load(Ordering::Acquire) {
            return Err("Sanim runtime stopped".into());
        }
        self.pending.event(Command::Refresh);
        Ok(())
    }
    /// Replace the complete reflected override snapshot. Missing keys reset to defaults.
    pub fn set_inputs(&self, inputs: BTreeMap<String, serde_json::Value>) -> Result<(), String> {
        self.configure(|configuration| {
            if configuration.inputs.as_ref() == &inputs {
                return false;
            }
            configuration.inputs = Arc::new(inputs);
            true
        })
    }
    pub fn set_viewport(&self, viewport: Viewport) -> Result<(), String> {
        viewport.validate()?;
        self.configure(|configuration| {
            if configuration.viewport == viewport {
                return false;
            }
            configuration.viewport = viewport;
            true
        })
    }
    pub fn set_time_extent(&self, duration: Option<sanim_core::Fraction>) -> Result<(), String> {
        if let Some(duration) = duration {
            sanim_core::math::temporal::viewport(duration)?;
        }
        self.configure(|configuration| {
            if configuration.available_duration == duration {
                return false;
            }
            configuration.available_duration = duration;
            true
        })
    }
    fn configure(&self, change: impl FnOnce(&mut Configuration) -> bool) -> Result<(), String> {
        let mut pending = self
            .pending
            .state
            .lock()
            .expect("Sanim pending state poisoned");
        if self.stopping.load(Ordering::Acquire) {
            return Err("Sanim runtime stopped".into());
        }
        if !change(&mut pending.configuration) {
            return Ok(());
        }
        pending.serial = pending
            .serial
            .checked_add(1)
            .expect("Sanim input generation overflow");
        {
            let mut state = self.shared.0.lock().expect("Sanim runtime state poisoned");
            state.requested_inputs = pending.serial;
            state.preparing = true;
            state.progress.state = State::Running;
            state.reloading = state.prepared.is_some();
        }
        drop(pending);
        self.pending.wake.notify_one();
        Ok(())
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
        self.pending.event(Command::Stop);
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
    pending: Arc<Pending>,
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
    let mut applied = raw.clone();
    let mut input_serial = 0;
    loop {
        let mut batch = pending
            .take(input_serial, None)
            .expect("blocking pending receive");
        if !batch.stop && batch.error.is_none() && batch.source && batch.configuration.is_none() {
            // Native editors can truncate/rewrite or atomically replace the same file.
            const QUIET: Duration = Duration::from_millis(25);
            const MAX_DEBOUNCE: Duration = Duration::from_millis(100);
            let limit = Instant::now() + MAX_DEBOUNCE;
            loop {
                let timeout = QUIET.min(limit.saturating_duration_since(Instant::now()));
                if timeout.is_zero() {
                    break;
                }
                let Some(next) = pending.take(input_serial, Some(timeout)) else {
                    break;
                };
                batch.merge(next);
                if batch.stop || batch.error.is_some() || batch.configuration.is_some() {
                    break;
                }
            }
        }
        if batch.stop || stopping.load(Ordering::Acquire) {
            return;
        }
        if let Some(error) = batch.error {
            stopping.store(true, Ordering::Release);
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
        if batch.refresh {
            sources.invalidate_all();
        }
        let source_dirty =
            batch.source || batch.refresh || batch.fonts && session.font_environment_changed();
        let input_dirty = batch.configuration.is_some();
        if let Some((serial, configuration)) = batch.configuration {
            input_serial = serial;
            raw = configuration;
        }
        if !source_dirty && !input_dirty {
            continue;
        }
        {
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
            let inputs_changed =
                !Arc::ptr_eq(&raw.inputs, &applied.inputs) && raw.inputs != applied.inputs;
            let layout_only =
                input_dirty && !source_dirty && session.current().is_some() && !inputs_changed;
            if input_dirty {
                session.engine.available_duration = raw.available_duration;
                session.engine.viewport = raw.viewport.clone();
                if inputs_changed {
                    session.set_json_inputs(raw.inputs.as_ref().clone());
                }
            }
            let outcome = if layout_only {
                session.relayout(&raw.viewport, raw.available_duration)
            } else {
                session.poll_with_duration(&sources, |duration| {
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
                })
            };
            applied = raw.clone();
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
                    s.indefinite = prepared.natural_extent() == sanim_core::plan::Extent::Infinite;
                    s.revision = revision;
                    s.prepared = Some(prepared);
                    s.diagnostic = session.last_error().cloned();
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
                        state.indefinite = session.current().is_some_and(|prepared| {
                            prepared.natural_extent() == sanim_core::plan::Extent::Infinite
                        });
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
        }
    }
}
