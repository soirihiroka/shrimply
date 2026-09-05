use std::{
    collections::{HashSet, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    io::Cursor,
    path::Path,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use cached::{Cached, stores::LruCache};
use rusqlite::{Connection, OptionalExtension, params};
use shrimply_project::project::{
    ItemAddress, Project, Time, TrackMut, VideoItem, VideoItemContent, VisualTrack,
    video_source_time_at,
};
use shrimply_video_modifiers::transparent_fill::TransparentFillModifier;
use uuid::Uuid;

use super::RasterModifierRuntime;
use crate::{
    gpu::modifiers::{CanvasRgbaFrame, GpuModifier, ModifierContext, ModifierModule},
    layer::RasterVisual,
    visual_source::VisualModifierContext,
};

const CACHE_DATABASE: &str = "cache/transparent-fill-masks.sqlite";
const CACHE_VERSION: i64 = 3;
const MEMORY_FRAMES: usize = 64;

struct CacheStore {
    memory: LruCache<(String, i64), Arc<[u8]>>,
    connection: Connection,
}

#[derive(Clone)]
pub(crate) struct TransparentFillMaskCache {
    store: Arc<Mutex<CacheStore>>,
}

impl TransparentFillMaskCache {
    pub(crate) fn shared() -> Self {
        static CACHE: LazyLock<TransparentFillMaskCache> = LazyLock::new(|| {
            TransparentFillMaskCache::open(Path::new(CACHE_DATABASE))
                .expect("open transparent fill mask cache")
        });
        CACHE.clone()
    }

    fn open(path: &Path) -> Result<Self, String> {
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

    pub(crate) fn get(
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

    pub(crate) fn begin_analysis(&self, staging_key: &str) -> Result<(), String> {
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

    #[cfg(test)]
    pub(crate) fn insert_staged(
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

    pub(crate) fn insert_staged_encoded(
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

    #[cfg(test)]
    pub(crate) fn complete_analysis(
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

    pub(crate) fn publish_analysis(
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

    pub(crate) fn abort_analysis(&self, key: &str) {
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

    pub(crate) fn analysis_complete(
        &self,
        key: &str,
        width: u32,
        height: u32,
        frame_count: u64,
    ) -> bool {
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

pub(crate) fn encode_mask(mask: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
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

pub(crate) fn analysis_cache_key(
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

pub(crate) fn render_input_project(
    project: &Project,
    address: &ItemAddress,
    modifier_index: usize,
) -> Result<Project, String> {
    let mut render_project = project.clone();
    render_project.format_version = 0;
    render_project.name.clear();
    render_project.expanded_sequence_paths.clear();
    render_project.cursor_position = None;
    render_project.timeline_zoom = None;
    render_project.preview_guides = Box::default();
    let target_item_id = address.item_id();
    let TrackMut::Video(track) = render_project
        .track_mut(&address.track())
        .ok_or_else(|| "transparent fill track no longer exists".to_string())?
    else {
        return Err("transparent fill requires a video track".to_string());
    };
    remove_incoming_transition(track, target_item_id);
    render_project
        .video_item_mut(address)
        .ok_or_else(|| "transparent fill item no longer exists".to_string())?
        .modifiers
        .truncate(modifier_index);
    Ok(render_project)
}

#[derive(Clone, Copy)]
pub(crate) struct AnalysisFrame {
    pub(crate) timeline_position: Time,
    pub(crate) sequence_position: Time,
    pub(crate) cache_index: u64,
}

pub(crate) fn analysis_frames(
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

fn remove_incoming_transition(
    track: &mut shrimply_project::project::VisualTrack,
    target_item_id: Uuid,
) {
    for item in &mut track.items {
        if item
            .transitions
            .to_next
            .as_ref()
            .is_some_and(|transition| transition.target_item_id == target_item_id)
        {
            item.transitions.to_next = None;
        }
    }
}

#[cfg(test)]
pub(crate) fn cache_key(
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

#[cfg(test)]
pub(crate) fn frame_count(project: &Project, item: &VideoItem) -> Option<u64> {
    let range = shrimply_math_core::frame_range(item.start, item.end, project.fps)?;
    Some(range.end.saturating_sub(range.start))
}

pub(crate) fn render_position(project: &Project, item: &VideoItem, position: Time) -> Time {
    let active = item.modifiers.iter().any(|modifier| {
        modifier.enabled
            && matches!(
                &modifier.effect,
                shrimply_video_modifiers::ModifierEffect::Raster(effect)
                    if matches!(
                        &**effect,
                        shrimply_video_modifiers::RasterModifierEffect::TransparentFill(fill)
                            if !fill.points.is_empty() && fill.analysis_generation > 0
                    )
            )
    });
    if !active {
        return position;
    }
    snapped_transparent_fill_position(project, item, position)
}

pub(crate) fn snapped_transparent_fill_position(
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

pub(crate) fn validate_cache(project: &Project) -> Result<(), String> {
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
                let shrimply_video_modifiers::ModifierEffect::Raster(effect) = &modifier.effect
                else {
                    continue;
                };
                let shrimply_video_modifiers::RasterModifierEffect::TransparentFill(fill) =
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

struct Resolved {
    cache_key: String,
    frame: i64,
    require_mask: bool,
}

impl GpuModifier for Resolved {
    fn name(&self) -> &'static str {
        "Transparent Fill"
    }

    fn apply(
        &self,
        context: &mut ModifierContext<'_>,
        input: CanvasRgbaFrame,
    ) -> Result<CanvasRgbaFrame, String> {
        let width = input.width();
        let height = input.height();
        let Some(mask) =
            context.transparent_fill_mask(&self.cache_key, self.frame, width, height)?
        else {
            if self.require_mask {
                return Err(format!(
                    "transparent fill mask for project frame {} at {width}x{height} is unavailable; analyze it again",
                    self.frame
                ));
            }
            return Ok(input);
        };
        let count = width as usize * height as usize;
        let mut pass = input.into_pass(context)?;
        let module = context.modifier_module(ModifierModule::Matte)?;
        unsafe {
            shrimply_cuda::cuda_launch! {
                kernel: transparent_fill_apply_mask,
                stream: context.stream(), module: &module,
                config: shrimply_cuda::LaunchConfig::for_num_elems(u32::try_from(count).map_err(|_| "canvas is too large")?),
                args: [pass.input_ptr(), mask, slice_mut(pass.output_buffer()), shrimply_render_core::TransparentFillMaskParams {
                    width,
                    height,
                    stride: width.div_ceil(8),
                }]
            }
        }
        .map_err(|error| format!("launch transparent fill mask kernel: {error:?}"))?;
        Ok(pass.finish(context))
    }
}

impl RasterModifierRuntime for TransparentFillModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        if self.points.is_empty() {
            return Ok(input);
        }
        let frame = shrimply_math_core::frame_index(context.position, context.project.fps)
            .ok_or("project frame rate must be positive for transparent fill")?;
        input.push_pixel(Box::new(Resolved {
            cache_key: context
                .analysis_cache_key
                .clone()
                .ok_or("transparent fill analysis identity was not prepared")?,
            frame,
            require_mask: context.require_complete_assets,
        }));
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        io::BufWriter,
        path::{Path, PathBuf},
        process::Command,
        thread,
        time::{Duration, Instant},
    };

    use glam::Vec2;
    use shrimply_asset::Asset;
    use shrimply_core::timeline_value::TimelineValue;
    use shrimply_cuda::CudaContext;
    use shrimply_gpu_memory::AllocationClass;
    use shrimply_math_core::{Time, fraction_new};
    use shrimply_project::project::{
        CanvasSize, ItemAddress, Project, VideoItem, VideoItemContent, VideoTrack, VisualModifier,
        activate_project, create_project_file, prepare_project, shutdown_history,
    };
    use shrimply_video_modifiers::{
        ModifierEffect, RasterModifierEffect, color_correction::ColorCorrectionModifier,
    };

    use super::*;
    use crate::{
        compositor::{
            CompositeAccuracy, VideoCommand, VideoEvent, VideoExportRenderer, spawn_worker,
        },
        transparent_fill_analysis::{self, Status},
    };

    const WIDTH: u32 = 96;
    const HEIGHT: u32 = 64;
    const FIRST_FRAME: u64 = 5_895;
    const FRAME_COUNT: u64 = 15;
    const E2E_WIDTH: u32 = 64;
    const E2E_HEIGHT: u32 = 64;
    const E2E_FIRST_PROJECT_FRAME: u64 = 5_895;
    const E2E_PROJECT_FRAMES: u64 = 31;
    const E2E_SOURCE_FPS: i64 = 24;
    const E2E_PROJECT_FPS: i64 = 30;
    const E2E_SOURCE_OFFSET_NUMERATOR: i64 = 103_101_571;
    const E2E_SOURCE_OFFSET_DENOMINATOR: i64 = 50_000_000;
    const E2E_SQUARE_SIZE: u32 = 16;
    const E2E_SQUARE_STEP: u32 = 4;
    const E2E_SQUARE_RANGE: u32 = E2E_WIDTH - E2E_SQUARE_SIZE;

    #[test]
    fn partial_first_project_frame_uses_the_item_start_mask() {
        let mut project: Project =
            serde_json::from_str("{}").expect("create partial-frame project");
        project.fps = fraction_new(30, 1);
        let start = Time::from_fraction(1, 120);
        let end = Time::from_fraction(1, 20);
        let mut item = VideoItem::background_item(project.canvas_size, start, end);
        item.modifiers
            .push(VisualModifier::new(ModifierEffect::Raster(Box::new(
                RasterModifierEffect::TransparentFill(TransparentFillModifier {
                    points: vec![
                        shrimply_video_modifiers::transparent_fill::TransparentFillPoint {
                            id: Uuid::new_v4(),
                            position: TimelineValue::new_const(Vec2::splat(0.5)),
                        },
                    ],
                    tolerance: TimelineValue::new_const(0.1),
                    maximum_gap: 0,
                    analysis_generation: 1,
                }),
            ))));

        assert_eq!(frame_count(&project, &item), Some(2));
        assert_eq!(
            render_position(&project, &item, Time::from_fraction(1, 60)),
            start
        );
    }

    #[test]
    fn generates_transparent_fill_end_to_end_fixture() {
        let fixture = transparent_fill_fixture();
        assert!(fixture.video.exists());
        assert!(fixture.project_path.exists());
    }

    #[test]
    #[ignore = "requires enough free GPU memory for Transparent Fill analysis and export"]
    fn transparent_fill_analyzes_and_renders_a_real_project_end_to_end() {
        let fixture = transparent_fill_fixture();
        let original = env::current_dir().expect("read end-to-end test working directory");
        env::set_current_dir(&fixture.directory)
            .expect("isolate end-to-end Transparent Fill cache");
        ffmpeg_next::init().expect("initialize FFmpeg");

        let prepared = prepare_project(&fixture.project_path).expect("load generated project");
        let project = activate_project(prepared);
        transparent_fill_analysis::analyze(project.clone(), &fixture.address, fixture.modifier_id)
            .expect("start real Transparent Fill analysis");
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            match transparent_fill_analysis::status(&project, &fixture.address, fixture.modifier_id)
            {
                Status::Complete => break,
                Status::Failed(error) => panic!("Transparent Fill analysis failed: {error}"),
                Status::Cancelled => panic!("Transparent Fill analysis was cancelled"),
                Status::Missing | Status::Running { .. } if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                status => panic!("Transparent Fill analysis timed out with status {status:?}"),
            }
        }
        validate_cache(&project).expect("validate analyzed Transparent Fill cache");

        fs::create_dir_all(&fixture.rendered).expect("create rendered fixture directory");
        let mut renderer = VideoExportRenderer::new(48_000).expect("create export renderer");
        for frame_index in E2E_FIRST_PROJECT_FRAME..E2E_FIRST_PROJECT_FRAME + E2E_PROJECT_FRAMES {
            let position =
                shrimply_math_core::time_from_frame(frame_index, fraction_new(E2E_PROJECT_FPS, 1))
                    .expect("end-to-end project frame position");
            let composited = renderer
                .render(&project, position, 0)
                .unwrap_or_else(|error| panic!("render project frame {frame_index}: {error}"));
            let mut rgba = ffmpeg_next::frame::Video::new(
                ffmpeg_next::format::Pixel::RGBA,
                E2E_WIDTH,
                E2E_HEIGHT,
            );
            renderer
                .copy_to_rgba_frame(composited, &mut rgba)
                .unwrap_or_else(|error| panic!("copy project frame {frame_index}: {error}"));
            let pixels = compact_rgba(&rgba);
            assert_transparent_fill_frame(&pixels, frame_index);
            write_rgba_png(
                &fixture.rendered.join(format!("frame-{frame_index:02}.png")),
                &pixels,
            );
        }
        renderer.shutdown();
        shutdown_history().expect("stop generated project history");
        env::set_current_dir(original).expect("restore end-to-end test working directory");
    }

    #[test]
    fn cache_round_trips_evicted_project_frame_masks() {
        let directory = tempfile::tempdir().expect("create cache round-trip directory");
        const FRAMES: u64 = MEMORY_FRAMES as u64 + 16;
        let modifier_id = Uuid::new_v4();
        let key = format!("{CACHE_VERSION}:{modifier_id}:test");
        let cache = TransparentFillMaskCache::open(&directory.path().join("masks.sqlite"))
            .expect("open cache round-trip database");
        cache.begin_analysis(&key).expect("begin cache round-trip");
        for frame in 0..FRAMES {
            let mut mask = vec![0_u8; WIDTH.div_ceil(8) as usize * HEIGHT as usize];
            let marker = (frame % u64::from(WIDTH)) as u32;
            for row in mask.chunks_exact_mut(WIDTH.div_ceil(8) as usize) {
                row[marker as usize / 8] |= 0x80 >> (marker % 8);
            }
            cache
                .insert_staged(&key, frame as i64, &mask, WIDTH, HEIGHT)
                .expect("insert cache round-trip frame");
        }
        cache
            .complete_analysis(&key, WIDTH, HEIGHT, FRAMES)
            .expect("complete cache round-trip");

        for frame in 0..FRAMES {
            let mask = cache
                .get(&key, frame as i64, WIDTH, HEIGHT)
                .expect("read cache round-trip frame")
                .expect("cache round-trip frame exists");
            let marker = (frame % u64::from(WIDTH)) as usize;
            assert_ne!(mask[marker / 8] & (0x80 >> (marker % 8)), 0);
        }
        cache.abort_analysis(&key);
    }

    #[test]
    fn cached_mask_applies_with_the_cuda_kernel() {
        const FRAME: i64 = 5_897;
        const MARKER: u32 = 17;
        let directory = tempfile::tempdir().expect("create CUDA mask test directory");
        let modifier_id = Uuid::new_v4();
        let key = format!("{CACHE_VERSION}:{modifier_id}:cuda-test");
        let cache = TransparentFillMaskCache::open(&directory.path().join("masks.sqlite"))
            .expect("open CUDA mask test cache");
        cache
            .begin_analysis(&key)
            .expect("begin CUDA mask test analysis");
        let mut packed = vec![0_u8; WIDTH.div_ceil(8) as usize * HEIGHT as usize];
        for row in packed.chunks_exact_mut(WIDTH.div_ceil(8) as usize) {
            row[MARKER as usize / 8] |= 0x80 >> (MARKER % 8);
        }
        cache
            .insert_staged(&key, FRAME, &packed, WIDTH, HEIGHT)
            .expect("insert CUDA mask test frame");
        cache
            .complete_analysis(&key, WIDTH, HEIGHT, 1)
            .expect("complete CUDA mask test analysis");
        let mask = cache
            .get(&key, FRAME, WIDTH, HEIGHT)
            .expect("read CUDA mask test frame")
            .expect("CUDA mask test frame exists");

        let context = CudaContext::new(0).expect("create CUDA mask test context");
        let stream = context.new_stream().expect("create CUDA mask test stream");
        let module = context
            .load_module_from_image(ModifierModule::Matte.image())
            .expect("load CUDA matte module");
        let pixels = WIDTH as usize * HEIGHT as usize;
        let input_pixels = vec![u32::MAX; pixels];
        let mut input = shrimply_gpu_memory::global()
            .allocate_buffer(
                &stream,
                pixels,
                AllocationClass::Transient,
                "CUDA mask input",
            )
            .expect("allocate CUDA mask input");
        input
            .copy_from_host(&stream, &input_pixels)
            .expect("upload CUDA mask input");
        let mut device_mask = shrimply_gpu_memory::global()
            .allocate_buffer(
                &stream,
                mask.len(),
                AllocationClass::Transient,
                "CUDA mask upload",
            )
            .expect("allocate CUDA mask");
        device_mask
            .copy_from_host(&stream, &mask)
            .expect("upload CUDA mask");
        let mut output = shrimply_gpu_memory::global()
            .allocate_buffer::<u32>(
                &stream,
                pixels,
                AllocationClass::Transient,
                "CUDA mask output",
            )
            .expect("allocate CUDA mask output");
        unsafe {
            shrimply_cuda::cuda_launch! {
                kernel: transparent_fill_apply_mask,
                stream: &stream,
                module: &module,
                config: shrimply_cuda::LaunchConfig::for_num_elems(pixels as u32),
                args: [
                    input.cu_deviceptr() as usize as *const u32,
                    device_mask.cu_deviceptr() as usize as *const u8,
                    slice_mut(&mut output),
                    shrimply_render_core::TransparentFillMaskParams {
                        width: WIDTH,
                        height: HEIGHT,
                        stride: WIDTH.div_ceil(8),
                    }
                ]
            }
        }
        .expect("launch Transparent Fill CUDA kernel");
        let output = output
            .to_host_vec(&stream)
            .expect("download CUDA mask output");
        let row = HEIGHT as usize / 2;
        assert_eq!(output[row * WIDTH as usize + MARKER as usize], 0);
        assert_eq!(output[row * WIDTH as usize + MARKER as usize + 1], u32::MAX);
        cache.abort_analysis(&key);
    }

    #[test]
    #[ignore = "requires enough free GPU memory for an independent preview compositor"]
    fn preview_compositor_applies_each_out_of_order_project_frame_mask() {
        let directory = tempfile::tempdir().expect("create preview mask test directory");
        let original = env::current_dir().expect("read preview mask test working directory");
        env::set_current_dir(directory.path()).expect("isolate preview mask test cache database");
        let mut project: Project = serde_json::from_str("{}").expect("create default project");
        project.fps = fraction_new(30, 1);
        project.canvas_size = CanvasSize {
            width: WIDTH,
            height: HEIGHT,
        };
        let start = Time::from_fraction(393, 2);
        let end = Time::from_seconds(197);
        let mut item = VideoItem::background_item(project.canvas_size, start, end);
        item.source_duration = end.saturating_sub(start);
        let fill = TransparentFillModifier {
            points: vec![
                shrimply_video_modifiers::transparent_fill::TransparentFillPoint {
                    id: Uuid::new_v4(),
                    position: TimelineValue::new_const(Vec2::splat(0.5)),
                },
            ],
            tolerance: TimelineValue::new_const(0.1),
            maximum_gap: 0,
            analysis_generation: 1,
        };
        let modifier = VisualModifier::new(ModifierEffect::Raster(Box::new(
            RasterModifierEffect::TransparentFill(fill.clone()),
        )));
        let modifier_id = modifier.id;
        item.modifiers
            .push(VisualModifier::new(ModifierEffect::Rasterize(
                Default::default(),
            )));
        item.modifiers.push(modifier);
        project.video_tracks.push(VideoTrack {
            items: vec![item],
            ..Default::default()
        });
        let item = &project.video_tracks[0].items[0];
        let key = cache_key(&project, item, modifier_id, 1, &fill);
        let cache = TransparentFillMaskCache::shared();
        cache
            .begin_analysis(&key)
            .expect("begin preview mask test analysis");
        insert_marker_masks(&cache, &key);

        let (commands, events) = spawn_worker(project);
        for frame in [5_897, 5_895, 5_899, 5_896, 5_902, 5_898, 5_909, 5_900] {
            let position = shrimply_math_core::time_from_frame(frame, fraction_new(30, 1))
                .expect("project frame position")
                .saturating_add(Time::from_fraction(1, 180));
            commands
                .send(VideoCommand::Render {
                    position,
                    accuracy: CompositeAccuracy::FULLY_ACCURATE,
                })
                .expect("request preview mask frame");
            loop {
                match events
                    .recv_timeout(Duration::from_secs(10))
                    .expect("receive preview mask frame")
                {
                    VideoEvent::Frame {
                        frame: output,
                        position: rendered_position,
                        settled,
                        ..
                    } if rendered_position == position && settled => {
                        assert_marker(output, frame);
                        break;
                    }
                    VideoEvent::Loading {
                        position: loading, ..
                    } if loading == position => commands
                        .send(VideoCommand::Render {
                            position,
                            accuracy: CompositeAccuracy::FULLY_ACCURATE,
                        })
                        .expect("retry loading preview mask frame"),
                    VideoEvent::Error(error) => panic!("preview mask test failed: {error}"),
                    _ => {}
                }
            }
        }
        commands
            .send(VideoCommand::Stop)
            .expect("stop preview mask worker");
        drop(commands);
        while events.recv().is_ok() {}
        cache.abort_analysis(&key);
        env::set_current_dir(original).expect("restore preview mask test working directory");
    }

    #[test]
    #[ignore = "requires enough free GPU memory for an independent preview compositor"]
    fn preview_uses_the_mask_for_each_project_frame() {
        let directory = tempfile::tempdir().expect("create transparent fill test directory");
        let original = env::current_dir().expect("read playback test working directory");
        env::set_current_dir(directory.path()).expect("isolate playback test cache database");
        let video = directory.path().join("24fps.mp4");
        let status = Command::new("ffmpeg")
            .args([
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=white:s=96x64:r=24:d=1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(&video)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "ffmpeg failed to generate the test video");

        let mut project: Project = serde_json::from_str("{}").expect("create default project");
        project.fps = fraction_new(30, 1);
        project.canvas_size = CanvasSize {
            width: WIDTH,
            height: HEIGHT,
        };
        let start = Time::from_fraction(393, 2);
        let end = Time::from_seconds(197);
        let mut item = VideoItem::background_item(project.canvas_size, start, end);
        item.content = VideoItemContent::Media;
        item.file = Asset::new(video);
        item.source_width = WIDTH;
        item.source_height = HEIGHT;
        item.source_duration = Time::from_seconds(1);
        item.playback_fps = fraction_new(24, 1);

        let fill = TransparentFillModifier {
            points: vec![
                shrimply_video_modifiers::transparent_fill::TransparentFillPoint {
                    id: Uuid::new_v4(),
                    position: TimelineValue::new_const(Vec2::splat(0.5)),
                },
            ],
            tolerance: TimelineValue::new_const(0.1),
            maximum_gap: 0,
            analysis_generation: 1,
        };
        let modifier = VisualModifier::new(ModifierEffect::Raster(Box::new(
            RasterModifierEffect::TransparentFill(fill.clone()),
        )));
        let modifier_id = modifier.id;
        item.modifiers.push(modifier);
        let track = VideoTrack {
            items: vec![item],
            ..Default::default()
        };
        project.video_tracks.push(track);

        let item = &project.video_tracks[0].items[0];
        let key = cache_key(&project, item, modifier_id, 0, &fill);
        let cache = TransparentFillMaskCache::shared();
        cache
            .begin_analysis(&key)
            .expect("begin test mask analysis");
        insert_marker_masks(&cache, &key);

        let (commands, events) = spawn_worker(project);
        let order = [
            5_897, 5_895, 5_899, 5_896, 5_902, 5_898, 5_909, 5_900, 5_908, 5_901, 5_907, 5_903,
            5_906, 5_904, 5_905,
        ];
        for accuracy in [
            CompositeAccuracy::TIME_ACCURATE,
            CompositeAccuracy::FULLY_ACCURATE,
            CompositeAccuracy::CONTINUOUS_TIME_ACCURATE,
        ] {
            for frame in order {
                let position = shrimply_math_core::time_from_frame(frame, fraction_new(30, 1))
                    .expect("project frame position")
                    .saturating_add(Time::from_fraction(1, 180));
                commands
                    .send(VideoCommand::Render { position, accuracy })
                    .expect("request preview frame");
                loop {
                    match events
                        .recv_timeout(Duration::from_secs(10))
                        .expect("receive preview frame")
                    {
                        VideoEvent::Frame {
                            frame: output,
                            position: rendered_position,
                            settled,
                            ..
                        } if rendered_position == position && settled => {
                            assert_marker(output, frame);
                            break;
                        }
                        VideoEvent::Loading {
                            position: loading, ..
                        } if loading == position => {
                            commands
                                .send(VideoCommand::Render { position, accuracy })
                                .expect("retry loading preview frame");
                        }
                        VideoEvent::Error(error) => panic!("preview failed: {error}"),
                        _ => {}
                    }
                }
            }
        }
        commands
            .send(VideoCommand::Stop)
            .expect("stop preview worker");
        drop(commands);
        while events.recv().is_ok() {}
        cache.abort_analysis(&key);
        env::set_current_dir(original).expect("restore playback test working directory");
    }

    struct EndToEndFixture {
        directory: PathBuf,
        video: PathBuf,
        project_path: PathBuf,
        rendered: PathBuf,
        address: ItemAddress,
        modifier_id: Uuid,
    }

    fn transparent_fill_fixture() -> EndToEndFixture {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("target/transparent-fill-e2e");
        fs::create_dir_all(&directory).expect("create Transparent Fill fixture directory");
        let video = directory.join("moving-square-24fps.mp4");
        let filter = format!(
            "[0:v][1:v]overlay=x='mod({E2E_SQUARE_STEP}*n,{E2E_SQUARE_RANGE})':y=24:eval=frame:shortest=1"
        );
        let status = Command::new("ffmpeg")
            .args([
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=white:s=64x64:r=24:d=4",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=16x16:r=24:d=4",
                "-filter_complex",
                &filter,
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(&video)
            .status()
            .expect("run FFmpeg for Transparent Fill fixture");
        assert!(
            status.success(),
            "FFmpeg failed to generate moving square video"
        );

        let mut project: Project = serde_json::from_str("{}").expect("create fixture project");
        project.name = "Transparent Fill end-to-end".to_string();
        project.fps = fraction_new(E2E_PROJECT_FPS, 1);
        project.canvas_size = CanvasSize {
            width: E2E_WIDTH,
            height: E2E_HEIGHT,
        };
        let start = Time::from_fraction(393, 2);
        let end = Time::from_fraction(4_938, 25);
        project.cursor_position = Some(start);
        let mut item = VideoItem::background_item(project.canvas_size, start, end);
        item.content = VideoItemContent::Media;
        item.file = Asset::new(video.clone());
        item.source_width = E2E_WIDTH;
        item.source_height = E2E_HEIGHT;
        item.source_duration = Time::from_seconds(4);
        item.time_offset =
            Time::from_fraction(E2E_SOURCE_OFFSET_NUMERATOR, E2E_SOURCE_OFFSET_DENOMINATOR);
        item.playback_fps = fraction_new(E2E_SOURCE_FPS, 1);

        let mut color_correction = ColorCorrectionModifier::default();
        color_correction.brightness = TimelineValue::new_const(-0.05);
        item.modifiers
            .push(VisualModifier::new(ModifierEffect::Raster(Box::new(
                RasterModifierEffect::ColorCorrection(Box::new(color_correction)),
            ))));
        let fill = TransparentFillModifier {
            points: vec![
                shrimply_video_modifiers::transparent_fill::TransparentFillPoint {
                    id: Uuid::new_v4(),
                    position: TimelineValue::new_const(Vec2::splat(0.05)),
                },
            ],
            tolerance: TimelineValue::new_const(0.12),
            maximum_gap: 2,
            analysis_generation: 1,
        };
        let fill_modifier = VisualModifier::new(ModifierEffect::Raster(Box::new(
            RasterModifierEffect::TransparentFill(fill),
        )));
        let modifier_id = fill_modifier.id;
        item.modifiers.push(fill_modifier);
        let item_id = item.id;
        let track = VideoTrack {
            items: vec![item],
            ..Default::default()
        };
        let track_id = track.id;
        project.video_tracks.push(track);

        let project_path = directory.join("transparent-fill-e2e.shrimp");
        create_project_file(&project_path, &project)
            .expect("write Transparent Fill fixture project");
        EndToEndFixture {
            rendered: directory.join("rendered-alpha"),
            directory,
            video,
            project_path,
            address: ItemAddress::Video {
                sequence_path: Vec::new(),
                track_id,
                item_id,
            },
            modifier_id,
        }
    }

    fn compact_rgba(frame: &ffmpeg_next::frame::Video) -> Vec<u8> {
        let row_bytes = E2E_WIDTH as usize * 4;
        frame
            .data(0)
            .chunks_exact(frame.stride(0))
            .take(E2E_HEIGHT as usize)
            .flat_map(|row| row[..row_bytes].iter().copied())
            .collect()
    }

    fn assert_transparent_fill_frame(pixels: &[u8], project_frame: u64) {
        let project_position =
            shrimply_math_core::time_from_frame(project_frame, fraction_new(E2E_PROJECT_FPS, 1))
                .expect("end-to-end assertion project position");
        let source_position =
            Time::from_fraction(E2E_SOURCE_OFFSET_NUMERATOR, E2E_SOURCE_OFFSET_DENOMINATOR)
                .saturating_add(project_position.saturating_sub(Time::from_fraction(393, 2)));
        let source_frame =
            shrimply_math_core::frame_index(source_position, fraction_new(E2E_SOURCE_FPS, 1))
                .and_then(|frame| u64::try_from(frame).ok())
                .expect("end-to-end assertion source frame");
        let square_x = ((source_frame as u32 + 1) * E2E_SQUARE_STEP) % E2E_SQUARE_RANGE;
        let alpha = |x: u32, y: u32| pixels[((y * E2E_WIDTH + x) * 4 + 3) as usize];
        assert_eq!(
            alpha(2, 2),
            0,
            "project frame {project_frame} kept the white background opaque"
        );
        for x in [square_x + 3, square_x + E2E_SQUARE_SIZE - 4] {
            assert_eq!(
                alpha(x, 24 + E2E_SQUARE_SIZE / 2),
                u8::MAX,
                "project frame {project_frame} used a mask from a different source frame"
            );
        }
    }

    fn write_rgba_png(path: &Path, pixels: &[u8]) {
        let output = fs::File::create(path).expect("create rendered Transparent Fill PNG");
        let mut encoder = png::Encoder::new(BufWriter::new(output), E2E_WIDTH, E2E_HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .expect("write rendered Transparent Fill PNG header")
            .write_image_data(pixels)
            .expect("write rendered Transparent Fill PNG pixels");
    }

    fn marker(frame: u64) -> u32 {
        ((frame - FIRST_FRAME) * 5 % u64::from(WIDTH - 1)) as u32
    }

    fn insert_marker_masks(cache: &TransparentFillMaskCache, key: &str) {
        for frame in FIRST_FRAME..FIRST_FRAME + FRAME_COUNT {
            let marker = marker(frame);
            let mut mask = vec![0_u8; WIDTH.div_ceil(8) as usize * HEIGHT as usize];
            for row in mask.chunks_exact_mut(WIDTH.div_ceil(8) as usize) {
                row[marker as usize / 8] |= 0x80 >> (marker % 8);
            }
            cache
                .insert_staged(key, frame as i64, &mask, WIDTH, HEIGHT)
                .expect("insert test frame mask");
        }
        cache
            .complete_analysis(key, WIDTH, HEIGHT, FRAME_COUNT)
            .expect("complete test mask analysis");
    }

    fn assert_marker(output: crate::gpu::CompositedVideoFrame, frame: u64) {
        output
            .buffer
            .context()
            .synchronize()
            .expect("synchronize preview frame");
        let stream = output.buffer.context().default_stream();
        let pixels = output
            .buffer
            .to_host_vec(&stream)
            .expect("download preview frame");
        let expected = marker(frame) as usize;
        let row = HEIGHT as usize / 2;
        let transparent: Vec<_> = pixels[row * WIDTH as usize..(row + 1) * WIDTH as usize]
            .iter()
            .enumerate()
            .filter_map(|(x, pixel)| (*pixel == 0).then_some(x))
            .collect();
        assert_eq!(
            transparent,
            vec![expected],
            "project frame {frame} did not use only its cached mask"
        );
    }
}
