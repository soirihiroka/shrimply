use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

static OWNER: Mutex<Option<Owner>> = Mutex::new(None);

struct Owner {
    path: PathBuf,
    editors: usize,
    stop: Arc<AtomicBool>,
    worker: JoinHandle<Result<(), String>>,
}

/// Keep the Android project owner alive while editors in this process are attached.
pub struct Lease {
    _private: (),
}

impl Lease {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut slot = OWNER.lock().expect("editing owner lock poisoned");
        if let Some(owner) = slot.as_mut() {
            if owner.path != path || owner.worker.is_finished() {
                return Err("Close the active editing session before opening this project".into());
            }
            owner.editors += 1;
            return Ok(Self { _private: () });
        }

        let path = path.to_path_buf();
        let target = path.clone();
        let (ready, started) = mpsc::sync_channel(0);
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let worker = thread::Builder::new()
            .name("shrimply-editing-owner".into())
            .spawn(move || {
                super::serve_until(&target, &signal, move || {
                    let _ = ready.send(());
                })
            })
            .map_err(|error| error.to_string())?;
        if started.recv().is_err() {
            worker.join().map_err(|_| "The editing service crashed")??;
            return Err("The editing service stopped during startup".into());
        }
        *slot = Some(Owner {
            path,
            editors: 1,
            stop,
            worker,
        });
        Ok(Self { _private: () })
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut slot = OWNER.lock().expect("editing owner lock poisoned");
        let owner = slot.as_mut().expect("lease has an owner");
        owner.editors -= 1;
        if owner.editors == 0 {
            let owner = slot.take().expect("editing owner is active");
            owner.stop.store(true, Ordering::Release);
            match owner.worker.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::error!(%error, "Could not close the editing service"),
                Err(_) => tracing::error!("The editing service crashed"),
            }
        }
    }
}
