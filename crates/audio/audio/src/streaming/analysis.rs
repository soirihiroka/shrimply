use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::thread;

use hashbrown::HashMap;
use shrimply_project::project::{Project, Time};

use super::{
    AudioRenderSession, AudioSourceKey, CHANNELS, OFFLINE_MIX_CHUNK_FRAMES,
    mix_project_selected_range,
};

const VOLUME_PEAK_CACHE_CHUNK_FRAMES: usize = 120;
const LIP_SYNC_SAMPLE_RATE: u32 = shrimply_lip_sync::SAMPLE_RATE;
pub const EXPRESSION_SAMPLE_RATE_HZ: u32 = 48_000;

enum MouthAnalysisCommand {
    Analyze {
        key: MouthAnalysisKey,
        project: Arc<Project>,
    },
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct MouthAnalysisKey {
    audio_revision: u64,
    track_ids: Vec<uuid::Uuid>,
    indices: Vec<usize>,
    start_frame: u64,
    end_frame: u64,
}

enum CachedMouthAnalysis {
    Pending,
    Ready(Vec<shrimply_lip_sync::MouthCue>),
    Failed(String),
}

#[derive(Default)]
struct MouthAnalysisCacheState {
    latest_audio_revisions: HashMap<Vec<uuid::Uuid>, u64>,
    latest_ranges: HashMap<(u128, Vec<usize>, bool), MouthAnalysisKey>,
    values: HashMap<MouthAnalysisKey, CachedMouthAnalysis>,
}

struct MouthAnalysisService {
    cache: Arc<(Mutex<MouthAnalysisCacheState>, Condvar)>,
    commands: mpsc::Sender<MouthAnalysisCommand>,
}

struct MouthAnalysisRequest<'a> {
    project: Arc<Project>,
    track_ids: &'a [uuid::Uuid],
    indices: &'a [usize],
    item_id: u128,
    audio_revision: u64,
    start: Time,
    end: Time,
    position: Time,
    blocking: bool,
}

pub struct FrameMouthSampler {
    revision: Option<u64>,
    audio_revision: u64,
    track_ids: Vec<uuid::Uuid>,
    project: Option<Arc<Project>>,
    blocking: bool,
}

static MOUTH_ANALYSIS_SERVICE: OnceLock<MouthAnalysisService> = OnceLock::new();

pub struct FrameAudioSampler {
    volume: FrameVolumeSampler,
    mouth: FrameMouthSampler,
}

impl Default for FrameAudioSampler {
    fn default() -> Self {
        Self::preview(EXPRESSION_SAMPLE_RATE_HZ)
    }
}

impl FrameAudioSampler {
    pub fn preview(sample_rate: u32) -> Self {
        Self {
            volume: FrameVolumeSampler::new(sample_rate),
            mouth: FrameMouthSampler::preview(),
        }
    }

    pub fn export(sample_rate: u32) -> Self {
        Self {
            volume: FrameVolumeSampler::new(sample_rate),
            mouth: FrameMouthSampler::export(),
        }
    }

    pub fn sample(
        &mut self,
        project: &Project,
        position: Time,
        revision: u64,
    ) -> shrimply_lip_sync::FrameAudioAnalysis {
        shrimply_lip_sync::FrameAudioAnalysis {
            volume: self.volume.sample(project, position, revision),
            mouth: self.mouth.sample(project, position, revision),
        }
    }
}
impl FrameMouthSampler {
    pub fn preview() -> Self {
        Self::new(false)
    }

    pub fn export() -> Self {
        Self::new(true)
    }

    fn new(blocking: bool) -> Self {
        Self {
            revision: None,
            audio_revision: 0,
            track_ids: Vec::new(),
            project: None,
            blocking,
        }
    }

    pub fn sample(
        &mut self,
        project: &Project,
        position: Time,
        revision: u64,
    ) -> shrimply_lip_sync::FrameMouthMixer {
        let track_ids = project
            .audio_tracks
            .iter()
            .map(|track| track.id)
            .collect::<Vec<_>>();
        if self.revision != Some(revision) || self.track_ids != track_ids {
            self.revision = Some(revision);
            self.audio_revision = mouth_audio_revision(project);
            self.track_ids = track_ids;
            self.project = Some(Arc::new(project.clone()));
            mouth_analysis_service().set_project(&self.track_ids, self.audio_revision);
        }

        let track_count = project.audio_tracks.len();
        let project = Arc::clone(
            self.project
                .as_ref()
                .expect("mouth sampler project was initialized"),
        );
        let track_ids = self.track_ids.clone();
        let audio_revision = self.audio_revision;
        let blocking = self.blocking;
        shrimply_lip_sync::FrameMouthMixer::resolving(
            track_count,
            move |indices, item_id, start, end| {
                mouth_analysis_service().resolve(MouthAnalysisRequest {
                    project: Arc::clone(&project),
                    track_ids: &track_ids,
                    indices,
                    item_id,
                    audio_revision,
                    start,
                    end,
                    position,
                    blocking,
                })
            },
        )
    }
}

