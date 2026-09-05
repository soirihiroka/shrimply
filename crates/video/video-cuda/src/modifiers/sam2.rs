use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use cached::{Cached, stores::LruCache};
use rusqlite::{Connection, OptionalExtension, params};
use shrimply_project::project::{Project, Time, VideoItem};
use shrimply_video_modifiers::sam2::Sam2Modifier;
use uuid::Uuid;

use super::RasterModifierRuntime;
use crate::{
    gpu::modifiers::{CanvasRgbaFrame, GpuModifier, ModifierContext, ModifierModule},
    layer::RasterVisual,
    visual_source::VisualModifierContext,
};

pub(crate) const MASK_SIZE: u32 = 256;
pub(crate) const MODEL_SIZE: u32 = 1024;
pub(crate) const MASK_LOGIT_QUANTIZATION_SCALE: f32 = 16.0;
const MASK_PIXELS: usize = MASK_SIZE as usize * MASK_SIZE as usize;
const MASK_CACHE_DIRECTORY: &str = "cache";
const MASK_CACHE_DATABASE: &str = "cache/sam2-masks.sqlite";
const MASK_CACHE_VERSION: i64 = 5;
const MASK_MEMORY_FRAMES: usize = 64;

struct MaskCacheStore {
    memory: LruCache<(String, i64), Arc<[i8]>>,
    connection: Connection,
}

#[derive(Clone)]
pub(crate) struct Sam2MaskCache {
    store: Arc<Mutex<MaskCacheStore>>,
}

impl Sam2MaskCache {
    pub(crate) fn shared() -> Self {
        static CACHE: LazyLock<Sam2MaskCache> = LazyLock::new(|| {
            fs::create_dir_all(MASK_CACHE_DIRECTORY).expect("create SAM2 cache directory");
            let connection = Connection::open(MASK_CACHE_DATABASE).expect("open SAM2 mask cache");
            connection
                .busy_timeout(Duration::from_secs(5))
                .expect("configure SAM2 mask cache timeout");
            connection
                .execute_batch(
                    "PRAGMA journal_mode = WAL;
                     PRAGMA synchronous = NORMAL;
                     CREATE TABLE IF NOT EXISTS masks (
                         cache_key TEXT NOT NULL,
                         frame INTEGER NOT NULL,
                         mask BLOB NOT NULL,
                         cache_version INTEGER NOT NULL,
                         updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                         PRIMARY KEY (cache_key, frame)
                     ) WITHOUT ROWID;
                     CREATE TABLE IF NOT EXISTS analyses (
                         cache_key TEXT PRIMARY KEY,
                         completed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                     ) WITHOUT ROWID;",
                )
                .expect("initialize SAM2 mask cache");
            Sam2MaskCache {
                store: Arc::new(Mutex::new(MaskCacheStore {
                    memory: LruCache::builder()
                        .max_size(MASK_MEMORY_FRAMES)
                        .build()
                        .expect("valid SAM2 memory cache size"),
                    connection,
                })),
            }
        });
        CACHE.clone()
    }

