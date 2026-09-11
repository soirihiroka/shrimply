use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use cached::{Cached, stores::LruCache};
use rusqlite::{Connection, OptionalExtension, params};
use shrimply_path_core::project_cache_directory;
use shrimply_project_document::project::{ItemAddress, Project, Time, VideoItem};
use shrimply_visual_modifiers::{ModifierEffect, RasterModifierEffect, sam2::Sam2Modifier};
use uuid::Uuid;

pub mod analysis;

pub const MASK_SIZE: u32 = 256;
pub const MODEL_SIZE: u32 = 1024;
pub const MASK_LOGIT_QUANTIZATION_SCALE: f32 = 16.0;
const MASK_PIXELS: usize = MASK_SIZE as usize * MASK_SIZE as usize;
const MASK_CACHE_VERSION: i64 = 6;
const MASK_MEMORY_FRAMES: usize = 64;

pub struct ResolvedMask {
    pub target: analysis::AnalysisTarget,
    pub modifier_id: Uuid,
    pub cache_key: String,
    pub frame: i64,
    pub threshold: f32,
    pub softness: f32,
    pub invert: bool,
    pub mask: Option<Arc<[i8]>>,
}

pub fn resolve(
    project: &Project,
    address: &ItemAddress,
    item: &VideoItem,
    position: Time,
    modifier_id: Uuid,
    modifier_index: usize,
    require_mask: bool,
) -> Result<Option<ResolvedMask>, String> {
    let modifier = item
        .modifiers
        .get(modifier_index)
        .ok_or("SAM2 modifier no longer exists")?;
    let ModifierEffect::Raster(effect) = &modifier.effect else {
        return Ok(None);
    };
    let RasterModifierEffect::Sam2(modifier) = &**effect else {
        return Ok(None);
    };
    if modifier.points.is_empty() && modifier.box_prompt.is_none() {
        return Ok(None);
    }
    let frame = shrimply_math_core::frame_index(position, project.fps)
        .ok_or("project frame rate must be positive for SAM2 video tracking")?;
    let frame_position = shrimply_math_core::time_from_frame(frame as u64, project.fps)
        .ok_or("project frame rate must be positive for SAM2 video tracking")?;
    let prompt_time = shrimply_project_document::project::generated_item_time(item, frame_position)
        .unwrap_or(Time::ZERO);
    let cache_key = cache_key(project, address, modifier_id, modifier_index, modifier)?;
    let mask = Sam2MaskCache::shared().get(&cache_key, frame);
    if mask.is_none() && require_mask {
        return Err("Segment Anything 2 mask is unavailable; analyze it again".to_string());
    }
    Ok(Some(ResolvedMask {
        target: analysis::AnalysisTarget {
            address: address.clone(),
            modifier_id,
        },
        modifier_id,
        cache_key,
        frame,
        threshold: modifier.threshold.value_at(prompt_time),
        softness: modifier.softness.value_at(prompt_time).max(0.0),
        invert: modifier.invert,
        mask,
    }))
}

pub fn invalidate_item_analysis(
    address: &ItemAddress,
    item: &VideoItem,
    modifier_id: Uuid,
) -> bool {
    let Some(modifier) = item
        .modifiers
        .iter()
        .find(|modifier| modifier.id == modifier_id)
    else {
        return false;
    };
    let ModifierEffect::Raster(effect) = &modifier.effect else {
        return false;
    };
    let RasterModifierEffect::Sam2(modifier) = &**effect else {
        return false;
    };
    analysis::invalidate_if_stale(
        &analysis::AnalysisTarget {
            address: address.clone(),
            modifier_id,
        },
        modifier.analysis_generation,
        modifier.prompt_signature(),
    )
}

struct MaskCacheStore {
    memory: LruCache<(String, i64), Arc<[i8]>>,
    connection: Connection,
}

#[derive(Clone)]
pub struct Sam2MaskCache {
    store: Arc<Mutex<MaskCacheStore>>,
}

