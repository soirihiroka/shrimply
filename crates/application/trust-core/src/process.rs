use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const TRUST_POLL_INTERVAL: Duration = Duration::from_millis(250);
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

struct State {
    child: std::process::Child,
    status: Option<ExitStatus>,
    failure: Option<String>,
}

/// Owns a worker process group and stops it when its source loses trust.
pub struct Child {
    state: Arc<Mutex<State>>,
    source: Option<PathBuf>,
    stop: mpsc::Sender<()>,
    monitor: Option<JoinHandle<()>>,
}

impl Child {
    pub fn spawn(command: &mut Command, source: Option<&Path>) -> Result<Self, String> {
        use std::os::unix::process::CommandExt;
        let source = source.map(super::require).transpose()?;
        let child = command
            .process_group(0)
            .spawn()
            .map_err(|error| error.to_string())?;
        let state = Arc::new(Mutex::new(State {
            child,
            status: None,
            failure: None,
        }));
        let (stop, receiver) = mpsc::channel();
        let mut result = Self {
            state: state.clone(),
            source: source.clone(),
            stop,
            monitor: None,
        };
        if let Some(source) = source {
            result.monitor = Some(
                thread::Builder::new()
                    .name("source-trust".into())
                    .spawn(move || {
                        while receiver.recv_timeout(TRUST_POLL_INTERVAL)
                            == Err(mpsc::RecvTimeoutError::Timeout)
                        {
                            if let Err(error) = super::require(&source) {
                                let mut state = state.lock().expect("trusted worker poisoned");
                                state.failure = Some(error);
                                kill_group(&mut state);
                                return;
                            }
                        }
                    })
                    .map_err(|error| format!("Could not monitor source trust: {error}"))?,
            );
        }
        Ok(result)
    }

    pub fn check_trust(&self) -> Result<(), String> {
        if let Some(error) = &self.state.lock().expect("trusted worker poisoned").failure {
            return Err(error.clone());
        }
        if let Some(source) = &self.source {
            super::require(source)?;
        }
        Ok(())
    }

    pub fn id(&self) -> u32 {
        self.state
            .lock()
            .expect("trusted worker poisoned")
            .child
            .id()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let mut state = self.state.lock().expect("trusted worker poisoned");
        if state.status.is_none() {
            state.status = state.child.try_wait()?;
        }
        Ok(state.status)
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            thread::sleep(EXIT_POLL_INTERVAL);
        }
    }

    pub fn terminate(&mut self) {
        kill_group(&mut self.state.lock().expect("trusted worker poisoned"));
    }
}

fn kill_group(state: &mut State) {
    // The lock serializes signaling with reaping, preventing PID reuse races.
    if state.status.is_some() {
        return;
    }
    let pid = i32::try_from(state.child.id()).expect("worker PID exceeds i32");
    if unsafe { libc::kill(-pid, libc::SIGKILL) } != 0
        && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    {
        std::process::abort();
    }
    state.status = Some(state.child.wait().expect("could not reap trusted worker"));
}

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(monitor) = self.monitor.take() {
            monitor.join().expect("source trust monitor panicked");
        }
        self.terminate();
    }
}
