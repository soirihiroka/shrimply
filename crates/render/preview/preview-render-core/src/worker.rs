//! Shared preview request ordering, cancellation, publication and worker ownership.
use crate::CompositeAccuracy;
use shrimply_math_core::{Fraction, Time};
use shrimply_project_document::project::Project;
use shrimply_project_evaluation::FrameAudioAnalysis;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};
use uuid::Uuid;

const PREVIEW_DISPLAY_SURFACES: usize = 2;

pub enum VideoCommand {
    SetProject {
        project: Arc<Project>,
        revision: u64,
    },
    SetPreviewExclusion(Option<Uuid>),
    ConfigureResources(RenderResourceConfig),
    Render {
        position: Time,
        accuracy: CompositeAccuracy,
    },
    Stop,
}

impl VideoCommand {
    pub fn set_project(project: Arc<Project>, revision: u64) -> Self {
        Self::SetProject { project, revision }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderResourceConfig {
    pub maximum_temporal_decoders: usize,
    pub gpu_host_memory_gib: Fraction,
}

impl Default for RenderResourceConfig {
    fn default() -> Self {
        Self {
            maximum_temporal_decoders: shrimply_video_decoder::DEFAULT_VIDEO_DECODER_POOL_SIZE,
            gpu_host_memory_gib: shrimply_preferences_core::physical_system_memory_gib()
                / Fraction::from(2_u8),
        }
    }
}

pub type PendingProject = Arc<Mutex<Option<(Arc<Project>, u64, u64)>>>;

#[derive(Clone)]
pub struct VideoCommandSender {
    sender: Sender<WorkerCommand>,
    next_request_id: Arc<AtomicU64>,
    cancel_generation: Arc<AtomicU64>,
    last_render: Arc<Mutex<Option<CompositeAccuracy>>>,
    latest_project_generation: Arc<AtomicU64>,
    pending_project: PendingProject,
    project_notification_pending: Arc<AtomicBool>,
    playback_observer: Option<PlaybackRenderObserver>,
}

pub use shrimply_preview_provider_skia::performance::{
    RenderEvent as PlaybackRenderEvent, RenderObserver as PlaybackRenderObserver,
};

impl VideoCommandSender {
    pub fn render_generation_is_current(&self, generation: u64) -> bool {
        self.cancel_generation.load(Ordering::Acquire) == generation
    }

    pub fn send(&self, command: VideoCommand) -> Result<(), mpsc::SendError<Box<VideoCommand>>> {
        let worker_command = match command {
            VideoCommand::SetProject { project, revision } => {
                self.cancel_generation.fetch_add(1, Ordering::AcqRel);
                self.last_render
                    .lock()
                    .expect("last video render mutex poisoned")
                    .take();
                let mut pending = self
                    .pending_project
                    .lock()
                    .expect("pending video project mutex poisoned");
                let project_generation = self
                    .latest_project_generation
                    .fetch_add(1, Ordering::AcqRel)
                    + 1;
                let previous = pending.replace((project, revision, project_generation));
                if self
                    .project_notification_pending
                    .swap(true, Ordering::AcqRel)
                {
                    drop(pending);
                    drop(previous);
                    return Ok(());
                }
                if self.sender.send(WorkerCommand::SetProject).is_err() {
                    self.project_notification_pending
                        .store(false, Ordering::Release);
                    let (project, revision, _) = pending
                        .take()
                        .expect("pending video project disappeared while sending notification");
                    *pending = previous;
                    return Err(mpsc::SendError(Box::new(VideoCommand::SetProject {
                        project,
                        revision,
                    })));
                }
                drop(pending);
                drop(previous);
                return Ok(());
            }
            VideoCommand::SetPreviewExclusion(item_id) => {
                self.cancel_generation.fetch_add(1, Ordering::AcqRel);
                WorkerCommand::SetPreviewExclusion(item_id)
            }
            VideoCommand::Render { position, accuracy } => {
                let request_id = self.next_request_id.fetch_add(1, Ordering::AcqRel) + 1;
                let mut previous = self
                    .last_render
                    .lock()
                    .expect("last video render mutex poisoned");
                let continuous = previous.is_some_and(|previous| previous == accuracy)
                    && accuracy.continuous_playback();
                if !continuous {
                    self.cancel_generation.fetch_add(1, Ordering::AcqRel);
                }
                *previous = Some(accuracy);
                let cancel_generation = self.cancel_generation.load(Ordering::Acquire);
                if accuracy.continuous_playback()
                    && let Some(observer) = &self.playback_observer
                {
                    observer(PlaybackRenderEvent::Requested {
                        request_id,
                        position,
                    });
                }
                WorkerCommand::Render {
                    position,
                    accuracy,
                    request_id,
                    cancel_generation,
                }
            }
            VideoCommand::ConfigureResources(config) => {
                self.cancel_generation.fetch_add(1, Ordering::AcqRel);
                WorkerCommand::ConfigureResources(config)
            }
            VideoCommand::Stop => {
                self.cancel_generation.fetch_add(1, Ordering::AcqRel);
                WorkerCommand::Stop
            }
        };
        self.sender.send(worker_command).map_err(|error| {
            mpsc::SendError(Box::new(match error.0 {
                WorkerCommand::SourcesChanged => unreachable!("source notifications are internal"),
                WorkerCommand::SetProject => {
                    unreachable!("project notifications return before common send handling")
                }
                WorkerCommand::SetPreviewExclusion(item_id) => {
                    VideoCommand::SetPreviewExclusion(item_id)
                }
                WorkerCommand::Render {
                    position, accuracy, ..
                } => VideoCommand::Render { position, accuracy },
                WorkerCommand::ConfigureResources(config) => {
                    VideoCommand::ConfigureResources(config)
                }
                WorkerCommand::ScheduleSam2Analysis => {
                    unreachable!("SAM2 scheduling is an internal compositor command")
                }
                WorkerCommand::Stop => VideoCommand::Stop,
            }))
        })
    }
}

pub enum WorkerCommand {
    SourcesChanged,
    SetProject,
    SetPreviewExclusion(Option<Uuid>),
    ConfigureResources(RenderResourceConfig),
    ScheduleSam2Analysis,
    Render {
        position: Time,
        accuracy: CompositeAccuracy,
        request_id: u64,
        cancel_generation: u64,
    },
    Stop,
}

pub enum VideoEvent<F> {
    Loading {
        position: Time,
        show_spinner: bool,
        render_elapsed: Duration,
        render_generation: u64,
    },
    Frame {
        frame: F,
        audio_analysis: FrameAudioAnalysis,
        position: Time,
        revision: u64,
        excluded_item_id: Option<Uuid>,
        settled: bool,
        render_elapsed: Duration,
        render_generation: u64,
    },
    Clear {
        audio_analysis: FrameAudioAnalysis,
        position: Time,
        revision: u64,
        excluded_item_id: Option<Uuid>,
        render_elapsed: Duration,
        render_generation: u64,
    },
    Manim(shrimply_manim_state::Update),
    Error(String),
}

pub fn publish_render_event<F>(
    event_tx: &EventSender<VideoEvent<F>>,
    event: VideoEvent<F>,
    accuracy: CompositeAccuracy,
) {
    if accuracy.continuous_playback() {
        let _ = event_tx.try_send(event);
    } else {
        let _ = event_tx.send(event);
    }
}

pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct EventSender<T> {
    sender: mpsc::SyncSender<T>,
    wake: Option<Wake>,
}
impl<T> Clone for EventSender<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            wake: self.wake.clone(),
        }
    }
}
impl<T> EventSender<T> {
    pub fn send(&self, event: T) -> Result<(), mpsc::SendError<T>> {
        // Wake a consumer before a potentially blocking send, then notify it
        // again once the new event is visible. Native document I/O can be busy.
        if let Some(wake) = &self.wake {
            wake();
        }
        let result = self.sender.send(event);
        if result.is_ok()
            && let Some(wake) = &self.wake
        {
            wake();
        }
        result
    }
    pub fn try_send(&self, event: T) -> Result<(), mpsc::TrySendError<T>> {
        let result = self.sender.try_send(event);
        if result.is_ok()
            && let Some(wake) = &self.wake
        {
            wake();
        }
        result
    }
}

