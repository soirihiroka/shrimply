use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use shrimply_decoder_pool::{DecoderActivityGuard, TemporalDecoder, TemporalDecoderPool};
use shrimply_math_core::Fraction;
use shrimply_project_document::project::{
    CanvasSize, Time, VideoItem, playback_speed_is_negative, playback_speed_is_zero,
    video_source_time_at,
};
use shrimply_visual_frame::GPU_FRAME_ALLOCATION_EXHAUSTED;
use uuid::Uuid;

use crate::session::{DecodeControl, DecodeOutcome, DecodedVisual, VideoDecoderSession};
use crate::startup::{DECODER_STARTUP_MEMORY_EXHAUSTED, VideoDecoderContext};
use crate::track::{VideoDecoderOwner, VideoPlane, VideoSource};
use crate::{
    DEFAULT_VIDEO_DECODER_POOL_SIZE, MAX_HANDOFF_FORWARD_FRAMES,
    MAX_LATEST_REQUEST_DISTANCE_FRAMES, NEXT_DECODER_WORKER_ID, NEXT_TEMPORAL_CONSUMER_ID,
    TEMPORAL_CURRENT_BYTES, TEMPORAL_CURRENT_FRAMES,
};

#[derive(Default)]
struct DecoderMetadata {
    position: Option<Time>,
    frame_duration: Time,
}

impl DecoderMetadata {
    fn update(&mut self, decoder: &VideoDecoderSession) {
        self.position = decoder.last_decoded_position;
        self.frame_duration = decoder.frame_duration;
    }
}

struct DecoderWork {
    owner: VideoDecoderOwner,
    position: Time,
    cached: Option<DecodedVisual>,
    control: Option<DecodeControl>,
    mode: DecodeMode,
    force_seek: bool,
    revision: u64,
    reply: Option<SyncSender<Result<DecodeOutcome, String>>>,
    _activity: Option<DecoderActivityGuard>,
}

#[derive(Clone, Eq, PartialEq)]
struct LatestTarget {
    owner: VideoDecoderOwner,
    position: Time,
    mode: DecodeMode,
    generation: Option<u64>,
}

impl LatestTarget {
    fn same_request(&self, other: &Self) -> bool {
        self == other
    }

