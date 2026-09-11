use hashbrown::HashMap;
use std::fs;
use std::sync::mpsc;
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use cached::{Cached, stores::LruCache};
use rusqlite::{Connection, params};
use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_path_core::project_cache_directory;
use shrimply_project_document::project::{
    AudioItem, AudioSource, Project, Time, audio_source_time_at,
};

const CACHE_VERSION: i64 = 16;
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
const CHUNK_NANOS: i128 = 3_000_000_000;
const MEMORY_CACHE_CHUNKS: usize = 4_096;
const MAX_WAVEFORM_WORKERS: usize = 4;

pub type WaveformMap = HashMap<uuid::Uuid, Option<Waveform>>;
static MEMORY_CACHE: LazyLock<Mutex<LruCache<String, Vec<u8>>>> = LazyLock::new(|| {
    Mutex::new(
        LruCache::builder()
            .max_size(MEMORY_CACHE_CHUNKS)
            .build()
            .expect("valid waveform memory cache size"),
    )
});

#[derive(Clone, Debug)]
pub struct Waveform {
    pub peaks: Vec<u8>,
    loaded: Vec<bool>,
    pub max_peak: u8,
    peak_pyramid: Option<Vec<Vec<u8>>>,
}

#[derive(Clone)]
pub enum WaveformUpdate {
    Prepare(Waveform),
    Replace(Option<Waveform>),
    Bins(Vec<(usize, u8)>),
    Finish,
}

pub fn apply_update(waveforms: &mut WaveformMap, key: uuid::Uuid, update: WaveformUpdate) {
    match update {
        WaveformUpdate::Prepare(mut waveform) => {
            if let Some(Some(previous)) = waveforms.get(&key) {
                let retained = waveform.peaks.len().min(previous.peaks.len());
                waveform.peaks[..retained].copy_from_slice(&previous.peaks[..retained]);
                waveform.loaded[..retained].copy_from_slice(&previous.loaded[..retained]);
                waveform.max_peak = waveform.peaks.iter().copied().max().unwrap_or(0);
            }
            waveforms.insert(key, Some(waveform));
        }
        WaveformUpdate::Replace(waveform) => {
            waveforms.insert(key, waveform);
        }
        WaveformUpdate::Bins(bins) => {
            if let Some(Some(waveform)) = waveforms.get_mut(&key) {
                waveform.set_bins(&bins);
            }
        }
        WaveformUpdate::Finish => {
            if let Some(Some(waveform)) = waveforms.get_mut(&key) {
                waveform.finish();
            }
        }
    }
}

pub fn load_project_waveforms(
    project: &Project,
    waveform_chunks_per_second: u32,
    on_waveform: impl FnMut(uuid::Uuid, WaveformUpdate),
) {
    load_project_waveforms_cancellable(project, waveform_chunks_per_second, || false, on_waveform);
}

pub fn load_project_waveforms_cancellable(
    project: &Project,
    waveform_chunks_per_second: u32,
    is_cancelled: impl Fn() -> bool + Sync,
    mut on_waveform: impl FnMut(uuid::Uuid, WaveformUpdate),
) {
    let items = project
        .audio_tracks
        .iter()
        .flat_map(|track| &track.items)
        .chain(
            project
                .folded_sequences
                .iter()
                .flat_map(|sequence| &sequence.audio_tracks)
                .flat_map(|track| &track.items),
        )
        .filter(|item| {
            matches!(&item.source, AudioSource::Generator(_))
                || (matches!(&item.source, AudioSource::Media | AudioSource::Tts(_))
                    && !item.file.as_os_str().is_empty())
        })
        .collect::<Vec<_>>();
    let item_count = items.len();
    tracing::info!("Loading waveforms for {item_count} audio item(s)");

    let mut groups: HashMap<(Asset, u32, Option<uuid::Uuid>), Vec<AudioItem>> = HashMap::new();
    for item in items.into_iter().cloned() {
        groups
            .entry((
                item.file.clone(),
                item.track_id,
                matches!(&item.source, AudioSource::Generator(_)).then_some(item.id),
            ))
            .or_default()
            .push(item);
    }
    if groups.is_empty() {
        return;
    }

    let workers = thread::available_parallelism()
        .map_or(1, usize::from)
        .min(MAX_WAVEFORM_WORKERS)
        .min(groups.len());
    let mut assignments = vec![Vec::new(); workers];
    for (index, group) in groups.into_values().enumerate() {
        assignments[index % workers].push(group);
    }

    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        for assignment in assignments {
            let tx = tx.clone();
            let is_cancelled = &is_cancelled;
            scope.spawn(move || {
                let cache = WaveformCache::open().inspect_err(|error| {
                    tracing::warn!("Waveform cache disabled: {error}");
                });
                for group in assignment {
                    if is_cancelled() {
                        break;
                    }
                    for item in group {
                        if is_cancelled() {
                            break;
                        }
                        load_item_waveform(
                            &cache,
                            &item,
                            waveform_chunks_per_second,
                            is_cancelled,
                            &mut |key, waveform| {
                                let _ = tx.send((key, waveform));
                            },
                        );
                    }
                }
            });
        }
        drop(tx);
        for (key, waveform) in rx {
            on_waveform(key, waveform);
        }
    });
}