    pub(crate) fn get(&self, key: &str, frame: i64) -> Option<Arc<[i8]>> {
        let mut store = self.store.lock().expect("SAM2 mask cache lock is poisoned");
        if let Some(mask) = store.memory.cache_get(&(key.to_string(), frame)).cloned() {
            return Some(mask);
        }
        let bytes = store
            .connection
            .query_row(
                "SELECT masks.mask FROM masks
                 INNER JOIN analyses USING (cache_key)
                 WHERE masks.cache_key = ?1 AND frame = ?2 AND cache_version = ?3",
                params![key, frame, MASK_CACHE_VERSION],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .expect("read SAM2 mask cache")?;
        if bytes.len() != MASK_PIXELS {
            return None;
        }
        let mask = Arc::<[i8]>::from(
            bytes
                .into_iter()
                .map(|value| value as i8)
                .collect::<Vec<_>>(),
        );
        store
            .memory
            .cache_set((key.to_string(), frame), mask.clone());
        Some(mask)
    }

    pub(crate) fn begin_analysis(&self, key: &str) {
        let mut store = self.store.lock().expect("SAM2 mask cache lock is poisoned");
        store.memory.retain(|(stored, _), _| stored != key);
        let transaction = store
            .connection
            .transaction()
            .expect("begin SAM2 cache reset");
        transaction
            .execute("DELETE FROM analyses WHERE cache_key = ?1", params![key])
            .expect("reset SAM2 analysis cache");
        transaction
            .execute("DELETE FROM masks WHERE cache_key = ?1", params![key])
            .expect("reset SAM2 masks");
        transaction.commit().expect("commit SAM2 cache reset");
    }

    pub(crate) fn insert_staged(&self, key: &str, frame: i64, mask: &[u8]) -> Result<(), String> {
        if mask.len() != MASK_PIXELS {
            return Err(format!(
                "invalid SAM2 mask length {}; expected {MASK_PIXELS}",
                mask.len()
            ));
        }
        self.store
            .lock()
            .expect("SAM2 mask cache lock is poisoned")
            .connection
            .execute(
                "INSERT INTO masks (cache_key, frame, mask, cache_version, updated_at)
                 VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)
                 ON CONFLICT(cache_key, frame) DO UPDATE SET
                     mask = excluded.mask,
                     cache_version = excluded.cache_version,
                     updated_at = CURRENT_TIMESTAMP",
                params![key, frame, mask, MASK_CACHE_VERSION],
            )
            .map_err(|error| format!("write SAM2 mask cache: {error}"))?;
        Ok(())
    }

    pub(crate) fn complete_analysis(&self, key: &str) {
        self.store
            .lock()
            .expect("SAM2 mask cache lock is poisoned")
            .connection
            .execute(
                "INSERT OR REPLACE INTO analyses (cache_key, completed_at)
                 VALUES (?1, CURRENT_TIMESTAMP)",
                params![key],
            )
            .expect("write SAM2 analysis completion");
    }

    pub(crate) fn abort_analysis(&self, key: &str) {
        let mut store = self.store.lock().expect("SAM2 mask cache lock is poisoned");
        store.memory.retain(|(stored, _), _| stored != key);
        let transaction = store
            .connection
            .transaction()
            .expect("begin discarding incomplete SAM2 analysis");
        transaction
            .execute("DELETE FROM analyses WHERE cache_key = ?1", params![key])
            .expect("discard incomplete SAM2 analysis");
        transaction
            .execute("DELETE FROM masks WHERE cache_key = ?1", params![key])
            .expect("discard incomplete SAM2 masks");
        transaction
            .commit()
            .expect("commit discarding incomplete SAM2 analysis");
    }

    pub(crate) fn analysis_complete(&self, key: &str) -> bool {
        self.store
            .lock()
            .expect("SAM2 mask cache lock is poisoned")
            .connection
            .query_row(
                "SELECT 1 FROM analyses WHERE cache_key = ?1",
                params![key],
                |_| Ok(()),
            )
            .optional()
            .expect("read SAM2 analysis completion")
            .is_some()
    }
}

pub(crate) fn cache_key(
    project: &Project,
    item: &VideoItem,
    modifier_id: Uuid,
    modifier_index: usize,
    modifier: &Sam2Modifier,
) -> String {
    let mut hasher = DefaultHasher::new();
    let mut analyzed_item = item.clone();
    analyzed_item.modifiers.truncate(modifier_index);
    serde_json::to_string(&analyzed_item)
        .expect("serialize SAM2 input item")
        .hash(&mut hasher);
    project.fps.hash(&mut hasher);
    project.canvas_size.width.hash(&mut hasher);
    project.canvas_size.height.hash(&mut hasher);
    if item.uses_file_asset() {
        item.file.snapshot().ok().hash(&mut hasher);
    }
    modifier.prompt_signature().hash(&mut hasher);
    format!(
        "{MASK_CACHE_VERSION}:{modifier_id}:{}:{:016x}:{}:{}",
        modifier.analysis_generation,
        hasher.finish(),
        project.canvas_size.width,
        project.canvas_size.height,
    )
}

