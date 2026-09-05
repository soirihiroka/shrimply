use crate::decode;
use shrimply_math_core::Time;
use shrimply_preview_core::accuracy::CompositeAccuracy;
use shrimply_project::project::{Asset, AssetSnapshot, ItemAddress, VideoItem, VideoItemContent};
use skia_safe::{Data, Image};
use std::{
    collections::HashMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
};

#[derive(Clone, PartialEq, Eq)]
struct Source {
    file: Asset,
    track: u32,
    kind: Kind,
    width: u32,
    height: u32,
    svg_color_overrides: Vec<shrimply_project::project::SvgColorOverride>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Video,
    Image,
    Svg,
    Pdf(u32),
}

#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    id: Key,
    source: Source,
    time: Time,
    accuracy: CompositeAccuracy,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Plane {
    Content,
    Alpha,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    address: ItemAddress,
    plane: Plane,
}

impl Request {
    pub fn new(
        item: &VideoItem,
        address: ItemAddress,
        time: Time,
        accuracy: CompositeAccuracy,
        plane: Plane,
    ) -> Option<Self> {
        let kind = match item.content {
            VideoItemContent::Media | VideoItemContent::Gif => Kind::Video,
            VideoItemContent::Image => Kind::Image,
            VideoItemContent::Svg => Kind::Svg,
            VideoItemContent::Pdf(ref pdf) => Kind::Pdf(pdf.page),
            _ => return None,
        };
        Some(Self {
            id: Key { address, plane },
            source: Source {
                file: item.file.clone(),
                track: item.track_id,
                kind,
                width: item.source_width,
                height: item.source_height,
                svg_color_overrides: item.svg_color_overrides.clone(),
            },
            time,
            accuracy,
        })
    }
}

#[derive(Clone)]
pub enum Frame {
    Image(Image),
    Svg(Arc<shrimply_video_core::svg::PreparedSvg>),
}

struct Batch {
    epoch: u64,
    generation: u64,
    requests: Vec<Request>,
}
struct Completed {
    epoch: u64,
    generation: u64,
    frames: Result<HashMap<Key, (Source, Frame)>, String>,
}
#[derive(Default)]
struct Slots {
    pending: Option<Batch>,
    completed: Option<Completed>,
    stop: bool,
    working: bool,
}
#[derive(Default)]
struct Shared {
    slots: Mutex<Slots>,
    wake: Condvar,
    epoch: Arc<AtomicU64>,
    generation: AtomicU64,
}

/// One decoder worker, one replaceable request, and one replaceable completion.
/// Scrubbing never queues an unbounded history of frames or blocks the UI on I/O.
pub struct Media {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    frames: HashMap<Key, (Source, Frame)>,
    requested: Vec<Request>,
    revision: u64,
    error: Option<String>,
}

impl Default for Media {
    fn default() -> Self {
        let shared = Arc::new(Shared::default());
        let worker_state = shared.clone();
        let worker = thread::Builder::new()
            .name("preview-media".into())
            .spawn(move || worker(worker_state))
            .expect("start preview media worker");
        Self {
            shared,
            worker: Some(worker),
            frames: HashMap::new(),
            requested: Vec::new(),
            revision: 0,
            error: None,
        }
    }
}

