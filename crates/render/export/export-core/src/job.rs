//! Bounded export progress and cancellation for native UI hosts.
use crate::video::ExportProgress;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

pub struct Update<T = ()> {
    pub progress: Option<ExportProgress>,
    pub finished: Option<Result<T, String>>,
}

impl<T> Default for Update<T> {
    fn default() -> Self {
        Self {
            progress: None,
            finished: None,
        }
    }
}

pub struct Job<T = ()> {
    cancelled: Arc<AtomicBool>,
    latest: Arc<Mutex<Update<T>>>,
    worker: Option<JoinHandle<()>>,
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
        let latest = Arc::new(Mutex::new(Update::default()));
        let pending = latest.clone();
        let worker_cancelled = cancelled.clone();
        let worker = std::thread::Builder::new()
            .name("shrimply-export".into())
            .spawn(move || {
                let result = operation(worker_cancelled, &mut |progress| {
                    pending.lock().expect("export progress poisoned").progress = Some(progress);
                    wake();
                });
                pending.lock().expect("export progress poisoned").finished = Some(result);
                wake();
            })
            .map_err(|error| format!("Start export worker: {error}"))?;
        Ok(Self {
            cancelled,
            latest,
            worker: Some(worker),
        })
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn poll(&mut self) -> Update<T> {
        let update = std::mem::take(&mut *self.latest.lock().expect("export progress poisoned"));
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            self.worker
                .take()
                .expect("finished export worker")
                .join()
                .expect("export worker panicked");
        }
        update
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("export worker panicked");
        }
    }
}