impl Sam2MaskCache {
    pub fn shared() -> Self {
        static CACHES: LazyLock<Mutex<HashMap<PathBuf, Sam2MaskCache>>> =
            LazyLock::new(|| Mutex::new(HashMap::new()));
        let directory = project_cache_directory();
        let path = directory.join("sam2-masks.sqlite");
        let mut caches = CACHES.lock().expect("SAM2 cache registry lock is poisoned");
        caches
            .entry(path.clone())
            .or_insert_with(|| {
                fs::create_dir_all(&directory).expect("create SAM2 cache directory");
                let connection = Connection::open(&path).expect("open SAM2 mask cache");
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
            })
            .clone()
    }

    pub fn get(&self, key: &str, frame: i64) -> Option<Arc<[i8]>> {
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

    pub fn begin_analysis(&self, key: &str) {
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

    pub fn insert_staged(&self, key: &str, frame: i64, mask: &[u8]) -> Result<(), String> {
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

    pub fn complete_analysis(&self, key: &str) {
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

    pub fn abort_analysis(&self, key: &str) {
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

    pub fn analysis_complete(&self, key: &str, frame_count: usize) -> bool {
        let Ok(frame_count) = i64::try_from(frame_count) else {
            return false;
        };
        self.store
            .lock()
            .expect("SAM2 mask cache lock is poisoned")
            .connection
            .query_row(
                "SELECT 1 FROM analyses
                 WHERE cache_key = ?1
                   AND (SELECT COUNT(*) FROM masks
                        WHERE masks.cache_key = analyses.cache_key
                          AND masks.cache_version = ?2) = ?3
                   AND (SELECT COUNT(*) FROM masks
                        WHERE masks.cache_key = analyses.cache_key) = ?3",
                params![key, MASK_CACHE_VERSION, frame_count],
                |_| Ok(()),
            )
            .optional()
            .expect("read SAM2 analysis completion")
            .is_some()
    }
}

pub fn cache_key(
    project: &Project,
    address: &ItemAddress,
    modifier_id: Uuid,
    modifier_index: usize,
    modifier: &Sam2Modifier,
) -> Result<String, String> {
    let mut hasher = DefaultHasher::new();
    let render_project =
        crate::modifier_input::render_input_project(project, address, modifier_index)?;
    serde_json::to_vec(&render_project)
        .expect("serialize SAM2 render input project")
        .hash(&mut hasher);
    let mut assets = render_project
        .assets()
        .into_iter()
        .map(|asset| (asset.path().to_path_buf(), asset.snapshot().ok()))
        .collect::<Vec<_>>();
    assets.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    assets.hash(&mut hasher);
    address.hash(&mut hasher);
    modifier.prompt_signature().hash(&mut hasher);
    Ok(format!(
        "{MASK_CACHE_VERSION}:{modifier_id}:{}:{:016x}:{}:{}",
        modifier.analysis_generation,
        hasher.finish(),
        project.canvas_size.width,
        project.canvas_size.height,
    ))
}

pub fn validate_cache(project: &Project) -> Result<(), String> {
    let cache = Sam2MaskCache::shared();
    for address in crate::sequence::video_item_addresses(project)? {
        let item = project
            .video_item(&address)
            .ok_or_else(|| format!("SAM2 item {} no longer exists", address.item_id()))?;
        for (modifier_index, modifier) in item.modifiers.iter().enumerate() {
            if !modifier.enabled {
                continue;
            }
            let shrimply_visual_modifiers::ModifierEffect::Raster(effect) = &modifier.effect else {
                continue;
            };
            let shrimply_visual_modifiers::RasterModifierEffect::Sam2(sam2) = &**effect else {
                continue;
            };
            if sam2.points.is_empty() && sam2.box_prompt.is_none() {
                continue;
            }
            let key = cache_key(project, &address, modifier.id, modifier_index, sam2)?;
            let frame_count = analysis::analysis_frame_count(project, &address)?;
            if sam2.analysis_generation == 0 || !cache.analysis_complete(&key, frame_count) {
                return Err(format!(
                    "Segment Anything 2 on item {} must be analyzed before export",
                    item.id
                ));
            }
        }
    }
    Ok(())
}