fn load_item_waveform(
    cache: &Result<WaveformCache, String>,
    item: &AudioItem,
    waveform_chunks_per_second: u32,
    is_cancelled: &impl Fn() -> bool,
    on_waveform: &mut impl FnMut(uuid::Uuid, WaveformUpdate),
) {
    if is_cancelled() {
        return;
    }
    let key = audio_key(item);
    on_waveform(
        key,
        WaveformUpdate::Prepare(Waveform::pending(
            item.end.saturating_sub(item.start),
            waveform_chunks_per_second,
        )),
    );

    if matches!(&item.source, AudioSource::Generator(_)) {
        load_generator_waveform(
            item,
            waveform_chunks_per_second,
            key,
            is_cancelled,
            on_waveform,
        );
        return;
    }

    let source_item = item
        .source_builder()
        .gain(item.gain.as_ref().clone())
        .modifiers(item.modifiers.clone())
        .build();
    let source_bins = source_bin_map(item, waveform_chunks_per_second);
    let mut source_chunks = source_bins.keys().copied().collect::<Vec<_>>();
    source_chunks.sort_unstable();
    let mut renderer = None;
    for chunk_index in source_chunks {
        if is_cancelled() {
            return;
        }
        let start = chunk_start(chunk_index);
        let duration = chunk_duration(start, item.source_duration);
        if duration == Time::ZERO {
            continue;
        }
        let result = load_chunk(
            cache,
            &item.file,
            item.track_id,
            if has_enabled_modifiers(item) {
                "modified-source"
            } else {
                "source"
            },
            source_cache_key(item, waveform_chunks_per_second, chunk_index),
            chunk_index,
            || {
                generate_chunk(
                    &mut renderer,
                    &source_item,
                    start,
                    duration,
                    waveform_chunks_per_second,
                )
            },
        );
        let Ok(peaks) = result else {
            tracing::warn!(
                "Could not load source waveform chunk {chunk_index} for {} audio {}",
                item.file.display(),
                item.track_id
            );
            on_waveform(key, WaveformUpdate::Replace(None));
            return;
        };
        if is_cancelled() {
            return;
        }
        on_waveform(
            key,
            WaveformUpdate::Bins(
                source_bins[&chunk_index]
                    .iter()
                    .map(|&(bin, source_bin)| (bin, peaks.get(source_bin).copied().unwrap_or(0)))
                    .collect(),
            ),
        );
    }
    if !is_cancelled() {
        on_waveform(key, WaveformUpdate::Finish);
    }
}

