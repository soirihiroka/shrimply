#![cfg(target_os = "macos")]

mod alpha_mask;
mod compositor;
mod effects;
pub use compositor::render_png;

use shrimply_math_core::Time;
use shrimply_project::project::Project;
use skia_safe::{Canvas, Image};
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(1);
const SLOW_FRAME_WARNING: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Target {
    revision: u64,
    project_revision: u64,
    excluded_item_id: Option<uuid::Uuid>,
    time: Time,
    playing: bool,
    scrubbing: bool,
}

struct Request {
    target: Target,
    project: Arc<Project>,
    request_id: u64,
}

#[derive(Clone, Copy)]
struct RequestTiming {
    target: Target,
    started: Instant,
    project_fps: shrimply_math_core::Fraction,
    reported: bool,
}

#[derive(Default)]
struct Slots {
    request: Option<Request>,
    completed: Option<(Target, Result<compositor::Presented, String>)>,
    manim_updates: Vec<shrimply_state::manim_status::Update>,
    stop: bool,
}

struct Shared {
    slots: Mutex<Slots>,
    wake: Condvar,
    playback_observer: Option<PlaybackObserver>,
}

pub type PlaybackObserver = shrimply_preview_core::performance::RenderObserver;

/// UI-side presentation only. The worker owns all shader compilation, source
/// rasterization, pixel uploads, compute dispatch and completed-frame readback.
pub struct Renderer {
    shared: Arc<Shared>,
    worker: JoinHandle<()>,
    project: Option<Arc<Project>>,
    requested: Option<Target>,
    presented: Option<compositor::Presented>,
    presented_target: Option<Target>,
    project_revision: u64,
    excluded_item_id: Option<uuid::Uuid>,
    revision: u64,
    playing: bool,
    scrubbing: bool,
    render_elapsed: Option<Duration>,
    next_request_id: u64,
    manim_updates: Vec<shrimply_state::manim_status::Update>,
    error: Option<String>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Renderer {
    pub fn new(playback_observer: Option<PlaybackObserver>) -> Self {
        let shared = Arc::new(Shared {
            slots: Mutex::new(Slots::default()),
            wake: Condvar::new(),
            playback_observer,
        });
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("preview-metal".into())
            .spawn(move || worker(state))
            .expect("start Metal preview worker");
        Self {
            shared,
            worker,
            project: None,
            requested: None,
            presented: None,
            presented_target: None,
            project_revision: 0,
            excluded_item_id: None,
            revision: 0,
            playing: false,
            scrubbing: false,
            render_elapsed: None,
            next_request_id: 0,
            manim_updates: Vec::new(),
            error: None,
        }
    }
    pub fn set_project_revision(&mut self, revision: u64) {
        if self.project_revision != revision {
            self.project_revision = revision;
            self.invalidate();
        }
    }

    pub fn set_exclusion(&mut self, excluded_item_id: Option<uuid::Uuid>) {
        if self.excluded_item_id != excluded_item_id {
            self.excluded_item_id = excluded_item_id;
            self.invalidate();
        }
    }

    pub fn presented_frame(&self) -> Option<(u32, u64, Option<uuid::Uuid>)> {
        self.presented
            .as_ref()
            .zip(self.presented_target)
            .map(|(image, target)| {
                (
                    image.image.unique_id(),
                    target.project_revision,
                    target.excluded_item_id,
                )
            })
    }

    pub fn set_interaction(&mut self, playing: bool, scrubbing: bool) {
        self.playing = playing;
        self.scrubbing = scrubbing;
    }

    pub fn invalidate(&mut self) {
        self.revision += 1;
        self.project = None;
        self.requested = None;
        self.error = None;
    }

    pub fn image(&self) -> Option<&Image> {
        self.presented.as_ref().map(|frame| &frame.image)
    }

    pub fn presented_audio(
        &self,
    ) -> Option<(Time, &shrimply_preview_render_core::FrameAudioAnalysis)> {
        self.presented
            .as_ref()
            .map(|frame| (frame.time, &frame.audio_analysis))
    }

    pub fn render_elapsed(&self) -> Option<Duration> {
        self.render_elapsed
    }

    pub fn draw(&mut self, canvas: &Canvas, project: &Project, time: Time) -> Result<(), String> {
        if self.worker.is_finished() {
            return Err("Metal preview worker stopped unexpectedly".into());
        }
        let target = Target {
            revision: self.revision,
            project_revision: self.project_revision,
            excluded_item_id: self.excluded_item_id,
            time,
            playing: self.playing,
            scrubbing: self.scrubbing,
        };
        let mut slots = self
            .shared
            .slots
            .lock()
            .expect("Metal preview slots poisoned");
        self.manim_updates.append(&mut slots.manim_updates);
        // Present a completed scrub frame before replacing the requested target.
        if let Some((completed_target, result)) = slots.completed.take()
            && completed_target.revision == self.revision
        {
            match result {
                Ok(image) => {
                    let completed_target = Target {
                        time: image.time,
                        ..completed_target
                    };
                    self.render_elapsed = Some(image.render_elapsed);
                    self.presented = Some(image);
                    self.presented_target = Some(completed_target);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        if self.requested != Some(target) {
            self.next_request_id = self.next_request_id.wrapping_add(1);
            let request_id = self.next_request_id;
            if target.playing
                && let Some(observer) = &self.shared.playback_observer
            {
                observer(shrimply_preview_core::performance::RenderEvent::Requested {
                    request_id,
                    position: target.time,
                });
            }
            let project = self
                .project
                .get_or_insert_with(|| Arc::new(project.clone()));
            slots.request = Some(Request {
                target,
                project: project.clone(),
                request_id,
            });
            self.requested = Some(target);
            self.shared.wake.notify_one();
        }
        drop(slots);
        if let Some(image) = &self.presented {
            canvas.draw_image(&image.image, (0.0, 0.0), None);
        }
        self.error.clone().map_or(Ok(()), Err)
    }

    pub fn take_manim_updates(&mut self) -> Vec<shrimply_state::manim_status::Update> {
        std::mem::take(&mut self.manim_updates)
    }

    pub fn loading(&self, tolerance: Time) -> bool {
        let (Some(requested), Some(presented)) = (self.requested, self.presented_target) else {
            return self.requested.is_some();
        };
        self.presented.as_ref().is_some_and(|frame| frame.loading)
            || requested.revision != presented.revision
            || requested.project_revision != presented.project_revision
            || requested.excluded_item_id != presented.excluded_item_id
            || if requested.playing {
                requested.time.abs_diff(presented.time) > tolerance
            } else {
                requested.time != presented.time
                    || self.presented.as_ref().is_none_or(|frame| {
                        frame.accuracy
                            != shrimply_preview_render_core::CompositeAccuracy::FULLY_ACCURATE
                    })
            }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.shared
            .slots
            .lock()
            .expect("Metal preview slots poisoned")
            .stop = true;
        self.shared.wake.notify_one();
        // Never join a shader compiler or decoder from the main thread. The
        // worker retains its own GPU/media resources until it observes shutdown.
    }
}

fn worker(shared: Arc<Shared>) {
    let mut renderer = compositor::Compositor::default();
    let mut current: Option<Request> = None;
    let mut timings = BTreeMap::<u64, RequestTiming>::new();
    let mut active = false;
    let mut request_started = Instant::now();
    let mut slow_request_reported = false;
    loop {
        let mut slots = shared.slots.lock().expect("Metal preview slots poisoned");
        while !slots.stop && slots.request.is_none() && !active {
            slots = shared
                .wake
                .wait(slots)
                .expect("Metal preview slots poisoned");
        }
        if slots.stop {
            return;
        }
        if let Some(request) = slots.request.take() {
            if current
                .as_ref()
                .is_some_and(|previous| previous.target.revision != request.target.revision)
            {
                renderer.invalidate();
            }
            renderer.set_interaction(request.target.playing, request.target.scrubbing);
            renderer.set_exclusion(request.target.excluded_item_id);
            request_started = Instant::now();
            timings.insert(
                request.request_id,
                RequestTiming {
                    target: request.target,
                    started: request_started,
                    project_fps: request.project.fps,
                    reported: false,
                },
            );
            current = Some(request);
            slow_request_reported = false;
        }
        drop(slots);
        let request = current.as_ref().expect("Metal preview request is active");
        let result = objc2::rc::autoreleasepool(|_| {
            renderer.update(&request.project, request.target.time, request.request_id)
        });
        let manim_updates = renderer.take_manim_updates();
        active = result.is_ok() && renderer.needs_update();
        let completed = match result {
            Ok(()) => renderer.take_presented().map(Ok),
            Err(error) => Some(Err(error)),
        };
        if !slow_request_reported && request_started.elapsed() >= SLOW_FRAME_WARNING {
            slow_request_reported = true;
            tracing::warn!(
                time = %request.target.time.as_label(),
                project_revision = request.target.project_revision,
                playing = request.target.playing,
                scrubbing = request.target.scrubbing,
                active,
                elapsed_ms = request_started.elapsed().as_millis(),
                "Metal preview frame is still rendering"
            );
        }
        let mut slots = shared.slots.lock().expect("Metal preview slots poisoned");
        slots.manim_updates.extend(manim_updates);
        if let Some(completed) = completed {
            let completed_request_id = completed
                .as_ref()
                .map_or(request.request_id, |frame| frame.request_id);
            let completed_target = timings
                .get(&completed_request_id)
                .map_or(request.target, |timing| timing.target);
            if let Ok(frame) = &completed
                && !frame.loading
                && completed_request_id == request.request_id
                && let Some(timing) = timings.get_mut(&completed_request_id)
                && timing.target.playing
                && !timing.reported
            {
                timing.reported = true;
                if let Some(observer) = &shared.playback_observer {
                    observer(shrimply_preview_core::performance::RenderEvent::Completed {
                        request_id: completed_request_id,
                        position: frame.time,
                        elapsed: timing.started.elapsed(),
                        project_fps: timing.project_fps,
                    });
                }
            }
            if slow_request_reported {
                let completed_time = completed
                    .as_ref()
                    .map_or(request.target.time, |frame| frame.time);
                tracing::info!(
                    completed_time = %completed_time.as_label(),
                    requested_time = %request.target.time.as_label(),
                    project_revision = request.target.project_revision,
                    current_request_elapsed_ms = request_started.elapsed().as_millis(),
                    success = completed.is_ok(),
                    "Metal preview frame finished"
                );
            }
            slots.completed = Some((completed_target, completed));
            timings.retain(|id, _| *id >= completed_request_id);
        }
        if active && slots.request.is_none() && !slots.stop {
            drop(
                shared
                    .wake
                    .wait_timeout(slots, FRAME_POLL_INTERVAL)
                    .expect("Metal preview slots poisoned"),
            );
        }
    }
}
