use shrimply_math_core::{Fraction, Time};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub enum RenderEvent {
    Requested {
        request_id: u64,
        position: Time,
    },
    Submitted {
        request_id: u64,
        position: Time,
    },
    Completed {
        request_id: u64,
        position: Time,
        elapsed: Duration,
        project_fps: Fraction,
    },
}

pub type RenderObserver = Arc<dyn Fn(RenderEvent) + Send + Sync>;

/// Distinguish new decoded content from repeated surface redraws. Submission
/// intervals must be checked against native display presentation timestamps.
#[derive(Default)]
pub struct SourceFrameTracker {
    previous: Option<u64>,
    at: Option<Instant>,
}

impl SourceFrameTracker {
    pub fn submitted(&mut self, source: Option<u64>) {
        crate::increment("Preview / Frames submitted");
        let Some(source) = source else {
            *self = Self::default();
            return;
        };
        if self.previous == Some(source) {
            crate::increment("Preview / Repeated source frames submitted");
            return;
        }
        let now = Instant::now();
        crate::increment("Preview / New source frames submitted");
        if let Some(at) = self.at.replace(now) {
            crate::record(
                "Preview / New source frame interval",
                now.saturating_duration_since(at),
            );
        }
        self.previous = Some(source);
    }
}
