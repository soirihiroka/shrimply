use shrimply_math_core::{Fraction, Time};
use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug)]
pub enum RenderEvent {
    Requested {
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
