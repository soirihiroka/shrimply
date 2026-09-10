#![cfg(target_os = "macos")]

mod alpha_mask;
pub mod camera_reconstruction;
mod capture;
mod compositor;
mod effects;
pub mod modifier_cache;
mod optical_flow;
pub mod transparent_fill_analysis;
pub use compositor::render_png;

use shrimply_math_core::Time;
use shrimply_project_document::project::Project;
use skia_safe::{Canvas, Image};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(1);
const SLOW_FRAME_WARNING: Duration = Duration::from_secs(1);

pub struct ExportRenderer {
    compositor: compositor::Compositor,
}

impl ExportRenderer {
    pub fn new(background_alpha: u8, maximum_decoders: usize) -> Self {
        let mut compositor = compositor::Compositor::default();
        compositor.set_background_alpha(background_alpha);
        compositor.set_decoder_limit(maximum_decoders);
        Self { compositor }
    }

    pub fn render_rgba(
        &mut self,
        project: &Project,
        time: Time,
        cancelled: &AtomicBool,
    ) -> Result<Vec<u8>, String> {
        objc2::rc::autoreleasepool(|_| {
            loop {
                if cancelled.load(Ordering::Relaxed) {
                    return Err("Export cancelled".to_string());
                }
                if let Some(image) = self.compositor.poll_accurate_image(project, time)? {
                    return capture::rgba(&image, project.canvas_size);
                }
                thread::sleep(FRAME_POLL_INTERVAL);
            }
        })
    }
}

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
    decoder_limit: Option<usize>,
    completed: Option<(Target, Result<compositor::Presented, String>)>,
    manim_updates: Vec<shrimply_manim_state::Update>,
    sam2_errors: Vec<String>,
    schedule_sam2: bool,
    stop: bool,
    warmup: Option<Result<(), String>>,
}

struct Shared {
    slots: Mutex<Slots>,
    wake: Condvar,
    playback_observer: Option<PlaybackObserver>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StartupStatus {
    CompilingShaders,
    PreparingPreview,
    Ready,
}

pub type PlaybackObserver = shrimply_preview_provider_skia::performance::RenderObserver;

/// UI-side presentation only. The worker owns all shader compilation, source
/// rasterization, pixel uploads, compute dispatch and GPU presentation copies.
pub struct Renderer {
    shared: Arc<Shared>,
    worker: JoinHandle<()>,
    project: Option<Arc<Project>>,
    requested: Option<Target>,
    presented: Option<compositor::Presented>,
    presented_id: u32,
    presented_target: Option<Target>,
    project_revision: u64,
    excluded_item_id: Option<uuid::Uuid>,
    revision: u64,
    playing: bool,
    scrubbing: bool,
    render_elapsed: Option<Duration>,
    next_request_id: u64,
    manim_updates: Vec<shrimply_manim_state::Update>,
    error: Option<String>,
    decoder_limit: Option<usize>,
    texture: Option<Image>,
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
        let sam2_scheduler = shrimply_visual_core::sam2::analysis::Scheduler::new({
            let shared = Arc::downgrade(&shared);
            move || {
                let Some(shared) = shared.upgrade() else {
                    return;
                };
                shared
                    .slots
                    .lock()
                    .expect("Metal preview slots poisoned")
                    .schedule_sam2 = true;
                shared.wake.notify_one();
            }
        });
        let worker = thread::Builder::new()
            .name("preview-metal".into())
            .spawn(move || worker(state, sam2_scheduler))
            .expect("start Metal preview worker");
        Self {
            shared,
            worker,
            project: None,
            requested: None,
            presented: None,
            presented_id: 0,
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
            decoder_limit: None,
            texture: None,
        }
    }
    pub fn set_decoder_limit(&mut self, maximum: usize) {
        assert!(maximum > 0, "video decoder limit must be positive");
        if self.decoder_limit == Some(maximum) {
            return;
        }
        self.decoder_limit = Some(maximum);
        self.shared
            .slots
            .lock()
            .expect("Metal preview slots poisoned")
            .decoder_limit = Some(maximum);
        self.shared.wake.notify_one();
    }

