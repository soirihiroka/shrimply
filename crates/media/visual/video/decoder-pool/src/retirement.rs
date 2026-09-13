use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};

pub(super) struct Retirement<D> {
    sender: Option<mpsc::Sender<RetirementWork<D>>>,
    pub(super) pending: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}

enum RetirementWork<D> {
    Decoder(D),
    Barrier(mpsc::SyncSender<()>),
}

impl<D: Send + 'static> Retirement<D> {
    pub(super) fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let pending = Arc::new(AtomicUsize::new(0));
        let worker_pending = pending.clone();
        let worker = thread::Builder::new()
            .name("decoder-retirement".to_string())
            .spawn(move || {
                for work in receiver {
                    match work {
                        RetirementWork::Decoder(decoder) => {
                            drop(decoder);
                            worker_pending
                                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                                    pending.checked_sub(1)
                                })
                                .expect("decoder retirement count underflowed");
                        }
                        RetirementWork::Barrier(reply) => {
                            let _ = reply.send(());
                        }
                    }
                }
            })
            .expect("could not spawn decoder retirement worker");
        Self {
            sender: Some(sender),
            pending,
            worker: Some(worker),
        }
    }
}

impl<D> Retirement<D> {
    pub(super) fn retire(&self, decoder: D) {
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                pending.checked_add(1)
            })
            .expect("decoder retirement count overflowed");
        self.sender
            .as_ref()
            .expect("decoder retirement worker is shutting down")
            .send(RetirementWork::Decoder(decoder))
            .unwrap_or_else(|_| panic!("decoder retirement worker stopped"));
    }

    pub(super) fn barrier(&self) -> mpsc::Receiver<()> {
        let (reply, completion) = mpsc::sync_channel(1);
        self.sender
            .as_ref()
            .expect("decoder retirement worker is shutting down")
            .send(RetirementWork::Barrier(reply))
            .unwrap_or_else(|_| panic!("decoder retirement worker stopped"));
        completion
    }
}

impl<D> Drop for Retirement<D> {
    fn drop(&mut self) {
        self.sender.take();
        self.worker
            .take()
            .expect("decoder retirement worker missing during shutdown")
            .join()
            .expect("decoder retirement worker panicked during shutdown");
    }
}