impl Media {
    pub fn needs_update(&self) -> bool {
        let slots = self
            .shared
            .slots
            .lock()
            .expect("preview media slots poisoned");
        slots.working || slots.pending.is_some() || slots.completed.is_some()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn invalidate(&mut self) {
        self.shared.epoch.fetch_add(1, Ordering::Relaxed);
        self.frames.clear();
        self.requested.clear();
        self.error = None;
        self.revision += 1;
        let mut slots = self
            .shared
            .slots
            .lock()
            .expect("preview media slots poisoned");
        slots.pending = None;
        slots.completed = None;
    }

    pub fn request(&mut self, requests: Vec<Request>) -> Result<bool, String> {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            return Err("Preview media worker stopped unexpectedly".into());
        }
        let epoch = self.shared.epoch.load(Ordering::Relaxed);
        let generation = self.shared.generation.load(Ordering::Relaxed);
        let mut slots = self
            .shared
            .slots
            .lock()
            .expect("preview media slots poisoned");
        // Accept the last completed target before advancing to the latest mouse
        // position. A new seek must not discard a frame that finished between paints.
        if let Some(completed) = slots
            .completed
            .take()
            .filter(|result| result.epoch == epoch && result.generation == generation)
        {
            match completed.frames {
                Ok(frames) => {
                    self.frames = frames;
                    self.revision += 1;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        let changed = requests != self.requested;
        if changed {
            self.error = None;
            // Continuous playback may finish its active frame. Interactive
            // seeks and quality changes supersede stale decode work immediately,
            // without dropping decoder contexts or immutable source caches.
            if !requests
                .iter()
                .all(|request| request.accuracy.continuous_playback())
                || !self
                    .requested
                    .iter()
                    .all(|request| request.accuracy.continuous_playback())
            {
                self.shared.generation.fetch_add(1, Ordering::Relaxed);
            }
            self.requested.clone_from(&requests);
        }
        let generation = self.shared.generation.load(Ordering::Relaxed);
        self.frames
            .retain(|id, (source, _)| requests.iter().any(|r| r.id == *id && r.source == *source));
        let ready = requests
            .iter()
            .all(|request| self.frames.contains_key(&request.id));
        if changed {
            slots.pending = Some(Batch {
                epoch,
                generation,
                requests,
            });
            self.shared.wake.notify_one();
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(ready)
    }

    pub fn frame(&self, address: &ItemAddress, plane: Plane) -> Option<&Frame> {
        self.frames
            .get(&Key {
                address: address.clone(),
                plane,
            })
            .map(|(_, frame)| frame)
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        self.shared.epoch.fetch_add(1, Ordering::Relaxed);
        self.shared
            .slots
            .lock()
            .expect("preview media slots poisoned")
            .stop = true;
        self.shared.wake.notify_one();
        // A local filesystem read cannot be interrupted safely. Do not join it on
        // the main thread; the worker owns its resources until it observes stop.
        drop(self.worker.take());
    }
}

struct Cached {
    source: Source,
    snapshot: AssetSnapshot,
    decoder: Option<decode::Decoder>,
    frame: Option<(Time, CompositeAccuracy, Result<Frame, String>)>,
}

fn worker(shared: Arc<Shared>) {
    let mut cache = HashMap::<Key, Cached>::new();
    let mut epoch = 0;
    loop {
        let batch = {
            let mut slots = shared.slots.lock().expect("preview media slots poisoned");
            while slots.pending.is_none() && !slots.stop {
                slots = shared
                    .wake
                    .wait(slots)
                    .expect("preview media slots poisoned");
            }
            if slots.stop {
                return;
            }
            slots.working = true;
            slots.pending.take().expect("pending preview request")
        };
        if batch.epoch != epoch {
            cache.clear();
            epoch = batch.epoch;
        }
        cache.retain(|id, _| batch.requests.iter().any(|request| request.id == *id));
        let mut frames = HashMap::new();
        let result = batch.requests.iter().try_for_each(|request| {
            if shared.epoch.load(Ordering::Relaxed) != epoch
                || shared.generation.load(Ordering::Relaxed) != batch.generation
            {
                return Err("preview request cancelled".into());
            }
            let frame = load(request, &mut cache, &shared, &batch).map_err(|error| {
                format!(
                    "Could not render {}: {error}",
                    request.source.file.path().display()
                )
            })?;
            frames.insert(request.id.clone(), (request.source.clone(), frame));
            Ok(())
        });
        let mut slots = shared.slots.lock().expect("preview media slots poisoned");
        slots.working = false;
        if shared.epoch.load(Ordering::Relaxed) == epoch
            && shared.generation.load(Ordering::Relaxed) == batch.generation
        {
            slots.completed = Some(Completed {
                epoch,
                generation: batch.generation,
                frames: result.map(|()| frames),
            });
        }
    }
}

fn load(
    request: &Request,
    cache: &mut HashMap<Key, Cached>,
    shared: &Shared,
    batch: &Batch,
) -> Result<Frame, String> {
    let source = &request.source;
    let snapshot = source.file.snapshot()?;
    if cache
        .get(&request.id)
        .is_some_and(|entry| entry.source != *source || entry.snapshot != snapshot)
    {
        cache.remove(&request.id);
    }
    if let std::collections::hash_map::Entry::Vacant(entry) = cache.entry(request.id.clone()) {
        entry.insert(Cached {
            source: source.clone(),
            snapshot,
            decoder: None,
            frame: None,
        });
    }
    let entry = cache.get_mut(&request.id).expect("preview cache inserted");
    if let Some((time, accuracy, frame)) = &entry.frame
        && (source.kind != Kind::Video || (*time == request.time && *accuracy == request.accuracy))
    {
        return frame.clone();
    }
    let result = (|| {
        let frame = match source.kind {
            Kind::Video => {
                if entry.decoder.is_none() {
                    entry.decoder = Some(decode::Decoder::new(
                        &source.file,
                        source.track,
                        shared.epoch.clone(),
                        batch.epoch,
                    )?);
                }
                Frame::Image(
                    entry
                        .decoder
                        .as_mut()
                        .expect("preview decoder inserted")
                        .image(
                            request.time,
                            request.accuracy,
                            &shared.generation,
                            batch.generation,
                        )?,
                )
            }
            Kind::Image => {
                let bytes = entry.snapshot.read()?;
                let image =
                    Image::from_encoded(Data::new_copy(&bytes)).ok_or("image cannot be decoded")?;
                // Force codec work off the UI thread rather than deferring lazy image decoding to draw_image.
                Frame::Image(
                    image
                        .make_raster_image(None, skia_safe::image::CachingHint::Allow)
                        .ok_or("image cannot be rasterized")?,
                )
            }
            Kind::Pdf(page) => {
                // Use the same document renderer as CUDA. Parsing and Poppler's
                // page work stay on this media worker; the completed page uses
                // the existing snapshot-aware source cache.
                let rendered = shrimply_pdf::render_page(entry.snapshot.read()?, page)?;
                let width = i32::try_from(rendered.size.width)
                    .map_err(|_| "PDF width exceeds Skia limits")?;
                let height = i32::try_from(rendered.size.height)
                    .map_err(|_| "PDF height exceeds Skia limits")?;
                let info = skia_safe::ImageInfo::new(
                    (width, height),
                    skia_safe::ColorType::RGBA8888,
                    skia_safe::AlphaType::Unpremul,
                    None,
                );
                Frame::Image(
                    skia_safe::images::raster_from_data(
                        &info,
                        Data::new_copy(&rendered.rgba),
                        rendered.size.width as usize * size_of::<u32>(),
                    )
                    .ok_or("Could not create PDF page image")?,
                )
            }
            Kind::Svg => {
                let source_text = entry.snapshot.read_to_string()?;
                let svg = shrimply_project::svg_color::apply_overrides(
                    &source_text,
                    &source.svg_color_overrides,
                );
                Frame::Svg(Arc::new(shrimply_video_core::svg::PreparedSvg::new(svg)?))
            }
        };
        entry.snapshot.ensure_current()?;
        Ok(frame)
    })();
    // A stable bad source must report its error, not reopen FFmpeg on every
    // paint. A new request or asset snapshot can retry; cancellation is not cached.
    if shared.epoch.load(Ordering::Relaxed) == batch.epoch
        && shared.generation.load(Ordering::Relaxed) == batch.generation
    {
        entry.frame = Some((request.time, request.accuracy, result.clone()));
    }
    result
}
