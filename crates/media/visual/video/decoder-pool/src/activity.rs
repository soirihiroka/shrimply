use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
pub struct DecoderActivity {
    pending: Arc<AtomicUsize>,
}

impl DecoderActivity {
    pub(crate) fn new() -> Self {
        Self {
            pending: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn begin(&self) -> DecoderActivityGuard {
        self.pending
            .try_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                pending.checked_add(1)
            })
            .expect("decoder activity count overflowed");
        DecoderActivityGuard {
            pending: self.pending.clone(),
        }
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.pending.load(Ordering::Acquire) == 0
    }
}

pub struct DecoderActivityGuard {
    pending: Arc<AtomicUsize>,
}

impl Drop for DecoderActivityGuard {
    fn drop(&mut self) {
        self.pending
            .try_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                pending.checked_sub(1)
            })
            .expect("decoder activity completed work that was not pending");
    }
}
