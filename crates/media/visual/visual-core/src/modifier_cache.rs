pub mod encoder;
mod frames;
pub use frames::render_and_encode;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};
use shrimply_math_core::Fraction;
use shrimply_project_document::project::{
    CanvasSize, ItemAddress, Project, RepeatStrategy, Time, Transform, VideoItem, VideoItemContent,
    fraction_denominator, fraction_numerator,
};
use shrimply_resource_pipeline::{
    Event, JobContext, Pipeline, Processor, RequestDisposition, Subscription, TryNext,
};
use shrimply_visual_modifiers::{ModifierEffect, RasterModifierEffect, cache::CacheModifier};
use uuid::Uuid;

const MANIFEST_NAME: &str = "manifest.json";
const MEDIA_NAME: &str = "visual.mkv";
const CACHE_VERSION: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Missing,
    Baking { completed: u64, total: u64 },
    Ready,
    Failed(String),
}

struct Job {
    status: Status,
    subscription: Option<Subscription<CacheKey, Progress, ()>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    root: PathBuf,
    address: ItemAddress,
    modifier_id: Uuid,
}

#[derive(Clone, Copy)]
pub struct Progress {
    pub completed: u64,
    pub total: u64,
}

pub struct BakeRequest {
    pub project: Project,
    pub address: ItemAddress,
    pub settings: CacheModifier,
    pub start: Time,
    pub duration: Time,
    pub first_frame: u64,
    pub total_frames: u64,
    pub width: u32,
    pub height: u32,
    pub coded_width: u32,
    pub coded_height: u32,
    pub output: PathBuf,
}

pub type BakeExecutor = fn(BakeRequest, &JobContext<Progress>) -> Result<(), String>;

struct BakeInput {
    project: Project,
    address: ItemAddress,
    key: CacheKey,
    start: Time,
    duration: Time,
    time_offset: Time,
    playback_speed: Fraction,
    settings: CacheModifier,
    executor: BakeExecutor,
}

struct BakeProcessor {
    inputs: Arc<Mutex<HashMap<CacheKey, BakeInput>>>,
}

struct Runtime {
    pipeline: Pipeline<CacheKey, BakeProcessor>,
    inputs: Arc<Mutex<HashMap<CacheKey, BakeInput>>>,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    kind: String,
    width: u32,
    height: u32,
    coded_width: u32,
    coded_height: u32,
    duration: Time,
    time_offset: Time,
    playback_speed_numerator: i64,
    playback_speed_denominator: i64,
    fps_numerator: i64,
    fps_denominator: i64,
}

#[derive(Clone)]
struct ReadyEntry {
    path: PathBuf,
    width: u32,
    height: u32,
    duration: Time,
    time_offset: Time,
    playback_speed: Fraction,
    fps: Fraction,
}

static JOBS: LazyLock<Mutex<HashMap<CacheKey, Job>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static READY: LazyLock<Mutex<HashMap<CacheKey, ReadyEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CACHE_OPERATIONS: Mutex<()> = Mutex::new(());
static RUNTIME: LazyLock<Runtime> = LazyLock::new(Runtime::new);

impl Job {
    fn refresh(&mut self) -> bool {
        let Some(subscription) = self.subscription.as_mut() else {
            return false;
        };
        let terminal = loop {
            match subscription.try_next() {
                TryNext::Event(Event::Progress(progress)) => {
                    self.status = Status::Baking {
                        completed: progress.completed,
                        total: progress.total,
                    };
                }
                TryNext::Event(Event::Finished(_)) => break Some(Status::Ready),
                TryNext::Event(Event::Failed(error)) => {
                    break Some(Status::Failed(error.to_string()));
                }
                TryNext::Event(Event::Cancelled) => break Some(Status::Missing),
                TryNext::Empty => break None,
                TryNext::Closed => {
                    break Some(Status::Failed(
                        "visual cache job closed without a terminal event".to_string(),
                    ));
                }
            }
        };
        let Some(status) = terminal else {
            return false;
        };
        self.status = status;
        self.subscription = None;
        true
    }
}

impl Runtime {
    fn new() -> Self {
        let inputs = Arc::new(Mutex::new(HashMap::new()));
        Self {
            pipeline: Pipeline::new(
                BakeProcessor {
                    inputs: inputs.clone(),
                },
                |job| {
                    let _ = thread::spawn(job);
                },
            ),
            inputs,
        }
    }

