use tracing_subscriber::EnvFilter;
use std::{cell::RefCell, collections::{BTreeMap, HashMap}, time::{Duration, Instant}};

const DEFAULT_FILTER: &str = "info,shrimply=debug";
const TIMING_LOG_INTERVAL: Duration = Duration::from_secs(1);

thread_local! {
    static TIMINGS: RefCell<HashMap<&'static str, (u64, Duration, Duration)>> = RefCell::new(HashMap::new());
    static COUNTS: RefCell<BTreeMap<&'static str, u64>> = const { RefCell::new(BTreeMap::new()) };
    static WINDOW: RefCell<Instant> = RefCell::new(Instant::now());
}

/// Plain periodic log summaries; independent of the performance inspector.
pub fn timing(stage: &'static str) -> Timing {
    Timing { stage, started: Instant::now() }
}

pub struct Timing {
    stage: &'static str,
    started: Instant,
}

impl Drop for Timing {
    fn drop(&mut self) {
        record_timing(self.stage, self.started.elapsed());
    }
}

pub fn record_timing(stage: &'static str, elapsed: Duration) {
    TIMINGS.with(|timings| {
        let mut timings = timings.borrow_mut();
        let (calls, total, maximum) = timings.entry(stage).or_default();
        *calls += 1;
        *total += elapsed;
        *maximum = (*maximum).max(elapsed);
    });
}

/// Count causes, including multiple causes for one redraw, without logging per frame.
pub fn count(event: &'static str) {
    COUNTS.with(|counts| *counts.borrow_mut().entry(event).or_default() += 1);
}

/// Called at the start of the UI callback: all completed stages share one window,
/// including stages that stopped running. Sparse draws cannot retain old samples.
pub fn flush_timings() {
    let window = WINDOW.with(|since| {
        let mut since = since.borrow_mut();
        let now = Instant::now();
        let elapsed = now.duration_since(*since);
        if elapsed < TIMING_LOG_INTERVAL { return None; }
        *since = now;
        Some(elapsed)
    });
    let Some(window) = window else { return; };
    let window_us = window.as_micros();
    TIMINGS.with(|timings| {
        for (stage, samples) in timings.borrow_mut().iter_mut() {
            let (calls, total, maximum) = std::mem::take(samples);
            let total_us = total.as_micros();
            tracing::info!(stage, window_us, calls, total_us,
                average_us = total_us.checked_div(u128::from(calls)).unwrap_or(0),
                max_us = maximum.as_micros(), "UI lifecycle timing");
        }
    });
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        tracing::info!(window_us, counts = ?*counts, "UI lifecycle counts");
        for count in counts.values_mut() { *count = 0; }
    });
}

/// Installs the process-wide diagnostics subscriber.
///
/// Application code emits `tracing` spans and events. Logs emitted by dependencies through the
/// `log` facade are forwarded by `tracing-subscriber` into the same output.
pub fn init() {
    let filter = EnvFilter::new(std::env::var("RUST_LOG").map_or_else(
        |_| DEFAULT_FILTER.to_string(),
        |directives| format!("{DEFAULT_FILTER},{directives}"),
    ));
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .with_thread_ids(true)
        .with_thread_names(true)
        .try_init()
        .expect("diagnostics subscriber should only be installed once");
}
