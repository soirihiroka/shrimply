pub(crate) mod export;
mod recording;

use crate::runtime::{Runtime, SharedLifecycle};
use jni::sys::{JNIEnv, jboolean, jfloat, jfloatArray, jint, jlong, jobject};
use jni::{
    EnvUnowned,
    errors::ThrowRuntimeExAndDefault,
    jni_sig, jni_str,
    objects::{JObject, JString},
};
use ndk::native_window::NativeWindow;
use serde_json::{Value, json};
use shrimply_math_core as math;
use shrimply_player_core as player;
use shrimply_render_vulkan::surface::Renderer;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const ANDROID_LOG_INFO: i32 = 4;
use crate::input::{INPUT_STRIDE, MAX_INPUT_VALUES};
const BENCHMARK_FRAMES: usize = 1200;
// Keep only neighboring graph areas warm when lazy inspector rows leave composition.
const MAX_IDLE_GRAPH_SURFACES: usize = 2;
const FRAME_BUDGET_NS: u128 = 1_000_000_000 / 60;
const SYNC_RESIZE_TIMEOUT: Duration = Duration::from_millis(32);
static FONT_RESOURCES: std::sync::OnceLock<(Vec<std::path::PathBuf>, std::path::PathBuf)> =
    std::sync::OnceLock::new();
static APPLICATION: std::sync::OnceLock<jni::refs::Global<JObject<'static>>> =
    std::sync::OnceLock::new();

fn initialize_fonts() {
    let (directories, cache) = FONT_RESOURCES
        .get()
        .expect("Android font resources configured");
    shrimply_fonts::native::fontconfig::initialize(directories, cache)
        .expect("initialize shared native font resources");
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(
        priority: i32,
        tag: *const std::ffi::c_char,
        text: *const std::ffi::c_char,
    ) -> i32;
}
fn log(text: String) {
    let text = std::ffi::CString::new(text).expect("Android log text");
    unsafe {
        __android_log_write(
            ANDROID_LOG_INFO,
            c"ShrimplyTimeline".as_ptr(),
            text.as_ptr(),
        );
    }
}

enum RenderCommand {
    Surface(Option<NativeWindow>, u32, u32, f32, bool, SyncSender<()>),
    Resize(u32, u32, SyncSender<()>),
    Active(bool, SyncSender<()>),
    Wake,
    Stop,
}
enum ProjectCommand {
    Request(
        Value,
        Option<std::fs::File>,
        SyncSender<Result<Value, String>>,
    ),
    Wake,
    Stop,
}
struct Frame {
    size: (u32, u32),
    palette: Option<crate::runtime::palette::NativePalette>,
    picture: Option<skia_safe::Picture>,
    preview: Option<crate::preview::Frame>,
    density: f32,
    input_at: Option<Instant>,
    record_ns: u128,
}
#[derive(Default)]
struct RenderSlot {
    frame: Option<Frame>,
    visible: Option<bool>,
}

type RenderTargets = Arc<Mutex<std::collections::BTreeMap<usize, RenderTarget>>>;

struct RenderTarget {
    latest: Arc<Mutex<RenderSlot>>,
    frame_ready: Arc<Condvar>,
    send: SyncSender<RenderCommand>,
    thread: JoinHandle<()>,
    detached: Option<Instant>,
    width: u32,
    height: u32,
    density: f32,
    visible: bool,
}

struct Session {
    targets: RenderTargets,
    input: Arc<Mutex<crate::input::Queue>>,
    lifecycle: SharedLifecycle,
    active: AtomicBool,
    benchmark: bool,
    benchmark_submitted: Arc<AtomicUsize>,
    project: SyncSender<ProjectCommand>,
    project_thread: Option<JoinHandle<()>>,
}

struct ProjectWorker {
    receive: mpsc::Receiver<ProjectCommand>,
    wake: SyncSender<ProjectCommand>,
    targets: RenderTargets,
    input: Arc<Mutex<crate::input::Queue>>,
    lifecycle: SharedLifecycle,
    benchmark_submitted: Arc<AtomicUsize>,
}

fn publish_frame(targets: &RenderTargets, surface: usize, frame: Frame) {
    if let Some(target) = targets
        .lock()
        .expect("Android surfaces poisoned")
        .get(&surface)
        .filter(|target| target.detached.is_none())
    {
        target
            .latest
            .lock()
            .expect("Android frame slot poisoned")
            .frame = Some(frame);
        target.frame_ready.notify_all();
        let _ = target.send.try_send(RenderCommand::Wake);
    }
}