pub fn channels<F>(
    playback_observer: Option<PlaybackRenderObserver>,
    wake: Option<Wake>,
) -> (
    VideoCommandSender,
    Receiver<VideoEvent<F>>,
    VideoWorkerChannels<F>,
) {
    let (command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::sync_channel(PREVIEW_DISPLAY_SURFACES);
    let event_tx = EventSender {
        sender: event_tx,
        wake,
    };
    let next_request_id = Arc::new(AtomicU64::new(0));
    let cancel_generation = Arc::new(AtomicU64::new(0));
    let last_render = Arc::new(Mutex::new(None));
    let latest_project_generation = Arc::new(AtomicU64::new(0));
    let pending_project = Arc::new(Mutex::new(None));
    let project_notification_pending = Arc::new(AtomicBool::new(false));
    let sam2_scheduler = shrimply_visual_core::sam2::analysis::Scheduler::new({
        let command_tx = command_tx.clone();
        move || {
            let _ = command_tx.send(WorkerCommand::ScheduleSam2Analysis);
        }
    });
    let source_waker: shrimply_visual_core::generated::SourceWake = Arc::new({
        let command_tx = command_tx.clone();
        move || {
            let _ = command_tx.send(WorkerCommand::SourcesChanged);
        }
    });
    let sender = VideoCommandSender {
        sender: command_tx,
        next_request_id,
        cancel_generation: cancel_generation.clone(),
        last_render,
        latest_project_generation: latest_project_generation.clone(),
        pending_project: pending_project.clone(),
        project_notification_pending: project_notification_pending.clone(),
        playback_observer: playback_observer.clone(),
    };
    let channels = VideoWorkerChannels {
        source_waker,
        command_rx,
        sam2_scheduler,
        event_tx,
        cancel_generation,
        latest_project_generation,
        pending_project,
        project_notification_pending,
        playback_observer,
    };
    (sender, event_rx, channels)
}

pub struct VideoWorkerChannels<F> {
    pub source_waker: shrimply_visual_core::generated::SourceWake,
    pub command_rx: Receiver<WorkerCommand>,
    pub sam2_scheduler: shrimply_visual_core::sam2::analysis::Scheduler,
    pub event_tx: EventSender<VideoEvent<F>>,
    pub cancel_generation: Arc<AtomicU64>,
    pub latest_project_generation: Arc<AtomicU64>,
    pub pending_project: PendingProject,
    pub project_notification_pending: Arc<AtomicBool>,
    pub playback_observer: Option<PlaybackRenderObserver>,
}

pub fn take_pending_project(
    pending: &Mutex<Option<(Arc<Project>, u64, u64)>>,
    notification_pending: &AtomicBool,
) -> Option<(Arc<Project>, u64, u64)> {
    let mut pending = pending
        .lock()
        .expect("pending video project mutex poisoned");
    let project = pending.take();
    notification_pending.store(false, Ordering::Release);
    project
}

pub struct PendingProjectReset {
    pub project: PendingProject,
    pub notification: Arc<AtomicBool>,
}

impl Drop for PendingProjectReset {
    fn drop(&mut self) {
        self.project
            .lock()
            .expect("pending video project mutex poisoned")
            .take();
        self.notification.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use shrimply_video_decoder::DecodeControl;

    #[test]
    fn continuous_playback_coalesces_until_an_explicit_discontinuity() {
        let (sender, receiver) = mpsc::channel();
        let cancel_generation = Arc::new(AtomicU64::new(0));
        let commands = VideoCommandSender {
            sender,
            next_request_id: Arc::new(AtomicU64::new(0)),
            cancel_generation: cancel_generation.clone(),
            last_render: Arc::new(Mutex::new(None)),
            latest_project_generation: Arc::new(AtomicU64::new(0)),
            pending_project: Arc::new(Mutex::new(None)),
            project_notification_pending: Arc::new(AtomicBool::new(false)),
            playback_observer: None,
        };

        commands
            .send(VideoCommand::Render {
                position: Time::ZERO,
                accuracy: CompositeAccuracy::CONTINUOUS_TIME_ACCURATE,
            })
            .expect("send first playback render");
        let first_generation = match receiver.recv().expect("receive first playback render") {
            WorkerCommand::Render {
                request_id,
                cancel_generation,
                ..
            } => {
                assert_eq!(request_id, 1);
                cancel_generation
            }
            _ => panic!("first playback command was not a render"),
        };
        let active = DecodeControl::new(first_generation, cancel_generation.clone());

        commands
            .send(VideoCommand::Render {
                position: Time::from_fraction(1, 30),
                accuracy: CompositeAccuracy::CONTINUOUS_TIME_ACCURATE,
            })
            .expect("send adjacent playback render");
        match receiver.recv().expect("receive adjacent playback render") {
            WorkerCommand::Render {
                request_id,
                cancel_generation,
                ..
            } => {
                assert_eq!(request_id, 2);
                assert_eq!(cancel_generation, first_generation);
            }
            _ => panic!("adjacent playback command was not a render"),
        }
        assert!(
            !active.superseded(),
            "adjacent playback canceled active work"
        );

        commands
            .send(VideoCommand::Render {
                position: Time::from_seconds(1),
                accuracy: CompositeAccuracy::CONTINUOUS_TIME_ACCURATE,
            })
            .expect("send accelerated playback render");
        match receiver
            .recv()
            .expect("receive accelerated playback render")
        {
            WorkerCommand::Render {
                request_id,
                cancel_generation,
                ..
            } => {
                assert_eq!(request_id, 3);
                assert_eq!(cancel_generation, first_generation);
            }
            _ => panic!("accelerated playback command was not a render"),
        }
        assert!(
            !active.superseded(),
            "accelerated playback canceled active work"
        );

        commands
            .send(VideoCommand::Render {
                position: Time::from_seconds(2),
                accuracy: CompositeAccuracy::TIME_ACCURATE,
            })
            .expect("send discontinuous render");
        match receiver.recv().expect("receive discontinuous render") {
            WorkerCommand::Render {
                request_id,
                cancel_generation,
                ..
            } => {
                assert_eq!(request_id, 4);
                assert!(cancel_generation > first_generation);
            }
            _ => panic!("discontinuous command was not a render"),
        }
        assert!(
            active.superseded(),
            "discontinuity did not cancel active work"
        );
        assert!(
            !commands.render_generation_is_current(first_generation),
            "queued output from the old playback generation remained current"
        );
        assert!(commands.render_generation_is_current(cancel_generation.load(Ordering::Acquire)));
    }
}
