use std::sync::{Arc, Condvar, Mutex};

use crate::DECODER_FREE_MEMORY_RESERVE_DIVISOR;
use crate::session::{DecodeControl, DecodeControls, cuda_memory_info};
use crate::track::VideoSource;

pub(crate) const DECODER_STARTUP_MEMORY_EXHAUSTED: &str =
    "not enough free CUDA memory to initialize video decoder";

pub fn is_decoder_startup_pressure(error: &str) -> bool {
    error.contains(DECODER_STARTUP_MEMORY_EXHAUSTED)
}

#[derive(Clone, Default)]
pub(crate) struct VideoDecoderContext {
    startup: Arc<DecoderStartupGate>,
}

#[derive(Default)]
struct DecoderStartupGate {
    state: Mutex<DecoderStartupState>,
    ready: Condvar,
}

#[derive(Default)]
struct DecoderStartupState {
    active: bool,
    observed_bytes: u64,
}

pub(crate) struct DecoderStartupMeasurement {
    startup: Arc<DecoderStartupGate>,
    free_before: u64,
    speculative: bool,
    finished: bool,
}

impl Drop for DecoderStartupMeasurement {
    fn drop(&mut self) {
        let mut state = self
            .startup
            .state
            .lock()
            .expect("video decoder startup mutex poisoned");
        if !self.finished {
            state.active = false;
            self.startup.ready.notify_all();
        }
    }
}

impl VideoDecoderContext {
    pub(crate) fn begin_startup(
        &self,
        source: &VideoSource,
        required: bool,
        controls: DecodeControls<'_>,
        retained_bytes: u64,
    ) -> Result<Option<DecoderStartupMeasurement>, String> {
        let mut state = self
            .startup
            .state
            .lock()
            .expect("video decoder startup mutex poisoned");
        while state.active {
            if !required
                || controls
                    .into_iter()
                    .flatten()
                    .any(DecodeControl::superseded)
            {
                return Ok(None);
            }
            state = self
                .startup
                .ready
                .wait(state)
                .expect("video decoder startup mutex poisoned");
        }
        if controls
            .into_iter()
            .flatten()
            .any(DecodeControl::superseded)
        {
            return Ok(None);
        }
        let (free, total) = cuda_memory_info()?;
        let free = u64::try_from(free).map_err(|_| "CUDA free memory exceeds u64".to_string())?;
        let total =
            u64::try_from(total).map_err(|_| "CUDA total memory exceeds u64".to_string())?;
        let required_free = (total / DECODER_FREE_MEMORY_RESERVE_DIVISOR as u64)
            .checked_add(state.observed_bytes.saturating_sub(retained_bytes))
            .ok_or_else(|| "video decoder startup memory requirement overflowed".to_string())?;
        if free < required_free {
            trace_startup_throttled(source, free, total, required_free, &state);
            if required {
                return Err(format!(
                    "{DECODER_STARTUP_MEMORY_EXHAUSTED}: free={free}, required={required_free}"
                ));
            }
            crate::report_decoder_pressure(state.observed_bytes);
            return Ok(None);
        }
        state.active = true;
        Ok(Some(DecoderStartupMeasurement {
            startup: self.startup.clone(),
            free_before: free,
            speculative: !required,
            finished: false,
        }))
    }
}

impl DecoderStartupMeasurement {
    pub(crate) fn finish(mut self, error: Option<&str>, retained_bytes: &mut u64) -> bool {
        let memory_after = cuda_memory_info()
            .and_then(|(free, total)| {
                Ok((
                    u64::try_from(free)
                        .map_err(|_| "CUDA free memory exceeds u64".to_string())?,
                    u64::try_from(total)
                        .map_err(|_| "CUDA total memory exceeds u64".to_string())?,
                ))
            })
            .inspect_err(|error| {
                tracing::warn!(%error, "could not measure CUDA memory after decoder startup")
            })
            .ok();
        let mut state = self
            .startup
            .state
            .lock()
            .expect("video decoder startup mutex poisoned");
        let pressure_failure = error.is_some_and(|error| {
            error.contains("out of memory")
                || error.contains("OUT_OF_MEMORY")
                || (error.starts_with("NVDEC") && error.contains("external library"))
        }) || memory_after.is_some_and(|(free, total)| {
            error.is_some() && free < total / DECODER_FREE_MEMORY_RESERVE_DIVISOR as u64
        });
        // A cancelled target can leave a healthy, partially initialized session. Account for
        // each gated slice so resuming it neither forgets its allocations nor reserves them twice.
        if let Some((free_after, _)) = memory_after {
            *retained_bytes =
                retained_bytes.saturating_add(self.free_before.saturating_sub(free_after));
            state.observed_bytes = state.observed_bytes.max(*retained_bytes);
            shrimply_profiling::set_counter(
                "Temporal decoder state / Observed startup GPU bytes",
                state.observed_bytes,
            );
        }
        if pressure_failure {
            shrimply_profiling::increment("Temporal decoder / Starts failed under GPU pressure");
        }
        if pressure_failure && self.speculative {
            crate::report_decoder_pressure(u64::MAX);
        }
        state.active = false;
        self.finished = true;
        self.startup.ready.notify_all();
        pressure_failure
    }
}

fn trace_startup_throttled(
    source: &VideoSource,
    free: u64,
    total: u64,
    required: u64,
    state: &DecoderStartupState,
) {
    shrimply_profiling::increment("Temporal decoder / Starts throttled by GPU pressure");
    tracing::trace!(
        file = %source.asset.path().display(),
        media_track_id = source.media_track_id,
        free_vram_bytes = free,
        total_vram_bytes = total,
        required_vram_bytes = required,
        observed_startup_bytes = state.observed_bytes,
        "throttled video decoder startup under GPU pressure",
    );
}