/// The canonical document, its history and Scene stay on one worker. Only Skia's
/// immutable display list crosses to the Vulkan worker; no pixels cross the CPU.
fn run_project(
    worker: ProjectWorker,
    benchmark: Option<bool>,
    publish: impl Fn(&str),
    mirror_listener: crate::project::MirrorListener,
) {
    initialize_fonts();
    let ProjectWorker {
        receive,
        wake,
        targets,
        input,
        lifecycle,
        benchmark_submitted,
    } = worker;
    let stress = benchmark.unwrap_or(false);
    let benchmark = benchmark.is_some();
    let project_wake = wake.clone();
    let mut runtime = Runtime::with_lifecycle(
        move || {
            let _ = wake.try_send(ProjectCommand::Wake);
        },
        lifecycle,
        Some(mirror_listener),
    )
    .expect("initialize Android project services");
    if benchmark {
        let path = shrimply_path_core::cache_directory()
            .expect("Android benchmark cache")
            .join("timeline-benchmark.shrimp");
        shrimply_project_document::create_project_file(&path, &crate::benchmark::project(stress))
            .expect("create benchmark project");
        runtime
            .request(&json!({"op":"open","path":path}))
            .expect("open actual benchmark project");
    }
    let mut viewport = crate::input::Viewport::default();
    let mut graph_viewports = std::collections::BTreeMap::<usize, crate::input::Viewport>::new();
    let mut preview_viewport = crate::input::Viewport::default();
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = project_wake.try_send(ProjectCommand::Wake);
    });
    if let Some(scene) = runtime.timeline.as_mut() {
        scene.set_wake(notify.clone());
    }
    let mut generation = runtime.generation;
    let mut changed = true;
    let mut picture_dirty = true;
    let mut audio_meter = shrimply_components_skia::audio_meter::AudioMeter::default();
    let mut meter_animating = false;
    let mut benchmark_frames = 0;
    let mut input_at = None;
    let mut preview_texture = None;
    let mut last_preview = None;
    let mut last_update = Instant::now();
    loop {
        let benchmark_running =
            benchmark && benchmark_submitted.load(Ordering::Acquire) < BENCHMARK_FRAMES;
        if changed {
            publish(&runtime.event());
            changed = false;
        }
        let frame_interval = Duration::from_nanos(FRAME_BUDGET_NS as u64);
        let frame_delay = frame_interval.saturating_sub(last_update.elapsed());
        let playback_delay = (runtime.foreground || runtime.screen_recording_clock_active())
            .then(|| player::next_update(&runtime.player, frame_interval))
            .flatten();
        let scene_delay = runtime
            .timeline
            .as_ref()
            .filter(|_| runtime.foreground && viewport.visible)
            .and_then(|scene| {
                if benchmark_running {
                    Some(frame_delay)
                } else {
                    scene.next_update()
                }
            });
        let delay = [
            (runtime.foreground && runtime.timeline.is_some())
                .then_some(shrimply_inspector_document::POLL_INTERVAL),
            playback_delay,
            (runtime.foreground
                && viewport.visible
                && meter_animating
                && runtime
                    .timeline
                    .as_ref()
                    .is_some_and(|scene| scene.audio_monitor_progress() > 0.0))
            .then_some(frame_delay),
            scene_delay,
            (runtime.foreground
                && graph_viewports.iter().any(|(surface, viewport)| {
                    viewport.visible
                        && runtime
                            .framegraphs
                            .sessions
                            .get(surface)
                            .is_some_and(|graph| {
                                runtime.graph_dirty
                                    || graph.core.is_animating()
                                    || graph.core.waiting_long_press()
                            })
                }))
            .then_some(frame_delay),
            runtime.media.as_ref().and_then(|media| media.next_update()),
        ]
        .into_iter()
        .flatten()
        .min();
        let mut command = if let Some(delay) = delay {
            match receive.recv_timeout(delay) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("Android project sender disappeared")
                }
            }
        } else {
            Some(receive.recv().expect("Android project sender disappeared"))
        };
        last_update = Instant::now();
        changed |= runtime.poll_inspector();
        while let Some(next) = command {
            match next {
                ProjectCommand::Request(request, source, response) => {
                    let result = runtime.request_with_source(&request, source);
                    if request["op"] == "native_palette" && result.is_ok() {
                        last_preview = None;
                    }
                    changed = true;
                    picture_dirty |= !matches!(
                        request["op"].as_str(),
                        Some(
                            "status"
                                | "settings_schema"
                                | "preferences_schema"
                                | "performance"
                                | "performance_copy"
                                | "export_schema"
                                | "export_destination"
                                | "caption_export_schema"
                        )
                    );
                    let _ = response.send(result);
                }
                ProjectCommand::Wake => {}
                ProjectCommand::Stop => {
                    if let Some(media) = runtime.media.take() {
                        media.stop();
                    }
                    runtime
                        .synchronize_lifecycle()
                        .expect("apply final project lifecycle");
                    if let Some(scene) = runtime.timeline.as_mut() {
                        scene.suspend();
                    }
                    runtime
                        .backend
                        .cursor(player::current_time(&runtime.player));
                    runtime
                        .backend
                        .close()
                        .expect("save and close Android project");
                    return;
                }
            }
            command = receive.try_recv().ok();
        }
        match runtime.synchronize_lifecycle() {
            Ok(value) => changed |= value,
            Err(error) => {
                log(format!("project lifecycle: {error}"));
                changed = true;
            }
        }
        match runtime.changed() {
            Ok(value) => {
                changed |= value;
                picture_dirty |= value;
            }
            Err(error) => {
                log(format!("project reload: {error}"));
                changed = true;
            }
        }
        // Export progress updates native controls, not the document or timeline
        // picture. A paused timeline stays idle while an export is running.
        changed |= runtime.poll_timeline_service();
        changed |= runtime.poll_export();
        changed |= runtime.poll_preview_image();
        let (
            events,
            next_viewport,
            next_graph_viewports,
            next_preview_viewport,
            at,
            overflow,
            surfaces_changed,
        ) = {
            let mut pending = input.lock().expect("Android input queue poisoned");
            let events = std::mem::take(&mut pending.events);
            (
                events,
                pending.viewport,
                pending.graph_viewports.clone(),
                pending.preview_viewport,
                pending.at.take(),
                std::mem::take(&mut pending.overflow),
                std::mem::take(&mut pending.surfaces_changed),
            )
        };
        if surfaces_changed {
            picture_dirty = true;
            runtime.graph_dirty = true;
            last_preview = None;
            changed = true;
        }
        let reset = generation != runtime.generation;
        generation = runtime.generation;
        if reset {
            runtime.preview_frame = None;
            preview_texture = None;
            publish_frame(
                &targets,
                0,
                Frame {
                    size: (0, 0),
                    palette: runtime.native_palette,
                    picture: None,
                    preview: None,
                    density: 1.0,
                    input_at: None,
                    record_ns: 0,
                },
            );
            picture_dirty = true;
        }
        if reset && let Some(scene) = runtime.timeline.as_mut() {
            scene.set_wake(notify.clone());
        }
        if next_viewport != viewport {
            picture_dirty = true;
            let capture = runtime.screen_recording_pending();
            if !next_viewport.visible
                && let Some(scene) = runtime.timeline.as_mut()
            {
                if capture {
                    scene.suspend_while_recording();
                } else {
                    scene.suspend();
                }
            }
            viewport = next_viewport;
        }
        if next_graph_viewports != graph_viewports {
            graph_viewports = next_graph_viewports;
            runtime.graph_dirty = true;
        }
        if next_preview_viewport != preview_viewport {
            if let Err(error) = runtime.cancel_preview_edit() {
                runtime.error = Some(error);
            }
            preview_viewport = next_preview_viewport;
            last_preview = None;
            changed = true;
        }
        if overflow {
            runtime.error = Some(runtime.cancel_graph_edit().err().unwrap_or_else(|| "Too many input events queued while the project was busy; the gesture was cancelled. Please retry.".into()));
            changed = true;
        }
        let before = player::snapshot(&runtime.player);
        let mut requests = None;
        if reset {
            if let Some(scene) = runtime.timeline.as_mut() {
                scene.event(shrimply_timeline_skia::scene::Event::Cancel);
            }
        } else if runtime.timeline.is_some() {
            let mut events = std::collections::VecDeque::from(events);
            while let Some(event) = events.pop_front() {
                use crate::input::Input;
                use shrimply_components_skia::input::TouchPhase;
                use shrimply_timeline_skia::scene::Event;
                let begins_edit = match &event {
                    Input::Scene(Event::Press { .. } | Event::ContextMenu { .. })
                    | Input::Key(_)
                    | Input::History { .. }
                    | Input::TogglePlaying
                    | Input::Preview { phase: 0, .. }
                    | Input::PreviewPen { phase: 0, .. }
                    | Input::PreviewContext { .. } => true,
                    Input::Scene(Event::Touch(changes)) | Input::PreviewTouch { changes, .. } => {
                        changes
                            .iter()
                            .any(|change| change.phase == TouchPhase::Begin)
                    }
                    _ => false,
                };
                if begins_edit
                    && let Err(error) = runtime
                        .cancel_graph_edit()
                        .and_then(|()| runtime.leave_numeric())
                {
                    runtime.error = Some(error);
                    changed = true;
                    continue;
                }
                if begins_edit
                    && !matches!(
                        &event,
                        Input::Preview { .. }
                            | Input::PreviewPen { .. }
                            | Input::PreviewTouch { .. }
                            | Input::PreviewContext { .. }
                    )
                    && let Err(error) = runtime.cancel_preview_edit()
                {
                    runtime.error = Some(error);
                    changed = true;
                    continue;
                }
                let timeline_event = !matches!(
                    &event,
                    crate::input::Input::Graph(..)
                        | crate::input::Input::Preview { .. }
                        | crate::input::Input::PreviewPen { .. }
                        | crate::input::Input::PreviewKey { .. }
                        | crate::input::Input::PreviewTouch { .. }
                        | crate::input::Input::PreviewScroll { .. }
                        | crate::input::Input::PreviewContext { .. }
                );
                match event {
                    crate::input::Input::Graph(surface, event) => {
                        let size =
                            graph_viewports
                                .get(&surface)
                                .map_or(glam::Vec2::ZERO, |viewport| {
                                    glam::vec2(
                                        viewport.width as f32 / viewport.density,
                                        viewport.height as f32 / viewport.density,
                                    )
                                });
                        if let Err(error) = runtime.graph_input(surface, event, size) {
                            runtime.error = Some(error);
                        }
                    }
                    crate::input::Input::Scene(event) => runtime
                        .timeline
                        .as_mut()
                        .expect("open timeline")
                        .event(event),
                    crate::input::Input::Key(action) => {
                        if let Err(error) = runtime.timeline_key_action(action) {
                            runtime.error = Some(error);
                        }
                    }
                    crate::input::Input::TogglePlaying => player::toggle_playing(&runtime.player),
                    crate::input::Input::AdvancePlaybackSpeed => {
                        player::step_playback_speed_forward(&runtime.player)
                    }
                    crate::input::Input::History { redo } => {
                        runtime
                            .timeline
                            .as_mut()
                            .expect("open timeline")
                            .event(shrimply_timeline_skia::scene::Event::Cancel);
                        if let Err(error) =
                            runtime.request(&json!({"op":if redo { "redo" } else { "undo" }}))
                        {
                            runtime.error = Some(error);
                        }
                    }
                    crate::input::Input::PreviewTouch { changes, size } => {
                        match runtime.preview_touch(&changes, size) {
                            Ok(timeline_changed) => picture_dirty |= timeline_changed,
                            Err(error) => runtime.error = Some(error),
                        }
                    }
                    crate::input::Input::PreviewContext { point, touch } => {
                        runtime.show_preview_context(point, touch)
                    }
                    crate::input::Input::PreviewKey { event, character } => {
                        use shrimply_preview_interaction_skia::surface::{KeyState, Modifiers};
                        match runtime.preview_keyboard(event) {
                            Ok(false)
                                if event.state == KeyState::Pressed
                                    && !event.modifiers.contains(Modifiers::ALT)
                                    && !event.modifiers.contains(Modifiers::META) =>
                            {
                                if let Some(fallback) = character.and_then(|character| {
                                    crate::input::key_input(
                                        character,
                                        event.modifiers.contains(Modifiers::CONTROL),
                                        event.modifiers.contains(Modifiers::SHIFT),
                                    )
                                }) {
                                    events.push_front(fallback);
                                }
                            }
                            Ok(_) => {}
                            Err(error) => runtime.error = Some(error),
                        }
                    }
                    crate::input::Input::PreviewPen {
                        phase,
                        flags,
                        size,
                        samples,
                    } => {
                        if let Err(error) = runtime.preview_pen(phase, flags, size, &samples) {
                            runtime.error = Some(error);
                        }
                    }
                    crate::input::Input::PreviewScroll {
                        position,
                        delta,
                        flags,
                    } => {
                        if let Err(error) = runtime.preview_scroll(position, delta, flags) {
                            runtime.error = Some(error);
                        }
                    }
                    crate::input::Input::Preview {
                        phase,
                        position,
                        size,
                        flags,
                    } => match runtime.preview_native_pointer(phase, position, size, flags) {
                        Ok(_) => {}
                        Err(error) => runtime.error = Some(error),
                    },
                }
                picture_dirty |= timeline_event;
                changed = true;
            }
        }
        changed |= runtime.poll_preview_input();
        meter_animating = audio_meter.update(runtime.take_audio_peaks(), Instant::now());
        if let Some(scene) = runtime.timeline.as_mut() {
            picture_dirty |= meter_animating && scene.audio_monitor_progress() > 0.0;
            if runtime.foreground && viewport.visible && viewport.width > 0 && viewport.height > 0 {
                let size = shrimply_components_skia::canvas::vec2(
                    viewport.width as f32 / viewport.density,
                    viewport.height as f32 / viewport.density,
                );
                let meter_width = shrimply_components_skia::audio_meter::DEFAULT_WIDTH as f32;
                let shown_meter_width = meter_width * scene.audio_monitor_progress() as f32;
                let timeline_size = shrimply_components_skia::canvas::vec2(
                    (size.x - shown_meter_width).max(1.0),
                    size.y,
                );
                if picture_dirty || scene.needs_redraw(timeline_size) || benchmark_running {
                    if benchmark_running {
                        let direction = if benchmark_frames / 120 % 2 == 0 {
                            -1.0
                        } else {
                            1.0
                        };
                        scene.scroll(
                            shrimply_components_skia::canvas::vec2(400.0, 80.0),
                            shrimply_components_skia::canvas::vec2(direction * 3.0, 0.0),
                            false,
                            shrimply_timeline_skia::view::TimelineScrollInput::Surface,
                        );
                        benchmark_frames += 1;
                    }
                    let record_start = Instant::now();
                    let mut recorder = skia_safe::PictureRecorder::new();
                    let canvas =
                        recorder.begin_recording(skia_safe::Rect::from_wh(size.x, size.y), false);
                    canvas.save();
                    canvas.clip_rect(
                        skia_safe::Rect::from_wh(timeline_size.x, timeline_size.y),
                        None,
                        false,
                    );
                    let layout_before = (scene.tools_progress(), scene.audio_monitor_progress());
                    scene.draw(canvas, timeline_size);
                    changed |=
                        layout_before != (scene.tools_progress(), scene.audio_monitor_progress());
                    canvas.restore();
                    if shown_meter_width > 0.0 {
                        canvas.save();
                        canvas.translate((timeline_size.x, 0.0));
                        canvas.clip_rect(
                            skia_safe::Rect::from_wh(shown_meter_width, size.y),
                            None,
                            false,
                        );
                        audio_meter.draw(canvas, meter_width, size.y);
                        canvas.restore();
                    }
                    let picture = recorder
                        .finish_recording_as_picture(None)
                        .expect("record timeline scene");
                    publish_frame(
                        &targets,
                        0,
                        Frame {
                            size: (viewport.width, viewport.height),
                            palette: runtime.native_palette,
                            picture: Some(picture),
                            preview: None,
                            density: viewport.density,
                            input_at: at.or(input_at.take()),
                            record_ns: record_start.elapsed().as_nanos(),
                        },
                    );
                    picture_dirty = false;
                }
            }
            requests = Some(scene.take_requests());
            while let Some(error) = scene.take_error() {
                runtime.error = Some(error);
                changed = true;
            }
        }
        if let Some(requests) = requests {
            if let Some((point, touch)) = requests.context_menu {
                runtime.show_context_menu(point, touch);
                changed = true;
            }
            if requests.pause_playback {
                player::set_playing(&runtime.player, false);
            }
            if let Some(request) = requests.track_add {
                let key = request.key;
                let track_id = runtime
                    .backend
                    .project()
                    .and_then(|project| {
                        shrimply_timeline_edit::selection_state::track_address(&project, key)
                    })
                    .map(|address| address.track_id().to_string());
                let import_targets: Vec<_> = runtime.backend.project().map(|project| request.import_targets.iter().map(|key| {
                    let id = shrimply_timeline_edit::selection_state::track_address(&project, *key).map(|address| address.track_id().to_string());
                    json!({"kind":key.kind.label().to_ascii_lowercase(),"track":key.track_index,"track_id":id})
                }).collect()).unwrap_or_default();
                if let Err(error) = runtime.request(&json!({"op":"show_track_add","kind":key.kind.label().to_ascii_lowercase(),"track":key.track_index,"track_id":track_id,"position":request.position,"import_targets":import_targets,"bottom_sheet":shrimply_components_core::layout::menu_uses_bottom_sheet(if request.touch { "touch" } else { "pointer" }),"x":request.point.x,"y":request.point.y})) { runtime.error = Some(error); }
                changed = true;
            }
            for request in [requests.audio_record, requests.video_record]
                .into_iter()
                .flatten()
            {
                if let Err(error) = runtime.request_recording(request) {
                    runtime.error = Some(error);
                }
                changed = true;
            }
        }
        let after = player::snapshot(&runtime.player);
        if after.revision != before.revision && !runtime.preview_interaction.active() {
            if !benchmark {
                runtime
                    .timeline_changed()
                    .expect("publish canonical timeline edit");
            }
            changed = true;
        } else if after != before {
            runtime.sync_cursor();
            changed = true;
        }
        if (runtime.foreground || runtime.screen_recording_clock_active())
            && player::snapshot(&runtime.player).playing
        {
            changed |= runtime.tick();
        }
        if runtime.screen_recording_clock_active()
            && let Some(scene) = runtime.timeline.as_mut()
        {
            changed |= scene.maintain_video_recording();
        }
        match runtime.poll_video_recording() {
            Ok(update) => changed |= update,
            Err(error) => {
                runtime.error = Some(error);
                changed = true;
            }
        }
        if runtime.foreground {
            for (surface, graph_viewport) in &graph_viewports {
                if !graph_viewport.visible
                    || graph_viewport.width == 0
                    || graph_viewport.height == 0
                {
                    continue;
                }
                let size = shrimply_components_skia::canvas::vec2(
                    graph_viewport.width as f32 / graph_viewport.density,
                    graph_viewport.height as f32 / graph_viewport.density,
                );
                changed |= runtime.poll_graph(*surface, size);
                if runtime
                    .framegraphs
                    .sessions
                    .get(surface)
                    .is_some_and(|graph| runtime.graph_dirty || graph.core.is_animating())
                {
                    let record_start = Instant::now();
                    let mut recorder = skia_safe::PictureRecorder::new();
                    let canvas =
                        recorder.begin_recording(skia_safe::Rect::from_wh(size.x, size.y), false);
                    runtime.graph_draw(*surface, canvas, size);
                    publish_frame(
                        &targets,
                        *surface,
                        Frame {
                            size: (graph_viewport.width, graph_viewport.height),
                            palette: runtime.native_palette,
                            picture: Some(
                                recorder
                                    .finish_recording_as_picture(None)
                                    .expect("record frame graph"),
                            ),
                            preview: None,
                            density: graph_viewport.density,
                            input_at: at,
                            record_ns: record_start.elapsed().as_nanos(),
                        },
                    );
                }
            }
            runtime.graph_dirty = false;
        }
        let mut preview_dirty = reset || changed;
        if let Some(media) = &runtime.media {
            let update = media.poll();
            changed |= runtime.preview_render_update(update.render_loading, update.render_elapsed);
            if let Some(event) = update.visual
                && runtime.preview_presented(&event)
            {
                use shrimply_preview_render_core::worker::VideoEvent;
                preview_texture = match event {
                    VideoEvent::Frame { frame, .. } => {
                        runtime.preview_frame = Some(frame.clone());
                        Some(frame.display)
                    }
                    VideoEvent::Clear { .. } => {
                        runtime.preview_frame = None;
                        None
                    }
                    _ => unreachable!("preview visual event"),
                };
                preview_dirty = true;
            }
            if let Some(error) = update.errors.into_iter().next_back() {
                runtime.error = Some(error);
                changed = true;
            }
        }
        if runtime.foreground && preview_dirty && preview_viewport.visible {
            let size = glam::vec2(
                preview_viewport.width as f32 / preview_viewport.density,
                preview_viewport.height as f32 / preview_viewport.density,
            );
            if let Err(error) = runtime.prepare_preview_overlay(size) {
                runtime.error = Some(error);
                changed = true;
            }
            let preview = crate::preview::Frame::new(&runtime, preview_texture.clone());
            if last_preview.as_ref() != Some(&preview) {
                last_preview = Some(preview.clone());
                publish_frame(
                    &targets,
                    1,
                    Frame {
                        size: (preview_viewport.width, preview_viewport.height),
                        palette: runtime.native_palette,
                        picture: None,
                        preview: Some(preview),
                        density: 1.0,
                        input_at: None,
                        record_ns: 0,
                    },
                );
            }
        }
        if at.is_some() && picture_dirty {
            input_at = input_at.or(at);
        }
    }
}

