use hashbrown::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::{LazyLock, Mutex};
use std::thread;

use shrimply_asset::Asset;
use uuid::Uuid;

use shrimply_project_document::project::{
    AudioItem, AudioSource, Project, RepeatStrategy, Time, scaled_time_delta, unscaled_time_delta,
};

use super::beat_cache::BeatCache;

pub(super) const SAMPLE_RATE: u32 = 22_050;
const ANALYSIS_CHUNK_SECONDS: i64 = 30;
const MIN_CONFIDENCE: f32 = 0.35;

pub type BeatMap = HashMap<Uuid, BeatState>;
static LOADING: LazyLock<Mutex<HashMap<Uuid, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn project_audio_items(project: &Project) -> impl Iterator<Item = &AudioItem> {
    project
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
            matches!(&item.source, AudioSource::Media | AudioSource::Tts(_))
                && !item.file.as_os_str().is_empty()
        })
}

pub fn begin_loading(project: &shrimply_project_document::project::Project) {
    let Ok(mut loading) = LOADING.lock() else {
        return;
    };
    for item in project_audio_items(project).filter(|item| item.beat_detection) {
        *loading.entry(item.id).or_default() += 1;
    }
}

pub fn is_loading(id: Uuid) -> bool {
    LOADING
        .lock()
        .is_ok_and(|loading| loading.get(&id).is_some_and(|count| *count > 0))
}

fn finish_loading(id: Uuid) {
    let Ok(mut loading) = LOADING.lock() else {
        return;
    };
    let Some(count) = loading.get_mut(&id) else {
        return;
    };
    *count = count.saturating_sub(1);
    if *count == 0 {
        loading.remove(&id);
    }
}

#[derive(Clone, Debug)]
pub enum BeatState {
    Loading,
    Ready(BeatAnalysis),
    LowConfidence,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct BeatAnalysis {
    pub sample_rate: u32,
    pub beat_frames: Vec<u64>,
    pub period_frames: u64,
    pub bar_phase: Option<u8>,
    pub confidence: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerKind {
    Beat,
    Bar,
}

#[derive(Clone, Copy, Debug)]
pub struct Marker {
    pub time: Time,
    pub kind: MarkerKind,
}

#[derive(Clone)]
pub enum BeatUpdate {
    Replace(BeatState),
}

pub fn apply_update(beats: &mut BeatMap, id: Uuid, update: BeatUpdate) {
    match update {
        BeatUpdate::Replace(state) => {
            beats.insert(id, state);
        }
    }
}

pub fn retain_enabled(beats: &mut BeatMap, project: &shrimply_project_document::project::Project) {
    beats.retain(|id, _| {
        project_audio_items(project).any(|item| item.id == *id && item.beat_detection)
    });
}

pub fn load_project_beats(
    project: &shrimply_project_document::project::Project,
    on_beat: impl FnMut(Uuid, BeatUpdate),
) {
    load_project_beats_cancellable(project, || false, on_beat);
}

pub fn load_project_beats_cancellable(
    project: &shrimply_project_document::project::Project,
    is_cancelled: impl Fn() -> bool + Sync,
    mut on_beat: impl FnMut(Uuid, BeatUpdate),
) {
    let mut groups: HashMap<(Asset, u32), Vec<AudioItem>> = HashMap::new();
    for item in project_audio_items(project).filter(|item| item.beat_detection) {
        on_beat(item.id, BeatUpdate::Replace(BeatState::Loading));
        groups
            .entry((item.file.clone(), item.track_id))
            .or_default()
            .push(item.clone());
    }
    if groups.is_empty() {
        return;
    }
    let mut pending = groups
        .values()
        .flatten()
        .map(|item| item.id)
        .collect::<HashSet<_>>();

    let workers = thread::available_parallelism()
        .map_or(1, usize::from)
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
                let cache = BeatCache::open().inspect_err(|error| {
                    tracing::warn!("Beat cache disabled: {error}");
                });
                for group in assignment {
                    if is_cancelled() {
                        break;
                    }
                    let representative = &group[0];
                    let state = match load_or_analyze(&cache, representative, is_cancelled) {
                        Ok(Some(analysis)) if analysis.confidence >= MIN_CONFIDENCE => {
                            BeatState::Ready(analysis)
                        }
                        Ok(Some(_)) | Ok(None) => BeatState::LowConfidence,
                        Err(error) => BeatState::Failed(error),
                    };
                    if is_cancelled() {
                        break;
                    }
                    for item in group {
                        let _ = tx.send((item.id, state.clone()));
                    }
                }
            });
        }
        drop(tx);
        for (id, state) in rx {
            on_beat(id, BeatUpdate::Replace(state));
            finish_loading(id);
            pending.remove(&id);
        }
    });
    for id in pending {
        finish_loading(id);
    }
}

