use hashbrown::HashMap;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use shrimply_asset::AssetSnapshot;
use shrimply_project::project::{
    MeshFlowAdaptiveWeights, Project, Time, VideoItem, VideoItemContent, VideoStabilizationMethod,
};

const CACHE_DIRECTORY: &str = "cache";
const CACHE_DATABASE: &str = "cache/video-stabilization.sqlite";
const CACHE_VERSION: i64 = 5;
const CHUNK_SECONDS: u32 = 10;
const CHUNK_OVERLAP_SECONDS: u32 = 1;
const NANOS_PER_SECOND: i128 = 1_000_000_000;
const GENERATION_DEBOUNCE: Duration = Duration::from_millis(300);

static JOBS: LazyLock<Mutex<HashMap<CacheKey, Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static FAILED: LazyLock<Mutex<HashMap<CacheKey, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static MEMORY: LazyLock<Mutex<HashMap<CacheKey, Arc<StabilizationChunk>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static GENERATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheBase {
    source: AssetSnapshot,
    track_id: u32,
    crop_ratio: u32,
    derivative_weights: [u32; 3],
    method: u8,
    mesh: [u32; 5],
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    base: CacheBase,
    chunk_index: u64,
}

#[derive(Clone, Debug)]
enum StabilizationChunk {
    L1(shrimply_affine_l1_stabilization::StabilizationChunk),
    MeshFlow(shrimply_mesh_flow_stabilization::StabilizationChunk),
}

#[derive(Clone, Debug)]
pub enum StabilizationWarp {
    Affine(glam::Mat3),
    Mesh {
        grid_width: u32,
        grid_height: u32,
        source_offsets: Vec<glam::Vec2>,
    },
}

#[derive(Clone, Copy)]
enum GenerationOptions {
    L1(shrimply_affine_l1_stabilization::StabilizationOptions),
    MeshFlow(shrimply_mesh_flow_stabilization::StabilizationOptions),
}

pub fn request(item: &VideoItem) {
    request_at(item, initial_position(item));
}

pub fn request_at(item: &VideoItem, source_position: Time) {
    if !eligible(item) {
        return;
    }
    let Ok(key) = cache_key(item, source_position) else {
        return;
    };
    cancel_obsolete_jobs(&key);
    if cached(&key).is_some()
        || FAILED
            .lock()
            .expect("video stabilization failure lock died")
            .contains_key(&key)
    {
        return;
    }
    let cancellation = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = JOBS.lock().expect("video stabilization job lock died");
        if jobs
            .get(&key)
            .is_some_and(|job| !job.load(Ordering::Relaxed))
        {
            return;
        }
        jobs.insert(key.clone(), cancellation.clone());
    }
    let options = options(item);
    thread::Builder::new()
        .name("video-stabilization".to_string())
        .spawn(move || {
            thread::sleep(GENERATION_DEBOUNCE);
            let result = if cancellation.load(Ordering::Relaxed) {
                Ok(false)
            } else {
                generate(&key, options, &cancellation)
            };
            if let Err(error) = result
                && !cancellation.load(Ordering::Relaxed)
            {
                tracing::error!(input = %key.base.source.path().display(), chunk = key.chunk_index, "Video stabilization failed: {error}");
                FAILED
                    .lock()
                    .expect("video stabilization failure lock died")
                    .insert(key.clone(), error);
            }
            finish_job(&key, &cancellation);
        })
        .expect("spawn video stabilization");
}

pub fn source_warp(item: &VideoItem, source_position: Time) -> Option<StabilizationWarp> {
    if !eligible(item) {
        return None;
    }
    let key = cache_key(item, source_position).ok()?;
    let chunk = cached(&key);
    let Some(chunk) = chunk else {
        request_at(item, source_position);
        return None;
    };
    match chunk.as_ref() {
        StabilizationChunk::L1(chunk) => {
            let frame = frame_at_time(
                source_position,
                chunk.frame_rate_numerator,
                chunk.frame_rate_denominator,
            );
            let index = frame.checked_sub(chunk.first_frame)? as usize;
            chunk
                .source_transforms
                .get(index)
                .copied()
                .map(StabilizationWarp::Affine)
        }
        StabilizationChunk::MeshFlow(chunk) => {
            let frame = frame_at_time(
                source_position,
                chunk.frame_rate_numerator,
                chunk.frame_rate_denominator,
            );
            let index = frame.checked_sub(chunk.first_frame)? as usize;
            Some(StabilizationWarp::Mesh {
                grid_width: chunk.grid_width,
                grid_height: chunk.grid_height,
                source_offsets: chunk.source_offsets.get(index)?.clone(),
            })
        }
    }
}

