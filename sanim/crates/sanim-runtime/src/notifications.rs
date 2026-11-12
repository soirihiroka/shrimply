//! At most one pending progress notification; snapshots always contain the latest observed work.
use super::{Shared, State, Waker, wake_host};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

pub(crate) const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) fn run(shared: Shared, waker: Waker, receiver: mpsc::Receiver<()>) {
    let mut next = Instant::now() + PROGRESS_INTERVAL;
    while receiver.recv().is_ok() {
        loop {
            let remaining = next.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match receiver.recv_timeout(remaining) {
                Ok(()) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        let running = {
            let mut state = shared.0.lock().expect("Sanim runtime state poisoned");
            if state.progress.state == State::Running {
                state.generation += 1;
                shared.1.notify_all();
                true
            } else {
                false
            }
        };
        // Observer code can read snapshots/configure inputs or replace its own waker.
        if running {
            wake_host(&waker);
        }
        next = Instant::now() + PROGRESS_INTERVAL;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Phase, Progress, Snapshot};
    use std::sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn progress_notifications_coalesce_and_observers_can_read_the_snapshot() {
        let shared = Arc::new((Mutex::new(Snapshot::default()), Condvar::new()));
        let (seen_tx, seen_rx) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let observer_shared = shared.clone();
        let observer_calls = calls.clone();
        let waker: Waker = Arc::new(Mutex::new(Some(Arc::new(move || {
            let snapshot = observer_shared.0.lock().unwrap().clone();
            observer_calls.fetch_add(1, Ordering::SeqCst);
            seen_tx.send(snapshot.progress.completed).unwrap();
        }))));
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = std::thread::spawn({
            let shared = shared.clone();
            move || run(shared, waker, receiver)
        });
        let start = Instant::now();
        const UPDATES: u64 = 10_000;
        for completed in 0..=UPDATES {
            shared.0.lock().unwrap().progress = Progress {
                phase: Phase::Evaluating,
                completed: Some(completed),
                total: Some(UPDATES),
                ..Progress::default()
            };
            let _ = sender.try_send(());
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let seen = seen_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("progress observer was not notified or deadlocked");
            if seen == Some(UPDATES) {
                break;
            }
        }
        let max_calls = start.elapsed().as_nanos() / PROGRESS_INTERVAL.as_nanos() + 2;
        assert!((calls.load(Ordering::SeqCst) as u128) <= max_calls);
        assert!(calls.load(Ordering::SeqCst) < UPDATES as usize);
        shared.0.lock().unwrap().progress.state = State::Ready;
        drop(sender);
        worker.join().unwrap();
    }
}
