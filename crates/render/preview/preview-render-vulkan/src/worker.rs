use super::{Compositor, Presented};
use shrimply_preview_render_core::CompositeAccuracy;
use shrimply_preview_render_core::worker::*;
use shrimply_project_document::project::{Project, Time};
use shrimply_render_vulkan::{Buffer, Texture};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct Frame {
    pub source_frame: Option<u64>,
    pub display: Texture,
    pub rendered: Buffer,
    pub width: u32,
    pub height: u32,
    pub serial: u64,
    pub sanim_sources: Arc<shrimply_visual_core::sanim::inspection::FrameSources>,
}

impl Frame {
    /// Explicit capture reads the completed GPU composite; presentation stays on the GPU.
    pub fn png(&self) -> Result<Vec<u8>, String> {
        let pixels = shrimply_render_vulkan::Renderer::new()?.readback(&self.rendered)?;
        let width = i32::try_from(self.width).map_err(|_| "Preview width is too large")?;
        let height = i32::try_from(self.height).map_err(|_| "Preview height is too large")?;
        shrimply_preview_render_core::capture::RgbaFrame {
            width,
            height,
            pixels,
        }
        .png()
    }
}

pub fn spawn_worker(
    project: Project,
    config: RenderResourceConfig,
    observer: Option<PlaybackRenderObserver>,
    wake: Option<Wake>,
) -> (VideoCommandSender, Receiver<VideoEvent<Frame>>) {
    let (sender, events, channels) = channels(observer, wake);
    std::thread::spawn(move || worker::<Frame>(project, config, channels));
    (sender, events)
}

const GPU_POLL_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Clone, Copy)]
struct Request {
    position: Time,
    accuracy: CompositeAccuracy,
    id: u64,
    generation: u64,
    started: Instant,
}

struct Pending {
    frame: Presented,
    request: Request,
    revision: u64,
    project_generation: u64,
    exclusion: Option<Uuid>,
    started: Instant,
    submitted: Instant,
}

