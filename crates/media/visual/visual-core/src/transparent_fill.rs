use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use cached::{Cached, stores::LruCache};
use rusqlite::{Connection, OptionalExtension, params};
use shrimply_path_core::project_cache_directory;
use shrimply_project_document::project::{
    ItemAddress, Project, Time, VideoItem, VideoItemContent, VisualTrack, video_source_time_at,
};
use shrimply_visual_modifiers::{
    ModifierEffect, RasterModifierEffect, transparent_fill::TransparentFillModifier,
};
use uuid::Uuid;

pub mod analysis;
pub use crate::modifier_input::render_input_project;

pub const CACHE_VERSION: i64 = 3;
pub const MEMORY_FRAMES: usize = 64;

pub struct ResolvedMask {
    pub cache_key: String,
    pub frame: i64,
    pub mask: Option<Arc<[u8]>>,
}

pub fn resolve_cache_key(
    cache_key: String,
    frame: i64,
    width: u32,
    height: u32,
    require_mask: bool,
) -> Result<ResolvedMask, String> {
    let mask = TransparentFillMaskCache::shared().get(&cache_key, frame, width, height)?;
    if mask.is_none() && require_mask {
        return Err(format!(
            "transparent fill mask for project frame {frame} at {width}x{height} is unavailable; analyze it again"
        ));
    }
    Ok(ResolvedMask {
        cache_key,
        frame,
        mask,
    })
}

pub fn resolve(
    project: &Project,
    address: &ItemAddress,
    item: &VideoItem,
    modifier_id: Uuid,
    modifier_index: usize,
    position: Time,
    require_mask: bool,
) -> Result<Option<ResolvedMask>, String> {
    let modifier = item
        .modifiers
        .get(modifier_index)
        .ok_or("transparent fill modifier no longer exists")?;
    let ModifierEffect::Raster(effect) = &modifier.effect else {
        return Ok(None);
    };
    let RasterModifierEffect::TransparentFill(fill) = &**effect else {
        return Ok(None);
    };
    if fill.points.is_empty() {
        return Ok(None);
    }
    let source_index = project
        .video_item(address)
        .and_then(|source| {
            source
                .modifiers
                .iter()
                .position(|source| source.id == modifier_id)
        })
        .ok_or("transparent fill modifier source no longer exists")?;
    let render_project = render_input_project(project, address, source_index)?;
    let frame = shrimply_math_core::frame_index(position, project.fps)
        .ok_or("project frame rate must be positive for transparent fill")?;
    let cache_key = analysis_cache_key(
        &render_project,
        address,
        modifier_id,
        fill.prompt_signature(),
    );
    let width = project.canvas_size.width;
    let height = project.canvas_size.height;
    resolve_cache_key(cache_key, frame, width, height, require_mask).map(Some)
}

struct CacheStore {
    memory: LruCache<(String, i64), Arc<[u8]>>,
    connection: Connection,
}

#[derive(Clone)]
pub struct TransparentFillMaskCache {
    store: Arc<Mutex<CacheStore>>,
}