pub fn is_generating(item: &VideoItem) -> bool {
    cache_base(item).is_ok_and(|base| {
        JOBS.lock()
            .expect("video stabilization job lock died")
            .iter()
            .any(|(key, cancellation)| key.base == base && !cancellation.load(Ordering::Relaxed))
    })
}

pub fn is_ready(item: &VideoItem) -> bool {
    cache_key(item, initial_position(item)).is_ok_and(|key| cached(&key).is_some())
}

pub fn has_failed(item: &VideoItem) -> bool {
    cache_base(item).is_ok_and(|base| {
        FAILED
            .lock()
            .expect("video stabilization failure lock died")
            .keys()
            .any(|key| key.base == base)
    })
}

pub fn cancel(item: &VideoItem) {
    let Ok(base) = cache_base(item) else {
        return;
    };
    for (key, cancellation) in JOBS
        .lock()
        .expect("video stabilization job lock died")
        .iter()
    {
        if key.base == base {
            cancellation.store(true, Ordering::Relaxed);
        }
    }
}

pub fn rebuild(item: &VideoItem, source_position: Time) {
    let Ok(key) = cache_key(item, source_position) else {
        return;
    };
    if let Some(cancellation) = JOBS
        .lock()
        .expect("video stabilization job lock died")
        .get(&key)
    {
        cancellation.store(true, Ordering::Relaxed);
    }
    MEMORY
        .lock()
        .expect("video stabilization memory lock died")
        .remove(&key);
    FAILED
        .lock()
        .expect("video stabilization failure lock died")
        .remove(&key);
    if let Ok(cache) = Cache::open()
        && let Err(error) = cache.remove(&key)
    {
        tracing::warn!("Could not remove video stabilization cache chunk: {error}");
    }
    request_at(item, source_position);
}

pub fn ensure_project(project: &Project) -> Result<(), String> {
    let mut keys = HashMap::new();
    for item in project
        .video_tracks
        .iter()
        .flat_map(|track| track.items.iter())
        .filter(|item| item.stabilize_video && matches!(item.content, VideoItemContent::Media))
    {
        if item.alpha_mask_video.is_some() {
            return Err(format!(
                "video {} cannot be stabilized while using an alpha-mask stream",
                item.id
            ));
        }
    }
    for item in project
        .video_tracks
        .iter()
        .flat_map(|track| track.items.iter())
        .filter(|item| eligible(item))
    {
        let duration = item.source_duration.as_nanos_i128().max(1);
        let chunk_duration = i128::from(CHUNK_SECONDS) * NANOS_PER_SECOND;
        let chunks = (duration + chunk_duration - 1) / chunk_duration;
        for chunk_index in 0..u64::try_from(chunks).unwrap_or(u64::MAX) {
            let key = cache_key_for_chunk(item, chunk_index)?;
            keys.entry(key).or_insert_with(|| options(item));
        }
    }
    for (key, options) in keys {
        let cancellation = AtomicBool::new(false);
        generate(&key, options, &cancellation)?;
    }
    Ok(())
}

