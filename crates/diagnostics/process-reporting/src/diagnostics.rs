use tracing_subscriber::EnvFilter;
use std::{cell::RefCell, collections::HashMap, time::{Duration, Instant}};

const DEFAULT_FILTER: &str = "info,shrimply=debug";
const TIMING_LOG_INTERVAL: Duration = Duration::from_secs(1);

thread_local! {
    static TIMINGS: RefCell<HashMap<&'static str, (Instant, u64, Duration, Duration)>> = RefCell::new(HashMap::new());
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
        let elapsed = self.started.elapsed();
        let summary = TIMINGS.with(|timings| {
            let mut timings = timings.borrow_mut();
            let (since, calls, total, maximum) = timings.entry(self.stage)
                .or_insert((Instant::now(), 0, Duration::ZERO, Duration::ZERO));
            *calls += 1;
            *total += elapsed;
            *maximum = (*maximum).max(elapsed);
            if since.elapsed() < TIMING_LOG_INTERVAL { return None; }
            let summary = (*calls, total.as_micros(), maximum.as_micros());
            *since = Instant::now();
            *calls = 0;
            *total = Duration::ZERO;
            *maximum = Duration::ZERO;
            Some(summary)
        });
        if let Some((calls, total_us, max_us)) = summary {
            tracing::info!(stage = self.stage, calls, total_us, average_us = total_us / u128::from(calls), max_us, "UI lifecycle timing");
        }
    }
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
