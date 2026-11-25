//! Bounded lifecycle events for native export jobs.
use crate::video::ExportProgress;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

pub enum Event<T> {
    Started,
    Progress(ExportProgress),
    Cancelling,
    Finished(Outcome<T>),
}

pub enum Outcome<T> {
    Complete(T),
    Cancelled,
    Failed(String),
}

struct Update<T> {
    started: bool,
    progress: Option<ExportProgress>,
    finished: Option<Outcome<T>>,
}

impl<T> Default for Update<T> {
    fn default() -> Self {
        Self {
            started: false,
            progress: None,
            finished: None,
        }
    }
}

pub fn run<T>(
    cancelled: Arc<AtomicBool>,
    operation: impl FnOnce(Arc<AtomicBool>, &mut dyn FnMut(ExportProgress)) -> Result<T, String>,
    mut emit: impl FnMut(Event<T>),
) {
    emit(Event::Started);
    let result = operation(cancelled.clone(), &mut |progress| {
        emit(Event::Progress(progress))
    });
    let outcome = match result {
        Ok(value) => Outcome::Complete(value),
        Err(_) if cancelled.load(Ordering::Acquire) => Outcome::Cancelled,
        Err(error) => Outcome::Failed(error),
    };
    emit(Event::Finished(outcome));
}

pub struct Job<T = ()> {
    cancelled: Arc<AtomicBool>,
    latest: Arc<Mutex<Update<T>>>,
    worker: Option<JoinHandle<()>>,
    cancellation_reported: bool,
    finished_reported: bool,
}

impl<T: Send + 'static> Job<T> {
    /// The operation constructs and owns its GPU backend on this worker.
    pub fn spawn(
        operation: impl FnOnce(Arc<AtomicBool>, &mut dyn FnMut(ExportProgress)) -> Result<T, String>
        + Send
        + 'static,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let latest = Arc::new(Mutex::new(Update {
            started: true,
            ..Update::default()
        }));
        let pending = latest.clone();
        let worker_cancelled = cancelled.clone();
        let worker = std::thread::Builder::new()
            .name("shrimply-export".into())
            .spawn(move || {
                run(worker_cancelled, operation, |event| {
                    {
                        let mut update = pending.lock().expect("export progress poisoned");
                        match event {
                            Event::Started => {}
                            Event::Progress(progress) => update.progress = Some(progress),
                            Event::Finished(outcome) => update.finished = Some(outcome),
                            Event::Cancelling => {
                                unreachable!("cancellation is reported by the job")
                            }
                        }
                    }
                    wake();
                });
            })
            .map_err(|error| format!("Start export worker: {error}"))?;
        Ok(Self {
            cancelled,
            latest,
            worker: Some(worker),
            cancellation_reported: false,
            finished_reported: false,
        })
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn cancellation_flag(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    pub fn is_cancelling(&self) -> bool {
        !self.finished_reported && self.cancelled.load(Ordering::Acquire)
    }

    pub fn poll(&mut self) -> impl Iterator<Item = Event<T>> + use<T> {
        let mut update =
            std::mem::take(&mut *self.latest.lock().expect("export progress poisoned"));
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished)
            && self
                .worker
                .take()
                .expect("finished export worker")
                .join()
                .is_err()
            && !self.finished_reported
            && update.finished.is_none()
        {
            update.finished = Some(Outcome::Failed(
                "The export worker stopped unexpectedly.".into(),
            ));
        }
        let cancelling = !self.cancellation_reported && self.is_cancelling();
        self.cancellation_reported |= cancelling;
        self.finished_reported |= update.finished.is_some();
        [
            update.started.then_some(Event::Started),
            update.progress.map(Event::Progress),
            cancelling.then_some(Event::Cancelling),
            update.finished.map(Event::Finished),
        ]
        .into_iter()
        .flatten()
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