fn mouth_audio_revision(project: &Project) -> u64 {
    let encoded = serde_json::to_vec(&(&project.audio_tracks, &project.folded_sequences))
        .expect("project audio must serialize for lip-sync caching");
    let mut hasher = DefaultHasher::new();
    encoded.hash(&mut hasher);
    for asset in project
        .audio_tracks
        .iter()
        .chain(
            project
                .folded_sequences
                .iter()
                .flat_map(|sequence| &sequence.audio_tracks),
        )
        .flat_map(|track| &track.items)
        .filter(|item| item.uses_file_asset())
        .filter_map(|item| item.file.snapshot().ok())
    {
        asset.hash(&mut hasher);
    }
    hasher.finish()
}

fn mouth_analysis_service() -> &'static MouthAnalysisService {
    MOUTH_ANALYSIS_SERVICE.get_or_init(|| {
        let cache = Arc::new((
            Mutex::new(MouthAnalysisCacheState::default()),
            Condvar::new(),
        ));
        let (commands, receiver) = mpsc::channel();
        let worker_cache = Arc::clone(&cache);
        thread::Builder::new()
            .name("mouth-analysis".to_string())
            .spawn(move || mouth_analysis_worker(receiver, worker_cache))
            .expect("could not start mouth analysis worker");
        MouthAnalysisService { cache, commands }
    })
}

impl MouthAnalysisService {
    fn set_project(&self, track_ids: &[uuid::Uuid], audio_revision: u64) {
        let (state, changed) = &*self.cache;
        let mut state = state.lock().expect("mouth analysis cache mutex poisoned");
        state
            .latest_audio_revisions
            .insert(track_ids.to_vec(), audio_revision);
        state
            .values
            .retain(|key, _| key.track_ids != track_ids || key.audio_revision == audio_revision);
        state
            .latest_ranges
            .retain(|_, key| key.track_ids != track_ids || key.audio_revision == audio_revision);
        changed.notify_all();
    }

    fn resolve(&self, request: MouthAnalysisRequest<'_>) -> shrimply_lip_sync::MouthValue {
        let MouthAnalysisRequest {
            project,
            track_ids,
            indices,
            item_id,
            audio_revision,
            start,
            end,
            position,
            blocking,
        } = request;
        let start_frame = start.as_sample_frame(LIP_SYNC_SAMPLE_RATE);
        let end_frame = end.as_sample_frame(LIP_SYNC_SAMPLE_RATE);
        if indices.is_empty() || end_frame <= start_frame {
            return shrimply_lip_sync::MouthValue::Ready(shrimply_lip_sync::MouthShape::X);
        }
        let key = MouthAnalysisKey {
            audio_revision,
            track_ids: track_ids.to_vec(),
            indices: indices.to_vec(),
            start_frame,
            end_frame,
        };
        let (state, changed) = &*self.cache;
        let mut state = state.lock().expect("mouth analysis cache mutex poisoned");
        if state.latest_audio_revisions.get(track_ids) != Some(&audio_revision) {
            return shrimply_lip_sync::MouthValue::Pending;
        }
        let range_owner = (item_id, indices.to_vec(), blocking);
        if let Some(previous) = state.latest_ranges.insert(range_owner, key.clone())
            && previous != key
            && !state
                .latest_ranges
                .values()
                .any(|latest| latest == &previous)
            && matches!(
                state.values.get(&previous),
                Some(CachedMouthAnalysis::Pending)
            )
        {
            state.values.remove(&previous);
            changed.notify_all();
        }
        if !state.values.contains_key(&key) {
            state
                .values
                .insert(key.clone(), CachedMouthAnalysis::Pending);
            self.commands
                .send(MouthAnalysisCommand::Analyze {
                    key: key.clone(),
                    project,
                })
                .expect("mouth analysis worker stopped unexpectedly");
        }
        loop {
            match state.values.get(&key) {
                Some(CachedMouthAnalysis::Ready(cues)) => {
                    let range_start = Time::from_fraction(
                        i64::try_from(start_frame).unwrap_or(i64::MAX),
                        i64::from(LIP_SYNC_SAMPLE_RATE),
                    );
                    let position = position.saturating_sub(range_start);
                    let shape = cues
                        .iter()
                        .find(|cue| position >= cue.start && position < cue.end)
                        .map(|cue| cue.shape)
                        .unwrap_or(shrimply_lip_sync::MouthShape::X);
                    return shrimply_lip_sync::MouthValue::Ready(shape);
                }
                Some(CachedMouthAnalysis::Failed(error)) => {
                    return shrimply_lip_sync::MouthValue::Failed(error.clone());
                }
                Some(CachedMouthAnalysis::Pending) if blocking => {
                    state = changed
                        .wait(state)
                        .expect("mouth analysis cache mutex poisoned");
                }
                Some(CachedMouthAnalysis::Pending) | None => {
                    return shrimply_lip_sync::MouthValue::Pending;
                }
            }
        }
    }
}