    fn request(
        &self,
        input: BakeInput,
    ) -> (RequestDisposition, Subscription<CacheKey, Progress, ()>) {
        let key = input.key.clone();
        let mut inputs = self
            .inputs
            .lock()
            .expect("visual cache input lock poisoned");
        assert!(
            !inputs.contains_key(&key),
            "visual cache input already exists"
        );
        inputs.insert(key.clone(), input);
        drop(inputs);
        let request = self.pipeline.request(key.clone());
        if request.0 == RequestDisposition::Joined {
            self.discard_input(&key);
        }
        request
    }

    fn cancel(&self, key: &CacheKey) {
        self.pipeline.cancel(key);
        self.discard_input(key);
    }

    fn discard_input(&self, key: &CacheKey) {
        self.inputs
            .lock()
            .expect("visual cache input lock poisoned")
            .remove(key);
    }
}

impl Processor<CacheKey> for BakeProcessor {
    type Progress = Progress;
    type Output = ();

    fn process(
        &self,
        key: CacheKey,
        context: &JobContext<Self::Progress>,
    ) -> Result<Self::Output, String> {
        let input = self
            .inputs
            .lock()
            .expect("visual cache input lock poisoned")
            .remove(&key)
            .ok_or_else(|| "visual cache bake input disappeared".to_string())?;
        bake_inner(input, context)
    }
}

pub fn status(address: &ItemAddress, modifier_id: Uuid) -> Status {
    let key = cache_key(address, modifier_id);
    status_for_key(&key)
}

fn status_for_key(key: &CacheKey) -> Status {
    let job = {
        let mut jobs = JOBS
            .lock()
            .expect("visual modifier cache job lock poisoned");
        jobs.get_mut(key).map(|job| {
            let terminal = job.refresh();
            (job.status.clone(), terminal)
        })
    };
    if let Some((status, terminal)) = job {
        if terminal {
            RUNTIME.discard_input(key);
        }
        return status;
    }
    match ready_entry(key) {
        Ok(_) => Status::Ready,
        Err(error) if cache_directory(key).exists() => Status::Failed(error),
        Err(_) => Status::Missing,
    }
}

pub fn bake(
    project: Project,
    address: ItemAddress,
    modifier_id: Uuid,
    executor: BakeExecutor,
) -> Result<(), String> {
    let _operation = CACHE_OPERATIONS
        .lock()
        .expect("visual cache operation lock poisoned");
    let key = cache_key(&address, modifier_id);
    if matches!(status_for_key(&key), Status::Baking { .. }) {
        return Err("this cache is already baking".to_string());
    }
    let input = bake_input(project, &address, modifier_id, key.clone(), executor)?;
    let (first_frame, end_frame) = frame_range(input.start, input.duration, input.project.fps)?;
    let total = end_frame - first_frame;
    invalidate_inner(&key)?;
    let (disposition, subscription) = RUNTIME.request(input);
    if disposition == RequestDisposition::Joined {
        subscription.cancel();
        return Err("this cache is already baking".to_string());
    }
    JOBS.lock()
        .expect("visual modifier cache job lock poisoned")
        .insert(
            key,
            Job {
                status: Status::Baking {
                    completed: 0,
                    total,
                },
                subscription: Some(subscription),
            },
        );
    Ok(())
}

pub fn invalidate(address: &ItemAddress, modifier_id: Uuid) -> Result<(), String> {
    let _operation = CACHE_OPERATIONS
        .lock()
        .expect("visual cache operation lock poisoned");
    invalidate_inner(&cache_key(address, modifier_id))
}

fn invalidate_inner(key: &CacheKey) -> Result<(), String> {
    RUNTIME.cancel(key);
    JOBS.lock()
        .expect("visual modifier cache job lock poisoned")
        .remove(key);
    READY
        .lock()
        .expect("visual modifier ready cache lock poisoned")
        .remove(key);
    match fs::remove_dir_all(cache_directory(key)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not invalidate visual cache: {error}")),
    }
}

pub fn effective_item(
    address: &ItemAddress,
    item: &VideoItem,
    canvas: CanvasSize,
) -> Result<Option<VideoItem>, String> {
    let mut ready = None;
    for (index, modifier) in item.modifiers.iter().enumerate().rev() {
        if !modifier.enabled
            || !matches!(
                modifier.effect,
                ModifierEffect::Raster(ref effect)
                    if matches!(&**effect, RasterModifierEffect::Cache(_))
            )
            || !cache_directory(&cache_key(address, modifier.id)).exists()
        {
            continue;
        }
        ready = Some((index, ready_entry(&cache_key(address, modifier.id))?));
        break;
    }
    let Some((index, entry)) = ready else {
        return Ok(None);
    };
    let mut effective = item.clone();
    effective.file = entry.path.into();
    effective.content = VideoItemContent::Media;
    effective.track_id = 0;
    effective.alpha_mask_video = Some(1);
    effective.source_width = entry.width;
    effective.source_height = entry.height;
    effective.source_duration = entry.duration;
    effective.time_offset = entry.time_offset;
    effective.playback_speed = entry.playback_speed;
    effective.playback_fps = entry.fps;
    effective.repeat_strategy = if matches!(item.repeat_strategy, RepeatStrategy::Hold) {
        RepeatStrategy::Hold
    } else {
        RepeatStrategy::Empty
    };
    effective.transform = Transform::fill(canvas);
    effective.default_transform = None;
    effective.motion_blur.enabled = false;
    effective.render_canvas_size = Some(canvas);
    effective.modifiers = item.modifiers.clone();
    for modifier in &mut effective.modifiers[..=index] {
        modifier.enabled = false;
    }
    Ok(Some(effective))
}

