use super::*;
use std::sync::mpsc::{Receiver, TryRecvError};

use shrimply_preview_core::accuracy::{FINAL_PREVIEW_DELAY, LOCAL_SCRUB_WINDOW_SECONDS};
const SETTLED_RENDER_RETRY_DELAY: Duration = Duration::from_millis(100);
type SettledRender = (Instant, u64, Time, CompositeAccuracy);

pub struct PreviewMediaUpdate {
    pub visual: Option<VideoEvent>,
    pub render_loading: Option<bool>,
    pub render_elapsed: Option<Duration>,
    pub running: bool,
}

#[derive(Clone, Copy)]
pub enum StepDirection {
    Backward,
    Forward,
}

#[derive(Clone)]
pub struct PreviewMedia {
    video_tx: VideoCommandSender,
    video_rx: Rc<RefCell<Receiver<VideoEvent>>>,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    expected_revision: Rc<Cell<u64>>,
    step_direction: Rc<Cell<Option<StepDirection>>>,
    settled: Rc<RefCell<Option<SettledRender>>>,
    retry: Rc<RefCell<Option<(Instant, Time)>>>,
    generation: Rc<Cell<u64>>,
    stopped: Rc<Cell<bool>>,
}

impl PreviewMedia {
    pub fn new(
        project: Rc<RefCell<Project>>,
        player_state: SharedPlayerState,
        playback_performance: playback_performance::SharedCollector,
        preferences: preferences_store::SharedPreferences,
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
        let (video_tx, video_rx) = compositor::spawn_worker_with_resources_and_observer(
            initial_project,
            resource_config,
            Some(observer),
        );
        let configured_resources = Rc::new(Cell::new(resource_config));
        let preference_video_tx = video_tx.clone();
        preferences_store::connect(&preferences, move |snapshot| {
            shrimply_audio::pneuma::set_server_url(&snapshot.compute_server_url);
            let config = RenderResourceConfig {
                maximum_temporal_decoders: snapshot.temporal_decoder_pool_size as usize,
                gpu_host_memory_gib: snapshot.gpu_host_memory_gib,
            };
            if configured_resources.replace(config) != config {
                send(
                    &preference_video_tx,
                    VideoCommand::ConfigureResources(config),
                );
            }
        });

        let snapshot = player_state::snapshot(&player_state);
        let media = Self {
            video_tx,
            video_rx: Rc::new(RefCell::new(video_rx)),
            project: project.clone(),
            player_state: player_state.clone(),
            expected_revision: Rc::new(Cell::new(snapshot.revision)),
            step_direction: Rc::new(Cell::new(None)),
            settled: Rc::new(RefCell::new(None)),
            retry: Rc::new(RefCell::new(None)),
            generation: Rc::new(Cell::new(0)),
            stopped: Rc::new(Cell::new(false)),
        };
        media.connect(project, player_state);
        send(
            &media.video_tx,
            VideoCommand::Render {
                position: snapshot.position,
                accuracy: CompositeAccuracy::FULLY_ACCURATE,
            },
        );
        media
    }

    pub fn sender(&self) -> VideoCommandSender {
        self.video_tx.clone()
    }

    pub fn step_direction(&self) -> Rc<Cell<Option<StepDirection>>> {
        self.step_direction.clone()
    }

    pub fn mark_step(&self, direction: StepDirection) {
        self.step_direction.set(Some(direction));
    }

    pub fn poll(&self) -> PreviewMediaUpdate {
        self.poll_deferred_renders();
        let mut update = PreviewMediaUpdate {
            visual: None,
            render_loading: None,
            render_elapsed: None,
            running: true,
        };
        loop {
            let event = match self.video_rx.borrow().try_recv() {
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
                    .video_tx
                    .render_generation_is_current(render_generation) =>
                {
                    update.render_loading = Some(show_spinner);
                    update.render_elapsed = Some(render_elapsed);
                    let mut retry = self.retry.borrow_mut();
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
                    .video_tx
                    .render_generation_is_current(render_generation) =>
                {
                    update.render_loading = Some(!settled);
                    update.render_elapsed = Some(render_elapsed);
                    if revision == self.expected_revision.get() {
                        update.visual = Some(event);
                    }
                }
                event @ VideoEvent::Clear {
                    revision,
                    render_elapsed,
                    render_generation,
                    ..
                } if self
                    .video_tx
                    .render_generation_is_current(render_generation) =>
                {
                    update.render_loading = Some(false);
                    update.render_elapsed = Some(render_elapsed);
                    if revision == self.expected_revision.get() {
                        update.visual = Some(event);
                    }
                }
                VideoEvent::Manim(update) => {
                    shrimply_state::manim_status::apply(&self.project, &self.player_state, update)
                }
                VideoEvent::Error(error) => {
                    tracing::error!(%error, "video compositor error");
                    update.render_loading = Some(false);
                }
                _ => {}
            }
        }
        update
    }

    pub fn stop(&self) {
        if !self.stopped.replace(true) {
            send(&self.video_tx, VideoCommand::Stop);
        }
    }

    fn poll_deferred_renders(&self) {
        let settled = *self.settled.borrow();
        if let Some((deadline, generation, position, accuracy)) = settled
            && Instant::now() >= deadline
        {
            self.settled.borrow_mut().take();
            if self.generation.get() == generation {
                send(&self.video_tx, VideoCommand::Render { position, accuracy });
            }
        }
        let retry = *self.retry.borrow();
        if let Some((deadline, position)) = retry
            && Instant::now() >= deadline
        {
            self.retry.borrow_mut().take();
            let snapshot = player_state::snapshot(&self.player_state);
            if !snapshot.playing && snapshot.position == position {
                send(
                    &self.video_tx,
                    VideoCommand::Render {
                        position,
                        accuracy: CompositeAccuracy::FULLY_ACCURATE,
                    },
                );
            }
        }
    }

    fn connect(&self, project: Rc<RefCell<Project>>, player_state: SharedPlayerState) {
        let media = self.clone();
        let last_snapshot = player_state::snapshot(&player_state);
        let last_position = Rc::new(Cell::new(last_snapshot.position));
        let last_playing = Rc::new(Cell::new(last_snapshot.playing));
        let listener_state = player_state.clone();
        player_state::connect_named(&player_state, "preview media sync", move |event| {
            let snapshot = player_state::snapshot(&listener_state);
            let position_changed = last_position.get() != snapshot.position;
            let playing_changed = last_playing.get() != snapshot.playing;
            let step = if position_changed && !snapshot.playing {
                media.step_direction.replace(None)
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
                let next_project = Arc::new(project.borrow().clone());
                send(
                    &media.video_tx,
                    VideoCommand::set_project(next_project, snapshot.revision),
                );
                media.expected_revision.set(snapshot.revision);
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
                } else if project_change.is_some_and(|change| change.video && change.live_preview) {
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
                let generation = media.generation.get().wrapping_add(1);
                media.generation.set(generation);
                send(
                    &media.video_tx,
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
                    *media.settled.borrow_mut() = Some((
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
        });
    }
}

fn send(sender: &VideoCommandSender, command: VideoCommand) {
    if let Err(error) = sender.send(command) {
        tracing::warn!(%error, "could not send video compositor command");
    }
}