fn sync_viewport(
    input: &Mutex<crate::input::Queue>,
    role: usize,
    width: u32,
    height: u32,
    density: f32,
    visible: bool,
    remove_graph: bool,
) {
    let mut pending = input.lock().expect("Android input queue poisoned");
    let next = crate::input::Viewport {
        width,
        height,
        density,
        visible: visible && width > 0 && height > 0,
    };
    let changed = if remove_graph && role >= 2 {
        pending.graph_viewports.remove(&role).is_some()
    } else {
        let slot = match role {
            0 => &mut pending.viewport,
            1 => &mut pending.preview_viewport,
            _ => pending.graph_viewports.entry(role).or_default(),
        };
        let changed = *slot != next;
        *slot = next;
        changed
    };
    pending.surfaces_changed = true;
    if !next.visible && (changed || remove_graph) {
        pending.push(match role {
            0 => crate::input::Input::Scene(shrimply_timeline_skia::scene::Event::Cancel),
            1 => crate::input::Input::Preview {
                flags: 0,
                phase: 3,
                position: glam::Vec2::ZERO,
                size: glam::Vec2::ZERO,
            },
            _ => crate::input::Input::Graph(
                role,
                crate::input::GraphInput::Pointer {
                    kind: 29,
                    point: glam::Vec2::ZERO,
                    detail: 0.0,
                    flags: 0,
                    extra: 0.0,
                },
            ),
        });
    }
    pending.surfaces_changed = true;
}