pub fn worker<F: From<Frame> + Send + 'static>(
    project: Project,
    config: RenderResourceConfig,
    mut channels: VideoWorkerChannels<F>,
) {
    let _reset = PendingProjectReset {
        project: channels.pending_project.clone(),
        notification: channels.project_notification_pending.clone(),
    };
    let mut compositor = match Compositor::new() {
        Ok(compositor) => compositor,
        Err(error) => {
            let _ = channels.event_tx.send(VideoEvent::Error(error));
            return;
        }
    };
    compositor.set_source_waker(Some(channels.source_waker.clone()));
    compositor.set_decoder_limit(config.maximum_temporal_decoders);
    let mut project = Arc::new(project);
    let mut revision = 0;
    let mut project_generation = 0;
    let mut exclusion = None;
    let mut request = None;
    let mut last_request = None;
    let mut pending: VecDeque<Pending> = VecDeque::new();
    let mut serial = 0_u64;
    loop {
        let next = if request.is_some() || !pending.is_empty() {
            match channels.command_rx.recv_timeout(GPU_POLL_INTERVAL) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match channels.command_rx.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };
        let commands = next
            .into_iter()
            .chain(channels.command_rx.try_iter())
            .collect::<Vec<_>>();
        for command in commands {
            match command {
                WorkerCommand::Stop => return,
                WorkerCommand::SourcesChanged => {
                    if request.is_none() && compositor.needs_update() {
                        request = last_request;
                    }
                }
                WorkerCommand::SetProject => {
                    if let Some((next, next_revision, next_generation)) = take_pending_project(
                        &channels.pending_project,
                        &channels.project_notification_pending,
                    ) {
                        project = next;
                        revision = next_revision;
                        project_generation = next_generation;
                        compositor.invalidate();
                        request = None;
                        schedule_analysis(&project, &mut channels);
                    }
                }
                WorkerCommand::SetPreviewExclusion(item) => {
                    exclusion = item;
                    compositor.set_exclusion(item);
                }
                WorkerCommand::ConfigureResources(config) => {
                    compositor.set_decoder_limit(config.maximum_temporal_decoders)
                }
                WorkerCommand::ScheduleSam2Analysis => {
                    channels.sam2_scheduler.consume_notification();
                    schedule_analysis(&project, &mut channels);
                }
                WorkerCommand::Render {
                    position,
                    accuracy,
                    request_id,
                    cancel_generation,
                } => {
                    schedule_analysis(&project, &mut channels);
                    // A prior submission at this position may be cancelled
                    // before publication. An explicit request must render again.
                    compositor.scene.request_frame();
                    request = Some(Request {
                        position,
                        accuracy,
                        id: request_id,
                        generation: cancel_generation,
                        started: Instant::now(),
                    });
                    last_request = request;
                }
            }
        }
        while let Some(frame) = pending.front() {
            match frame.frame.completed() {
                Ok(false) => break,
                Err(error) => {
                    let _ = channels.event_tx.try_send(VideoEvent::Error(error));
                    pending.pop_front();
                    continue;
                }
                Ok(true) => {}
            }
            let pending_frame = pending
                .pop_front()
                .expect("polled Vulkan frame disappeared");
            shrimply_profiling::record(
                "Vulkan compositor / GPU completion and polling",
                pending_frame.submitted.elapsed(),
            );
            let current = pending_frame.request;
            if channels.cancel_generation.load(Ordering::Acquire) == current.generation
                && channels.latest_project_generation.load(Ordering::Acquire)
                    == pending_frame.project_generation
                && pending_frame.revision == revision
                && pending_frame.exclusion == exclusion
            {
                shrimply_profiling::increment("Vulkan compositor / GPU frames completed");
                let elapsed = pending_frame.started.elapsed();
                let width = pending_frame.frame.rendered.width;
                let height = pending_frame.frame.rendered.height;
                let loading = pending_frame.frame.rendered.loading;
                let audio_analysis = pending_frame.frame.rendered.audio_analysis.clone();
                let settled = pending_frame.frame.rendered.accuracy.settled(loading);
                let sanim_sources = pending_frame.frame.rendered.sanim_sources.clone();
                let source_frame = pending_frame.frame.rendered.source_frame;
                let (display, rendered) = pending_frame.frame.into_buffers();
                serial = serial
                    .checked_add(1)
                    .expect("Vulkan presentation serial overflow");
                let frame = F::from(Frame {
                    source_frame,
                    display,
                    rendered,
                    width,
                    height,
                    serial,
                    sanim_sources,
                });
                if current.accuracy.continuous_playback()
                    && let Some(observer) = &channels.playback_observer
                {
                    observer(PlaybackRenderEvent::Completed {
                        request_id: current.id,
                        position: current.position,
                        elapsed,
                        project_fps: project.fps,
                    });
                }
                publish_render_event(
                    &channels.event_tx,
                    VideoEvent::Frame {
                        frame,
                        audio_analysis,
                        position: current.position,
                        revision,
                        excluded_item_id: exclusion,
                        settled,
                        render_elapsed: elapsed,
                        render_generation: current.generation,
                    },
                    current.accuracy,
                );
                if loading {
                    publish_render_event(
                        &channels.event_tx,
                        VideoEvent::Loading {
                            position: current.position,
                            show_spinner: true,
                            render_elapsed: elapsed,
                            render_generation: current.generation,
                        },
                        current.accuracy,
                    );
                } else if request.is_some_and(|request| request.id == current.id)
                    && !compositor.needs_update()
                {
                    request = None;
                }
            }
        }
        let Some(current) = request else {
            continue;
        };
        if pending.len() >= shrimply_render_core::PREVIEW_FRAMES_IN_FLIGHT
            || pending
                .iter()
                .any(|pending| pending.request.id == current.id)
        {
            continue;
        }
        if channels.cancel_generation.load(Ordering::Acquire) != current.generation {
            request = None;
            continue;
        }
        let prepared = {
            let _measurement = shrimply_profiling::measure("Vulkan compositor / Prepare scene");
            compositor.prepare(&project, current.position, current.accuracy)
        };
        for error in compositor.take_sanim_errors() {
            let _ = channels.event_tx.try_send(VideoEvent::Error(error));
        }
        for update in compositor.take_manim_updates() {
            if channels.event_tx.send(VideoEvent::Manim(update)).is_err() {
                return;
            }
        }
        match prepared.and_then(|plan| {
            plan.map(|plan| {
                let _measurement = shrimply_profiling::measure("Vulkan compositor / Submit frame");
                compositor.render(plan)
            })
            .transpose()
        }) {
            Ok(Some(frame)) => {
                if current.accuracy.continuous_playback()
                    && !frame.rendered.loading
                    && let Some(observer) = &channels.playback_observer
                {
                    observer(PlaybackRenderEvent::Submitted {
                        request_id: current.id,
                        position: current.position,
                    });
                }
                pending.push_back(Pending {
                    frame,
                    request: current,
                    revision,
                    project_generation,
                    exclusion,
                    started: current.started,
                    submitted: Instant::now(),
                });
            }
            Ok(None) => {
                if !compositor.needs_update() {
                    request = None;
                }
            }
            Err(error) => {
                let _ = channels.event_tx.try_send(VideoEvent::Error(error));
                request = None;
            }
        }
    }
}

fn schedule_analysis<F: Send + 'static>(project: &Project, channels: &mut VideoWorkerChannels<F>) {
    let event_tx = channels.event_tx.clone();
    if let Err(error) = channels.sam2_scheduler.schedule_next_with(
        project,
        crate::Sam2ProxyFrameSource::new,
        move |error| {
            let _ = event_tx.try_send(VideoEvent::Error(error));
        },
    ) {
        let _ = channels.event_tx.try_send(VideoEvent::Error(error));
    }
}