fn bake_input(
    mut project: Project,
    address: &ItemAddress,
    modifier_id: Uuid,
    key: CacheKey,
    executor: BakeExecutor,
) -> Result<BakeInput, String> {
    let (start, end) = project
        .projected_item_times(address)
        .ok_or_else(|| "visual cache item is outside its folded-sequence hosts".to_string())?;
    let item = project
        .video_item(address)
        .ok_or_else(|| "visual cache item no longer exists".to_string())?;
    let index = item
        .modifiers
        .iter()
        .position(|modifier| modifier.id == modifier_id)
        .ok_or_else(|| "visual cache modifier no longer exists".to_string())?;
    let ModifierEffect::Raster(effect) = &item.modifiers[index].effect else {
        return Err("selected visual modifier is not a cache".to_string());
    };
    let RasterModifierEffect::Cache(settings) = &**effect else {
        return Err("selected visual modifier is not a cache".to_string());
    };
    let settings = settings.clone();
    let duration = end.saturating_sub(start);
    if duration == Time::ZERO {
        return Err("cannot cache an empty visual item".to_string());
    }
    let item_id = item.id;
    let item_start = item.start;
    let track = address.track();
    let timeline_start = project
        .sequence_time_to_timeline(&track, item_start)
        .ok_or_else(|| "visual cache item has an invalid folded-sequence clock".to_string())?;
    let timeline_next = project
        .sequence_time_to_timeline(&track, item_start.saturating_add(Time::from_seconds(1)))
        .ok_or_else(|| "visual cache item has an invalid folded-sequence clock".to_string())?;
    let playback_speed = timeline_next.signed_sub(timeline_start).seconds;
    if shrimply_project_document::project::playback_speed_is_zero(playback_speed) {
        return Err("visual cache item has a stopped folded-sequence clock".to_string());
    }
    let time_offset = timeline_start.signed_sub(start);
    let shrimply_project_document::project::TrackMut::Video(track) = project
        .track_mut(&address.track())
        .ok_or_else(|| "visual cache track no longer exists".to_string())?
    else {
        return Err("visual cache requires a video track".to_string());
    };
    for item in &mut track.items {
        if item
            .transitions
            .to_next
            .as_ref()
            .is_some_and(|transition| transition.target_item_id == item_id)
        {
            item.transitions.to_next = None;
        }
    }
    let item = project
        .video_item_mut(address)
        .expect("visual cache item disappeared from cloned project");
    item.modifiers.truncate(index);
    Ok(BakeInput {
        project,
        address: address.clone(),
        key,
        start,
        duration,
        time_offset,
        playback_speed,
        settings,
        executor,
    })
}

