//! Ephemeral compiled-source timing shared by importers, preview workers and timelines.
use crate::project::{CanvasSize, Time, VideoItem, VideoItemContent};
use shrimply_resource_pipeline::cache::Cache;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCES: usize = 512;
type Wake = dyn Fn() + Send + Sync;

#[derive(Clone, Hash, PartialEq, Eq)]
enum Key {
    Import(PathBuf),
    Clip(uuid::Uuid),
}

pub struct Events {
    pub duration: Time,
    pub index: Arc<shrimply_math_core::events::Index>,
}

#[derive(Clone, PartialEq)]
struct Context {
    path: PathBuf,
    parameters: BTreeMap<String, serde_json::Value>,
    canvas: CanvasSize,
    timing: Option<(Time, shrimply_math_core::Fraction, Time)>,
}
impl Context {
    fn matches(&self, item: &VideoItem, canvas: CanvasSize, duration: Time) -> bool {
        let VideoItemContent::Sanim(settings) = &item.content else {
            return false;
        };
        self.path == item.file.path()
            && self.canvas == canvas
            && self.parameters == settings.parameters
            && match self.timing {
                Some(timing) => {
                    timing
                        == (
                            item.end.signed_sub(item.start),
                            item.playback_speed,
                            item.animation_time_offset,
                        )
                }
                None => {
                    item.end.signed_sub(item.start) == duration
                        && item.playback_speed == shrimply_math_core::Fraction::from(1)
                        && item.animation_time_offset == Time::ZERO
                }
            }
    }
}
/// Minimal immutable compilation identity; no scene, modifiers or project are retained.
#[derive(Clone)]
pub struct ClipContext {
    id: uuid::Uuid,
    context: Context,
}
impl ClipContext {
    pub fn for_item(item: &VideoItem, canvas: CanvasSize) -> Option<Self> {
        let VideoItemContent::Sanim(settings) = &item.content else {
            return None;
        };
        Some(Self {
            id: item.id,
            context: Context {
                path: item.file.path().into(),
                parameters: settings.parameters.clone(),
                canvas,
                timing: Some((
                    item.end.signed_sub(item.start),
                    item.playback_speed,
                    item.animation_time_offset,
                )),
            },
        })
    }
    pub fn matches(&self, item: &VideoItem, canvas: CanvasSize) -> bool {
        self.id == item.id && self.context.matches(item, canvas, Time::ZERO)
    }
}
struct Published {
    context: Context,
    events: Arc<Events>,
}
struct Registry {
    sources: Cache<Key, Published>,
    listeners: Vec<Weak<Change>>,
}
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            sources: Cache::new(MAX_BYTES, MAX_SOURCES),
            listeners: Vec::new(),
        })
    })
}
static REVISION: AtomicU64 = AtomicU64::new(0);
pub fn revision() -> u64 {
    REVISION.load(Ordering::Acquire)
}

/// A weak host wake subscription; producers never retain a closed editor.
pub struct Change {
    dirty: AtomicBool,
    wake: Mutex<Option<Weak<Wake>>>,
}
impl Change {
    pub fn subscribe() -> Arc<Self> {
        let change = Arc::new(Self {
            dirty: AtomicBool::new(false),
            wake: Mutex::new(None),
        });
        let mut registry = registry().lock().expect("source event registry poisoned");
        registry
            .listeners
            .retain(|listener| listener.strong_count() > 0);
        registry.listeners.push(Arc::downgrade(&change));
        change
    }
    pub fn set_wake(&self, wake: Weak<Wake>) {
        *self.wake.lock().expect("source event wake poisoned") = Some(wake);
    }
    pub fn pending(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }
    pub fn take(&self) {
        self.dirty.store(false, Ordering::Release);
    }
}

pub fn publish_import(path: &Path, canvas: CanvasSize, events: Events) {
    publish(
        Key::Import(path.into()),
        Published {
            context: Context {
                path: path.into(),
                parameters: BTreeMap::new(),
                canvas,
                timing: None,
            },
            events: Arc::new(events),
        },
    );
}
pub fn publish_clip(clip: &ClipContext, events: Events) {
    publish(
        Key::Clip(clip.id),
        Published {
            context: clip.context.clone(),
            events: Arc::new(events),
        },
    );
}
fn publish(key: Key, published: Published) {
    let listeners = {
        let mut registry = registry().lock().expect("source event registry poisoned");
        if registry.sources.get(&key).is_some_and(|previous| {
            previous.context == published.context
                && previous.events.duration == published.events.duration
                && previous.events.index == published.events.index
        }) {
            return;
        }
        let bytes = std::mem::size_of::<Published>()
            + published.context.path.as_os_str().len()
            + published
                .context
                .parameters
                .iter()
                .map(|(key, value)| key.len() + value.to_string().len())
                .sum::<usize>()
            + published.events.index.estimated_bytes();
        registry.sources.insert(key, Arc::new(published), bytes);
        REVISION.fetch_add(1, Ordering::Release);
        registry
            .listeners
            .retain(|listener| listener.strong_count() > 0);
        registry
            .listeners
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>()
    };
    for listener in listeners {
        listener.dirty.store(true, Ordering::Release);
        let wake = listener
            .wake
            .lock()
            .expect("source event wake poisoned")
            .as_ref()
            .and_then(Weak::upgrade);
        if let Some(wake) = wake {
            wake();
        }
    }
}
pub fn for_item(item: &VideoItem, canvas: CanvasSize) -> Option<Arc<Events>> {
    let VideoItemContent::Sanim(_) = &item.content else {
        return None;
    };
    let mut registry = registry().lock().expect("source event registry poisoned");
    let valid = |published: &Published| {
        published
            .context
            .matches(item, canvas, published.events.duration)
    };
    if let Some(published) = registry
        .sources
        .get(&Key::Clip(item.id))
        .filter(|value| valid(value))
    {
        return Some(published.events.clone());
    }
    registry
        .sources
        .get(&Key::Import(item.file.path().into()))
        .filter(|value| valid(value))
        .map(|published| published.events.clone())
}

#[cfg(test)]
mod tests;