fn generate(
    key: &CacheKey,
    options: GenerationOptions,
    cancellation: &AtomicBool,
) -> Result<bool, String> {
    let _generation = GENERATION_LOCK
        .lock()
        .map_err(|_| "video stabilization generation lock died".to_string())?;
    if cancellation.load(Ordering::Relaxed) {
        return Ok(false);
    }
    if cached(key).is_some() {
        return Ok(true);
    }
    let chunk = match options {
        GenerationOptions::L1(options) => {
            let Some(chunk) = shrimply_affine_l1_stabilization::analyze_chunk(
                key.base.source.path(),
                key.base.track_id,
                key.chunk_index,
                CHUNK_SECONDS,
                CHUNK_OVERLAP_SECONDS,
                options,
                || cancellation.load(Ordering::Relaxed),
            )?
            else {
                return Ok(false);
            };
            StabilizationChunk::L1(chunk)
        }
        GenerationOptions::MeshFlow(options) => {
            let Some(chunk) = shrimply_mesh_flow_stabilization::analyze_chunk(
                key.base.source.path(),
                key.base.track_id,
                key.chunk_index,
                CHUNK_SECONDS,
                CHUNK_OVERLAP_SECONDS,
                options,
                || cancellation.load(Ordering::Relaxed),
            )?
            else {
                return Ok(false);
            };
            StabilizationChunk::MeshFlow(chunk)
        }
    };
    if cancellation.load(Ordering::Relaxed) {
        return Ok(false);
    }
    key.base.source.ensure_current()?;
    let cache = Cache::open()?;
    if cancellation.load(Ordering::Relaxed) {
        return Ok(false);
    }
    cache.store(key, &chunk)?;
    MEMORY
        .lock()
        .expect("video stabilization memory lock died")
        .insert(key.clone(), Arc::new(chunk));
    Ok(true)
}

fn cancel_obsolete_jobs(key: &CacheKey) {
    for (job_key, cancellation) in JOBS
        .lock()
        .expect("video stabilization job lock died")
        .iter()
    {
        if job_key.base.source.asset() == key.base.source.asset()
            && job_key.base.track_id == key.base.track_id
            && job_key.base != key.base
        {
            cancellation.store(true, Ordering::Relaxed);
        }
    }
}

fn finish_job(key: &CacheKey, cancellation: &Arc<AtomicBool>) {
    let mut jobs = JOBS.lock().expect("video stabilization job lock died");
    if jobs
        .get(key)
        .is_some_and(|current| Arc::ptr_eq(current, cancellation))
    {
        jobs.remove(key);
    }
}

fn cached(key: &CacheKey) -> Option<Arc<StabilizationChunk>> {
    if let Some(chunk) = MEMORY
        .lock()
        .expect("video stabilization memory lock died")
        .get(key)
        .cloned()
    {
        return Some(chunk);
    }
    let chunk = Cache::open().ok()?.load(key).ok()??;
    let chunk = Arc::new(chunk);
    MEMORY
        .lock()
        .expect("video stabilization memory lock died")
        .insert(key.clone(), chunk.clone());
    Some(chunk)
}

fn eligible(item: &VideoItem) -> bool {
    !matches!(item.stabilization_method(), VideoStabilizationMethod::Off)
        && item.alpha_mask_video.is_none()
        && matches!(item.content, VideoItemContent::Media)
}

fn initial_position(item: &VideoItem) -> Time {
    shrimply_project::project::video_source_time_at(item, item.start).unwrap_or(item.time_offset)
}

fn options(item: &VideoItem) -> GenerationOptions {
    match item.stabilization_method() {
        VideoStabilizationMethod::Off | VideoStabilizationMethod::L1 => {
            GenerationOptions::L1(shrimply_affine_l1_stabilization::StabilizationOptions {
                crop_ratio: f64::from(item.stabilization_crop_ratio),
                derivative_weights: [
                    f64::from(item.stabilization_first_derivative_weight),
                    f64::from(item.stabilization_second_derivative_weight),
                    f64::from(item.stabilization_third_derivative_weight),
                ],
            })
        }
        VideoStabilizationMethod::MeshFlow => {
            GenerationOptions::MeshFlow(shrimply_mesh_flow_stabilization::StabilizationOptions {
                crop_ratio: item.stabilization_crop_ratio,
                mesh_rows: item.mesh_flow_rows,
                mesh_columns: item.mesh_flow_columns,
                temporal_smoothing_radius: item.mesh_flow_smoothing_radius,
                optimization_iterations: item.mesh_flow_iterations,
                adaptive_weights: match item.mesh_flow_adaptive_weights {
                    MeshFlowAdaptiveWeights::Original => {
                        shrimply_mesh_flow_stabilization::AdaptiveWeights::Original
                    }
                    MeshFlowAdaptiveWeights::Flipped => {
                        shrimply_mesh_flow_stabilization::AdaptiveWeights::Flipped
                    }
                    MeshFlowAdaptiveWeights::ConstantHigh => {
                        shrimply_mesh_flow_stabilization::AdaptiveWeights::ConstantHigh
                    }
                    MeshFlowAdaptiveWeights::ConstantLow => {
                        shrimply_mesh_flow_stabilization::AdaptiveWeights::ConstantLow
                    }
                },
            })
        }
    }
}