    fn continues(&self, other: &Self, maximum_distance: Time) -> bool {
        self.owner == other.owner
            && self.mode == other.mode
            && (self.mode.continuous()
                || self.position.abs_diff(other.position) <= maximum_distance)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DecodeMode {
    BestEffort,
    Accurate,
    Continuous,
    LocalScrub,
}

impl DecodeMode {
    const fn time_accurate(self) -> bool {
        !matches!(self, Self::BestEffort)
    }

    const fn continuous(self) -> bool {
        matches!(self, Self::Continuous)
    }
}

#[derive(Default)]
struct DecoderInboxState {
    shutdown: bool,
    failed: Option<String>,
    exact: Option<DecoderWork>,
    latest: Option<DecoderWork>,
    active_latest: Option<LatestTarget>,
    last_request: Option<LatestTarget>,
    revision: u64,
}

#[derive(Default)]
struct DecoderInbox {
    state: Mutex<DecoderInboxState>,
    ready: Condvar,
    revision: Arc<AtomicU64>,
}

struct PooledVideoDecoder {
    frame_size: CanvasSize,
    metadata: Arc<Mutex<DecoderMetadata>>,
    current: Arc<Mutex<Option<DecodedVisual>>>,
    inbox: Arc<DecoderInbox>,
    worker: Option<JoinHandle<()>>,
}

pub struct PendingDecode {
    response: Option<Receiver<Result<DecodeOutcome, String>>>,
    _activity: Option<DecoderActivityGuard>,
}

#[derive(Clone, Copy)]
struct VideoDecoderInfo {
    frame_size: CanvasSize,
    frame_duration: Time,
    position: Option<Time>,
}

struct VideoDecoderRequest {
    owner: VideoDecoderOwner,
    cached: Option<DecodedVisual>,
    control: Option<DecodeControl>,
    mode: DecodeMode,
    latest: bool,
}

impl PendingDecode {
    pub fn receive(self) -> Result<DecodeOutcome, String> {
        self.response
            .ok_or_else(|| "real-time video request has no blocking response".to_string())?
            .recv()
            .map_err(|_| "video decoder worker stopped before returning a frame".to_string())?
    }
}

impl TemporalDecoder<VideoSource> for PooledVideoDecoder {
    type Error = String;
    type Context = VideoDecoderContext;
    type Metadata = VideoDecoderInfo;
    type Current = DecodedVisual;
    type Request = VideoDecoderRequest;
    type Response = PendingDecode;

    fn create(source: &VideoSource, context: &Self::Context) -> Result<Self, Self::Error> {
        Self::spawn(source.clone(), context.clone())
    }

    fn try_create(
        source: &VideoSource,
        context: &Self::Context,
    ) -> Result<Option<Self>, Self::Error> {
        Self::spawn(source.clone(), context.clone()).map(Some)
    }

    fn metadata(&self) -> Self::Metadata {
        let metadata = self
            .metadata
            .lock()
            .expect("video decoder metadata mutex poisoned");
        VideoDecoderInfo {
            frame_size: self.frame_size,
            frame_duration: metadata.frame_duration,
            position: metadata.position,
        }
    }

    fn current(&self) -> Option<Self::Current> {
        self.current
            .lock()
            .expect("video decoder current frame mutex poisoned")
            .clone()
    }

    fn request(
        &mut self,
        position: Time,
        request: Self::Request,
        activity: DecoderActivityGuard,
    ) -> Result<Self::Response, Self::Error> {
        self.submit(position, request, activity)
    }
}

impl PooledVideoDecoder {
    fn spawn(source: VideoSource, context: VideoDecoderContext) -> Result<Self, String> {
        let frame_size = CanvasSize {
            width: source.width,
            height: source.height,
        };
        let metadata = Arc::new(Mutex::new(DecoderMetadata {
            position: None,
            frame_duration: Time::from_fraction(1, 30),
        }));
        let worker_metadata = metadata.clone();
        let current = Arc::new(Mutex::new(None::<DecodedVisual>));
        let worker_current = current.clone();
        let worker_source = source.clone();
        let worker_context = context;
        let inbox = Arc::new(DecoderInbox::default());
        let worker_inbox = inbox.clone();
        let worker_id = NEXT_DECODER_WORKER_ID.fetch_add(1, Ordering::Relaxed);
        let worker = thread::Builder::new()
            .name(format!(
                "video-decoder-{}-{worker_id}",
                source.media_track_id
            ))
            .spawn(move || {
                let mut decoder: Option<VideoDecoderSession> = None;
                loop {
                    let work = {
                        let mut state = worker_inbox
                            .state
                            .lock()
                            .expect("video decoder inbox mutex poisoned");
                        while !state.shutdown && state.exact.is_none() && state.latest.is_none() {
                            state = worker_inbox
                                .ready
                                .wait(state)
                                .expect("video decoder inbox mutex poisoned");
                        }
                        if state.shutdown {
                            break;
                        }
                        let work = state
                            .exact
                            .take()
                            .or_else(|| state.latest.take())
                            .expect("decoder inbox woke without work");
                        if work.reply.is_none() {
                            state.active_latest = Some(LatestTarget {
                                owner: work.owner.clone(),
                                position: work.position,
                                mode: work.mode,
                                generation: work.control.as_ref().map(DecodeControl::generation),
                            });
                        }
                        work
                    };

                    let DecoderWork {
                        owner,
                        position,
                        cached,
                        control,
                        mode,
                        force_seek,
                        revision,
                        reply,
                        _activity,
                    } = work;
                    let latest = reply.is_none();
                    let latest_control =
                        latest.then(|| DecodeControl::new(revision, worker_inbox.revision.clone()));
                    let controls = [control.as_ref(), latest_control.as_ref()];
                    let decode = |decoder: &mut VideoDecoderSession, cached| {
                        if force_seek {
                            decoder.seek(position).and_then(|()| {
                                decoder.frame(
                                    position,
                                    cached,
                                    controls,
                                    mode.time_accurate(),
                                    mode.continuous(),
                                )
                            })
                        } else {
                            decoder.frame(
                                position,
                                cached,
                                controls,
                                mode.time_accurate(),
                                mode.continuous(),
                            )
                        }
                    };
                    let mut startup_pressure_failure = false;
                    let mut result = (|| {
                        if decoder.as_ref().is_none_or(|decoder| !decoder.initialized) {
                            let Some(startup) = worker_context.begin_startup(
                                &worker_source,
                                !latest,
                                controls,
                                decoder.as_ref().map_or(0, |decoder| decoder.startup_bytes),
                            )?
                            else {
                                return Ok(DecodeOutcome::Superseded(cached));
                            };
                            if controls
                                .into_iter()
                                .flatten()
                                .any(DecodeControl::superseded)
                            {
                                return Ok(DecodeOutcome::Superseded(cached));
                            }
                            if decoder.is_none() {
                                match VideoDecoderSession::open(&worker_source) {
                                    Ok(opened) => {
                                        shrimply_profiling::increment(
                                            "Temporal decoder / Sessions opened",
                                        );
                                        worker_metadata
                                            .lock()
                                            .expect("video decoder metadata mutex poisoned")
                                            .update(&opened);
                                        decoder = Some(opened);
                                    }
                                    Err(error) => {
                                        startup_pressure_failure =
                                            startup.finish(Some(&error), &mut 0);
                                        return Err(error);
                                    }
                                }
                            } else {
                                shrimply_profiling::increment("Temporal decoder / Startup resumed");
                            }
                            decoder.as_mut().expect("opened decoder missing").startup =
                                Some(startup);
                        }
                        decode(decoder.as_mut().expect("opened decoder missing"), cached)
                    })();
                    // The session releases the gate on its first CUDA frame. Cancellation or
                    // failure before that point releases it here, retaining healthy session state.
                    if let Some(decoder) = decoder.as_mut()
                        && let Some(startup) = decoder.startup.take()
                    {
                        startup_pressure_failure = startup.finish(
                            result.as_ref().err().map(String::as_str),
                            &mut decoder.startup_bytes,
                        );
                    }
                    if startup_pressure_failure
                        && let Err(error) = &result
                        && !error.starts_with(DECODER_STARTUP_MEMORY_EXHAUSTED)
                    {
                        result = Err(format!("{DECODER_STARTUP_MEMORY_EXHAUSTED}: {error}"));
                    }
                    if startup_pressure_failure {
                        decoder = None;
                        *worker_metadata
                            .lock()
                            .expect("video decoder metadata mutex poisoned") = DecoderMetadata {
                            position: None,
                            frame_duration: Time::from_fraction(1, 30),
                        };
                    }
                    if controls
                        .into_iter()
                        .flatten()
                        .any(DecodeControl::superseded)
                    {
                        result = match result {
                            Ok(DecodeOutcome::Frame(frame))
                            | Ok(DecodeOutcome::Superseded(frame)) => {
                                Ok(DecodeOutcome::Superseded(frame))
                            }
                            Err(error) => Err(error),
                        };
                    }
                    if let Some(decoder) = &decoder {
                        worker_metadata
                            .lock()
                            .expect("video decoder metadata mutex poisoned")
                            .update(decoder);
                    }
                    if let Ok(
                        DecodeOutcome::Frame(Some(frame)) | DecodeOutcome::Superseded(Some(frame)),
                    ) = &result
                    {
                        let mut current = worker_current
                            .lock()
                            .expect("video decoder current frame mutex poisoned");
                        replace_current_frame(current.as_ref(), Some(frame));
                        *current = Some(frame.clone());
                    }
                    let recoverable_oom = result.as_ref().err().is_some_and(|error| {
                        error.starts_with(GPU_FRAME_ALLOCATION_EXHAUSTED)
                            || error.starts_with(DECODER_STARTUP_MEMORY_EXHAUSTED)
                    });
                    if let Err(error) = &result {
                        if startup_pressure_failure {
                            tracing::warn!(
                                worker_id,
                                file = %worker_source.asset.path().display(),
                                media_track_id = worker_source.media_track_id,
                                position = %position.as_label(),
                                ?mode,
                                generation = control
                                    .as_ref()
                                    .or(latest_control.as_ref())
                                    .map(DecodeControl::generation),
                                %error,
                                "video decoder initialization hit GPU pressure and can be retried",
                            );
                        } else if recoverable_oom {
                            tracing::warn!(
                                worker_id,
                                file = %worker_source.asset.path().display(),
                                media_track_id = worker_source.media_track_id,
                                position = %position.as_label(),
                                ?mode,
                                generation = control
                                    .as_ref()
                                    .or(latest_control.as_ref())
                                    .map(DecodeControl::generation),
                                %error,
                                "video decoder request ran out of memory and can be retried",
                            );
                        } else {
                            tracing::error!(
                                worker_id,
                                file = %worker_source.asset.path().display(),
                                media_track_id = worker_source.media_track_id,
                                position = %position.as_label(),
                                ?mode,
                                generation = control
                                    .as_ref()
                                    .or(latest_control.as_ref())
                                    .map(DecodeControl::generation),
                                %error,
                                "video decoder request failed",
                            );
                        }
                    }
                    if recoverable_oom {
                        let mut current = worker_current
                            .lock()
                            .expect("video decoder current frame mutex poisoned");
                        replace_current_frame(current.as_ref(), None);
                        current.take();
                    }
                    let fatal_error = result.as_ref().err().filter(|_| !recoverable_oom).cloned();
                    if let Some(reply) = reply {
                        let _ = reply.send(result);
                    }
                    let mut state = worker_inbox
                        .state
                        .lock()
                        .expect("video decoder inbox mutex poisoned");
                    if latest
                        && state.active_latest
                            == Some(LatestTarget {
                                owner,
                                position,
                                mode,
                                generation: control.as_ref().map(DecodeControl::generation),
                            })
                    {
                        state.active_latest = None;
                    }
                    if let Some(error) = fatal_error {
                        shrimply_profiling::increment("Temporal decoder / Worker failed");
                        state.failed = Some(error.clone());
                        fail_pending_work(&mut state, &error);
                        worker_inbox.ready.notify_all();
                        break;
                    }
                }
            })
            .map_err(|error| format!("spawn video decoder worker: {error}"))?;
        Ok(Self {
            frame_size,
            metadata,
            current,
            inbox,
            worker: Some(worker),
        })
    }

    fn submit(
        &mut self,
        position: Time,
        request: VideoDecoderRequest,
        activity: DecoderActivityGuard,
    ) -> Result<PendingDecode, String> {
        let VideoDecoderRequest {
            owner,
            cached,
            control,
            mode,
            latest,
        } = request;
        let maximum_latest_distance = {
            let metadata = self
                .metadata
                .lock()
                .expect("video decoder metadata mutex poisoned");
            Time {
                seconds: metadata.frame_duration.seconds
                    * Fraction::from(MAX_LATEST_REQUEST_DISTANCE_FRAMES),
            }
        };
        let mut state = self
            .inbox
            .state
            .lock()
            .expect("video decoder inbox mutex poisoned");
        if let Some(error) = &state.failed {
            return Err(error.clone());
        }
        if state.shutdown {
            return Err("video decoder worker is shutting down".to_string());
        }
        let generation = control.as_ref().map(DecodeControl::generation);
        if latest {
            let target = LatestTarget {
                owner: owner.clone(),
                position,
                mode,
                generation,
            };
            if state.latest.as_ref().is_some_and(|work| {
                work.owner == owner
                    && work.position == position
                    && work.mode == mode
                    && work.control.as_ref().map(DecodeControl::generation) == generation
            }) || (state.latest.is_none()
                && state
                    .active_latest
                    .as_ref()
                    .is_some_and(|active| active.same_request(&target)))
            {
                shrimply_profiling::increment("Temporal decoder / Latest requests coalesced");
                return Ok(PendingDecode {
                    response: None,
                    _activity: None,
                });
            }
            let previous = state
                .active_latest
                .clone()
                .or_else(|| {
                    state.latest.as_ref().map(|work| LatestTarget {
                        owner: work.owner.clone(),
                        position: work.position,
                        mode: work.mode,
                        generation: work.control.as_ref().map(DecodeControl::generation),
                    })
                })
                .or_else(|| state.last_request.clone());
            let adjacent = previous
                .is_some_and(|previous| previous.continues(&target, maximum_latest_distance));
            if !adjacent {
                state.revision = state.revision.wrapping_add(1);
            }
            let revision = state.revision;
            self.inbox.revision.store(revision, Ordering::Release);
            state.latest = Some(DecoderWork {
                owner,
                position,
                cached,
                control,
                mode,
                force_seek: !adjacent,
                revision,
                reply: None,
                _activity: Some(activity),
            });
            state.last_request = Some(target);
            shrimply_profiling::increment("Temporal decoder / Latest requests submitted");
            self.inbox.ready.notify_one();
            return Ok(PendingDecode {
                response: None,
                _activity: None,
            });
        }
        if state.exact.is_some() {
            return Err("video decoder already has an exact request queued".to_string());
        }
        let (reply, response) = mpsc::sync_channel(1);
        state.revision = state.revision.wrapping_add(1);
        let revision = state.revision;
        self.inbox.revision.store(revision, Ordering::Release);
        state.latest = None;
        state.exact = Some(DecoderWork {
            owner: owner.clone(),
            position,
            cached,
            control,
            mode,
            force_seek: false,
            revision,
            reply: Some(reply),
            _activity: None,
        });
        state.last_request = Some(LatestTarget {
            owner,
            position,
            mode,
            generation,
        });
        self.inbox.ready.notify_one();
        Ok(PendingDecode {
            response: Some(response),
            _activity: Some(activity),
        })
    }
}

fn fail_pending_work(state: &mut DecoderInboxState, error: &str) {
    for work in [state.exact.take(), state.latest.take()]
        .into_iter()
        .flatten()
    {
        if let Some(reply) = work.reply {
            let _ = reply.send(Err(error.to_string()));
        }
    }
}

impl Drop for PooledVideoDecoder {
    fn drop(&mut self) {
        let _measurement = shrimply_profiling::measure("Temporal decoder / Retirement time");
        {
            let mut state = self
                .inbox
                .state
                .lock()
                .expect("video decoder inbox mutex poisoned");
            state.shutdown = true;
            state.revision = state.revision.wrapping_add(1);
            self.inbox.revision.store(state.revision, Ordering::Release);
            self.inbox.ready.notify_all();
        }
        self.worker
            .take()
            .expect("video decoder worker missing during shutdown")
            .join()
            .expect("video decoder worker panicked during shutdown");
        let mut current = self
            .current
            .lock()
            .expect("video decoder current frame mutex poisoned");
        replace_current_frame(current.as_ref(), None);
        current.take();
    }
}

pub struct VideoDecoderPool {
    decoders: Arc<TemporalDecoderPool<VideoSource, VideoDecoderOwner, PooledVideoDecoder>>,
    consumer: u64,
}

impl Default for VideoDecoderPool {
    fn default() -> Self {
        Self::new(DEFAULT_VIDEO_DECODER_POOL_SIZE)
    }
}

impl VideoDecoderPool {
    pub fn new(maximum_decoders: usize) -> Self {
        static DECODERS: OnceLock<
            Arc<TemporalDecoderPool<VideoSource, VideoDecoderOwner, PooledVideoDecoder>>,
        > = OnceLock::new();
        let decoders = DECODERS
            .get_or_init(|| {
                Arc::new(TemporalDecoderPool::new(
                    DEFAULT_VIDEO_DECODER_POOL_SIZE,
                    VideoDecoderContext::default(),
                ))
            })
            .clone();
        decoders.set_maximum(maximum_decoders);
        Self {
            decoders,
            consumer: NEXT_TEMPORAL_CONSUMER_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub fn configure(&mut self, maximum_decoders: usize) {
        self.decoders.set_maximum(maximum_decoders);
    }

    pub fn has_request_capacity(&self, reserved: usize) -> bool {
        self.decoders.has_activity_capacity(reserved)
    }

    pub fn owner(
        &self,
        sequence_path: &[Uuid],
        track_id: Uuid,
        item_id: Uuid,
        plane: VideoPlane,
    ) -> VideoDecoderOwner {
        VideoDecoderOwner {
            consumer: self.consumer,
            sequence_path: sequence_path.to_vec(),
            track_id,
            item_id,
            plane,
        }
    }

    pub fn decoder(
        &mut self,
        item: &VideoItem,
        owner: VideoDecoderOwner,
    ) -> Result<VideoDecoderHandle, String> {
        let source = VideoSource::for_item(item)?;
        Ok(VideoDecoderHandle {
            decoders: self.decoders.clone(),
            frame_size: CanvasSize {
                width: source.width,
                height: source.height,
            },
            frame_duration: Time::from_fraction(1, 30),
            source,
            owner,
        })
    }

    pub fn can_handoff(
        &self,
        sequence_path: &[Uuid],
        track_id: Uuid,
        previous: &VideoItem,
        next: &VideoItem,
        plane: VideoPlane,
    ) -> bool {
        if previous.end != next.start
            || playback_speed_is_zero(previous.playback_speed)
            || playback_speed_is_negative(previous.playback_speed)
            || playback_speed_is_zero(next.playback_speed)
            || playback_speed_is_negative(next.playback_speed)
            || previous
                .transitions
                .to_next
                .as_ref()
                .is_some_and(|transition| {
                    transition.target_item_id == next.id && transition.duration > Time::ZERO
                })
        {
            return false;
        }
        let Ok(Some(previous_source)) = VideoSource::for_plane(previous, plane) else {
            return false;
        };
        let Ok(Some(next_source)) = VideoSource::for_plane(next, plane) else {
            return false;
        };
        if previous_source != next_source {
            return false;
        }
        let previous_owner = self.owner(sequence_path, track_id, previous.id, plane);
        let Some(metadata) = self
            .decoders
            .metadata_for_owner(&previous_source, &previous_owner)
        else {
            return false;
        };
        let Some(target) = video_source_time_at(next, next.start) else {
            return false;
        };
        let Some(decoded) = metadata.position else {
            return false;
        };
        if target.saturating_add(metadata.frame_duration) < decoded {
            return false;
        }
        let maximum_gap = Time {
            seconds: metadata.frame_duration.seconds * Fraction::from(MAX_HANDOFF_FORWARD_FRAMES),
        };
        target.saturating_sub(decoded) <= maximum_gap
    }

    pub fn prepare(&mut self, item: &VideoItem, owner: VideoDecoderOwner) -> Result<bool, String> {
        let source = VideoSource::for_item(item)?;
        if self.decoders.contains_owner(&source, &owner) {
            return Ok(true);
        }
        let prepared = self.decoders.prepare(source, owner, 0)?;
        shrimply_profiling::increment(if prepared {
            "Temporal decoder / Prewarm accepted"
        } else {
            "Temporal decoder / Prewarm skipped at capacity"
        });
        Ok(prepared)
    }

    pub fn retain<'a>(
        &mut self,
        items: impl IntoIterator<Item = (&'a [Uuid], Uuid, &'a VideoItem)>,
    ) {
        let mut retained = HashMap::new();
        for (sequence_path, track_id, item) in items {
            if !matches!(
                item.content,
                shrimply_project_document::project::VideoItemContent::Media
            ) {
                continue;
            }
            if let Ok(source) = VideoSource::for_item(item) {
                retained.insert(
                    self.owner(sequence_path, track_id, item.id, VideoPlane::Color),
                    source,
                );
            }
            if let Ok(Some(source)) = VideoSource::for_plane(item, VideoPlane::Alpha) {
                retained.insert(
                    self.owner(sequence_path, track_id, item.id, VideoPlane::Alpha),
                    source,
                );
            }
        }
        let consumer = self.consumer;
        self.decoders.retain_owners(|owner, source| {
            owner.consumer != consumer || retained.get(owner) == Some(source)
        });
    }

    pub fn session_count(&self) -> usize {
        self.decoders.len()
    }

    pub fn reclaim_idle(&mut self) {
        self.decoders.reclaim_idle();
    }

    pub fn retire_idle(&mut self) {
        self.decoders.retire_idle();
    }
}

impl Drop for VideoDecoderPool {
    fn drop(&mut self) {
        let consumer = self.consumer;
        self.decoders
            .retain_owners(|owner, _| owner.consumer != consumer);
    }
}

pub struct VideoDecoderHandle {
    decoders: Arc<TemporalDecoderPool<VideoSource, VideoDecoderOwner, PooledVideoDecoder>>,
    source: VideoSource,
    owner: VideoDecoderOwner,
    frame_size: CanvasSize,
    frame_duration: Time,
}

pub struct DecodeRequest {
    handoff_from: Option<VideoDecoderOwner>,
    position: Time,
    control: Option<DecodeControl>,
    mode: DecodeMode,
}

impl DecodeRequest {
    pub fn best_effort(position: Time) -> Self {
        Self::new(position, DecodeMode::BestEffort)
    }

    pub fn accurate(position: Time) -> Self {
        Self::new(position, DecodeMode::Accurate)
    }

    pub fn continuous(position: Time) -> Self {
        Self::new(position, DecodeMode::Continuous)
    }

    pub fn local_scrub(position: Time) -> Self {
        Self::new(position, DecodeMode::LocalScrub)
    }

    fn new(position: Time, mode: DecodeMode) -> Self {
        Self {
            handoff_from: None,
            position,
            control: None,
            mode,
        }
    }

    pub fn handoff_from(mut self, owner: Option<VideoDecoderOwner>) -> Self {
        self.handoff_from = owner;
        self
    }

    pub fn control(mut self, control: Option<DecodeControl>) -> Self {
        self.control = control;
        self
    }
}

impl VideoDecoderHandle {
    pub fn matches(&self, item: &VideoItem) -> bool {
        VideoSource::for_item(item).is_ok_and(|source| self.source == source)
    }

    pub fn current(&self) -> Option<DecodedVisual> {
        self.decoders.current_for_owner(&self.source, &self.owner)
    }

    pub fn frame_duration(&self) -> Time {
        self.decoders
            .metadata_for_owner(&self.source, &self.owner)
            .map_or(self.frame_duration, |metadata| metadata.frame_duration)
    }

    pub fn frame_size(&self) -> CanvasSize {
        self.decoders
            .metadata_for_owner(&self.source, &self.owner)
            .map_or(self.frame_size, |metadata| metadata.frame_size)
    }

    pub fn touch_foreground(&self) {
        self.decoders.touch_foreground(&self.source, &self.owner);
    }

    pub fn request(&self, request: DecodeRequest) -> Result<PendingDecode, String> {
        self.decoders.request(
            self.source.clone(),
            self.owner.clone(),
            request.handoff_from,
            request.position,
            true,
            VideoDecoderRequest {
                owner: self.owner.clone(),
                cached: self.current(),
                control: request.control,
                mode: request.mode,
                latest: false,
            },
        )
    }

    pub fn try_request(
        &self,
        request: DecodeRequest,
        foreground: bool,
    ) -> Result<Option<PendingDecode>, String> {
        self.decoders.try_request(
            self.source.clone(),
            self.owner.clone(),
            request.handoff_from,
            request.position,
            foreground,
            VideoDecoderRequest {
                owner: self.owner.clone(),
                cached: self.current(),
                control: request.control,
                mode: request.mode,
                latest: false,
            },
        )
    }

    pub fn try_latest(&self, request: DecodeRequest, foreground: bool) -> Result<bool, String> {
        let _measurement = shrimply_profiling::measure("Temporal decoder / Real-time submission");
        self.decoders
            .try_request(
                self.source.clone(),
                self.owner.clone(),
                request.handoff_from,
                request.position,
                foreground,
                VideoDecoderRequest {
                    owner: self.owner.clone(),
                    cached: self.current(),
                    control: request.control,
                    mode: request.mode,
                    latest: true,
                },
            )
            .map(|request| request.is_some())
    }
}

fn update_temporal_frame_counter() {
    shrimply_profiling::set_counter(
        "Temporal decoder state / GPU bytes retained",
        TEMPORAL_CURRENT_BYTES.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "Temporal decoder state / GPU frames retained",
        TEMPORAL_CURRENT_FRAMES.load(Ordering::Acquire),
    );
}

fn replace_current_frame(previous: Option<&DecodedVisual>, current: Option<&DecodedVisual>) {
    replace_atomic_count(
        &TEMPORAL_CURRENT_BYTES,
        previous.map_or(0, |(_, frame)| frame.bytes()),
        current.map_or(0, |(_, frame)| frame.bytes()),
    );
    replace_atomic_count(
        &TEMPORAL_CURRENT_FRAMES,
        u64::from(previous.is_some()),
        u64::from(current.is_some()),
    );
    update_temporal_frame_counter();
}

fn replace_atomic_count(counter: &AtomicU64, previous: u64, current: u64) {
    if current >= previous {
        counter.fetch_add(current - previous, Ordering::AcqRel);
    } else {
        counter.fetch_sub(previous - current, Ordering::AcqRel);
    }
}
