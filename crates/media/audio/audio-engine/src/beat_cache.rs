use std::fs;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use super::beat::{BeatAnalysis, SAMPLE_RATE};
use shrimply_path_core::project_cache_directory;
use shrimply_project_document::project::AudioItem;

const CACHE_VERSION: i64 = 2;

pub(super) struct BeatCache {
    conn: Connection,
}

impl BeatCache {
    pub(super) fn open() -> Result<Self, String> {
        let directory = project_cache_directory();
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let conn =
            Connection::open(directory.join("beats.sqlite")).map_err(|error| error.to_string())?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS beat_analyses (
                cache_key TEXT PRIMARY KEY,
                file_path TEXT NOT NULL,
                track_id INTEGER NOT NULL,
                file_size INTEGER NOT NULL,
                modified_ns INTEGER NOT NULL,
                cache_version INTEGER NOT NULL,
                available INTEGER NOT NULL,
                sample_rate INTEGER,
                period_frames INTEGER,
                bar_phase INTEGER,
                confidence REAL,
                beat_frames BLOB NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .map_err(|error| error.to_string())?;
        Ok(Self { conn })
    }

    pub(super) fn load(&self, item: &AudioItem) -> Result<Option<Option<BeatAnalysis>>, String> {
        let identity = item.file.snapshot()?;
        let row = self
            .conn
            .query_row(
                "SELECT available, sample_rate, period_frames, bar_phase, confidence, beat_frames
                 FROM beat_analyses
                 WHERE cache_key = ?1 AND file_size = ?2 AND modified_ns = ?3
                   AND cache_version = ?4",
                params![
                    cache_key(item, &identity),
                    snapshot_len(&identity),
                    snapshot_modified_ns(&identity),
                    CACHE_VERSION,
                ],
                |row| {
                    Ok((
                        row.get::<_, bool>(0)?,
                        row.get::<_, Option<u32>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<u8>>(3)?,
                        row.get::<_, Option<f32>>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((available, sample_rate, period_frames, bar_phase, confidence, bytes)) = row
        else {
            return Ok(None);
        };
        if !available {
            return Ok(Some(None));
        }
        if bytes.len() % 8 != 0 {
            return Ok(None);
        }
        let beat_frames = bytes
            .chunks_exact(8)
            .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("eight-byte beat frame")))
            .collect();
        Ok(Some(Some(BeatAnalysis {
            sample_rate: sample_rate.unwrap_or(SAMPLE_RATE),
            beat_frames,
            period_frames: period_frames.unwrap_or_default().max(0) as u64,
            bar_phase,
            confidence: confidence.unwrap_or_default(),
        })))
    }

    pub(super) fn store(
        &self,
        item: &AudioItem,
        analysis: Option<&BeatAnalysis>,
    ) -> Result<(), String> {
        let identity = item.file.snapshot()?;
        let beat_frames = analysis
            .map(|analysis| {
                analysis
                    .beat_frames
                    .iter()
                    .flat_map(|frame| frame.to_le_bytes())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.conn
            .execute(
                "INSERT OR REPLACE INTO beat_analyses
                 (cache_key, file_path, track_id, file_size, modified_ns, cache_version,
                  available, sample_rate, period_frames, bar_phase, confidence, beat_frames)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    cache_key(item, &identity),
                    item.file.to_string_lossy(),
                    item.track_id,
                    snapshot_len(&identity),
                    snapshot_modified_ns(&identity),
                    CACHE_VERSION,
                    analysis.is_some(),
                    analysis.map(|analysis| analysis.sample_rate),
                    analysis.map(|analysis| analysis.period_frames.min(i64::MAX as u64) as i64),
                    analysis.and_then(|analysis| analysis.bar_phase),
                    analysis.map(|analysis| analysis.confidence),
                    beat_frames,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn cache_key(item: &AudioItem, snapshot: &shrimply_asset::AssetSnapshot) -> String {
    format!(
        "v{CACHE_VERSION}:{}#{}:{}",
        item.file.display(),
        item.track_id,
        snapshot.cache_key()
    )
}

fn snapshot_len(snapshot: &shrimply_asset::AssetSnapshot) -> i64 {
    snapshot.len().min(i64::MAX as u64) as i64
}

fn snapshot_modified_ns(snapshot: &shrimply_asset::AssetSnapshot) -> i64 {
    snapshot.modified_ns().clamp(0, i128::from(i64::MAX)) as i64
}