fn cache_key(item: &VideoItem, source_position: Time) -> Result<CacheKey, String> {
    let chunk_index = source_position
        .as_nanos_i128()
        .max(0)
        .div_euclid(i128::from(CHUNK_SECONDS) * NANOS_PER_SECOND);
    cache_key_for_chunk(
        item,
        u64::try_from(chunk_index).map_err(|_| "video stabilization time is too large")?,
    )
}

fn cache_key_for_chunk(item: &VideoItem, chunk_index: u64) -> Result<CacheKey, String> {
    Ok(CacheKey {
        base: cache_base(item)?,
        chunk_index,
    })
}

fn cache_base(item: &VideoItem) -> Result<CacheBase, String> {
    let source = item.file.snapshot()?;
    Ok(CacheBase {
        source,
        track_id: item.track_id,
        crop_ratio: item.stabilization_crop_ratio.to_bits(),
        derivative_weights: [
            item.stabilization_first_derivative_weight.to_bits(),
            item.stabilization_second_derivative_weight.to_bits(),
            item.stabilization_third_derivative_weight.to_bits(),
        ],
        method: match item.stabilization_method() {
            VideoStabilizationMethod::Off => 0,
            VideoStabilizationMethod::L1 => 1,
            VideoStabilizationMethod::MeshFlow => 2,
        },
        mesh: [
            item.mesh_flow_rows,
            item.mesh_flow_columns,
            item.mesh_flow_smoothing_radius,
            item.mesh_flow_iterations,
            match item.mesh_flow_adaptive_weights {
                MeshFlowAdaptiveWeights::Original => 0,
                MeshFlowAdaptiveWeights::Flipped => 1,
                MeshFlowAdaptiveWeights::ConstantHigh => 2,
                MeshFlowAdaptiveWeights::ConstantLow => 3,
            },
        ],
    })
}

fn frame_at_time(position: Time, numerator: u32, denominator: u32) -> u64 {
    let frames = position
        .as_nanos_i128()
        .max(0)
        .saturating_mul(i128::from(numerator))
        .div_euclid(NANOS_PER_SECOND.saturating_mul(i128::from(denominator.max(1))));
    u64::try_from(frames).unwrap_or(u64::MAX)
}

struct Cache {
    connection: Connection,
}

impl Cache {
    fn open() -> Result<Self, String> {
        fs::create_dir_all(CACHE_DIRECTORY).map_err(|error| error.to_string())?;
        let connection = Connection::open(CACHE_DATABASE).map_err(|error| error.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS stabilization_chunks (
                    cache_key TEXT PRIMARY KEY,
                    file_path TEXT NOT NULL,
                    track_id INTEGER NOT NULL,
                    file_size INTEGER NOT NULL,
                    modified_ns INTEGER NOT NULL,
                    cache_version INTEGER NOT NULL,
                    chunk_index INTEGER NOT NULL,
                    first_frame INTEGER NOT NULL,
                    frame_rate_numerator INTEGER NOT NULL,
                    frame_rate_denominator INTEGER NOT NULL,
                    source_transforms BLOB NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                )",
                [],
            )
            .map_err(|error| error.to_string())?;
        Ok(Self { connection })
    }