pub fn timeline_markers(
    item: &AudioItem,
    analysis: &BeatAnalysis,
    visible_start: Time,
    visible_end: Time,
) -> Vec<Marker> {
    let timeline_start = item.start.max(visible_start);
    let timeline_end = item.end.min(visible_end);
    if analysis.sample_rate == 0 || timeline_end <= timeline_start {
        return Vec::new();
    }
    let source_duration = item.source_duration.as_nanos_i128();
    if source_duration <= 0 {
        return Vec::new();
    }
    let raw_anchor = item.time_offset.as_nanos_i128();
    let raw_start = item
        .time_offset
        .saturating_add(scaled_time_delta(
            timeline_start.signed_sub(item.start),
            item.playback_speed,
        ))
        .as_nanos_i128();
    let raw_end = item
        .time_offset
        .saturating_add(scaled_time_delta(
            timeline_end.signed_sub(item.start),
            item.playback_speed,
        ))
        .as_nanos_i128();
    let range = MarkerRange {
        timeline_start,
        timeline_end,
        raw_anchor,
        raw_min: raw_start.min(raw_end),
        raw_max: raw_start.max(raw_end),
    };
    let mut markers = Vec::new();

    let (first_beat, last_beat) = if matches!(
        item.repeat_strategy,
        RepeatStrategy::Empty | RepeatStrategy::Hold
    ) {
        let first = analysis
            .beat_frames
            .partition_point(|frame| frame_nanos(*frame, analysis.sample_rate) < range.raw_min);
        let last = analysis
            .beat_frames
            .partition_point(|frame| frame_nanos(*frame, analysis.sample_rate) <= range.raw_max);
        (first, last)
    } else {
        (0, analysis.beat_frames.len())
    };
    for beat_index in first_beat..last_beat {
        let frame = analysis.beat_frames[beat_index];
        let source = Time::from_fraction(
            frame.min(i64::MAX as u64) as i64,
            i64::from(analysis.sample_rate),
        );
        let source_nanos = source.as_nanos_i128();
        let kind = if analysis.bar_phase == Some((beat_index % 4) as u8) {
            MarkerKind::Bar
        } else {
            MarkerKind::Beat
        };
        match item.repeat_strategy {
            RepeatStrategy::Empty | RepeatStrategy::Hold => {
                push_occurrence(&mut markers, item, source_nanos, &range, kind);
            }
            RepeatStrategy::Repeat => push_periodic_occurrences(
                &mut markers,
                item,
                source_nanos,
                source_duration,
                &range,
                kind,
            ),
            RepeatStrategy::PingPong => {
                let cycle = source_duration.saturating_mul(2);
                push_periodic_occurrences(&mut markers, item, source_nanos, cycle, &range, kind);
                let reflected = cycle.saturating_sub(source_nanos);
                if reflected != source_nanos && reflected != cycle {
                    push_periodic_occurrences(&mut markers, item, reflected, cycle, &range, kind);
                }
            }
        }
    }
    markers.sort_by_key(|marker| marker.time);
    markers
}

struct MarkerRange {
    timeline_start: Time,
    timeline_end: Time,
    raw_anchor: i128,
    raw_min: i128,
    raw_max: i128,
}

fn frame_nanos(frame: u64, sample_rate: u32) -> i128 {
    i128::from(frame).saturating_mul(1_000_000_000) / i128::from(sample_rate.max(1))
}

fn push_periodic_occurrences(
    markers: &mut Vec<Marker>,
    item: &AudioItem,
    base: i128,
    period: i128,
    range: &MarkerRange,
    kind: MarkerKind,
) {
    if period <= 0 {
        return;
    }
    let first = (range.raw_min - base).div_euclid(period);
    let last = (range.raw_max - base).div_euclid(period);
    for cycle in first..=last {
        push_occurrence(
            markers,
            item,
            base.saturating_add(cycle.saturating_mul(period)),
            range,
            kind,
        );
    }
}

fn push_occurrence(
    markers: &mut Vec<Marker>,
    item: &AudioItem,
    source_nanos: i128,
    range: &MarkerRange,
    kind: MarkerKind,
) {
    if !(range.raw_min..=range.raw_max).contains(&source_nanos) {
        return;
    }
    let source_delta = Time::from_nanos_i128(source_nanos - range.raw_anchor);
    let timeline = item
        .start
        .saturating_add(unscaled_time_delta(source_delta, item.playback_speed));
    if timeline >= range.timeline_start && timeline <= range.timeline_end {
        markers.push(Marker {
            time: timeline,
            kind,
        });
    }
}

fn load_or_analyze(
    cache: &Result<BeatCache, String>,
    item: &AudioItem,
    is_cancelled: &impl Fn() -> bool,
) -> Result<Option<BeatAnalysis>, String> {
    if let Ok(cache) = cache
        && let Some(analysis) = cache.load(item)?
    {
        return Ok(analysis);
    }
    let analysis = analyze(item, is_cancelled)?;
    if let Ok(cache) = cache
        && let Err(error) = cache.store(item, analysis.as_ref())
    {
        tracing::warn!("Could not cache beat analysis: {error}");
    }
    Ok(analysis)
}

fn analyze(
    item: &AudioItem,
    is_cancelled: &impl Fn() -> bool,
) -> Result<Option<BeatAnalysis>, String> {
    let source = item.source_builder().build();
    let mut renderer = super::streaming::OfflineAudioRenderer::new(&source, SAMPLE_RATE)?;
    let mut mono = Vec::new();
    let mut start = Time::ZERO;
    while start < source.source_duration {
        if is_cancelled() {
            return Err("beat analysis cancelled".to_string());
        }
        let duration = source
            .source_duration
            .saturating_sub(start)
            .min(Time::from_seconds(ANALYSIS_CHUNK_SECONDS));
        if duration <= Time::ZERO {
            break;
        }
        let samples = renderer.render(&source, start, duration)?;
        mono.extend(
            samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|channels| (channels[0] + channels[1]) * 0.5),
        );
        start = start.saturating_add(duration);
    }
    if is_cancelled() {
        return Err("beat analysis cancelled".to_string());
    }
    Ok(
        super::beat_math::detect_beats(&mono, SAMPLE_RATE).map(|track| BeatAnalysis {
            sample_rate: SAMPLE_RATE,
            beat_frames: track.beat_frames,
            period_frames: track.period_frames,
            bar_phase: track.bar_phase,
            confidence: track.confidence,
        }),
    )
}
