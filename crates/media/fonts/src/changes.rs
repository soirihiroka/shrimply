//! Portable font-environment identity and event subscriptions.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FontEpoch {
    pub revision: u64,
    pub serial: u32,
}

pub type FontChange = Arc<dyn Fn() + Send + Sync>;
type Subscribers = Mutex<Vec<Weak<dyn Fn() + Send + Sync>>>;
static SUBSCRIBERS: OnceLock<Subscribers> = OnceLock::new();

/// Keep the returned handle alive to receive registry changes. Dropped hosts
/// disappear on the next subscription/change, with no worker thread or polling.
pub fn subscribe(callback: FontChange) -> FontChange {
    let mut subscribers = SUBSCRIBERS
        .get_or_init(Default::default)
        .lock()
        .expect("font change subscribers lock died");
    subscribers.retain(|callback| callback.strong_count() > 0);
    subscribers.push(Arc::downgrade(&callback));
    callback
}

#[cfg(feature = "native")]
pub(crate) fn notify_subscribers() {
    let callbacks = {
        let mut subscribers = SUBSCRIBERS
            .get_or_init(Default::default)
            .lock()
            .expect("font change subscribers lock died");
        let mut callbacks = Vec::new();
        subscribers.retain(|callback| match callback.upgrade() {
            Some(callback) => {
                callbacks.push(callback);
                true
            }
            None => false,
        });
        callbacks
    };
    for callback in callbacks {
        callback();
    }
}
