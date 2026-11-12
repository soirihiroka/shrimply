//! Observed preparation work. Counts describe the current phase, never an estimated overall percentage.
use crate::Sources;
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, sync::Arc, time::Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    #[default]
    Running,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Sources,
    Checking,
    Duration,
    Restoring,
    Evaluating,
    Reading,
    Decoding,
    Querying,
    Shaping,
    Verifying,
    Finalizing,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sources => "Loading sources",
            Self::Checking => "Checking types",
            Self::Duration => "Planning duration",
            Self::Restoring => "Restoring saved execution state",
            Self::Evaluating => "Evaluating statements",
            Self::Reading => "Reading resource",
            Self::Decoding => "Decoding data",
            Self::Querying => "Running SQLite query",
            Self::Shaping => "Shaping text",
            Self::Verifying => "Verifying visual contracts",
            Self::Finalizing => "Verifying source snapshot",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub state: State,
    pub phase: Phase,
    /// Actual completed phase units: modules for Sources/Checking, top-level
    /// statements for Duration/Evaluating, bytes for Reading, contracts for
    /// Verifying, and dependencies for Finalizing. Other phases are indeterminate.
    /// None means the operation provides no count.
    pub completed: Option<u64>,
    /// A total is supplied only when known; do not derive an overall percentage from it.
    pub total: Option<u64>,
    pub resource: Option<String>,
    pub elapsed_micros: u64,
}

/// Engine observers run on the preparing thread. Runtime hosts use the coalesced wake API instead.
pub type Observer = Arc<dyn Fn(Progress) + Send + Sync>;

pub(crate) struct Reporter {
    observer: Option<Observer>,
    pub started: Instant,
    last: RefCell<Progress>,
}
impl Reporter {
    pub fn new(observer: Option<Observer>, resource: &str) -> Self {
        let reporter = Self {
            observer,
            started: Instant::now(),
            last: RefCell::new(Progress::default()),
        };
        reporter.emit(Phase::Sources, Some(0), None, Some(resource));
        reporter
    }
    pub fn enabled(&self) -> bool {
        self.observer.is_some()
    }
    pub fn emit(
        &self,
        phase: Phase,
        completed: Option<u64>,
        total: Option<u64>,
        resource: Option<&str>,
    ) {
        let Some(observer) = &self.observer else {
            return;
        };
        let progress = Progress {
            state: State::Running,
            phase,
            completed,
            total,
            resource: resource.map(str::to_owned),
            elapsed_micros: self
                .started
                .elapsed()
                .as_micros()
                .try_into()
                .unwrap_or(u64::MAX),
        };
        // Release the borrow before invoking user code, including reentrant observers.
        self.last.replace(progress.clone());
        observer(progress);
    }
    pub fn finish(&self, error: Option<&sanim_core::Diagnostic>) {
        let Some(observer) = &self.observer else {
            return;
        };
        let mut progress = self.last.borrow().clone();
        progress.state = if error.is_some() {
            State::Failed
        } else {
            State::Ready
        };
        if let Some(error) = error
            && !error.source.is_empty()
        {
            progress.resource = Some(error.source.clone());
        }
        progress.elapsed_micros = self
            .started
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX);
        self.last.replace(progress.clone());
        observer(progress);
    }
}

pub(crate) struct ObservedSources<'a> {
    pub provider: &'a dyn Sources,
    pub reporter: &'a Reporter,
}
impl Sources for ObservedSources<'_> {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.reporter.emit(Phase::Reading, None, None, Some(path));
        let bytes = self.provider.read(path)?;
        let count = bytes.len() as u64;
        self.reporter
            .emit(Phase::Reading, Some(count), Some(count), Some(path));
        Ok(bytes)
    }
}