fn load_generator_waveform(
    item: &AudioItem,
    waveform_chunks_per_second: u32,
    key: uuid::Uuid,
    is_cancelled: &impl Fn() -> bool,
    on_waveform: &mut impl FnMut(uuid::Uuid, WaveformUpdate),
) {
    let duration = item.end.saturating_sub(item.start);
    let mut renderer = None;
    let mut start = Time::ZERO;
    while start < duration {
        if is_cancelled() {
            return;
        }
        let chunk_duration = duration
            .saturating_sub(start)
            .min(Time::from_nanos_i128(CHUNK_NANOS));
        let peaks = match generate_chunk(
            &mut renderer,
            item,
            start,
            chunk_duration,
            waveform_chunks_per_second,
        ) {
            Ok(peaks) => peaks,
            Err(error) => {
                tracing::warn!("Could not generate audio-generator waveform: {error}");
                on_waveform(key, WaveformUpdate::Replace(None));
                return;
            }
        };
        let first_bin = bin_at(start, waveform_chunks_per_second);
        if is_cancelled() {
            return;
        }
        on_waveform(
            key,
            WaveformUpdate::Bins(
                peaks
                    .into_iter()
                    .enumerate()
                    .map(|(offset, peak)| (first_bin + offset, peak))
                    .collect(),
            ),
        );
        start = start.saturating_add(chunk_duration);
    }
    if !is_cancelled() {
        on_waveform(key, WaveformUpdate::Finish);
    }
}

fn load_chunk(
    cache: &Result<WaveformCache, String>,
    file: &Asset,
    track_id: u32,
    namespace: &str,
    cache_key: String,
    chunk_index: usize,
    generate: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    match cache {
        Ok(cache) => {
            cache.load_or_generate(file, track_id, namespace, cache_key, chunk_index, generate)
        }
        Err(_) => generate(),
    }
}

fn source_bin_map(
    item: &AudioItem,
    waveform_chunks_per_second: u32,
) -> HashMap<usize, Vec<(usize, usize)>> {
    let bins = waveform_bin_count(
        item.end.saturating_sub(item.start),
        waveform_chunks_per_second,
    );
    let mut chunks: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
    for bin in 0..bins {
        let Some(source_time) = source_time_for_bin(item, bin, waveform_chunks_per_second) else {
            continue;
        };
        let chunk = chunk_index_at(source_time);
        let source_bin = bin_at(
            source_time.saturating_sub(chunk_start(chunk)),
            waveform_chunks_per_second,
        );
        chunks.entry(chunk).or_default().push((bin, source_bin));
    }
    chunks
}

fn source_time_for_bin(
    item: &AudioItem,
    bin: usize,
    waveform_chunks_per_second: u32,
) -> Option<Time> {
    if waveform_chunks_per_second == 0 {
        return None;
    }
    let local = Time::from_seconds_f64((bin as f64 + 0.5) / f64::from(waveform_chunks_per_second));
    audio_source_time_at(item, item.start.saturating_add(local))
}

fn has_enabled_modifiers(item: &AudioItem) -> bool {
    item.modifiers.iter().any(|modifier| modifier.enabled)
}

fn generate_chunk(
    renderer: &mut Option<super::streaming::OfflineAudioRenderer>,
    item: &AudioItem,
    start: Time,
    duration: Time,
    waveform_chunks_per_second: u32,
) -> Result<Vec<u8>, String> {
    if renderer.is_none() {
        *renderer = Some(super::streaming::OfflineAudioRenderer::new(
            item,
            SAMPLE_RATE,
        )?);
    }
    let samples = renderer
        .as_mut()
        .expect("waveform renderer initialized")
        .render(item, start, duration)?;
    Ok(absolute_peaks_from_stereo_samples(
        &samples,
        SAMPLE_RATE,
        waveform_chunks_per_second,
    ))
}

pub fn audio_key(item: &AudioItem) -> uuid::Uuid {
    item.id
}

pub fn from_stereo_samples(
    samples: &[f32],
    sample_rate: u32,
    waveform_chunks_per_second: u32,
) -> Waveform {
    Waveform::ready(absolute_peaks_from_stereo_samples(
        samples,
        sample_rate,
        waveform_chunks_per_second,
    ))
}

struct WaveformCache {
    conn: Connection,
}