struct RenderWorker {
    receive: mpsc::Receiver<RenderCommand>,
    benchmark: bool,
    latest: Arc<Mutex<RenderSlot>>,
    frame_ready: Arc<Condvar>,
    benchmark_submitted: Arc<AtomicUsize>,
    role: usize,
    active: bool,
}

struct RenderState {
    role: usize,
    current: Option<Frame>,
    current_palette: Option<crate::runtime::palette::NativePalette>,
    source_frames: shrimply_profiling::SourceFrameTracker,
    renderer: Option<Renderer>,
    window: Option<NativeWindow>,
    width: u32,
    height: u32,
    density: f32,
    active: bool,
    visible: bool,
    dirty: bool,
}

impl RenderState {
    fn new(role: usize, active: bool) -> Self {
        Self {
            role,
            current: None,
            current_palette: None,
            source_frames: shrimply_profiling::SourceFrameTracker::default(),
            renderer: None,
            window: None,
            width: 0,
            height: 0,
            density: 1.0,
            active,
            visible: role < 2,
            dirty: false,
        }
    }

    fn ensure_renderer(&mut self) {
        if self.renderer.is_none()
            && self.active
            && self.visible
            && self.width > 0
            && self.height > 0
        {
            self.renderer = self
                .window
                .as_ref()
                .map(|window| Renderer::new(window, self.width, self.height));
        }
    }