impl TransparentFillMaskCache {
    pub fn shared() -> Self {
        static CACHES: LazyLock<Mutex<HashMap<PathBuf, TransparentFillMaskCache>>> =
            LazyLock::new(|| Mutex::new(HashMap::new()));
        let path = project_cache_directory().join("transparent-fill-masks.sqlite");
        let mut caches = CACHES
            .lock()
            .expect("transparent fill cache registry lock is poisoned");
        caches
            .entry(path.clone())
            .or_insert_with(|| {
                TransparentFillMaskCache::open(&path).expect("open transparent fill mask cache")
            })
            .clone()
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))
            .map_err(|error| format!("create transparent fill cache directory: {error}"))?;
        let mut connection = Connection::open(path)
            .map_err(|error| format!("open transparent fill mask cache: {error}"))?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| format!("configure transparent fill cache timeout: {error}"))?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;",
            )
            .map_err(|error| format!("configure transparent fill mask cache: {error}"))?;
        let transaction = connection.transaction().map_err(|error| {
            format!("begin transparent fill mask cache initialization: {error}")
        })?;
        transaction
            .execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS masks (
                         cache_key TEXT NOT NULL,
                         frame INTEGER NOT NULL,
                         png BLOB NOT NULL,
                         cache_version INTEGER NOT NULL,
                         PRIMARY KEY (cache_key, frame)
                     ) WITHOUT ROWID;
                     CREATE TABLE IF NOT EXISTS analyses (
                         cache_key TEXT PRIMARY KEY,
                         width INTEGER NOT NULL,
                         height INTEGER NOT NULL,
                         frame_count INTEGER NOT NULL,
                         completed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                     ) WITHOUT ROWID;
                     DELETE FROM analyses WHERE cache_key NOT LIKE '{CACHE_VERSION}:%' OR cache_key LIKE '%:run:%';
                     DELETE FROM masks WHERE cache_version != {CACHE_VERSION} OR cache_key LIKE '%:run:%';
                     DELETE FROM analyses
                     WHERE width <= 0 OR height <= 0 OR frame_count < 0
                        OR frame_count != (
                            SELECT COUNT(*) FROM masks
                            WHERE masks.cache_key = analyses.cache_key
                              AND masks.cache_version = {CACHE_VERSION}
                        );
                     DELETE FROM masks
                     WHERE NOT EXISTS (
                         SELECT 1 FROM analyses
                         WHERE analyses.cache_key = masks.cache_key
                     );"
            ))
            .map_err(|error| format!("initialize transparent fill mask cache: {error}"))?;
        transaction.commit().map_err(|error| {
            format!("commit transparent fill mask cache initialization: {error}")
        })?;
        Ok(Self {
            store: Arc::new(Mutex::new(CacheStore {
                memory: LruCache::builder()
                    .max_size(MEMORY_FRAMES)
                    .build()
                    .expect("valid transparent fill memory cache size"),
                connection,
            })),
        })
    }

    pub fn get(
        &self,
        key: &str,
        frame: i64,
        width: u32,
        height: u32,
    ) -> Result<Option<Arc<[u8]>>, String> {
        let mut store = self
            .store
            .lock()
            .expect("transparent fill mask cache lock is poisoned");
        if let Some(mask) = store.memory.cache_get(&(key.to_string(), frame)).cloned() {
            return Ok(Some(mask));
        }
        let encoded = store
            .connection
            .query_row(
                "SELECT masks.png FROM masks
                 INNER JOIN analyses USING (cache_key)
                 WHERE masks.cache_key = ?1 AND frame = ?2 AND cache_version = ?3
                   AND width = ?4 AND height = ?5",
                params![key, frame, CACHE_VERSION, width, height],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|error| format!("read transparent fill mask cache: {error}"))?;
        let Some(encoded) = encoded else {
            return Ok(None);
        };
        let mask = Arc::<[u8]>::from(decode_mask(&encoded, width, height)?);
        store
            .memory
            .cache_set((key.to_string(), frame), mask.clone());
        Ok(Some(mask))
    }

    pub fn begin_analysis(&self, staging_key: &str) -> Result<(), String> {
        let mut store = self
            .store
            .lock()
            .expect("transparent fill mask cache lock is poisoned");
        store.memory.retain(|(stored, _), _| stored != staging_key);
        let transaction = store
            .connection
            .transaction()
            .map_err(|error| format!("begin transparent fill cache reset: {error}"))?;
        transaction
            .execute(
                "DELETE FROM analyses WHERE cache_key = ?1",
                params![staging_key],
            )
            .and_then(|_| {
                transaction.execute(
                    "DELETE FROM masks WHERE cache_key = ?1",
                    params![staging_key],
                )
            })
            .map_err(|error| format!("reset transparent fill mask cache: {error}"))?;
        transaction.commit().map_err(|error| {
            format!("commit transparent fill cache reset for {staging_key}: {error}")
        })
    }

    pub fn insert_staged(
        &self,
        key: &str,
        frame: i64,
        mask: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let png = encode_mask(mask, width, height)?;
        self.insert_staged_encoded(key, frame, mask, png)
    }

    pub fn insert_staged_encoded(
        &self,
        key: &str,
        frame: i64,
        _mask: &[u8],
        png: Vec<u8>,
    ) -> Result<(), String> {
        let store = self
            .store
            .lock()
            .expect("transparent fill mask cache lock is poisoned");
        store
            .connection
            .execute(
                "INSERT OR REPLACE INTO masks (cache_key, frame, png, cache_version)
                 VALUES (?1, ?2, ?3, ?4)",
                params![key, frame, png, CACHE_VERSION],
            )
            .map_err(|error| format!("write transparent fill mask cache: {error}"))?;
        Ok(())
    }

    pub fn complete_analysis(
        &self,
        key: &str,
        width: u32,
        height: u32,
        frame_count: u64,
    ) -> Result<(), String> {
        let frame_count = i64::try_from(frame_count)
            .map_err(|_| "transparent fill frame count is too large".to_string())?;
        self.store
            .lock()
            .expect("transparent fill mask cache lock is poisoned")
            .connection
            .execute(
                "INSERT OR REPLACE INTO analyses
                 (cache_key, width, height, frame_count, completed_at)
                 VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)",
                params![key, width, height, frame_count],
            )
            .map_err(|error| format!("complete transparent fill mask cache: {error}"))?;
        Ok(())
    }

    pub fn publish_analysis(
        &self,
        staging_key: &str,
        key: &str,
        width: u32,
        height: u32,
        frame_count: u64,
    ) -> Result<(), String> {
        let frame_count = i64::try_from(frame_count)
            .map_err(|_| "transparent fill frame count is too large".to_string())?;
        let mut store = self
            .store
            .lock()
            .expect("transparent fill mask cache lock is poisoned");
        store.memory.retain(|(stored, _), _| stored != key);
        let transaction = store
            .connection
            .transaction()
            .map_err(|error| format!("begin transparent fill cache completion: {error}"))?;
        transaction
            .execute("DELETE FROM analyses WHERE cache_key = ?1", params![key])
            .and_then(|_| {
                transaction.execute("DELETE FROM masks WHERE cache_key = ?1", params![key])
            })
            .and_then(|_| {
                transaction.execute(
                    "UPDATE masks SET cache_key = ?1 WHERE cache_key = ?2",
                    params![key, staging_key],
                )
            })
            .and_then(|_| {
                transaction.execute(
                    "INSERT INTO analyses
                 (cache_key, width, height, frame_count, completed_at)
                 VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)",
                    params![key, width, height, frame_count],
                )
            })
            .map_err(|error| format!("complete transparent fill mask cache: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit transparent fill mask cache: {error}"))
    }

    pub fn abort_analysis(&self, key: &str) {
        let mut store = self
            .store
            .lock()
            .expect("transparent fill mask cache lock is poisoned");
        store.memory.retain(|(stored, _), _| stored != key);
        let _ = store
            .connection
            .execute("DELETE FROM masks WHERE cache_key = ?1", params![key]);
        let _ = store
            .connection
            .execute("DELETE FROM analyses WHERE cache_key = ?1", params![key]);
    }

    pub fn analysis_complete(&self, key: &str, width: u32, height: u32, frame_count: u64) -> bool {
        let Ok(frame_count) = i64::try_from(frame_count) else {
            return false;
        };
        self.store
            .lock()
            .expect("transparent fill mask cache lock is poisoned")
            .connection
            .query_row(
                "SELECT 1
                 FROM analyses
                 WHERE cache_key = ?1 AND width = ?2 AND height = ?3 AND frame_count = ?4
                   AND (SELECT COUNT(*) FROM masks WHERE masks.cache_key = analyses.cache_key) = ?4",
                params![key, width, height, frame_count],
                |_| Ok(()),
            )
            .optional()
            .is_ok_and(|value| value.is_some())
    }
}