impl WaveformCache {
    fn open() -> Result<Self, String> {
        let directory = project_cache_directory();
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let conn = Connection::open(directory.join("waveforms.sqlite"))
            .map_err(|error| error.to_string())?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS modifier_waveforms (
                cache_key TEXT PRIMARY KEY,
                namespace TEXT NOT NULL,
                file_path TEXT NOT NULL,
                track_id INTEGER NOT NULL,
                file_size INTEGER NOT NULL,
                modified_ns INTEGER NOT NULL,
                sample_rate INTEGER NOT NULL,
                bins INTEGER NOT NULL,
                peaks BLOB NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .map_err(|error| error.to_string())?;
        Ok(Self { conn })
    }

    fn load_or_generate(
        &self,
        file_path: &Asset,
        track_id: u32,
        namespace: &str,
        cache_key: String,
        chunk_index: usize,
        generate: impl FnOnce() -> Result<Vec<u8>, String>,
    ) -> Result<Vec<u8>, String> {
        let file = file_path.snapshot()?;
        let cache_key = format!("{cache_key}:{}", file.cache_key());
        let memory_key = cache_key.clone();
        if let Some(peaks) = MEMORY_CACHE
            .lock()
            .ok()
            .and_then(|mut cache| cache.cache_get(&memory_key).cloned())
        {
            return Ok(peaks);
        }

        let cached = self
            .conn
            .query_row(
                "SELECT peaks FROM modifier_waveforms
                 WHERE cache_key = ?1 AND file_size = ?2 AND modified_ns = ?3",
                params![cache_key, snapshot_len(&file), snapshot_modified_ns(&file)],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .ok();
        if let Some(peaks) = cached {
            if let Ok(mut cache) = MEMORY_CACHE.lock() {
                cache.cache_set(memory_key, peaks.clone());
            }
            return Ok(peaks);
        }

        tracing::info!(
            "Waveform {namespace} chunk cache miss for {} audio {} chunk {chunk_index}; decoding",
            file_path.display(),
            track_id
        );
        let peaks = generate()?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO modifier_waveforms
                 (cache_key, namespace, file_path, track_id, file_size, modified_ns, sample_rate,
                  bins, peaks)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    cache_key,
                    namespace,
                    file_path.to_string_lossy(),
                    track_id,
                    snapshot_len(&file),
                    snapshot_modified_ns(&file),
                    SAMPLE_RATE,
                    peaks.len() as i64,
                    &peaks,
                ],
            )
            .map_err(|error| error.to_string())?;
        if let Ok(mut cache) = MEMORY_CACHE.lock() {
            cache.cache_set(memory_key, peaks.clone());
        }
        Ok(peaks)
    }
}

fn source_cache_key(
    item: &AudioItem,
    waveform_chunks_per_second: u32,
    chunk_index: usize,
) -> String {
    serde_json::to_string(&(
        CACHE_VERSION,
        &item.file,
        item.track_id,
        &item.gain,
        &item.modifiers,
        waveform_chunks_per_second,
        chunk_index,
    ))
    .expect("waveform source cache key should serialize")
}

fn snapshot_len(snapshot: &AssetSnapshot) -> i64 {
    snapshot.len().min(i64::MAX as u64) as i64
}

fn snapshot_modified_ns(snapshot: &AssetSnapshot) -> i64 {
    snapshot.modified_ns().clamp(0, i128::from(i64::MAX)) as i64
}

fn chunk_index_at(time: Time) -> usize {
    (time.as_nanos_i128().max(0) / CHUNK_NANOS) as usize
}

fn chunk_start(index: usize) -> Time {
    Time::from_nanos_i128((index as i128).saturating_mul(CHUNK_NANOS))
}

fn chunk_duration(start: Time, end: Time) -> Time {
    end.saturating_sub(start)
        .min(Time::from_nanos_i128(CHUNK_NANOS))
}

fn bin_at(time: Time, waveform_chunks_per_second: u32) -> usize {
    (time.as_secs_f64() * f64::from(waveform_chunks_per_second)).floor() as usize
}

fn waveform_bin_count(duration: Time, waveform_chunks_per_second: u32) -> usize {
    (duration.as_secs_f64() * f64::from(waveform_chunks_per_second))
        .ceil()
        .max(1.0) as usize
}