fn mouth_analysis_worker(
    commands: mpsc::Receiver<MouthAnalysisCommand>,
    cache: Arc<(Mutex<MouthAnalysisCacheState>, Condvar)>,
) {
    while let Ok(command) = commands.recv() {
        match command {
            MouthAnalysisCommand::Analyze { key, project } => {
                let should_analyze = {
                    let (state, _) = &*cache;
                    let state = state.lock().expect("mouth analysis cache mutex poisoned");
                    state.latest_audio_revisions.get(&key.track_ids) == Some(&key.audio_revision)
                        && matches!(state.values.get(&key), Some(CachedMouthAnalysis::Pending))
                };
                if !should_analyze {
                    continue;
                }
                let result = if key
                    .indices
                    .iter()
                    .all(|&index| index < project.audio_tracks.len())
                {
                    analyze_project_mouth(&project, &key.indices, key.start_frame, key.end_frame)
                } else {
                    Err("mouth analysis track selection is out of range".to_string())
                };
                let (state, changed) = &*cache;
                let mut state = state.lock().expect("mouth analysis cache mutex poisoned");
                if state.latest_audio_revisions.get(&key.track_ids) == Some(&key.audio_revision)
                    && matches!(state.values.get(&key), Some(CachedMouthAnalysis::Pending))
                {
                    state.values.insert(
                        key,
                        match result {
                            Ok(cues) => CachedMouthAnalysis::Ready(cues),
                            Err(error) => CachedMouthAnalysis::Failed(error),
                        },
                    );
                } else {
                    state.values.remove(&key);
                }
                changed.notify_all();
            }
        }
    }
}

fn analyze_project_mouth(
    project: &Project,
    indices: &[usize],
    start_frame: u64,
    end_frame: u64,
) -> Result<Vec<shrimply_lip_sync::MouthCue>, String> {
    let samples = render_mouth_samples(project, indices, start_frame, end_frame)?;
    shrimply_lip_sync::analyze(&samples)
}