pub fn encode_mask(mask: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let expected = (width as usize).div_ceil(8).saturating_mul(height as usize);
    if mask.len() != expected {
        return Err(format!(
            "transparent fill mask has {} bytes; expected {expected}",
            mask.len()
        ));
    }
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::One);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_filter(png::Filter::NoFilter);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(mask))
        .map_err(|error| format!("encode transparent fill mask PNG: {error}"))?;
    Ok(encoded)
}

fn decode_mask(encoded: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let decoder = png::Decoder::new(Cursor::new(encoded));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("decode transparent fill mask PNG header: {error}"))?;
    let mut mask = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("transparent fill mask PNG is too large")?
    ];
    let info = reader
        .next_frame(&mut mask)
        .map_err(|error| format!("decode transparent fill mask PNG: {error}"))?;
    if info.width != width
        || info.height != height
        || info.color_type != png::ColorType::Grayscale
        || info.bit_depth != png::BitDepth::One
    {
        return Err("transparent fill mask PNG format does not match the frame".to_string());
    }
    mask.truncate(info.buffer_size());
    let expected = (width as usize).div_ceil(8).saturating_mul(height as usize);
    if mask.len() != expected {
        return Err("transparent fill mask PNG has invalid packed row data".to_string());
    }
    Ok(mask)
}

