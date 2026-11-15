//! One pending configuration and sticky native events, independent of worker progress.
use super::{Command, Configuration};
use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};

#[derive(Default)]
pub(super) struct Batch {
    pub source: bool,
    pub refresh: bool,
    pub fonts: bool,
    pub error: Option<String>,
    pub stop: bool,
    pub configuration: Option<(u64, Configuration)>,
}
impl Batch {
    fn is_empty(&self) -> bool {
        !self.source && !self.refresh && !self.fonts && self.error.is_none() && !self.stop
    }
    pub fn merge(&mut self, next: Self) {
        self.source |= next.source;
        self.refresh |= next.refresh;
        self.fonts |= next.fonts;
        self.stop |= next.stop;
        if next.error.is_some() {
            self.error = next.error;
        }
        if next.configuration.is_some() {
            self.configuration = next.configuration;
        }
    }
}
pub(super) struct State {
    pub configuration: Configuration,
    pub serial: u64,
    changes: Batch,
}
pub(super) struct Pending {
    pub state: Mutex<State>,
    pub wake: Condvar,
}
impl Pending {
    pub fn new(configuration: Configuration) -> Self {
        Self {
            state: Mutex::new(State {
                configuration,
                serial: 0,
                changes: Batch {
                    source: true,
                    ..Default::default()
                },
            }),
            wake: Condvar::new(),
        }
    }
    pub fn event(&self, command: Command) {
        let mut state = self.state.lock().expect("Sanim pending state poisoned");
        match command {
            Command::Changed => state.changes.source = true,
            Command::Refresh => state.changes.refresh = true,
            Command::FontsChanged => state.changes.fonts = true,
            Command::WatchError(error) => state.changes.error = Some(error),
            Command::Stop => state.changes.stop = true,
        }
        drop(state);
        self.wake.notify_one();
    }
    pub fn take(&self, serial: u64, timeout: Option<Duration>) -> Option<Batch> {
        let state = self.state.lock().expect("Sanim pending state poisoned");
        let idle = |state: &mut State| state.changes.is_empty() && state.serial <= serial;
        let mut state = if let Some(timeout) = timeout {
            self.wake
                .wait_timeout_while(state, timeout, idle)
                .expect("Sanim pending state poisoned")
                .0
        } else {
            self.wake
                .wait_while(state, idle)
                .expect("Sanim pending state poisoned")
        };
        if state.changes.is_empty() && state.serial <= serial {
            return None;
        }
        let mut batch = std::mem::take(&mut state.changes);
        if state.serial > serial {
            batch.configuration = Some((state.serial, state.configuration.clone()));
        }
        Some(batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn source_and_failure_flags_survive_coalesced_configuration_bursts() {
        let pending = Pending::new(Configuration::default());
        pending.take(0, Some(Duration::ZERO)).unwrap();
        for serial in 1..=10_000 {
            let mut state = pending.state.lock().unwrap();
            state.serial = serial;
            state.configuration.available_duration = Some(serial.into());
            drop(state);
            pending.event(Command::Changed);
        }
        pending.event(Command::FontsChanged);
        pending.event(Command::Refresh);
        pending.event(Command::WatchError("watch failed".into()));
        let batch = pending.take(0, Some(Duration::ZERO)).unwrap();
        assert!(batch.source && batch.fonts && batch.refresh);
        assert_eq!(batch.error.as_deref(), Some("watch failed"));
        let (serial, configuration) = batch.configuration.unwrap();
        assert_eq!(serial, 10_000);
        assert_eq!(configuration.available_duration, Some(10_000.into()));
        assert!(pending.take(serial, Some(Duration::ZERO)).is_none());
        // A notification during preparation belongs to the next batch, even
        // when there is no subsequent configuration or filesystem event.
        pending.event(Command::Changed);
        let next = pending.take(serial, Some(Duration::ZERO)).unwrap();
        assert!(next.source);
        assert!(next.configuration.is_none());
    }

    #[test]
    fn configuration_batches_share_large_input_snapshots() {
        let inputs = std::sync::Arc::new(std::collections::BTreeMap::from([(
            "data".into(),
            serde_json::json!((0..32_768).collect::<Vec<_>>()),
        )]));
        let pending = Pending::new(Configuration {
            inputs: inputs.clone(),
            ..Default::default()
        });
        for serial in 1..=128 {
            let mut state = pending.state.lock().unwrap();
            state.serial = serial;
            state.configuration.available_duration = Some(serial.into());
            drop(state);
            let batch = pending.take(serial - 1, Some(Duration::ZERO)).unwrap();
            assert!(std::sync::Arc::ptr_eq(
                &inputs,
                &batch.configuration.unwrap().1.inputs
            ));
        }
        assert_eq!(std::sync::Arc::strong_count(&inputs), 2);
    }

    #[test]
    #[ignore = "release coalesced runtime notification benchmark"]
    fn pending_event_benchmark() {
        const EVENTS: u64 = 100_000;
        let pending = Pending::new(Configuration::default());
        let started = Instant::now();
        for serial in 1..=EVENTS {
            let mut state = pending.state.lock().unwrap();
            state.serial = serial;
            state.configuration.available_duration = Some(serial.into());
            drop(state);
            pending.event(Command::Changed);
        }
        let batch = pending.take(0, Some(Duration::ZERO)).unwrap();
        assert!(batch.source);
        assert_eq!(batch.configuration.unwrap().0, EVENTS);
        println!(
            "{EVENTS} sticky source/configuration updates coalesced into one batch in {}us",
            started.elapsed().as_micros()
        );
    }
}
