use super::*;
use shrimply_preview_render_vulkan::{Compositor, Presented};

const GPU_POLL_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Clone, Copy)]
struct Request {
    position: Time,
    accuracy: CompositeAccuracy,
    id: u64,
    generation: u64,
}

struct Pending {
    frame: Presented,
    request: Request,
    revision: u64,
    project_generation: u64,
    exclusion: Option<Uuid>,
    started: Instant,
}

pub(super) fn worker(
    mut project: Project,
    config: RenderResourceConfig,
    mut channels: VideoWorkerChannels,
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
    compositor.set_decoder_limit(config.maximum_temporal_decoders);
    let mut revision = 0;
    let mut project_generation = 0;
    let mut exclusion = None;
    let mut request = None;
    let mut pending: Option<Pending> = None;
    let mut serial = 0_u64;
    loop {
        let next = if request.is_some() || pending.is_some() {
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
                WorkerCommand::SetProject => {
                    if let Some((next, next_revision, next_generation)) = take_pending_project(
                        &channels.pending_project,
                        &channels.project_notification_pending,
                    ) {
                        project = (*next).clone();
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
                    request = Some(Request {
                        position,
                        accuracy,
                        id: request_id,
                        generation: cancel_generation,
                    })
                }
            }
        }
        if let Some(frame) = &pending {
            match frame.frame.completed() {
                Ok(false) => continue,
                Err(error) => {
                    let _ = channels.event_tx.try_send(VideoEvent::Error(error));
                    pending = None;
                    continue;
                }
                Ok(true) => {}
            }
            let pending_frame = pending.take().expect("polled Intel frame disappeared");
            let current = pending_frame.request;
            if channels.cancel_generation.load(Ordering::Acquire) == current.generation
                && channels.latest_project_generation.load(Ordering::Acquire)
                    == pending_frame.project_generation
                && pending_frame.revision == revision
                && pending_frame.exclusion == exclusion
            {
                let elapsed = pending_frame.started.elapsed();
                let width = pending_frame.frame.rendered.width;
                let height = pending_frame.frame.rendered.height;
                let loading = pending_frame.frame.rendered.loading;
                let audio_analysis = pending_frame.frame.rendered.audio_analysis.clone();
                let settled = pending_frame.frame.rendered.accuracy.content_accurate() && !loading;
                let (display, rendered) = pending_frame.frame.into_buffers();
                serial = serial
                    .checked_add(1)
                    .expect("Intel presentation serial overflow");
                let frame = CompositedVideoFrame::intel(display, rendered, width, height, serial);
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
                } else if request.is_some_and(|request| request.id == current.id) {
                    request = None;
                }
            }
        }
        let Some(current) = request else {
            continue;
        };
        if channels.cancel_generation.load(Ordering::Acquire) != current.generation {
            request = None;
            continue;
        }
        let started = Instant::now();
        let prepared = compositor.prepare(&project, current.position, current.accuracy);
        for update in compositor.take_manim_updates() {
            if channels.event_tx.send(VideoEvent::Manim(update)).is_err() {
                return;
            }
        }
        match prepared.and_then(|plan| plan.map(|plan| compositor.render(plan)).transpose()) {
            Ok(Some(frame)) => {
                pending = Some(Pending {
                    frame,
                    request: current,
                    revision,
                    project_generation,
                    exclusion,
                    started,
                })
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

fn schedule_analysis(project: &Project, channels: &mut VideoWorkerChannels) {
    let event_tx = channels.event_tx.clone();
    if let Err(error) = channels.sam2_scheduler.schedule_next_with(
        project,
        shrimply_preview_render_vulkan::Sam2ProxyFrameSource::new,
        move |error| {
            let _ = event_tx.try_send(VideoEvent::Error(error));
        },
    ) {
        let _ = channels.event_tx.try_send(VideoEvent::Error(error));
    }
}