pub fn analysis_cache_key(
    project: &Project,
    address: &ItemAddress,
    modifier_id: Uuid,
    prompt_signature: u64,
) -> String {
    let mut hasher = DefaultHasher::new();
    serde_json::to_vec(project)
        .expect("serialize transparent fill render input project")
        .hash(&mut hasher);
    let mut assets = project
        .assets()
        .into_iter()
        .map(|asset| (asset.path().to_path_buf(), asset.snapshot().ok()))
        .collect::<Vec<_>>();
    assets.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    assets.hash(&mut hasher);
    address.hash(&mut hasher);
    prompt_signature.hash(&mut hasher);
    format!("{CACHE_VERSION}:{modifier_id}:{:016x}", hasher.finish())
}

#[derive(Clone, Copy)]
pub struct AnalysisFrame {
    pub timeline_position: Time,
    pub sequence_position: Time,
    pub cache_index: u64,
}

pub fn analysis_frames(
    project: &Project,
    address: &ItemAddress,
) -> Result<Vec<AnalysisFrame>, String> {
    let ItemAddress::Video { sequence_path, .. } = address else {
        return Err("transparent fill requires a video item".to_string());
    };
    let item = project
        .video_item(address)
        .ok_or_else(|| "transparent fill item no longer exists".to_string())?;
    let (start, end) = if let Some(host_id) = sequence_path.first() {
        let host = project
            .video_tracks
            .iter()
            .flat_map(|track| &track.items)
            .find(|host| host.id == *host_id)
            .ok_or_else(|| "transparent fill sequence host no longer exists".to_string())?;
        (host.start, host.end)
    } else {
        (item.start, item.end)
    };
    let timeline_frames = shrimply_math_core::frame_range(start, end, project.fps)
        .ok_or("project frame rate must be positive for transparent fill")?;
    let mut cache_indices = HashSet::new();
    let mut frames = Vec::new();
    for timeline_frame in timeline_frames {
        let timeline_position = shrimply_math_core::time_from_frame(timeline_frame, project.fps)
            .ok_or("project frame rate must be positive for transparent fill")?
            .max(start);
        let Some(sequence_position) = target_sequence_position(project, address, timeline_position)
        else {
            continue;
        };
        let cache_index = shrimply_math_core::frame_index(
            snapped_transparent_fill_position(project, item, sequence_position),
            project.fps,
        )
        .and_then(|frame| u64::try_from(frame).ok())
        .ok_or("transparent fill sequence frame is outside the cache range")?;
        if cache_indices.insert(cache_index) {
            frames.push(AnalysisFrame {
                timeline_position,
                sequence_position,
                cache_index,
            });
        }
    }
    if frames.is_empty() {
        return Err("cannot analyze an item shorter than one project frame".to_string());
    }
    Ok(frames)
}

fn target_sequence_position(
    project: &Project,
    address: &ItemAddress,
    mut position: Time,
) -> Option<Time> {
    let ItemAddress::Video { sequence_path, .. } = address else {
        return None;
    };
    let mut tracks = project.video_tracks.as_slice();
    for host_id in sequence_path {
        let host = tracks
            .iter()
            .flat_map(|track| &track.items)
            .find(|host| host.id == *host_id)?;
        if position < host.start || position >= host.end {
            return None;
        }
        let VideoItemContent::FoldedSequence(reference) = host.content else {
            return None;
        };
        position = video_source_time_at(host, position)?;
        tracks = &project.folded_sequence(reference.sequence_id)?.video_tracks;
    }
    let item = project.video_item(address)?;
    (position >= item.start && position < item.end).then_some(position)
}

