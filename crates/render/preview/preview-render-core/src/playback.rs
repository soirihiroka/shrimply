use crate::{
    CompositeAccuracy,
    worker::{
        PlaybackRenderObserver, RenderResourceConfig, VideoCommand, VideoCommandSender, VideoEvent,
    },
};
use shrimply_editor_state::player_state::{self, SharedPlayerState};
use shrimply_playback_performance as playback_performance;
use shrimply_preferences_core as preferences_store;
use shrimply_project_document::project::{Project, Time};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use shrimply_preview_provider_skia::accuracy::{FINAL_PREVIEW_DELAY, LOCAL_SCRUB_WINDOW_SECONDS};
const SETTLED_RENDER_RETRY_DELAY: Duration = Duration::from_millis(100);
type SettledRender = (Instant, u64, Time, CompositeAccuracy);

pub struct PreviewMediaUpdate<F> {
    pub visual: Option<VideoEvent<F>>,
    pub render_loading: Option<bool>,
    pub render_elapsed: Option<Duration>,
    pub running: bool,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy)]
pub enum StepDirection {
    Backward,
    Forward,
}

pub struct PreviewMedia<F> {
    inner: Rc<State<F>>,
}

struct State<F> {
    video_tx: VideoCommandSender,
    video_rx: RefCell<Receiver<VideoEvent<F>>>,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    expected_revision: Cell<u64>,
    step_direction: Rc<Cell<Option<StepDirection>>>,
    settled: RefCell<Option<SettledRender>>,
    retry: RefCell<Option<(Instant, Time)>>,
    generation: Cell<u64>,
    stopped: Cell<bool>,
}

impl<F> Clone for PreviewMedia<F> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<F> Drop for State<F> {
    fn drop(&mut self) {
        if !self.stopped.replace(true) {
            send(&self.video_tx, VideoCommand::Stop);
        }
    }
}

