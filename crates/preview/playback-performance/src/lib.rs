use async_channel::{Receiver, Sender, TrySendError};
use rangemap::{RangeMap, RangeSet};
use serde::Serialize;
use shrimply_math_core::{Fraction, Time, fraction_denominator, fraction_numerator};
use shrimply_project::project::{FoldedSequence, Project, VideoItem, VideoItemContent};
use std::collections::{BTreeMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

pub use shrimply_preview_core::performance::RenderEvent;

const SLOW_FPS: u128 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PerformanceLevel {
    Unknown,
    Fast,
    Low,
    Slow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeRange {
    pub start: Time,
    pub end: Time,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerformanceRange {
    pub start: Time,
    pub end: Time,
    pub level: PerformanceLevel,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub generation: u64,
    pub visual_ranges: Vec<TimeRange>,
    pub performance_ranges: Vec<PerformanceRange>,
}

#[derive(Clone)]
pub struct SharedCollector {
    sender: mpsc::Sender<Command>,
    snapshot: Arc<RwLock<Arc<Snapshot>>>,
    subscribers: Arc<Mutex<Vec<Sender<()>>>>,
    active_session: Arc<AtomicU64>,
    next_session: Arc<AtomicU64>,
}

enum Command {
    Begin { session: u64, position: Time },
    Seek { session: u64, position: Time },
    End { session: u64, position: Time },
    Render { session: u64, event: RenderEvent },
    Project { project: Arc<Project> },
}

#[derive(Clone, Eq, PartialEq)]
struct VisualElement {
    start: Time,
    end: Time,
    signature: u64,
}

#[derive(Clone, Copy)]
struct PendingRender {
    frame: u64,
    level: PerformanceLevel,
}

struct WorkerState {
    fps: Fraction,
    visual_elements: BTreeMap<Uuid, VisualElement>,
    visual_ranges: Vec<TimeRange>,
    visual_frame_ranges: RangeSet<u64>,
    performance_runs: RangeMap<u64, PerformanceLevel>,
    session: Option<u64>,
    segment_start: Option<u64>,
    last_requested_end: Option<u64>,
    pending_render: Option<PendingRender>,
    request_sessions: BTreeMap<u64, u64>,
}

pub fn open(project: Arc<Project>) -> SharedCollector {
    let (sender, receiver) = mpsc::channel();
    let snapshot = Arc::new(RwLock::new(Arc::new(Snapshot::default())));
    let worker_snapshot = snapshot.clone();
    let subscribers = Arc::new(Mutex::new(Vec::new()));
    let worker_subscribers = subscribers.clone();
    thread::Builder::new()
        .name("playback-performance".to_string())
        .spawn(move || worker(receiver, worker_snapshot, worker_subscribers, project))
        .expect("could not start playback performance worker");
    SharedCollector {
        sender,
        snapshot,
        subscribers,
        active_session: Arc::new(AtomicU64::new(0)),
        next_session: Arc::new(AtomicU64::new(0)),
    }
}

pub fn snapshot(collector: &SharedCollector) -> Arc<Snapshot> {
    collector
        .snapshot
        .read()
        .expect("playback performance snapshot lock poisoned")
        .clone()
}

pub fn generation(collector: &SharedCollector) -> u64 {
    collector
        .snapshot
        .read()
        .expect("playback performance snapshot lock poisoned")
        .generation
}

pub fn subscribe(collector: &SharedCollector) -> Receiver<()> {
    let (sender, receiver) = async_channel::bounded(1);
    collector
        .subscribers
        .lock()
        .expect("playback performance subscriber lock poisoned")
        .push(sender);
    receiver
}

pub fn begin_playback(collector: &SharedCollector, position: Time) {
    let session = collector
        .next_session
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    collector.active_session.store(session, Ordering::Release);
    send(collector, Command::Begin { session, position });
}

pub fn end_playback(collector: &SharedCollector, position: Time) {
    let session = collector.active_session.swap(0, Ordering::AcqRel);
    if session != 0 {
        send(collector, Command::End { session, position });
    }
}

pub fn seek_playback(collector: &SharedCollector, position: Time) {
    let session = collector.active_session.load(Ordering::Acquire);
    if session != 0 {
        send(collector, Command::Seek { session, position });
    }
}

pub fn record_render_event(collector: &SharedCollector, event: RenderEvent) {
    let session = collector.active_session.load(Ordering::Acquire);
    if session != 0 {
        send(collector, Command::Render { session, event });
    }
}

pub fn set_project(collector: &SharedCollector, project: Arc<Project>) {
    send(collector, Command::Project { project });
}

fn send(collector: &SharedCollector, command: Command) {
    if let Err(error) = collector.sender.send(command) {
        tracing::warn!(%error, "playback performance worker stopped");
    }
}

fn worker(
    receiver: mpsc::Receiver<Command>,
    snapshot: Arc<RwLock<Arc<Snapshot>>>,
    subscribers: Arc<Mutex<Vec<Sender<()>>>>,
    project: Arc<Project>,
) {
    let initial_elements = visual_elements(&project);
    let initial_ranges = visual_ranges(&initial_elements);
    let mut state = WorkerState {
        fps: project.fps,
        visual_elements: initial_elements,
        visual_frame_ranges: frame_ranges(&initial_ranges, project.fps),
        visual_ranges: initial_ranges,
        performance_runs: RangeMap::new(),
        session: None,
        segment_start: None,
        last_requested_end: None,
        pending_render: None,
        request_sessions: BTreeMap::new(),
    };
    publish(&snapshot, &subscribers, &state);

    while let Ok(first) = receiver.recv() {
        let mut commands = vec![first];
        commands.extend(receiver.try_iter());
        let mut ranges_changed = false;
        let mut elements_changed = false;
        for command in commands {
            match command {
                Command::Begin { session, position } => {
                    let frame = frame_at(position, state.fps);
                    state.session = Some(session);
                    state.segment_start = Some(frame);
                    state.last_requested_end = Some(frame.saturating_add(1));
                    state.pending_render = None;
                    state.request_sessions.clear();
                }
                Command::Seek { session, position } if state.session == Some(session) => {
                    let frame = frame_at(position, state.fps);
                    let frame_end = frame.saturating_add(1);
                    let previous_end = state.last_requested_end.unwrap_or(frame_end);
                    ranges_changed |= finish_segment(&mut state, previous_end);
                    state.segment_start = Some(frame);
                    state.last_requested_end = Some(frame_end);
                }
                Command::Seek { .. } => {}
                Command::End { session, position } if state.session == Some(session) => {
                    let position_end = frame_at(position, state.fps).saturating_add(1);
                    let end = state
                        .last_requested_end
                        .unwrap_or(position_end)
                        .max(position_end);
                    ranges_changed |= finish_segment(&mut state, end);
                    state.session = None;
                }
                Command::End { .. } => {}
                Command::Render {
                    session,
                    event:
                        RenderEvent::Requested {
                            request_id,
                            position,
                        },
                } if state.session == Some(session) => {
                    let frame = frame_at(position, state.fps);
                    let frame_end = frame.saturating_add(1);
                    state.request_sessions.insert(request_id, session);
                    state.segment_start.get_or_insert(frame);
                    if let Some(pending) = state.pending_render {
                        ranges_changed |= insert_performance(
                            &mut state.performance_runs,
                            &state.visual_frame_ranges,
                            pending.frame,
                            frame_end,
                            pending.level,
                        );
                    }
                    state.last_requested_end = Some(frame_end);
                }
                Command::Render {
                    event:
                        RenderEvent::Completed {
                            request_id,
                            position,
                            elapsed,
                            project_fps,
                        },
                    ..
                } => {
                    let Some(session) = state.request_sessions.remove(&request_id) else {
                        continue;
                    };
                    if state.session != Some(session) {
                        continue;
                    }
                    let frame = frame_at(position, project_fps);
                    let frame_end = frame.saturating_add(1);
                    state.request_sessions.retain(|id, _| *id > request_id);
                    let level = classify(elapsed, project_fps);
                    match state.pending_render.take() {
                        Some(pending) if frame > pending.frame => {
                            ranges_changed |= insert_performance(
                                &mut state.performance_runs,
                                &state.visual_frame_ranges,
                                pending.frame,
                                frame,
                                pending.level,
                            );
                        }
                        Some(_) => {}
                        None => {
                            if let Some(start) = state.segment_start
                                && frame > start
                            {
                                ranges_changed |= insert_performance(
                                    &mut state.performance_runs,
                                    &state.visual_frame_ranges,
                                    start,
                                    frame,
                                    level,
                                );
                            }
                        }
                    }
                    ranges_changed |= insert_performance(
                        &mut state.performance_runs,
                        &state.visual_frame_ranges,
                        frame,
                        frame_end,
                        level,
                    );
                    state.pending_render = Some(PendingRender { frame, level });
                }
                Command::Render { .. } => {}
                Command::Project { project: next } => {
                    let next_elements = visual_elements(&next);
                    if next.fps == state.fps && next_elements == state.visual_elements {
                        continue;
                    }
                    if next.fps == state.fps {
                        let affected =
                            changed_visual_ranges(&state.visual_elements, &next_elements);
                        for range in frame_ranges(&affected, state.fps) {
                            ranges_changed |=
                                invalidate(&mut state.performance_runs, range.start, range.end);
                        }
                    } else if !state.performance_runs.is_empty() {
                        state.performance_runs.clear();
                        ranges_changed = true;
                    }
                    state.visual_elements = next_elements;
                    state.visual_ranges = visual_ranges(&state.visual_elements);
                    state.fps = next.fps;
                    state.visual_frame_ranges = frame_ranges(&state.visual_ranges, state.fps);
                    let clipped =
                        clip_performance_runs(&state.performance_runs, &state.visual_frame_ranges);
                    ranges_changed |= clipped != state.performance_runs;
                    state.performance_runs = clipped;
                    state.segment_start = None;
                    state.last_requested_end = None;
                    state.pending_render = None;
                    state.request_sessions.clear();
                    elements_changed = true;
                    drop(next);
                }
            }
        }
        if elements_changed || ranges_changed {
            publish(&snapshot, &subscribers, &state);
        }
    }
}

fn finish_segment(state: &mut WorkerState, end: u64) -> bool {
    let pending = state.pending_render.take().or_else(|| {
        (!state.request_sessions.is_empty()).then(|| PendingRender {
            frame: state.segment_start.unwrap_or(end),
            level: PerformanceLevel::Slow,
        })
    });
    let changed = pending.is_some_and(|pending| {
        insert_performance(
            &mut state.performance_runs,
            &state.visual_frame_ranges,
            pending.frame,
            end,
            pending.level,
        )
    });
    state.segment_start = None;
    state.last_requested_end = None;
    state.request_sessions.clear();
    changed
}

fn publish(
    snapshot: &RwLock<Arc<Snapshot>>,
    subscribers: &Mutex<Vec<Sender<()>>>,
    state: &WorkerState,
) {
    {
        let mut snapshot = snapshot
            .write()
            .expect("playback performance snapshot lock poisoned");
        *snapshot = Arc::new(Snapshot {
            generation: snapshot.generation.wrapping_add(1),
            visual_ranges: state.visual_ranges.clone(),
            performance_ranges: state
                .performance_runs
                .iter()
                .map(|(range, level)| PerformanceRange {
                    start: frame_position(range.start, state.fps),
                    end: frame_position(range.end, state.fps),
                    level: *level,
                })
                .collect(),
        });
    }
    subscribers
        .lock()
        .expect("playback performance subscriber lock poisoned")
        .retain(|subscriber| !matches!(subscriber.try_send(()), Err(TrySendError::Closed(_))));
}

fn classify(elapsed: Duration, fps: Fraction) -> PerformanceLevel {
    let elapsed_ns = elapsed.as_nanos();
    let numerator = fraction_numerator(fps);
    let denominator = fraction_denominator(fps);
    if numerator > 0
        && denominator > 0
        && elapsed_ns.saturating_mul(numerator as u128)
            <= 1_000_000_000_u128.saturating_mul(denominator as u128)
    {
        PerformanceLevel::Fast
    } else if elapsed_ns.saturating_mul(SLOW_FPS) > 1_000_000_000 {
        PerformanceLevel::Slow
    } else {
        PerformanceLevel::Low
    }
}

fn frame_at(position: Time, fps: Fraction) -> u64 {
    shrimply_math_core::frame_index(position, fps)
        .and_then(|frame| u64::try_from(frame).ok())
        .expect("project frame rate and playback position must be positive")
}

fn frame_position(frame: u64, fps: Fraction) -> Time {
    shrimply_math_core::time_from_frame(frame, fps).expect("project frame rate must be positive")
}

fn visual_elements(project: &Project) -> BTreeMap<Uuid, VisualElement> {
    let mut elements = BTreeMap::new();
    for (track_index, track) in project.video_tracks.iter().enumerate() {
        if !track.enabled {
            continue;
        }
        for item in &track.items {
            elements.insert(
                item.id,
                VisualElement {
                    start: item.start,
                    end: item.end,
                    signature: visual_signature(project, track_index, track.enabled, item),
                },
            );
        }
    }
    elements
}

#[derive(Serialize)]
struct Signature<'a> {
    fps_numerator: i64,
    fps_denominator: i64,
    canvas_size: shrimply_project::project::CanvasSize,
    track_index: usize,
    track_enabled: bool,
    item: &'a VideoItem,
    folded_sequences: Vec<&'a FoldedSequence>,
    assets: Vec<AssetSignature>,
}

#[derive(Serialize)]
struct AssetSignature {
    path: String,
    revision: Option<u64>,
    length: Option<u64>,
    modified_ns: Option<i128>,
}

fn visual_signature(
    project: &Project,
    track_index: usize,
    track_enabled: bool,
    item: &VideoItem,
) -> u64 {
    let mut sequences = Vec::new();
    collect_folded_sequences(project, item, &mut sequences, &mut Vec::new());
    let mut assets = vec![asset_signature(item)];
    for sequence in &sequences {
        assets.extend(
            sequence
                .video_tracks
                .iter()
                .flat_map(|track| &track.items)
                .map(asset_signature),
        );
    }
    let bytes = serde_json::to_vec(&Signature {
        fps_numerator: fraction_numerator(project.fps),
        fps_denominator: fraction_denominator(project.fps),
        canvas_size: project.canvas_size,
        track_index,
        track_enabled,
        item,
        folded_sequences: sequences,
        assets,
    })
    .expect("visual performance signature is serializable");
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn collect_folded_sequences<'a>(
    project: &'a Project,
    item: &'a VideoItem,
    sequences: &mut Vec<&'a FoldedSequence>,
    visited: &mut Vec<Uuid>,
) {
    let VideoItemContent::FoldedSequence(reference) = &item.content else {
        return;
    };
    if visited.contains(&reference.sequence_id) {
        return;
    }
    let Some(sequence) = project.folded_sequence(reference.sequence_id) else {
        return;
    };
    visited.push(reference.sequence_id);
    sequences.push(sequence);
    for child in sequence.video_tracks.iter().flat_map(|track| &track.items) {
        collect_folded_sequences(project, child, sequences, visited);
    }
}

fn asset_signature(item: &VideoItem) -> AssetSignature {
    if !item.uses_file_asset() || item.file.as_os_str().is_empty() {
        return AssetSignature {
            path: String::new(),
            revision: None,
            length: None,
            modified_ns: None,
        };
    }
    match item.file.snapshot() {
        Ok(snapshot) => AssetSignature {
            path: snapshot.path().to_string_lossy().into_owned(),
            revision: Some(snapshot.revision()),
            length: Some(snapshot.len()),
            modified_ns: Some(snapshot.modified_ns()),
        },
        Err(_) => AssetSignature {
            path: item.file.path().to_string_lossy().into_owned(),
            revision: None,
            length: None,
            modified_ns: None,
        },
    }
}

fn changed_visual_ranges(
    previous: &BTreeMap<Uuid, VisualElement>,
    next: &BTreeMap<Uuid, VisualElement>,
) -> Vec<TimeRange> {
    let mut ranges = Vec::new();
    for id in previous.keys().chain(next.keys()) {
        let before = previous.get(id);
        let after = next.get(id);
        if matches!((before, after), (Some(before), Some(after)) if before.start == after.start && before.end == after.end && before.signature == after.signature)
        {
            continue;
        }
        if let Some(before) = before {
            ranges.push(TimeRange {
                start: before.start,
                end: before.end,
            });
        }
        if let Some(after) = after {
            ranges.push(TimeRange {
                start: after.start,
                end: after.end,
            });
        }
    }
    merge_time_ranges(ranges)
}

fn visual_ranges(elements: &BTreeMap<Uuid, VisualElement>) -> Vec<TimeRange> {
    merge_time_ranges(
        elements
            .values()
            .filter(|element| element.end > element.start)
            .map(|element| TimeRange {
                start: element.start,
                end: element.end,
            })
            .collect(),
    )
}

fn merge_time_ranges(mut ranges: Vec<TimeRange>) -> Vec<TimeRange> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<TimeRange> = Vec::new();
    for range in ranges.into_iter().filter(|range| range.end > range.start) {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn frame_ranges(ranges: &[TimeRange], fps: Fraction) -> RangeSet<u64> {
    let mut result = RangeSet::new();
    for range in ranges {
        let start = frame_at(range.start, fps);
        if let Some(end) = shrimply_math_core::frame_count(range.end, fps)
            && end > start
        {
            result.insert(start..end);
        }
    }
    result
}

fn invalidate(runs: &mut RangeMap<u64, PerformanceLevel>, start: u64, end: u64) -> bool {
    if end <= start {
        return false;
    }
    let previous = runs.clone();
    runs.remove(start..end);
    *runs != previous
}

fn insert_performance(
    runs: &mut RangeMap<u64, PerformanceLevel>,
    coverage: &RangeSet<u64>,
    start: u64,
    end: u64,
    level: PerformanceLevel,
) -> bool {
    if end <= start || level == PerformanceLevel::Unknown {
        return false;
    }
    let previous = runs.clone();
    for visual in coverage.overlapping(&(start..end)) {
        let clipped_start = start.max(visual.start);
        let clipped_end = end.min(visual.end);
        if clipped_end > clipped_start {
            runs.insert(clipped_start..clipped_end, level);
        }
    }
    *runs != previous
}

fn clip_performance_runs(
    runs: &RangeMap<u64, PerformanceLevel>,
    coverage: &RangeSet<u64>,
) -> RangeMap<u64, PerformanceLevel> {
    let mut result = RangeMap::new();
    for (run, level) in runs.iter() {
        for visual in coverage.overlapping(run) {
            let start = run.start.max(visual.start);
            let end = run.end.min(visual.end);
            if end > start {
                result.insert(start..end, *level);
            }
        }
    }
    result
}