    fn load(&self, key: &CacheKey) -> Result<Option<StabilizationChunk>, String> {
        let row = self
            .connection
            .query_row(
                "SELECT first_frame, frame_rate_numerator, frame_rate_denominator, source_transforms
                 FROM stabilization_chunks
                 WHERE cache_key = ?1 AND file_size = ?2 AND modified_ns = ?3
                   AND cache_version = ?4",
                params![
                    database_key(key),
                    key.base.source.len().min(i64::MAX as u64) as i64,
                    key.base.source.modified_ns().clamp(0, i128::from(i64::MAX)) as i64,
                    CACHE_VERSION,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((first_frame, frame_rate_numerator, frame_rate_denominator, bytes)) = row else {
            return Ok(None);
        };
        if first_frame < 0 {
            return Ok(None);
        }
        if key.base.method == 1 {
            if bytes.len() % 36 != 0 {
                return Ok(None);
            }
            let source_transforms = bytes
                .chunks_exact(36)
                .map(|transform| {
                    glam::Mat3::from_cols_array(&std::array::from_fn(|component| {
                        let start = component * 4;
                        f32::from_le_bytes(
                            transform[start..start + 4]
                                .try_into()
                                .expect("four-byte transform component"),
                        )
                    }))
                })
                .collect();
            return Ok(Some(StabilizationChunk::L1(
                shrimply_affine_l1_stabilization::StabilizationChunk {
                    first_frame: first_frame as u64,
                    frame_rate_numerator,
                    frame_rate_denominator,
                    source_transforms,
                },
            )));
        }
        let grid_width = key.base.mesh[1].clamp(2, 32) + 1;
        let grid_height = key.base.mesh[0].clamp(2, 32) + 1;
        let frame_bytes = grid_width as usize * grid_height as usize * 8;
        if frame_bytes == 0 || bytes.len() % frame_bytes != 0 {
            return Ok(None);
        }
        let source_offsets = bytes
            .chunks_exact(frame_bytes)
            .map(|frame| {
                frame
                    .chunks_exact(8)
                    .map(|offset| {
                        glam::Vec2::new(
                            f32::from_le_bytes(offset[..4].try_into().expect("MeshFlow x offset")),
                            f32::from_le_bytes(offset[4..].try_into().expect("MeshFlow y offset")),
                        )
                    })
                    .collect()
            })
            .collect();
        Ok(Some(StabilizationChunk::MeshFlow(
            shrimply_mesh_flow_stabilization::StabilizationChunk {
                first_frame: first_frame as u64,
                frame_rate_numerator,
                frame_rate_denominator,
                grid_width,
                grid_height,
                source_offsets,
            },
        )))
    }

    fn store(&self, key: &CacheKey, chunk: &StabilizationChunk) -> Result<(), String> {
        let (first_frame, frame_rate_numerator, frame_rate_denominator, bytes) = match chunk {
            StabilizationChunk::L1(chunk) => (
                chunk.first_frame,
                chunk.frame_rate_numerator,
                chunk.frame_rate_denominator,
                chunk
                    .source_transforms
                    .iter()
                    .flat_map(|transform| transform.to_cols_array())
                    .flat_map(|value| value.to_le_bytes())
                    .collect::<Vec<_>>(),
            ),
            StabilizationChunk::MeshFlow(chunk) => (
                chunk.first_frame,
                chunk.frame_rate_numerator,
                chunk.frame_rate_denominator,
                chunk
                    .source_offsets
                    .iter()
                    .flatten()
                    .flat_map(|offset| offset.to_array())
                    .flat_map(|value| value.to_le_bytes())
                    .collect::<Vec<_>>(),
            ),
        };
        self.connection
            .execute(
                "INSERT OR REPLACE INTO stabilization_chunks
                 (cache_key, file_path, track_id, file_size, modified_ns, cache_version,
                  chunk_index, first_frame, frame_rate_numerator, frame_rate_denominator,
                  source_transforms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    database_key(key),
                    key.base.source.path().to_string_lossy(),
                    key.base.track_id,
                    key.base.source.len().min(i64::MAX as u64) as i64,
                    key.base.source.modified_ns().clamp(0, i128::from(i64::MAX)) as i64,
                    CACHE_VERSION,
                    key.chunk_index.min(i64::MAX as u64) as i64,
                    first_frame.min(i64::MAX as u64) as i64,
                    frame_rate_numerator,
                    frame_rate_denominator,
                    bytes,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn remove(&self, key: &CacheKey) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM stabilization_chunks WHERE cache_key = ?1",
                params![database_key(key)],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn database_key(key: &CacheKey) -> String {
    format!(
        "v{CACHE_VERSION}:{}#{}:{}:{:08x}:{:08x}:{:08x}:{:08x}:{:08x}:{:08x}:{:08x}:{:08x}:{:08x}:{:08x}:{}",
        key.base.source.path().display(),
        key.base.track_id,
        key.base.source.cache_key(),
        key.base.method,
        key.base.crop_ratio,
        key.base.derivative_weights[0],
        key.base.derivative_weights[1],
        key.base.derivative_weights[2],
        key.base.mesh[0],
        key.base.mesh[1],
        key.base.mesh[2],
        key.base.mesh[3],
        key.base.mesh[4],
        key.chunk_index,
    )
}