fn render_mouth_samples(
    project: &Project,
    indices: &[usize],
    start_frame: u64,
    end_frame: u64,
) -> Result<Vec<i16>, String> {
    let frame_count = end_frame.saturating_sub(start_frame);
    let frame_count = usize::try_from(frame_count)
        .map_err(|_| "project audio is too long for lip-sync analysis".to_string())?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(frame_count)
        .map_err(|error| format!("could not allocate lip-sync audio: {error}"))?;
    let mut sessions = HashMap::new();
    let mut timeline_frame = start_frame;
    while timeline_frame < end_frame {
        let frames = (end_frame - timeline_frame).min(OFFLINE_MIX_CHUNK_FRAMES) as usize;
        let samples = mix_project_selected_range(
            project,
            &mut sessions,
            indices,
            timeline_frame,
            frames,
            LIP_SYNC_SAMPLE_RATE,
        );
        for channels in samples.chunks_exact(CHANNELS) {
            let mono = channels.iter().copied().sum::<f32>() / CHANNELS as f32;
            let sample = (mono.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            output.push(sample);
        }
        timeline_frame += frames as u64;
    }
    Ok(output)
}

enum VolumePeakCacheCommand {
    SetProject {
        project: Arc<Project>,
        revision: u64,
    },
    Cache {
        indices: Vec<usize>,
        revision: u64,
        first_frame: usize,
    },
}

#[derive(Default)]
struct VolumePeakCacheState {
    revision: Option<u64>,
    frame_count: usize,
    values: HashMap<Vec<usize>, Vec<f32>>,
}

struct VolumePeakCache {
    state: Arc<Mutex<VolumePeakCacheState>>,
    commands: mpsc::Sender<VolumePeakCacheCommand>,
}

enum VolumeFrameResolverCommand {
    Resolve {
        project: Arc<Project>,
        indices: Vec<usize>,
        start_frame: u64,
        frame_count: usize,
        response: mpsc::SyncSender<f32>,
    },
}

struct VolumeFrameResolver {
    commands: mpsc::Sender<VolumeFrameResolverCommand>,
}

impl VolumeFrameResolver {
    fn new(sample_rate: u32) -> Arc<Self> {
        let (commands, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("volume-frame-resolver".to_string())
            .spawn(move || volume_frame_resolver_worker(receiver, sample_rate))
            .expect("could not start volume frame resolver worker");
        Arc::new(Self { commands })
    }

    fn request(
        &self,
        project: Arc<Project>,
        indices: &[usize],
        start_frame: u64,
        frame_count: usize,
    ) -> mpsc::Receiver<f32> {
        let (response, result) = mpsc::sync_channel(1);
        self.commands
            .send(VolumeFrameResolverCommand::Resolve {
                project,
                indices: indices.to_vec(),
                start_frame,
                frame_count,
                response,
            })
            .expect("volume frame resolver worker stopped unexpectedly");
        result
    }
}

impl VolumePeakCache {
    fn new(sample_rate: u32) -> Arc<Self> {
        let (commands, receiver) = mpsc::channel();
        let state = Arc::new(Mutex::new(VolumePeakCacheState::default()));
        let worker_state = Arc::clone(&state);
        thread::Builder::new()
            .name("volume-peak-cache".to_string())
            .spawn(move || volume_peak_cache_worker(receiver, worker_state, sample_rate))
            .expect("could not start volume peak cache worker");
        Arc::new(Self { state, commands })
    }

    fn set_project(&self, project: Arc<Project>, revision: u64) {
        let frame_count = shrimply_math_core::frame_count(project.duration(), project.fps)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(0);
        *self.state.lock().expect("volume peak cache mutex poisoned") = VolumePeakCacheState {
            revision: Some(revision),
            frame_count,
            values: HashMap::new(),
        };
        self.commands
            .send(VolumePeakCacheCommand::SetProject { project, revision })
            .expect("volume peak cache worker stopped unexpectedly");
    }

    fn get_or_request(&self, revision: u64, frame_index: usize, indices: &[usize]) -> Option<f32> {
        let mut state = self.state.lock().expect("volume peak cache mutex poisoned");
        if state.revision != Some(revision) {
            return None;
        }
        if let Some(value) = state
            .values
            .get(indices)
            .and_then(|values| values.get(frame_index))
            .copied()
            .filter(|value| value.is_finite())
        {
            shrimply_benchmarking::increment("Volume peak cache / Hit");
            return Some(value);
        }

        shrimply_benchmarking::increment("Volume peak cache / Miss");
        if !state.values.contains_key(indices) {
            let frame_count = state.frame_count;
            state
                .values
                .insert(indices.to_vec(), vec![f32::NAN; frame_count]);
            drop(state);
            self.commands
                .send(VolumePeakCacheCommand::Cache {
                    indices: indices.to_vec(),
                    revision,
                    first_frame: frame_index,
                })
                .expect("volume peak cache worker stopped unexpectedly");
        }
        None
    }

    fn store(&self, revision: u64, frame_index: usize, indices: &[usize], value: f32) {
        let mut state = self.state.lock().expect("volume peak cache mutex poisoned");
        if state.revision == Some(revision)
            && let Some(value_slot) = state
                .values
                .get_mut(indices)
                .and_then(|values| values.get_mut(frame_index))
        {
            *value_slot = value;
        }
    }
}

pub struct FrameVolumeSampler {
    sample_rate: u32,
    project: Option<Arc<Project>>,
    cache_revision: Option<u64>,
    cache: Option<(u64, shrimply_math_media::FrameVolumeMixer)>,
    peaks: Arc<VolumePeakCache>,
    resolver: Arc<VolumeFrameResolver>,
}

impl FrameVolumeSampler {
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate.max(1);
        Self {
            sample_rate,
            project: None,
            cache_revision: None,
            cache: None,
            peaks: VolumePeakCache::new(sample_rate),
            resolver: VolumeFrameResolver::new(sample_rate),
        }
    }

    pub fn sample(
        &mut self,
        project: &Project,
        position: Time,
        revision: u64,
    ) -> shrimply_math_media::FrameVolumeMixer {
        let _measurement = shrimply_benchmarking::measure("Volume sampling / Total");
        if self.cache_revision != Some(revision) {
            self.cache_revision = Some(revision);
            self.cache = None;
            let project = Arc::new(project.clone());
            self.peaks.set_project(Arc::clone(&project), revision);
            self.project = Some(project);
        }
        let Some(spans) = shrimply_math_media::timeline_sample_frame_spans(
            position,
            project.fps,
            self.sample_rate,
            1,
        ) else {
            return shrimply_math_media::FrameVolumeMixer::silent(project.audio_tracks.len());
        };
        let start_frame = spans[0].0;
        if let Some((_, mixer)) = self
            .cache
            .as_ref()
            .filter(|(cached_start, _)| *cached_start == start_frame)
        {
            return mixer.clone();
        }

        let frame_index = shrimply_math_core::frame_index(position, project.fps)
            .and_then(|index| usize::try_from(index).ok())
            .unwrap_or(0);
        let end_frame = spans[0].1;
        let track_count = project.audio_tracks.len();
        let peaks = Arc::clone(&self.peaks);
        let resolver = Arc::clone(&self.resolver);
        let deferred = Arc::clone(&resolver);
        let project = self
            .project
            .as_ref()
            .expect("volume project snapshot")
            .clone();
        let deferred_project = Arc::clone(&project);
        let frame_count = usize::try_from(end_frame.saturating_sub(start_frame))
            .expect("volume frame exceeds addressable memory");
        let requests = Mutex::new(HashMap::<Vec<usize>, mpsc::Receiver<f32>>::new());
        let mixer = shrimply_math_media::FrameVolumeMixer::resolving(track_count, move |indices| {
            if let Some(value) = peaks.get_or_request(revision, frame_index, indices) {
                return value;
            }
            let _measurement = shrimply_benchmarking::measure("Volume sampling / Mix tracks");
            let value = resolver
                .request(Arc::clone(&project), indices, start_frame, frame_count)
                .recv()
                .expect("volume frame resolver worker dropped a request");
            peaks.store(revision, frame_index, indices, value);
            value
        })
        .with_deferred_resolver(move |indices| {
            use shrimply_math_media::VolumeValue;
            let mut requests = requests.lock().expect("deferred volume requests poisoned");
            let request = requests.entry(indices.to_vec()).or_insert_with(|| {
                deferred.request(
                    Arc::clone(&deferred_project),
                    indices,
                    start_frame,
                    frame_count,
                )
            });
            match request.try_recv() {
                Ok(value) => {
                    requests.remove(indices);
                    VolumeValue::Ready(value)
                }
                Err(mpsc::TryRecvError::Empty) => VolumeValue::Pending,
                Err(mpsc::TryRecvError::Disconnected) => {
                    VolumeValue::Failed("Volume analysis worker dropped a request".into())
                }
            }
        });
        self.cache = Some((start_frame, mixer.clone()));
        mixer
    }
}

fn volume_frame_resolver_worker(
    commands: mpsc::Receiver<VolumeFrameResolverCommand>,
    sample_rate: u32,
) {
    let mut project = None;
    let mut sessions = HashMap::new();
    while let Ok(command) = commands.recv() {
        match command {
            VolumeFrameResolverCommand::Resolve {
                project: requested_project,
                indices,
                start_frame,
                frame_count,
                response,
            } => {
                if project
                    .as_ref()
                    .is_none_or(|previous| !Arc::ptr_eq(previous, &requested_project))
                {
                    sessions.clear();
                    project = Some(Arc::clone(&requested_project));
                }
                assert!(
                    indices
                        .iter()
                        .all(|&index| index < requested_project.audio_tracks.len()),
                    "volume selection exceeds project tracks"
                );
                let mixed = mix_project_selected_range(
                    &requested_project,
                    &mut sessions,
                    &indices,
                    start_frame,
                    frame_count,
                    sample_rate,
                );
                // Scrubbing can retire a frame before its query completes. That
                // cancels only this response, not the shared resolver worker.
                let _ = response.send(shrimply_math_media::peak_amplitude(&mixed));
            }
        }
    }
}
fn volume_peak_cache_worker(
    commands: mpsc::Receiver<VolumePeakCacheCommand>,
    state: Arc<Mutex<VolumePeakCacheState>>,
    sample_rate: u32,
) {
    let mut project = None;
    let mut revision = None;
    let mut sessions = HashMap::new();
    while let Ok(command) = commands.recv() {
        match command {
            VolumePeakCacheCommand::SetProject {
                project: next_project,
                revision: next_revision,
            } => {
                sessions.clear();
                project = Some(next_project);
                revision = Some(next_revision);
            }
            VolumePeakCacheCommand::Cache {
                indices,
                revision: requested_revision,
                first_frame,
            } => {
                let Some(project) = project.as_ref() else {
                    continue;
                };
                if revision != Some(requested_revision)
                    || indices
                        .iter()
                        .any(|&index| index >= project.audio_tracks.len())
                {
                    continue;
                }
                cache_project_volume_peaks(
                    project,
                    &mut sessions,
                    &state,
                    &indices,
                    requested_revision,
                    first_frame,
                    sample_rate,
                );
            }
        }
    }
}

fn cache_project_volume_peaks(
    project: &Project,
    sessions: &mut HashMap<AudioSourceKey, AudioRenderSession>,
    state: &Mutex<VolumePeakCacheState>,
    indices: &[usize],
    revision: u64,
    requested_frame: usize,
    sample_rate: u32,
) {
    let Some(frame_count) = shrimply_math_core::frame_count(project.duration(), project.fps)
        .and_then(|count| usize::try_from(count).ok())
    else {
        return;
    };
    let _measurement = shrimply_benchmarking::measure("Volume peak cache / Build timeline");
    let first_chunk = requested_frame.min(frame_count.saturating_sub(1))
        / VOLUME_PEAK_CACHE_CHUNK_FRAMES
        * VOLUME_PEAK_CACHE_CHUNK_FRAMES;
    let chunks = (first_chunk..frame_count)
        .step_by(VOLUME_PEAK_CACHE_CHUNK_FRAMES)
        .chain((0..first_chunk).step_by(VOLUME_PEAK_CACHE_CHUNK_FRAMES));
    for first_frame in chunks {
        if state
            .lock()
            .expect("volume peak cache mutex poisoned")
            .revision
            != Some(revision)
        {
            return;
        }
        let chunk_frames = (frame_count - first_frame).min(VOLUME_PEAK_CACHE_CHUNK_FRAMES);
        let Some(position) = shrimply_math_core::time_from_frame(first_frame as u64, project.fps)
        else {
            return;
        };
        let Some(spans) = shrimply_math_media::timeline_sample_frame_spans(
            position,
            project.fps,
            sample_rate,
            chunk_frames,
        ) else {
            return;
        };
        let start_sample_frame = spans[0].0;
        let end_sample_frame = spans
            .last()
            .expect("volume peak cache chunk always contains frames")
            .1;
        let sample_frame_count =
            usize::try_from(end_sample_frame.saturating_sub(start_sample_frame))
                .expect("volume peak cache chunk exceeds addressable memory");
        let mixed = mix_project_selected_range(
            project,
            sessions,
            indices,
            start_sample_frame,
            sample_frame_count,
            sample_rate,
        );
        let peaks = spans
            .into_iter()
            .map(|(start, end)| {
                let start = usize::try_from(start.saturating_sub(start_sample_frame))
                    .expect("volume peak cache offset exceeds addressable memory")
                    .checked_mul(CHANNELS)
                    .expect("volume peak cache sample offset overflow");
                let end = usize::try_from(end.saturating_sub(start_sample_frame))
                    .expect("volume peak cache offset exceeds addressable memory")
                    .checked_mul(CHANNELS)
                    .expect("volume peak cache sample offset overflow");
                shrimply_math_media::peak_amplitude(&mixed[start..end])
            })
            .collect::<Vec<_>>();
        let mut state = state.lock().expect("volume peak cache mutex poisoned");
        if state.revision != Some(revision) {
            return;
        }
        let values = state
            .values
            .get_mut(indices)
            .expect("requested volume peak cache entry disappeared");
        values[first_frame..first_frame + peaks.len()].copy_from_slice(&peaks);
    }
}