    fn draw_current(&mut self) -> bool {
        let Some(renderer) = self.renderer.as_mut() else {
            return true;
        };
        let Some(frame) = self.current.as_ref() else {
            return true;
        };
        if let Some(palette) = frame.palette
            && self.current_palette != Some(palette)
        {
            palette.apply();
            self.current_palette = Some(palette);
        }
        let valid = if let Some(preview) = &frame.preview {
            crate::preview::draw(
                renderer,
                preview,
                self.density,
                frame.size == (self.width, self.height),
            )
        } else {
            renderer.draw(|canvas, _, _| {
                canvas.clear(skia_safe::Color4f::from(
                    shrimply_cross_ui_theme::current().view_bg,
                ));
                if let Some(picture) = &frame.picture {
                    canvas.save();
                    canvas.scale((frame.density, frame.density));
                    canvas.draw_picture(picture, None, None);
                    canvas.restore();
                }
                None
            })
        };
        if valid {
            self.source_frames.submitted(
                frame
                    .preview
                    .as_ref()
                    .and_then(|preview| preview.source_frame),
            );
        } else {
            renderer.resize(self.width, self.height);
        }
        valid
    }

    fn present_sync(&mut self, latest: &Mutex<RenderSlot>, frame_ready: &Condvar) {
        if let Some(next) = latest
            .lock()
            .expect("Android frame slot poisoned")
            .visible
            .take()
        {
            self.visible = next;
        }
        self.ensure_renderer();
        if !self.active
            || !self.visible
            || self.width == 0
            || self.height == 0
            || self.renderer.is_none()
        {
            self.dirty = false;
            return;
        }
        let deadline = Instant::now() + SYNC_RESIZE_TIMEOUT;
        let mut slot = latest.lock().expect("Android frame slot poisoned");
        loop {
            if let Some(frame) = slot.frame.take() {
                self.current = Some(frame);
            }
            if self
                .current
                .as_ref()
                .is_some_and(|frame| frame.size == (self.width, self.height))
            {
                break;
            }
            let Some(remaining) = deadline
                .checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
            else {
                break;
            };
            slot = frame_ready
                .wait_timeout(slot, remaining)
                .expect("Android frame slot poisoned")
                .0;
        }
        drop(slot);
        if self
            .current
            .as_ref()
            .is_some_and(|frame| frame.size == (self.width, self.height) || self.role == 1)
        {
            // Retry once if acquire_next_image returned OUT_OF_DATE_KHR so the
            // TextureView receives a presented buffer before returning to the UI thread.
            let presented = self.draw_current() || self.draw_current();
            self.dirty = !presented;
        } else {
            self.dirty = false;
        }
    }
}