fn absolute_peaks_from_stereo_samples(
    samples: &[f32],
    sample_rate: u32,
    waveform_chunks_per_second: u32,
) -> Vec<u8> {
    let sample_rate = sample_rate.max(1);
    let seconds = samples.len() as f64 / CHANNELS as f64 / sample_rate as f64;
    let bins = (seconds * f64::from(waveform_chunks_per_second.min(sample_rate)))
        .ceil()
        .max(1.0) as usize;
    let mut peaks = vec![0; bins];
    if samples.is_empty() {
        return peaks;
    }
    for (sample_index, sample) in samples.iter().enumerate() {
        let bin = sample_index * bins / samples.len();
        peaks[bin] = peaks[bin].max(peak_to_byte(*sample));
    }
    peaks
}

fn peak_to_byte(sample: f32) -> u8 {
    (sample.abs().clamp(0.0, 1.0) * u8::MAX as f32).round() as u8
}

impl Waveform {
    fn pending(duration: Time, waveform_chunks_per_second: u32) -> Self {
        let bins = waveform_bin_count(duration, waveform_chunks_per_second);
        Self {
            peaks: vec![0; bins],
            loaded: vec![false; bins],
            max_peak: 0,
            peak_pyramid: None,
        }
    }

    fn ready(peaks: Vec<u8>) -> Self {
        let max_peak = peaks.iter().copied().max().unwrap_or(0);
        let loaded = vec![true; peaks.len()];
        let peak_pyramid = Some(waveform_peak_pyramid(&peaks));
        Self {
            peaks,
            loaded,
            max_peak,
            peak_pyramid,
        }
    }

    fn set_bins(&mut self, bins: &[(usize, u8)]) {
        self.peak_pyramid = None;
        for &(bin, peak) in bins {
            let Some(destination) = self.peaks.get_mut(bin) else {
                continue;
            };
            *destination = peak;
            self.loaded[bin] = true;
            self.max_peak = self.max_peak.max(peak);
        }
    }

    fn finish(&mut self) {
        self.loaded.fill(true);
        self.peak_pyramid = Some(waveform_peak_pyramid(&self.peaks));
    }

    pub fn range(&self, left: f64, right: f64) -> Option<f64> {
        if self.peaks.is_empty() {
            return Some(0.0);
        }
        let first = left.floor().max(0.0) as usize;
        let last = right
            .ceil()
            .max(0.0)
            .min(self.peaks.len().saturating_sub(1) as f64) as usize;
        if let Some(peak_pyramid) = &self.peak_pyramid {
            let mut peak = 0_u8;
            let mut first = first.min(self.peaks.len() - 1);
            let end = last.saturating_add(1);
            while first < end {
                let remaining_level = usize::BITS - 1 - (end - first).leading_zeros();
                let alignment_level = if first == 0 {
                    remaining_level
                } else {
                    first.trailing_zeros()
                };
                let level = remaining_level.min(alignment_level) as usize;
                let value = if level == 0 {
                    self.peaks[first]
                } else {
                    peak_pyramid[level - 1][first >> level]
                };
                peak = peak.max(value);
                first += 1 << level;
            }
            return Some(f64::from(peak));
        }
        let mut peak = 0_u8;
        for index in first.min(self.peaks.len() - 1)..=last {
            if !self.loaded[index] {
                return None;
            }
            peak = peak.max(self.peaks[index]);
        }
        Some(f64::from(peak))
    }

    pub fn peak(&self, index: usize) -> Option<u8> {
        self.loaded.get(index).copied()?.then(|| self.peaks[index])
    }

    pub fn has_pending(&self) -> bool {
        self.loaded.contains(&false)
    }
}

fn waveform_peak_pyramid(peaks: &[u8]) -> Vec<Vec<u8>> {
    let mut pyramid = Vec::new();
    let mut previous = peaks;
    while previous.len() > 1 {
        let level = previous
            .chunks(2)
            .map(|pair| pair.iter().copied().max().unwrap_or(0))
            .collect::<Vec<_>>();
        pyramid.push(level);
        previous = pyramid.last().expect("waveform peak pyramid level");
    }
    pyramid
}
