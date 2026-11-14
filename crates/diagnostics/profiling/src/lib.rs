use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use hashbrown::HashMap;
use serde::Serialize;

mod render;
pub use render::{RenderEvent, RenderObserver};

const SAMPLE_LIMIT: usize = 120;

#[derive(Clone, Debug)]
pub struct TimingSnapshot {
    pub name: &'static str,
    pub samples: usize,
    pub last: Duration,
    pub average: Duration,
    pub minimum: Duration,
    pub maximum: Duration,
}

#[derive(Clone, Debug)]
pub struct CounterSnapshot {
    pub name: &'static str,
    pub value: u64,
}

#[derive(Clone, Debug, Default)]
pub struct BenchmarkSnapshot {
    pub timings: Vec<TimingSnapshot>,
    pub counters: Vec<CounterSnapshot>,
}

#[must_use]
pub struct Measurement {
    name: &'static str,
    started: Instant,
}

#[derive(Clone, Default)]
struct Registry {
    timings: HashMap<&'static str, VecDeque<Duration>>,
    counters: HashMap<&'static str, u64>,
}

pub fn measure(name: &'static str) -> Measurement {
    Measurement {
        name,
        started: Instant::now(),
    }
}

pub fn record(name: &'static str, elapsed: Duration) {
    let mut registry = registry()
        .lock()
        .expect("benchmark registry mutex poisoned");
    let samples = registry.timings.entry(name).or_default();
    if samples.len() == SAMPLE_LIMIT {
        samples.pop_front();
    }
    samples.push_back(elapsed);
}

pub fn increment(name: &'static str) {
    add_to_counter(name, 1);
}

pub fn add_to_counter(name: &'static str, amount: u64) {
    let mut registry = registry()
        .lock()
        .expect("benchmark registry mutex poisoned");
    let value = registry.counters.entry(name).or_default();
    *value = value.saturating_add(amount);
}

pub fn set_counter(name: &'static str, value: u64) {
    registry()
        .lock()
        .expect("benchmark registry mutex poisoned")
        .counters
        .insert(name, value);
}

pub fn snapshot() -> BenchmarkSnapshot {
    let registry = registry()
        .lock()
        .expect("benchmark registry mutex poisoned")
        .clone();
    BenchmarkSnapshot {
        timings: registry
            .timings
            .iter()
            .filter_map(|(&name, samples)| {
                let last = samples.back().copied()?;
                let total_ns = samples.iter().fold(0_u128, |total, sample| {
                    total.saturating_add(sample.as_nanos())
                });
                let average_ns = total_ns / samples.len() as u128;
                Some(TimingSnapshot {
                    name,
                    samples: samples.len(),
                    last,
                    average: Duration::from_nanos(average_ns.min(u128::from(u64::MAX)) as u64),
                    minimum: samples
                        .iter()
                        .min()
                        .copied()
                        .expect("samples are not empty"),
                    maximum: samples
                        .iter()
                        .max()
                        .copied()
                        .expect("samples are not empty"),
                })
            })
            .collect(),
        counters: registry
            .counters
            .iter()
            .map(|(&name, &value)| CounterSnapshot { name, value })
            .collect(),
    }
}

pub fn clear() {
    *registry()
        .lock()
        .expect("benchmark registry mutex poisoned") = Registry::default();
}

pub fn report_json() -> String {
    #[derive(Serialize)]
    struct Report {
        timing_columns: [&'static str; 5],
        timings_ns: BTreeMap<&'static str, [u128; 5]>,
        counters: BTreeMap<&'static str, u64>,
    }

    let snapshot = snapshot();
    serde_json::to_string(&Report {
        timing_columns: ["samples", "last", "average", "minimum", "maximum"],
        timings_ns: snapshot
            .timings
            .into_iter()
            .map(|timing| {
                (
                    timing.name,
                    [
                        timing.samples as u128,
                        timing.last.as_nanos(),
                        timing.average.as_nanos(),
                        timing.minimum.as_nanos(),
                        timing.maximum.as_nanos(),
                    ],
                )
            })
            .collect(),
        counters: snapshot
            .counters
            .into_iter()
            .map(|counter| (counter.name, counter.value))
            .collect(),
    })
    .expect("benchmark report is serializable")
}

impl Drop for Measurement {
    fn drop(&mut self) {
        record(self.name, self.started.elapsed());
    }
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}