fn bake_inner(input: BakeInput, context: &JobContext<Progress>) -> Result<(), String> {
    if context.is_cancelled() {
        return Err("visual cache bake cancelled".to_string());
    }
    let root = input.key.root.clone();
    fs::create_dir_all(&root).map_err(|error| format!("could not create cache folder: {error}"))?;
    let temporary = tempfile::Builder::new()
        .prefix(&format!(".{}-", input.key.modifier_id.simple()))
        .tempdir_in(&root)
        .map_err(|error| format!("could not create temporary cache folder: {error}"))?;
    let BakeInput {
        project,
        address,
        start,
        duration,
        time_offset,
        playback_speed,
        settings,
        executor,
        key,
        ..
    } = input;
    let width = project.canvas_size.width.max(1);
    let height = project.canvas_size.height.max(1);
    let coded_width = even(width);
    let coded_height = even(height);
    let fps = project.fps;
    let (first_frame, end_frame) = frame_range(start, duration, fps)?;
    let total = end_frame - first_frame;
    executor(
        BakeRequest {
            project,
            address,
            settings,
            start,
            duration,
            first_frame,
            total_frames: total,
            width,
            height,
            coded_width,
            coded_height,
            output: temporary.path().join(MEDIA_NAME),
        },
        context,
    )?;
    if !temporary.path().join(MEDIA_NAME).is_file() {
        return Err("visual cache executor did not produce media".to_string());
    }
    let manifest = Manifest {
        version: CACHE_VERSION,
        kind: "visual".to_string(),
        width,
        height,
        coded_width,
        coded_height,
        duration,
        time_offset,
        playback_speed_numerator: fraction_numerator(playback_speed),
        playback_speed_denominator: fraction_denominator(playback_speed),
        fps_numerator: fraction_numerator(fps),
        fps_denominator: fraction_denominator(fps),
    };
    fs::write(
        temporary.path().join(MANIFEST_NAME),
        serde_json::to_vec(&manifest)
            .map_err(|error| format!("could not encode visual cache manifest: {error}"))?,
    )
    .map_err(|error| format!("could not write visual cache manifest: {error}"))?;
    let destination = cache_directory(&key);
    let _operation = CACHE_OPERATIONS
        .lock()
        .expect("visual cache operation lock poisoned");
    if context.is_cancelled() {
        return Err("visual cache bake cancelled".to_string());
    }
    fs::create_dir_all(
        destination
            .parent()
            .expect("visual cache destination must have a parent"),
    )
    .map_err(|error| format!("could not create visual cache destination: {error}"))?;
    fs::rename(temporary.path(), &destination)
        .map_err(|error| format!("could not finish visual cache: {error}"))?;
    READY
        .lock()
        .expect("visual modifier ready cache lock poisoned")
        .remove(&key);
    Ok(())
}

fn ready_entry(key: &CacheKey) -> Result<ReadyEntry, String> {
    if let Some(entry) = READY
        .lock()
        .expect("visual modifier ready cache lock poisoned")
        .get(key)
        .cloned()
    {
        return Ok(entry);
    }
    let directory = cache_directory(key);
    let bytes = fs::read(directory.join(MANIFEST_NAME))
        .map_err(|error| format!("visual cache is missing its manifest: {error}"))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("visual cache manifest is invalid: {error}"))?;
    if manifest.version != CACHE_VERSION || manifest.kind != "visual" {
        return Err("visual cache version is unsupported; invalidate and rebake it".to_string());
    }
    if manifest.coded_width != even(manifest.width)
        || manifest.coded_height != even(manifest.height)
        || manifest.playback_speed_numerator == 0
        || manifest.playback_speed_denominator <= 0
        || manifest.fps_numerator <= 0
        || manifest.fps_denominator <= 0
    {
        return Err("visual cache manifest has an unsupported layout".to_string());
    }
    let path = directory.join(MEDIA_NAME);
    if !path.is_file() {
        return Err("visual cache media is missing".to_string());
    }
    let entry = ReadyEntry {
        path,
        width: manifest.width,
        height: manifest.height,
        duration: manifest.duration,
        time_offset: manifest.time_offset,
        playback_speed: shrimply_project_document::project::fraction_new(
            manifest.playback_speed_numerator,
            manifest.playback_speed_denominator,
        ),
        fps: shrimply_project_document::project::fraction_new(
            manifest.fps_numerator,
            manifest.fps_denominator,
        ),
    };
    READY
        .lock()
        .expect("visual modifier ready cache lock poisoned")
        .insert(key.clone(), entry.clone());
    Ok(entry)
}

fn cache_key(address: &ItemAddress, modifier_id: Uuid) -> CacheKey {
    CacheKey {
        root: cache_root(),
        address: address.clone(),
        modifier_id,
    }
}

fn cache_directory(key: &CacheKey) -> PathBuf {
    let mut directory = key.root.join(key.modifier_id.simple().to_string());
    for host in key.address.sequence_path() {
        directory.push(host.simple().to_string());
    }
    directory.push(key.address.track_id().simple().to_string());
    directory.push(key.address.item_id().simple().to_string());
    directory
}

fn cache_root() -> PathBuf {
    shrimply_path_core::project_cache_directory()
}

const fn even(value: u32) -> u32 {
    value.saturating_add(value % 2)
}

fn frame_range(start: Time, duration: Time, fps: Fraction) -> Result<(u64, u64), String> {
    let first =
        shrimply_math_core::frame_count(start, fps).ok_or("cache frame rate must be positive")?;
    let end = shrimply_math_core::frame_count(start.saturating_add(duration), fps)
        .ok_or("cache frame rate must be positive")?;
    if first >= end {
        return Err("cannot cache an item shorter than one project frame".to_string());
    }
    Ok((first, end))
}