pub fn cache_key(
    project: &Project,
    item: &VideoItem,
    modifier_id: Uuid,
    modifier_index: usize,
    modifier: &TransparentFillModifier,
) -> String {
    let track_id = project
        .video_tracks
        .iter()
        .find(|track| track.items.iter().any(|candidate| candidate.id == item.id))
        .map(|track| track.id)
        .expect("transparent fill cache key requires a root video item");
    let address = ItemAddress::Video {
        sequence_path: Vec::new(),
        track_id,
        item_id: item.id,
    };
    let render_project = render_input_project(project, &address, modifier_index)
        .expect("transparent fill cache input must be available");
    analysis_cache_key(
        &render_project,
        &address,
        modifier_id,
        modifier.prompt_signature(),
    )
}

pub fn frame_count(project: &Project, item: &VideoItem) -> Option<u64> {
    let range = shrimply_math_core::frame_range(item.start, item.end, project.fps)?;
    Some(range.end.saturating_sub(range.start))
}

pub fn render_position(project: &Project, item: &VideoItem, position: Time) -> Time {
    let active = item.modifiers.iter().any(|modifier| {
        modifier.enabled
            && matches!(
                &modifier.effect,
                shrimply_visual_modifiers::ModifierEffect::Raster(effect)
                    if matches!(
                        &**effect,
                        shrimply_visual_modifiers::RasterModifierEffect::TransparentFill(fill)
                            if !fill.points.is_empty() && fill.analysis_generation > 0
                    )
            )
    });
    if !active {
        return position;
    }
    snapped_transparent_fill_position(project, item, position)
}

pub fn snapped_transparent_fill_position(
    project: &Project,
    item: &VideoItem,
    position: Time,
) -> Time {
    shrimply_math_core::frame_index(position, project.fps)
        .and_then(|frame| u64::try_from(frame).ok())
        .and_then(|frame| shrimply_math_core::time_from_frame(frame, project.fps))
        .map(|position| position.max(item.start))
        .unwrap_or(position)
}

pub fn validate_cache(project: &Project) -> Result<(), String> {
    let cache = TransparentFillMaskCache::shared();
    validate_track_caches(
        project,
        &project.video_tracks,
        &mut Vec::new(),
        &mut Vec::new(),
        &cache,
    )
}

fn validate_track_caches(
    project: &Project,
    tracks: &[VisualTrack],
    sequence_path: &mut Vec<Uuid>,
    sequence_stack: &mut Vec<Uuid>,
    cache: &TransparentFillMaskCache,
) -> Result<(), String> {
    for track in tracks {
        for item in &track.items {
            let address = ItemAddress::Video {
                sequence_path: sequence_path.clone(),
                track_id: track.id,
                item_id: item.id,
            };
            for (modifier_index, modifier) in item.modifiers.iter().enumerate() {
                if !modifier.enabled {
                    continue;
                }
                let shrimply_visual_modifiers::ModifierEffect::Raster(effect) = &modifier.effect
                else {
                    continue;
                };
                let shrimply_visual_modifiers::RasterModifierEffect::TransparentFill(fill) =
                    &**effect
                else {
                    continue;
                };
                if fill.points.is_empty() {
                    continue;
                }
                let count = u64::try_from(analysis_frames(project, &address)?.len())
                    .map_err(|_| "transparent fill frame count is too large")?;
                let render_project = render_input_project(project, &address, modifier_index)?;
                let key = analysis_cache_key(
                    &render_project,
                    &address,
                    modifier.id,
                    fill.prompt_signature(),
                );
                if fill.analysis_generation == 0
                    || !cache.analysis_complete(
                        &key,
                        project.canvas_size.width,
                        project.canvas_size.height,
                        count,
                    )
                {
                    return Err(format!(
                        "Transparent Fill on item {} must be analyzed before export",
                        item.id
                    ));
                }
            }
            let VideoItemContent::FoldedSequence(reference) = item.content else {
                continue;
            };
            if sequence_stack.contains(&reference.sequence_id) {
                return Err(format!(
                    "cyclic folded sequence reference involving {}",
                    reference.sequence_id
                ));
            }
            let sequence = project
                .folded_sequence(reference.sequence_id)
                .ok_or_else(|| format!("missing folded sequence {}", reference.sequence_id))?;
            sequence_stack.push(reference.sequence_id);
            sequence_path.push(item.id);
            let result = validate_track_caches(
                project,
                &sequence.video_tracks,
                sequence_path,
                sequence_stack,
                cache,
            );
            sequence_path.pop();
            sequence_stack.pop();
            result?;
        }
    }
    Ok(())
}