pub(crate) fn validate_cache(project: &Project) -> Result<(), String> {
    let cache = Sam2MaskCache::shared();
    for item in project.video_tracks.iter().flat_map(|track| &track.items) {
        for (modifier_index, modifier) in item.modifiers.iter().enumerate() {
            if !modifier.enabled {
                continue;
            }
            let shrimply_video_modifiers::ModifierEffect::Raster(effect) = &modifier.effect else {
                continue;
            };
            let shrimply_video_modifiers::RasterModifierEffect::Sam2(sam2) = &**effect else {
                continue;
            };
            if sam2.points.is_empty() && sam2.box_prompt.is_none() {
                continue;
            }
            let key = cache_key(project, item, modifier.id, modifier_index, sam2);
            if sam2.analysis_generation == 0 || !cache.analysis_complete(&key) {
                return Err(format!(
                    "Segment Anything 2 on item {} must be analyzed before export",
                    item.id
                ));
            }
        }
    }
    Ok(())
}

struct Resolved {
    modifier_id: Uuid,
    cache_key: String,
    frame: i64,
    threshold: f32,
    softness: f32,
    invert: bool,
    require_mask: bool,
}

impl GpuModifier for Resolved {
    fn name(&self) -> &'static str {
        "Segment Anything 2"
    }

    fn apply(
        &self,
        context: &mut ModifierContext<'_>,
        input: CanvasRgbaFrame,
    ) -> Result<CanvasRgbaFrame, String> {
        if context.capture_sam2(self.modifier_id, &input)? {
            return Ok(input);
        }
        let Some(mask) = context.sam2_mask(&self.cache_key, self.frame)? else {
            if self.require_mask {
                return Err("Segment Anything 2 mask is unavailable; analyze it again".to_string());
            }
            return Ok(input);
        };
        let width = input.width();
        let height = input.height();
        let count = width as usize * height as usize;
        let mut pass = input.into_pass(context)?;
        let module = context.modifier_module(ModifierModule::Matte)?;
        unsafe {
            shrimply_cuda::cuda_launch! {
                kernel: sam2_apply_mask,
                stream: context.stream(), module: &module,
                config: shrimply_cuda::LaunchConfig::for_num_elems(u32::try_from(count).map_err(|_| "canvas is too large")?),
                args: [pass.input_ptr(), mask, slice_mut(pass.output_buffer()), shrimply_render_core::Sam2MaskParams {
                    output_width: width,
                    output_height: height,
                    mask_size: MASK_SIZE,
                    threshold: self.threshold,
                    softness: self.softness,
                    invert: self.invert,
                    quantization_scale: MASK_LOGIT_QUANTIZATION_SCALE,
                    _padding_0: [0; 3],
                }]
            }
        }
        .map_err(|error| format!("launch SAM2 mask kernel: {error:?}"))?;
        Ok(pass.finish(context))
    }
}

impl RasterModifierRuntime for Sam2Modifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        if self.points.is_empty() && self.box_prompt.is_none() {
            return Ok(input);
        }
        let frame = shrimply_math_core::frame_index(context.position, context.project.fps)
            .ok_or("project frame rate must be positive for SAM2 video tracking")?;
        let frame_position = shrimply_math_core::time_from_frame(frame as u64, context.project.fps)
            .ok_or("project frame rate must be positive for SAM2 video tracking")?;
        let prompt_time =
            shrimply_project::project::generated_item_time(context.item, frame_position)
                .unwrap_or(Time::ZERO);
        input.push_pixel(Box::new(Resolved {
            modifier_id: context.modifier_id,
            cache_key: cache_key(
                context.project,
                context.item,
                context.modifier_id,
                context.modifier_index,
                self,
            ),
            frame,
            threshold: self.threshold.value_at(prompt_time),
            softness: self.softness.value_at(prompt_time).max(0.0),
            invert: self.invert,
            require_mask: context.require_complete_assets,
        }));
        Ok(input)
    }
}