    pub fn set_project_revision(&mut self, revision: u64) {
        if self.project_revision != revision {
            self.project_revision = revision;
            // A newer edit must not prevent the preceding edit from appearing.
            // Explicit invalidation/exclusion changes still fence old frames.
            self.project = None;
            self.requested = None;
            self.error = None;
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
            .map(|(_, target)| {
                (
                    self.presented_id,
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

    /// Read CPU pixels only when the user explicitly captures the displayed frame.
    pub fn capture_image(&self) -> Result<Option<Image>, String> {
        self.presented
            .as_ref()
            .map(|frame| capture::image(&frame.buffer, frame.size))
            .transpose()
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

    /// Paint the frame collected by `prepare`, without polling or requesting worker work.
    pub fn draw(
        &mut self,
        canvas: &Canvas,
        sampling: skia_safe::SamplingOptions,
    ) -> Result<(), String> {
        if let Some(frame) = &self.presented {
            use objc2::rc::Retained;
            use skia_safe::gpu::{
                Budgeted, Mipmapped, SurfaceOrigin, backend_textures, images, mtl,
            };
            let mut context = canvas
                .direct_context()
                .ok_or("Metal preview requires a GPU canvas")?;
            if self.texture.is_none() {
                // The worker publishes only after compute and the texture copy
                // complete. Metal's Skia wrapper retains the native texture, so
                // queued draws remain valid after the next frame replaces it.
                let info = unsafe {
                    mtl::TextureInfo::new(Retained::as_ptr(&frame.texture) as mtl::Handle)
                };
                let backend = unsafe {
                    backend_textures::make_mtl(
                        (frame.size.0 as i32, frame.size.1 as i32),
                        Mipmapped::No,
                        &info,
                        "preview composite",
                    )
                };
                let texture = images::borrow_texture_from(
                    &mut context,
                    &backend,
                    SurfaceOrigin::TopLeft,
                    skia_safe::ColorType::RGBA8888,
                    skia_safe::AlphaType::Unpremul,
                    None,
                )
                .ok_or("Could not wrap the Metal preview texture")?;
                self.texture = Some(texture);
            }
            let texture = self.texture.as_mut().expect("preview texture wrapped");
            if sampling.mipmap != skia_safe::MipmapMode::None
                && (texture.width() > 1 || texture.height() > 1)
                && !texture.has_mipmaps()
            {
                // The source is now GPU-backed: Skia copies the base level on the
                // GPU and regenerates its mip chain during submission, not on CPU.
                *texture = images::texture_from_image(
                    &mut context,
                    texture,
                    Mipmapped::Yes,
                    Budgeted::Yes,
                )
                .filter(Image::has_mipmaps)
                .ok_or("Could not create GPU preview mipmaps")?;
            }
            canvas.draw_image_with_sampling_options(&*texture, (0.0, 0.0), sampling, None);
        }
        Ok(())
    }

    /// Request and collect preview frames without attaching an editor view to a window.
    pub fn prepare(&mut self, project: &Project, time: Time) -> Result<(), String> {
        {
            let slots = self
                .shared
                .slots
                .lock()
                .expect("Metal preview slots poisoned");
            if let Some(Err(error)) = &slots.warmup {
                return Err(error.clone());
            }
        }
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
        if let Some(error) = slots.sam2_errors.pop() {
            self.error = Some(error);
        }
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
                    self.presented_id = self.presented_id.wrapping_add(1);
                    self.presented = Some(image);
                    self.texture = None;
                    self.presented_target = Some(completed_target);
                    self.error = None;
                }
                Err(error) if completed_target.project_revision == self.project_revision => {
                    self.error = Some(error);
                }
                Err(_) => {}
            }
        }
        if self.requested != Some(target) {
            self.next_request_id = self.next_request_id.wrapping_add(1);
            let request_id = self.next_request_id;
            if target.playing
                && let Some(observer) = &self.shared.playback_observer
            {
                observer(
                    shrimply_preview_provider_skia::performance::RenderEvent::Requested {
                        request_id,
                        position: target.time,
                    },
                );
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
        self.error.clone().map_or(Ok(()), Err)
    }

    pub fn startup_status(&self) -> Result<StartupStatus, String> {
        let slots = self
            .shared
            .slots
            .lock()
            .expect("Metal preview slots poisoned");
        if let Some(Err(error)) = &slots.warmup {
            return Err(error.clone());
        }
        if self.worker.is_finished() {
            return Err("Metal preview worker stopped unexpectedly".into());
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(if slots.warmup.is_none() {
            StartupStatus::CompilingShaders
        } else if self.requested.is_some() && self.presented.is_some() && !self.loading(Time::ZERO)
        {
            StartupStatus::Ready
        } else {
            StartupStatus::PreparingPreview
        })
    }

    pub fn take_manim_updates(&mut self) -> Vec<shrimply_manim_state::Update> {
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

fn worker(
    shared: Arc<Shared>,
    mut sam2_scheduler: shrimply_visual_core::sam2::analysis::Scheduler,
) {
    let mut renderer = compositor::Compositor::default();
    let warmup = objc2::rc::autoreleasepool(|_| renderer.warmup());
    let failed = warmup.is_err();
    shared
        .slots
        .lock()
        .expect("Metal preview slots poisoned")
        .warmup = Some(warmup);
    if failed {
        return;
    }
    let mut current: Option<Request> = None;
    let mut timings = BTreeMap::<u64, RequestTiming>::new();
    let mut active = false;
    let mut project_frame_completed = false;
    let mut request_started = Instant::now();
    let mut slow_request_reported = false;
    loop {
        let mut slots = shared.slots.lock().expect("Metal preview slots poisoned");
        while !slots.stop
            && slots.request.is_none()
            && slots.decoder_limit.is_none()
            && !slots.schedule_sam2
            && !active
        {
            slots = shared
                .wake
                .wait(slots)
                .expect("Metal preview slots poisoned");
        }
        if slots.stop {
            return;
        }
        if let Some(maximum) = slots.decoder_limit.take() {
            renderer.set_decoder_limit(maximum);
        }
        if current.is_none() && slots.request.is_none() && !slots.schedule_sam2 {
            continue;
        }
        let schedule_sam2 = std::mem::take(&mut slots.schedule_sam2);
        if schedule_sam2 {
            sam2_scheduler.consume_notification();
        }
        let mut project_changed = false;
        // Coalesce live edits while one project revision is rendering. Replacing
        // it before its first frame completes can starve presentation for an
        // entire number drag. Time-only playback/scrub requests remain immediate.
        let accept_request = slots.request.as_ref().is_some_and(|request| {
            !active
                || project_frame_completed
                || current.as_ref().is_none_or(|previous| {
                    previous.target.revision != request.target.revision
                        || previous.target.project_revision == request.target.project_revision
                })
        });
        if accept_request && let Some(request) = slots.request.take() {
            if current.as_ref().is_some_and(|previous| {
                previous.target.revision != request.target.revision
                    || previous.target.project_revision != request.target.project_revision
            }) {
                renderer.invalidate();
                project_frame_completed = false;
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
            project_changed = true;
        }
        drop(slots);
        let request = current.as_ref().expect("Metal preview request is active");
        if schedule_sam2 || project_changed {
            schedule_sam2_analysis(&request.project, &mut sam2_scheduler, &shared);
        }
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
            project_frame_completed |= completed_target.revision == request.target.revision
                && completed_target.project_revision == request.target.project_revision;
            if let Ok(frame) = &completed
                && !frame.loading
                && completed_request_id == request.request_id
                && let Some(timing) = timings.get_mut(&completed_request_id)
                && timing.target.playing
                && !timing.reported
            {
                timing.reported = true;
                if let Some(observer) = &shared.playback_observer {
                    observer(
                        shrimply_preview_provider_skia::performance::RenderEvent::Completed {
                            request_id: completed_request_id,
                            position: frame.time,
                            elapsed: timing.started.elapsed(),
                            project_fps: timing.project_fps,
                        },
                    );
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
        if active && (slots.request.is_none() || !accept_request) && !slots.stop {
            drop(
                shared
                    .wake
                    .wait_timeout(slots, FRAME_POLL_INTERVAL)
                    .expect("Metal preview slots poisoned"),
            );
        }
    }
}

fn schedule_sam2_analysis(
    project: &Project,
    scheduler: &mut shrimply_visual_core::sam2::analysis::Scheduler,
    shared: &Arc<Shared>,
) {
    let errors = shared.clone();
    match scheduler.schedule_next_with(
        project,
        compositor::Sam2ProxyFrameSource::new,
        move |error| {
            tracing::error!(%error, "Metal SAM2 analysis failed");
            errors
                .slots
                .lock()
                .expect("Metal preview slots poisoned")
                .sam2_errors
                .push(error);
            errors.wake.notify_one();
        },
    ) {
        Ok(_) => {}
        Err(error) => {
            shared
                .slots
                .lock()
                .expect("Metal preview slots poisoned")
                .sam2_errors
                .push(error);
            shared.wake.notify_one();
        }
    }
}