/// Native window lifecycle never waits for project imports or document commits.
fn run_render(worker: RenderWorker) {
    let RenderWorker {
        receive,
        benchmark,
        latest,
        frame_ready,
        benchmark_submitted,
        role,
        active,
    } = worker;
    let mut state = RenderState::new(role, active);
    let mut frames = Vec::new();
    let mut latency = Vec::new();
    let mut recordings = Vec::new();
    let mut intervals = Vec::new();
    let mut previous_frame: Option<Instant> = None;
    let initial_rss = if benchmark { rss_kib() } else { 0 };
    let (mut min_rss, mut max_rss) = (initial_rss, initial_rss);
    'render: loop {
        let mut command = if state.dirty {
            receive.try_recv().ok()
        } else {
            Some(receive.recv().expect("Android render sender disappeared"))
        };
        while let Some(next) = command {
            match next {
                RenderCommand::Surface(next, w, h, d, visible, ack) => {
                    if next.is_none() {
                        let mut slot = latest.lock().expect("Android frame slot poisoned");
                        slot.frame = None;
                        slot.visible = None;
                    }
                    // Release the old VkSurface before its retained native window.
                    if next.is_some() && (w == 0 || h == 0) {
                        drop(state.renderer.take());
                    } else if let Some(renderer) = state.renderer.as_mut() {
                        renderer.set_window(next.as_ref(), w, h);
                    }
                    state.window = next;
                    (state.width, state.height, state.density) = (w, h, d);
                    if state.window.is_none() {
                        state.current = None;
                        state.visible = role < 2;
                        state.dirty = false;
                    } else {
                        state.visible = visible;
                        state.present_sync(&latest, &frame_ready);
                    }
                    ack.send(()).expect("surface acknowledgment");
                }
                RenderCommand::Resize(w, h, ack) => {
                    (state.width, state.height) = (w, h);
                    if state.width == 0 || state.height == 0 {
                        drop(state.renderer.take());
                        state.dirty = false;
                    } else {
                        if let Some(renderer) = state.renderer.as_mut()
                            && state.window.is_some()
                        {
                            renderer.resize(state.width, state.height);
                        }
                        state.present_sync(&latest, &frame_ready);
                    }
                    ack.send(()).expect("resize acknowledgment");
                }
                RenderCommand::Active(next, ack) => {
                    state.active = next;
                    if !state.active {
                        drop(state.renderer.take());
                    } else {
                        state.ensure_renderer();
                    }
                    state.dirty = state.renderer.is_some();
                    ack.send(()).expect("render lifecycle acknowledgment");
                }
                RenderCommand::Wake => {}
                RenderCommand::Stop => break 'render,
            }
            command = receive.try_recv().ok();
        }
        let (frame, visibility) = {
            let mut slot = latest.lock().expect("Android frame slot poisoned");
            (slot.frame.take(), slot.visible.take())
        };
        if let Some(next) = visibility {
            state.visible = next;
            state.ensure_renderer();
            state.dirty |= state.visible;
        }
        if let Some(next) = frame {
            state.current = Some(next);
            state.dirty = true;
        }
        if !state.active || !state.visible || state.window.is_none() {
            state.dirty = false;
        }
        if !state.dirty || state.renderer.is_none() {
            state.dirty = false;
            continue;
        }
        let Some(frame) = state.current.as_ref() else {
            state.dirty = false;
            continue;
        };
        if role != 1 && frame.size != (0, 0) && frame.size != (state.width, state.height) {
            state.dirty = false;
            continue;
        }
        let frame_record_ns = frame.record_ns;
        let frame_input_at = frame.input_at;
        let start = Instant::now();
        let valid = state.draw_current();
        state.dirty = !valid;
        if benchmark && valid && frames.len() < BENCHMARK_FRAMES {
            frames.push(start.elapsed().as_nanos());
            recordings.push(frame_record_ns);
            benchmark_submitted.store(frames.len(), Ordering::Release);
            if let Some(previous) = previous_frame.replace(start) {
                intervals.push(start.duration_since(previous).as_nanos());
            }
            if let Some(at) = frame_input_at {
                latency.push(at.elapsed().as_nanos());
            }
            let rss = rss_kib();
            min_rss = min_rss.min(rss);
            max_rss = max_rss.max(rss);
            if frames.len() == BENCHMARK_FRAMES {
                let missed: u128 = intervals
                    .iter()
                    .map(|duration| {
                        ((duration + FRAME_BUDGET_NS / 2) / FRAME_BUDGET_NS).saturating_sub(1)
                    })
                    .sum();
                recordings.sort_unstable();
                frames.sort_unstable();
                latency.sort_unstable();
                intervals.sort_unstable();
                log(format!(
                    "benchmark frames={} submit_p50_us={} submit_p95_us={} missed_60hz_intervals={} input_samples={} input_p50_us={} input_p95_us={} rss_initial_kib={} rss_final_kib={} rss_min_kib={} rss_max_kib={} (submission/acquire/present wall time; input through submission)",
                    frames.len(),
                    math::percentile(&frames, 50) / 1000,
                    math::percentile(&frames, 95) / 1000,
                    missed,
                    latency.len(),
                    math::percentile(&latency, 50) / 1000,
                    math::percentile(&latency, 95) / 1000,
                    initial_rss,
                    rss,
                    min_rss,
                    max_rss
                ));
                log(format!(
                    "frame_interval_p50_us={} frame_interval_p95_us={} native_debug={} input_metric=JNI_queue_to_submission",
                    math::percentile(&intervals, 50) / 1000,
                    math::percentile(&intervals, 95) / 1000,
                    cfg!(debug_assertions)
                ));
                log(format!(
                    "scene_record_p50_us={} scene_record_p95_us={}",
                    math::percentile(&recordings, 50) / 1000,
                    math::percentile(&recordings, 95) / 1000
                ));
                log("rendering idle; waiting for input or lifecycle changes".into());
            }
        }
    }
    drop(state.renderer);
    drop(state.window);
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .expect("read Android memory status")
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:").map(|value| {
                value
                    .split_whitespace()
                    .next()
                    .expect("RSS value")
                    .parse()
                    .expect("numeric RSS")
            })
        })
        .expect("RSS status")
}
unsafe fn session<'a>(handle: jlong) -> &'a Session {
    assert_ne!(handle, 0, "closed Android native session");
    unsafe { &*(handle as *const Session) }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeTimeline_initialize<'caller>(
    mut unowned: EnvUnowned<'caller>,
    this: JObject<'caller>,
    application: JObject<'caller>,
    config: JString<'caller>,
    cache: JString<'caller>,
    fonts: JString<'caller>,
    video_encoder_names: JString<'caller>,
) {
    unowned
        .with_env(|env| -> jni::errors::Result<()> {
            let certificate_context = env.new_local_ref(&application)?;
            rustls_platform_verifier::android::init_with_env(env, certificate_context)?;
            let application = env.new_global_ref(application)?;
            let application = APPLICATION.get_or_init(|| application);
            let vm = env.get_java_vm()?.get_raw().cast();
            crate::localization::initialize_vm(env.get_java_vm()?);
            export::initialize(env.get_java_vm()?, env.new_global_ref(this)?);
            shrimply_export_core::android::set_encoder_names(
                serde_json::from_str(&video_encoder_names.try_to_string(env)?)
                    .expect("Android video encoder names"),
            );
            // Native adapters retain and pass the process resources once. The
            // existing shared audio engine and MediaCodec decoder own their use.
            unsafe {
                shrimply_audio_engine::initialize_android_context(
                    vm,
                    application.as_obj().as_raw().cast(),
                );
                shrimply_video_decoder::initialize_java_vm(vm)
                    .expect("initialize Android media Java VM");
            }
            let config = config.try_to_string(env)?;
            let cache = cache.try_to_string(env)?;
            shrimply_path_core::initialize_android_directories(config.into(), cache.into())
                .expect("initialize Android application directories");
            shrimply_render_vulkan::device::Device::set_pipeline_cache_path(
                shrimply_path_core::cache_directory()
                    .expect("Android cache directory")
                    .join("vulkan-pipelines.bin"),
            )
            .expect("initialize shared Vulkan pipeline cache");
            let font_directories: Vec<std::path::PathBuf> =
                serde_json::from_str(&fonts.try_to_string(env)?)
                    .expect("Android system font directories");
            let resources = (
                font_directories,
                shrimply_path_core::cache_directory()
                    .expect("Android cache directory")
                    .join("fontconfig"),
            );
            if FONT_RESOURCES.set(resources.clone()).is_err()
                && FONT_RESOURCES.get() != Some(&resources)
            {
                panic!("Android font resources changed after initialization");
            }
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeTimeline_throughput<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _this: JObject<'caller>,
    path: JString<'caller>,
    kind: JString<'caller>,
    frames: jint,
) -> JString<'caller> {
    unowned
        .with_env(|env| -> jni::errors::Result<JString<'caller>> {
            let path = path.try_to_string(env)?;
            let kind = kind.try_to_string(env)?;
            let result: Result<String, String> = (|| {
                initialize_fonts();
                let count = u64::try_from(frames).map_err(|_| "Invalid frame count")?;
                let project = shrimply_project_document::project::load_project_snapshot(
                    std::path::Path::new(&path),
                )?;
                let project = shrimply_preview_render_core::benchmark::scene(project, &kind)?;
                let report = shrimply_preview_render_core::benchmark::run(
                    project,
                    count,
                    |project, observer| {
                        shrimply_preview_render_vulkan::worker::spawn_worker(
                            project,
                            Default::default(),
                            Some(observer),
                            None,
                        )
                    },
                    // The production Vulkan worker publishes only after its GPU fence completes.
                    |_| Ok(()),
                )?;
                std::fs::write(
                    shrimply_path_core::cache_directory()?.join("pipeline-last-frame.png"),
                    report.frame.png()?,
                )
                .map_err(|error| error.to_string())?;
                Ok(report.text)
            })();
            let response = match result {
                Ok(report) => json!({"ok":true,"report":report}),
                Err(error) => json!({"ok":false,"error":error}),
            };
            env.new_string(response.to_string())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeTimeline_create<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _this: JObject<'caller>,
    stress: jboolean,
    benchmark: jboolean,
    files: JObject<'caller>,
    listener: JObject<'caller>,
) -> jlong {
    static PANIC_LOG: std::sync::Once = std::sync::Once::new();
    PANIC_LOG.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            log(format!("native editor panic: {info}"));
            std::process::abort();
        }))
    });
    unowned
        .with_env(|env| -> jni::errors::Result<jlong> {
            let vm = env.get_java_vm()?;
            let listener = env.new_global_ref(listener)?;
            let files = env.new_global_ref(files)?;
            let storage_vm = env.get_java_vm()?;
            let mirror_listener: crate::project::MirrorListener =
                Arc::new(move |root, relative| {
                    storage_vm
                        .attach_current_thread(|env| -> jni::errors::Result<()> {
                            let root = env.new_string(root)?;
                            let relative = env.new_string(relative)?;
                            env.call_method(
                                &files,
                                jni_str!("localChanged"),
                                jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
                                &[(&root).into(), (&relative).into()],
                            )?;
                            Ok(())
                        })
                        .expect("notify Android project storage");
                });
            let (project, project_receive) = mpsc::sync_channel(8);
            let targets = RenderTargets::default();
            let input = Arc::new(Mutex::new(crate::input::Queue::default()));
            let lifecycle = SharedLifecycle::default();
            let benchmark_submitted = Arc::new(AtomicUsize::new(0));
            let project_thread = {
                let targets = targets.clone();
                let lifecycle = lifecycle.clone();
                let wake = project.clone();
                let input = input.clone();
                let benchmark_submitted = benchmark_submitted.clone();
                thread::Builder::new()
                    .name("shrimply-project".into())
                    .spawn(move || {
                        run_project(
                            ProjectWorker {
                                receive: project_receive,
                                wake,
                                targets,
                                input,
                                lifecycle,
                                benchmark_submitted,
                            },
                            benchmark.then_some(stress),
                            move |payload| {
                                vm.attach_current_thread(|env| -> jni::errors::Result<()> {
                                    let payload = env.new_string(payload)?;
                                    env.call_method(
                                        &listener,
                                        jni_str!("changed"),
                                        jni_sig!("(Ljava/lang/String;)V"),
                                        &[(&payload).into()],
                                    )?;
                                    Ok(())
                                })
                                .expect("publish Android editor state");
                            },
                            mirror_listener,
                        )
                    })
                    .expect("start project thread")
            };
            Ok(Box::into_raw(Box::new(Session {
                input,
                lifecycle,
                project,
                targets,
                active: AtomicBool::new(true),
                benchmark,
                benchmark_submitted,
                project_thread: Some(project_thread),
            })) as jlong)
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeTimeline_listProjects<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _this: JObject<'caller>,
    include_remote: jboolean,
) -> JString<'caller> {
    unowned
        .with_env(|env| -> jni::errors::Result<JString<'caller>> {
            use shrimply_project_service::catalog;
            let result = if include_remote {
                catalog::list()
            } else {
                catalog::local_projects()
            };
            let payload = match result {
                Ok(projects) => json!({ "projects": projects }),
                Err(error) => json!({ "error": error }),
            };
            env.new_string(payload.to_string())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shrimply_app_NativeTimeline_request<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _this: JObject<'caller>,
    handle: jlong,
    request: JString<'caller>,
    descriptor: jint,
) -> JString<'caller> {
    use std::os::fd::FromRawFd;
    // The Java adapter transfers a detached ParcelFileDescriptor duplicate.
    // Keep ownership even if JSON parsing or JNI string conversion fails.
    let source = (descriptor >= 0).then(|| unsafe { std::fs::File::from_raw_fd(descriptor) });
    unowned
        .with_env(|env| -> jni::errors::Result<JString<'caller>> {
            let request = request.try_to_string(env)?;
            let response = match serde_json::from_str::<Value>(&request) {
                Ok(request) => {
                    let (send, receive) = mpsc::sync_channel(0);
                    unsafe { session(handle) }
                        .project
                        .send(ProjectCommand::Request(request, source, send))
                        .expect("send project request");
                    match receive.recv().expect("project request response") {
                        Ok(result) => json!({"ok":true,"result":result}),
                        Err(error) => json!({"ok":false,"error":error}),
                    }
                }
                Err(error) => {
                    json!({"ok":false,"error":format!("invalid editor request: {error}")})
                }
            };
            env.new_string(response.to_string())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeTimeline_surface(
    env: *mut JNIEnv,
    _this: jobject,
    handle: jlong,
    role: jint,
    surface: jobject,
    width: jint,
    height: jint,
    density: jfloat,
    visible: jboolean,
) {
    assert!(density.is_finite() && density > 0.0 && width >= 0 && height >= 0);
    unsafe {
        let window = if surface.is_null() {
            None
        } else {
            Some(
                NativeWindow::from_surface(env.cast(), surface.cast())
                    .expect("retain Android native window"),
            )
        };
        let session = session(handle);
        let role = usize::try_from(role).expect("nonnegative surface role");
        let detached = window.is_none();
        let active = session.active.load(Ordering::Acquire);
        let initial_visible = !detached && (role < 2 || visible);
        let target_send = {
            let mut targets = session.targets.lock().expect("Android surfaces poisoned");
            if !detached
                && let std::collections::btree_map::Entry::Vacant(entry) = targets.entry(role)
            {
                let (send, receive) = mpsc::sync_channel(8);
                let latest = Arc::new(Mutex::new(RenderSlot::default()));
                let frame_ready = Arc::new(Condvar::new());
                let slot = latest.clone();
                let ready = frame_ready.clone();
                let benchmark = session.benchmark && role == 0;
                let benchmark_submitted = session.benchmark_submitted.clone();
                let thread = thread::Builder::new()
                    .name(format!("shrimply-surface-{role}"))
                    .spawn(move || {
                        run_render(RenderWorker {
                            receive,
                            benchmark,
                            latest: slot,
                            frame_ready: ready,
                            benchmark_submitted,
                            role,
                            active,
                        })
                    })
                    .expect("start Vulkan surface thread");
                entry.insert(RenderTarget {
                    latest,
                    frame_ready,
                    send,
                    thread,
                    detached: None,
                    width: width as u32,
                    height: height as u32,
                    density,
                    visible: initial_visible,
                });
            }
            targets.get_mut(&role).map(|target| {
                if !detached && target.detached.is_some() {
                    shrimply_profiling::increment("Android / Graph workers reused");
                }
                target.detached = detached.then(Instant::now);
                target.width = if detached { 0 } else { width as u32 };
                target.height = if detached { 0 } else { height as u32 };
                target.density = density;
                target.visible = initial_visible;
                target.send.clone()
            })
        };
        if let Some(target_send) = target_send {
            if !detached {
                sync_viewport(
                    &session.input,
                    role,
                    width as u32,
                    height as u32,
                    density,
                    active && initial_visible,
                    false,
                );
                let _ = session.project.try_send(ProjectCommand::Wake);
            }
            let (send, receive) = mpsc::sync_channel(0);
            target_send
                .send(RenderCommand::Surface(
                    window,
                    width as u32,
                    height as u32,
                    density,
                    initial_visible,
                    send,
                ))
                .expect("send native surface");
            receive.recv().expect("wait for native surface release");
            if detached {
                let removed = if role < 2 {
                    session
                        .targets
                        .lock()
                        .expect("Android surfaces poisoned")
                        .remove(&role)
                } else {
                    None
                };
                if let Some(target) = removed {
                    target
                        .send
                        .send(RenderCommand::Stop)
                        .expect("stop detached surface");
                    target.thread.join().expect("Vulkan surface thread failed");
                }
                sync_viewport(&session.input, role, 0, 0, density, false, true);
                let _ = session.project.try_send(ProjectCommand::Wake);
            }
        }
        loop {
            let evicted = {
                let mut targets = session.targets.lock().expect("Android surfaces poisoned");
                if targets
                    .values()
                    .filter(|target| target.detached.is_some())
                    .count()
                    <= MAX_IDLE_GRAPH_SURFACES
                {
                    None
                } else {
                    let oldest = targets
                        .iter()
                        .filter_map(|(role, target)| target.detached.map(|at| (*role, at)))
                        .min_by_key(|(_, at)| *at)
                        .expect("idle graph surface")
                        .0;
                    targets.remove(&oldest)
                }
            };
            let Some(target) = evicted else {
                break;
            };
            shrimply_profiling::increment("Android / Graph workers evicted");
            target
                .send
                .send(RenderCommand::Stop)
                .expect("evict idle graph surface");
            target.thread.join().expect("Vulkan surface thread failed");
        }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeTimeline_visible(
    _env: *mut JNIEnv,
    _this: jobject,
    handle: jlong,
    role: jint,
    visible: jboolean,
) {
    let session = unsafe { session(handle) };
    let role = usize::try_from(role).expect("nonnegative surface role");
    let updated = {
        let mut targets = session.targets.lock().expect("Android surfaces poisoned");
        targets
            .get_mut(&role)
            .filter(|target| target.detached.is_none())
            .map(|target| {
                target.visible = visible;
                target
                    .latest
                    .lock()
                    .expect("Android frame slot poisoned")
                    .visible = Some(visible);
                (
                    target.send.clone(),
                    target.width,
                    target.height,
                    target.density,
                )
            })
    };
    if let Some((send, width, height, density)) = updated {
        let active = session.active.load(Ordering::Acquire);
        sync_viewport(
            &session.input,
            role,
            width,
            height,
            density,
            active && visible,
            false,
        );
        let _ = session.project.try_send(ProjectCommand::Wake);
        let _ = send.try_send(RenderCommand::Wake);
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeTimeline_resize(
    _env: *mut JNIEnv,
    _this: jobject,
    handle: jlong,
    role: jint,
    width: jint,
    height: jint,
) {
    assert!(width >= 0 && height >= 0);
    let current = unsafe { session(handle) };
    let role = usize::try_from(role).expect("nonnegative surface role");
    let updated = {
        let mut targets = current.targets.lock().expect("Android surfaces poisoned");
        targets
            .get_mut(&role)
            .filter(|target| target.detached.is_none())
            .map(|target| {
                target.width = width as u32;
                target.height = height as u32;
                (target.send.clone(), target.density, target.visible)
            })
    };
    if let Some((target_send, density, visible)) = updated {
        let active = current.active.load(Ordering::Acquire);
        sync_viewport(
            &current.input,
            role,
            width as u32,
            height as u32,
            density,
            active && visible,
            false,
        );
        let _ = current.project.try_send(ProjectCommand::Wake);
        let (ack_send, ack_recv) = mpsc::sync_channel(0);
        if target_send
            .send(RenderCommand::Resize(width as u32, height as u32, ack_send))
            .is_ok()
        {
            let _ = ack_recv.recv();
        }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeTimeline_active(
    _env: *mut JNIEnv,
    _this: jobject,
    handle: jlong,
    active: jboolean,
) {
    let observed = Instant::now();
    let session = unsafe { session(handle) };
    session
        .lifecycle
        .lock()
        .expect("Android lifecycle latch poisoned")
        .observe(active, observed);
    session.active.store(active, Ordering::Release);
    let targets: Vec<_> = session
        .targets
        .lock()
        .expect("Android surfaces poisoned")
        .iter()
        .map(|(&role, target)| {
            (
                role,
                target.send.clone(),
                target.detached.is_none(),
                target.width,
                target.height,
                target.density,
                target.visible,
            )
        })
        .collect();
    for (role, sender, attached, width, height, density, visible) in targets {
        if attached {
            sync_viewport(
                &session.input,
                role,
                width,
                height,
                density,
                active && visible,
                false,
            );
        }
        let (send, receive) = mpsc::sync_channel(0);
        if sender.send(RenderCommand::Active(active, send)).is_ok() {
            let _ = receive.recv();
        }
    }
    let _ = session.project.try_send(ProjectCommand::Wake);
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeTimeline_input(
    env: *mut JNIEnv,
    _this: jobject,
    handle: jlong,
    role: jint,
    array: jfloatArray,
) -> jboolean {
    unsafe {
        let length = ((**env).v1_1.GetArrayLength)(env, array) as usize;
        assert!(
            length > 0 && length <= MAX_INPUT_VALUES && length.is_multiple_of(INPUT_STRIDE),
            "invalid timeline input batch size"
        );
        let mut values = vec![0.0; length];
        ((**env).v1_1.GetFloatArrayRegion)(env, array, 0, length as jint, values.as_mut_ptr());
        assert!(!((**env).v1_2.ExceptionCheck)(env), "read input array");
        assert!(
            values.iter().all(|value| value.is_finite()),
            "nonfinite timeline input"
        );
        let Some(input) = crate::input::decode(
            &values,
            usize::try_from(role).expect("nonnegative input surface"),
        ) else {
            return false;
        };
        let session = session(handle);
        {
            let mut pending = session.input.lock().expect("Android input queue poisoned");
            // Lifecycle cancellations share the queue but are not user-input samples.
            pending.at.get_or_insert_with(Instant::now);
            pending.push(input);
        }
        let _ = session.project.try_send(ProjectCommand::Wake);
        true
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeTimeline_destroy(
    _env: *mut JNIEnv,
    _this: jobject,
    handle: jlong,
) {
    unsafe {
        let mut session = Box::from_raw(handle as *mut Session);
        let targets =
            std::mem::take(&mut *session.targets.lock().expect("Android surfaces poisoned"));
        for target in targets.values() {
            target
                .send
                .send(RenderCommand::Stop)
                .expect("stop rendering");
        }
        for target in targets.into_values() {
            target.thread.join().expect("Vulkan thread failed");
        }
        session
            .project
            .send(ProjectCommand::Stop)
            .expect("stop project");
        session
            .project_thread
            .take()
            .expect("project thread")
            .join()
            .expect("project thread failed");
    }
}