impl<F: 'static> PreviewMedia<F> {
    pub fn new(
        project: Rc<RefCell<Project>>,
        player_state: SharedPlayerState,
        playback_performance: playback_performance::SharedCollector,
        preferences: preferences_store::SharedPreferences,
        spawn: impl FnOnce(
            Project,
            RenderResourceConfig,
            Option<PlaybackRenderObserver>,
        ) -> (VideoCommandSender, Receiver<VideoEvent<F>>),
    ) -> Self {
        let initial_project = project.borrow().clone();
        let preference = preferences_store::snapshot(&preferences);
        let resource_config = RenderResourceConfig {
            maximum_temporal_decoders: preference.temporal_decoder_pool_size as usize,
            gpu_host_memory_gib: preference.gpu_host_memory_gib,
        };
        let performance_observer = playback_performance.clone();
        let observer = Arc::new(move |event| {
            playback_performance::record_render_event(&performance_observer, event);
        });
        let (video_tx, video_rx) = spawn(initial_project, resource_config, Some(observer));
        let snapshot = player_state::snapshot(&player_state);
        let media = Self {
            inner: Rc::new(State {
                video_tx,
                video_rx: RefCell::new(video_rx),
                project: project.clone(),
                player_state: player_state.clone(),
                expected_revision: Cell::new(snapshot.revision),
                step_direction: Rc::new(Cell::new(None)),
                settled: RefCell::new(None),
                retry: RefCell::new(None),
                generation: Cell::new(0),
                stopped: Cell::new(false),
            }),
        };
        let configured_resources = Cell::new(resource_config);
        let preference_media = Rc::downgrade(&media.inner);
        preferences_store::connect(&preferences, move |snapshot| {
            let Some(media) = preference_media
                .upgrade()
                .filter(|state| !state.stopped.get())
            else {
                return;
            };
            shrimply_audio_engine::pneuma::set_server_url(&snapshot.compute_server_url);
            let config = RenderResourceConfig {
                maximum_temporal_decoders: snapshot.temporal_decoder_pool_size as usize,
                gpu_host_memory_gib: snapshot.gpu_host_memory_gib,
            };
            if configured_resources.replace(config) != config {
                send(&media.video_tx, VideoCommand::ConfigureResources(config));
            }
        });
        media.connect();
        send(
            &media.inner.video_tx,
            VideoCommand::Render {
                position: snapshot.position,
                accuracy: CompositeAccuracy::FULLY_ACCURATE,
            },
        );
        media
    }

    pub fn sender(&self) -> VideoCommandSender {
        self.inner.video_tx.clone()
    }

    pub fn step_direction(&self) -> Rc<Cell<Option<StepDirection>>> {
        self.inner.step_direction.clone()
    }

    pub fn mark_step(&self, direction: StepDirection) {
        self.inner.step_direction.set(Some(direction));
    }

    pub fn poll(&self) -> PreviewMediaUpdate<F> {
        self.poll_deferred_renders();
        let mut update = PreviewMediaUpdate {
            visual: None,
            render_loading: None,
            render_elapsed: None,
            running: true,
            errors: Vec::new(),
        };
        loop {
            let event = match self.inner.video_rx.borrow().try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    update.running = false;
                    break;
                }
            };
            match event {
                VideoEvent::Loading {
                    position,
                    show_spinner,
                    render_elapsed,
                    render_generation,
                } if self
                    .inner
                    .video_tx
                    .render_generation_is_current(render_generation) =>
                {
                    update.render_loading = Some(show_spinner);
                    update.render_elapsed = Some(render_elapsed);
                    let mut retry = self.inner.retry.borrow_mut();
                    if retry.is_none() {
                        *retry = Some((Instant::now() + SETTLED_RENDER_RETRY_DELAY, position));
                    }
                }
                event @ VideoEvent::Frame {
                    revision,
                    settled,
                    render_elapsed,
                    render_generation,
                    ..
                } if self
                    .inner
                    .video_tx
                    .render_generation_is_current(render_generation) =>
                {
                    update.render_loading = Some(!settled);
                    update.render_elapsed = Some(render_elapsed);
                    if revision == self.inner.expected_revision.get() {
                        update.visual = Some(event);
                    }
                }
                event @ VideoEvent::Clear {
                    revision,
                    render_elapsed,
                    render_generation,
                    ..
                } if self
                    .inner
                    .video_tx
                    .render_generation_is_current(render_generation) =>
                {
                    update.render_loading = Some(false);
                    update.render_elapsed = Some(render_elapsed);
                    if revision == self.inner.expected_revision.get() {
                        update.visual = Some(event);
                    }
                }
                VideoEvent::Manim(update) => shrimply_editor_state::manim_status::apply(
                    &self.inner.project,
                    &self.inner.player_state,
                    update,
                ),
                VideoEvent::Error(error) => {
                    tracing::error!(%error, "video compositor error");
                    update.render_loading = Some(false);
                    update.errors.push(error);
                }
                _ => {}
            }
        }
        update
    }

    pub fn stop(&self) {
        if !self.inner.stopped.replace(true) {
            send(&self.inner.video_tx, VideoCommand::Stop);
        }
    }

    pub fn next_update(&self) -> Option<Duration> {
        let now = Instant::now();
        [
            self.inner.settled.borrow().map(|(deadline, ..)| deadline),
            self.inner.retry.borrow().map(|(deadline, _)| deadline),
        ]
        .into_iter()
        .flatten()
        .map(|deadline| deadline.saturating_duration_since(now))
        .min()
    }

    fn poll_deferred_renders(&self) {
        let settled = *self.inner.settled.borrow();
        if let Some((deadline, generation, position, accuracy)) = settled
            && Instant::now() >= deadline
        {
            self.inner.settled.borrow_mut().take();
            if self.inner.generation.get() == generation {
                send(
                    &self.inner.video_tx,
                    VideoCommand::Render { position, accuracy },
                );
            }
        }
        let retry = *self.inner.retry.borrow();
        if let Some((deadline, position)) = retry
            && Instant::now() >= deadline
        {
            self.inner.retry.borrow_mut().take();
            let snapshot = player_state::snapshot(&self.inner.player_state);
            if !snapshot.playing && snapshot.position == position {
                send(
                    &self.inner.video_tx,
                    VideoCommand::Render {
                        position,
                        accuracy: CompositeAccuracy::FULLY_ACCURATE,
                    },
                );
            }
        }
    }

    fn connect(&self) {
        let player_state = &self.inner.player_state;
        let alive = Rc::downgrade(&self.inner);
        let weak = alive.clone();
        let last_snapshot = player_state::snapshot(player_state);
        let last_position = Rc::new(Cell::new(last_snapshot.position));
        let last_playing = Rc::new(Cell::new(last_snapshot.playing));
        player_state::connect_while_alive_named(
            player_state,
            "preview media sync",
            move || alive.upgrade().is_some_and(|state| !state.stopped.get()),
            move |event| {
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                if inner.stopped.get() {
                    return;
                }
                let media = Self { inner };
                let snapshot = player_state::snapshot(&media.inner.player_state);
                let position_changed = last_position.get() != snapshot.position;
                let playing_changed = last_playing.get() != snapshot.playing;
                let step = if position_changed && !snapshot.playing {
                    media.inner.step_direction.replace(None)
                } else {
                    None
                };
                let project_change = match event {
                    player_state::PlayerEvent::Project(change) => Some(change),
                    player_state::PlayerEvent::State(_) => None,
                };
                let natural_playback = matches!(
                    event,
                    player_state::PlayerEvent::State(player_state::StateChange {
                        position: Some(player_state::PositionChange::Playback),
                        ..
                    })
                );
                if let Some(change) = project_change
                    && (change.audio || change.video)
                {
                    let next_project = Arc::new(media.inner.project.borrow().clone());
                    send(
                        &media.inner.video_tx,
                        VideoCommand::set_project(next_project, snapshot.revision),
                    );
                    media.inner.expected_revision.set(snapshot.revision);
                }
                if position_changed
                    || playing_changed
                    || project_change.is_some_and(|change| change.video || change.audio)
                {
                    let previous_position = last_position.get();
                    let local_scrub = position_changed
                        && !snapshot.playing
                        && !project_change.is_some_and(|change| change.video)
                        && snapshot
                            .position
                            .max(previous_position)
                            .saturating_sub(snapshot.position.min(previous_position))
                            <= Time::from_seconds(LOCAL_SCRUB_WINDOW_SECONDS);
                    let accuracy = if snapshot.playing {
                        if position_changed && !natural_playback && !playing_changed {
                            CompositeAccuracy::TIME_ACCURATE
                        } else {
                            CompositeAccuracy::CONTINUOUS_TIME_ACCURATE
                        }
                    } else if project_change
                        .is_some_and(|change| change.video && change.live_preview)
                    {
                        CompositeAccuracy::CONTINUOUS_TIME_ACCURATE
                    } else if project_change.is_some_and(|change| change.video) {
                        CompositeAccuracy::BEST_EFFORT
                    } else if position_changed && step.is_none() {
                        if local_scrub {
                            CompositeAccuracy::LOCAL_TIME_ACCURATE
                        } else {
                            CompositeAccuracy::BEST_EFFORT
                        }
                    } else if local_scrub {
                        CompositeAccuracy::LOCAL_FULLY_ACCURATE
                    } else {
                        CompositeAccuracy::FULLY_ACCURATE
                    };
                    let generation = media.inner.generation.get().wrapping_add(1);
                    media.inner.generation.set(generation);
                    send(
                        &media.inner.video_tx,
                        VideoCommand::Render {
                            position: snapshot.position,
                            accuracy,
                        },
                    );
                    if !accuracy.content_accurate() && !snapshot.playing {
                        let final_accuracy = if accuracy.local_scrub() {
                            CompositeAccuracy::LOCAL_FULLY_ACCURATE
                        } else {
                            CompositeAccuracy::FULLY_ACCURATE
                        };
                        *media.inner.settled.borrow_mut() = Some((
                            Instant::now() + FINAL_PREVIEW_DELAY,
                            generation,
                            snapshot.position,
                            final_accuracy,
                        ));
                    }
                }
                if position_changed || playing_changed {
                    last_position.set(snapshot.position);
                    last_playing.set(snapshot.playing);
                }
            },
        );
    }
}

fn send(sender: &VideoCommandSender, command: VideoCommand) {
    if let Err(error) = sender.send(command) {
        tracing::warn!(%error, "could not send video compositor command");
    }
}
