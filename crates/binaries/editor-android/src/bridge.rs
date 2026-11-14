use crate::{
    math,
    runtime::{Runtime, SharedLifecycle},
};
use jni::sys::{JNIEnv, jboolean, jfloat, jfloatArray, jint, jlong, jobject};
use jni::{
    EnvUnowned,
    errors::ThrowRuntimeExAndDefault,
    jni_sig, jni_str,
    objects::{JObject, JString},
};
use ndk::native_window::NativeWindow;
use serde_json::{Value, json};
use shrimply_player_core as player;
use shrimply_render_vulkan::surface::Renderer;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const ANDROID_LOG_INFO: i32 = 4;
use crate::input::{INPUT_STRIDE, MAX_INPUT_VALUES};
const BENCHMARK_FRAMES: usize = 1200;
const FRAME_BUDGET_NS: u128 = 1_000_000_000 / 60;
const INSPECTOR_POLL_INTERVAL: Duration = Duration::from_millis(100);
static FONT_RESOURCES: std::sync::OnceLock<(Vec<std::path::PathBuf>, std::path::PathBuf)> =
    std::sync::OnceLock::new();

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
    Surface(Option<NativeWindow>, u32, u32, f32, SyncSender<()>),
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
    picture: Option<skia_safe::Picture>,
    preview: Option<crate::preview::Frame>,
    density: f32,
    input_at: Option<Instant>,
    record_ns: u128,
}
#[derive(Default)]
struct RenderSlot {
    frame: Option<Frame>,
    resize: Option<(u32, u32)>,
}

struct Session {
    latest: [Arc<Mutex<RenderSlot>>; 2],
    input: Arc<Mutex<crate::input::Queue>>,
    lifecycle: SharedLifecycle,
    render: [SyncSender<RenderCommand>; 2],
    project: SyncSender<ProjectCommand>,
    render_threads: Vec<JoinHandle<()>>,
    project_thread: Option<JoinHandle<()>>,
}

struct ProjectWorker {
    receive: mpsc::Receiver<ProjectCommand>,
    wake: SyncSender<ProjectCommand>,
    render: [SyncSender<RenderCommand>; 2],
    latest: [Arc<Mutex<RenderSlot>>; 2],
    input: Arc<Mutex<crate::input::Queue>>,
    lifecycle: SharedLifecycle,
    benchmark_submitted: Arc<AtomicUsize>,
}

/// The canonical document, its history and Scene stay on one worker. Only Skia's
/// immutable display list crosses to the Vulkan worker; no pixels cross the CPU.
fn run_project(worker: ProjectWorker, benchmark: Option<bool>, publish: impl Fn(&str)) {
    let (directories, cache) = FONT_RESOURCES
        .get()
        .expect("Android font resources configured");
    shrimply_fonts::native::fontconfig::initialize(directories, cache)
        .expect("initialize shared native font resources");
    let ProjectWorker {
        receive,
        wake,
        render,
        latest,
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
    )
    .expect("initialize Android project services");
    shrimply_cross_ui_theme::set_dark(true);
    if benchmark {
        let path = shrimply_path_core::cache_directory()
            .expect("Android benchmark cache")
            .join("timeline-benchmark.shrimp");
        shrimply_project_document::create_project_file(&path, &crate::benchmark::project(stress))
            .expect("create benchmark project");
        runtime
            .request(&json!({"op":"open","path":path}))
            .expect("open actual benchmark project");
        runtime
            .request(&json!({"op":"remove_recent","path":path}))
            .expect("keep benchmark out of project history");
    }
    let mut viewport = crate::input::Viewport::default();
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = project_wake.try_send(ProjectCommand::Wake);
    });
    if let Some(scene) = runtime.timeline.as_mut() {
        scene.set_wake(notify.clone());
    }
    let mut generation = runtime.generation;
    let mut changed = true;
    let mut picture_dirty = true;
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
        let playback_delay = (runtime.foreground && player::snapshot(&runtime.player).playing)
            .then_some(frame_delay);
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
            (runtime.foreground && runtime.inspector_visible && runtime.timeline.is_some())
                .then_some(INSPECTOR_POLL_INTERVAL),
            playback_delay,
            scene_delay,
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
                    changed = true;
                    picture_dirty = true;
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
        changed |= runtime.poll_export();
        let (events, next_viewport, at, overflow) = {
            let mut pending = input.lock().expect("Android input queue poisoned");
            let events = std::mem::take(&mut pending.events);
            (
                events,
                pending.viewport,
                pending.at.take(),
                std::mem::take(&mut pending.overflow),
            )
        };
        let reset = generation != runtime.generation;
        generation = runtime.generation;
        if reset {
            preview_texture = None;
            latest[0].lock().expect("Android frame slot poisoned").frame = Some(Frame {
                picture: None,
                preview: None,
                density: 1.0,
                input_at: None,
                record_ns: 0,
            });
            let _ = render[0].try_send(RenderCommand::Wake);
            picture_dirty = true;
        }
        if reset && let Some(scene) = runtime.timeline.as_mut() {
            scene.set_wake(notify.clone());
        }
        if next_viewport != viewport {
            picture_dirty = true;
            if !next_viewport.visible
                && let Some(scene) = runtime.timeline.as_mut()
            {
                scene.suspend();
            }
            viewport = next_viewport;
        }
        if overflow {
            runtime.error = Some("Too many input events queued while the project was busy; the gesture was cancelled. Please retry.".into());
            changed = true;
        }
        let before = player::snapshot(&runtime.player);
        let mut requests = None;
        if reset {
            if let Some(scene) = runtime.timeline.as_mut() {
                scene.event(shrimply_timeline_skia::scene::Event::Cancel);
            }
        } else if runtime.timeline.is_some() {
            for event in events {
                match event {
                    crate::input::Input::Scene(event) => runtime
                        .timeline
                        .as_mut()
                        .expect("open timeline")
                        .event(event),
                    crate::input::Input::Key(action) => {
                        match runtime
                            .timeline
                            .as_mut()
                            .expect("open timeline")
                            .key_action(action)
                        {
                            Ok(None) => {}
                            Ok(Some(_)) => runtime.error = Some(
                                "This action requires a native dialog which is not available yet."
                                    .into(),
                            ),
                            Err(error) => runtime.error = Some(error),
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
                }
                picture_dirty = true;
                changed = true;
            }
        }
        if let Some(scene) = runtime.timeline.as_mut() {
            if viewport.visible && viewport.width > 0 && viewport.height > 0 {
                let size = shrimply_components_skia::canvas::vec2(
                    viewport.width as f32 / viewport.density,
                    viewport.height as f32 / viewport.density,
                );
                if picture_dirty || scene.needs_redraw(size) || benchmark_running {
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
                    scene.draw(canvas, size);
                    let picture = recorder
                        .finish_recording_as_picture(None)
                        .expect("record timeline scene");
                    latest[0].lock().expect("Android frame slot poisoned").frame = Some(Frame {
                        picture: Some(picture),
                        preview: None,
                        density: viewport.density,
                        input_at: at.or(input_at.take()),
                        record_ns: record_start.elapsed().as_nanos(),
                    });
                    let _ = render[0].try_send(RenderCommand::Wake);
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
                if let Err(error) = runtime.request(&json!({"op":"show_track_add","kind":key.kind.label().to_ascii_lowercase(),"track":key.track_index,"track_id":track_id})) { runtime.error = Some(error); }
                changed = true;
            }
            if let Some(key) = requests.audio_record {
                let track_id = runtime
                    .backend
                    .project()
                    .and_then(|project| {
                        shrimply_timeline_edit::selection_state::track_address(&project, key)
                    })
                    .map(|address| address.track_id().to_string());
                if let Err(error) = runtime.request(&json!({"op":"show_audio_record","kind":"audio","track":key.track_index,"track_id":track_id})) { runtime.error = Some(error); }
                changed = true;
            }
            if requests.video_record.is_some() {
                runtime.error = Some("Video recording requires an Android camera capture backend; this build cannot record video.".into());
                changed = true;
            }
        }
        let after = player::snapshot(&runtime.player);
        if after.revision != before.revision {
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
        if runtime.foreground && player::snapshot(&runtime.player).playing {
            changed |= runtime.tick();
        }
        let mut preview_dirty = reset || changed;
        if let Some(media) = &runtime.media {
            let update = media.poll();
            if let Some(event) = update.visual {
                use shrimply_preview_render_core::worker::VideoEvent;
                preview_texture = match event {
                    VideoEvent::Frame { frame, .. } => Some(frame.display),
                    VideoEvent::Clear { .. } => None,
                    _ => unreachable!("preview visual event"),
                };
                preview_dirty = true;
            }
            if let Some(error) = update.errors.into_iter().next_back() {
                runtime.error = Some(error);
                changed = true;
            }
        }
        if preview_dirty {
            let preview = crate::preview::Frame::new(&runtime, preview_texture.clone());
            if last_preview.as_ref() != Some(&preview) {
                last_preview = Some(preview.clone());
                latest[1]
                    .lock()
                    .expect("Android preview slot poisoned")
                    .frame = Some(Frame {
                    picture: None,
                    preview: Some(preview),
                    density: 1.0,
                    input_at: None,
                    record_ns: 0,
                });
                let _ = render[1].try_send(RenderCommand::Wake);
            }
        }
        if at.is_some() && picture_dirty {
            input_at = input_at.or(at);
        }
    }
}

/// Native window lifecycle never waits for project imports or document commits.
fn run_render(
    receive: mpsc::Receiver<RenderCommand>,
    project_send: SyncSender<ProjectCommand>,
    benchmark: bool,
    input: Arc<Mutex<crate::input::Queue>>,
    latest: Arc<Mutex<RenderSlot>>,
    benchmark_submitted: Arc<AtomicUsize>,
    preview: bool,
) {
    let mut current: Option<Frame> = None;
    let mut renderer: Option<Renderer> = None;
    let mut window = None;
    let (mut width, mut height, mut density) = (0, 0, 1.0);
    let mut active = true;
    let mut dirty = false;
    let mut frames = Vec::new();
    let mut latency = Vec::new();
    let mut recordings = Vec::new();
    let mut intervals = Vec::new();
    let mut previous_frame: Option<Instant> = None;
    let initial_rss = if benchmark { rss_kib() } else { 0 };
    let (mut min_rss, mut max_rss) = (initial_rss, initial_rss);
    'render: loop {
        let mut command = if dirty {
            receive.try_recv().ok()
        } else {
            Some(receive.recv().expect("Android render sender disappeared"))
        };
        while let Some(next) = command {
            match next {
                RenderCommand::Surface(next, w, h, d, ack) => {
                    latest.lock().expect("Android frame slot poisoned").resize = None;
                    drop(renderer.take());
                    window = next;
                    (width, height, density) = (w, h, d);
                    if active && width > 0 && height > 0 {
                        renderer = window
                            .as_ref()
                            .map(|window| Renderer::new(window, width, height));
                    }
                    dirty = renderer.is_some();
                    ack.send(()).expect("surface acknowledgment");
                }
                RenderCommand::Active(next, ack) => {
                    active = next;
                    if !active {
                        drop(renderer.take());
                    } else if renderer.is_none() && width > 0 && height > 0 {
                        renderer = window
                            .as_ref()
                            .map(|window| Renderer::new(window, width, height));
                    }
                    dirty = renderer.is_some();
                    ack.send(()).expect("render lifecycle acknowledgment");
                }
                RenderCommand::Wake => {}
                RenderCommand::Stop => break 'render,
            }
            command = receive.try_recv().ok();
        }
        let (resize, frame) = {
            let mut slot = latest.lock().expect("Android frame slot poisoned");
            (slot.resize.take(), slot.frame.take())
        };
        if let Some((w, h)) = resize {
            (width, height) = (w, h);
            if width == 0 || height == 0 {
                drop(renderer.take());
            } else if let Some(renderer) = renderer.as_mut() {
                renderer.resize(width, height);
            } else if active {
                renderer = window
                    .as_ref()
                    .map(|window| Renderer::new(window, width, height));
            }
            dirty = renderer.is_some();
        }
        if !preview {
            let viewport = crate::input::Viewport {
                width,
                height,
                density,
                visible: active && window.is_some() && width > 0 && height > 0,
            };
            let mut pending = input.lock().expect("Android input queue poisoned");
            if viewport != pending.viewport {
                pending.viewport = viewport;
                if !viewport.visible {
                    pending.push(crate::input::Input::Scene(
                        shrimply_timeline_skia::scene::Event::Cancel,
                    ));
                }
                let _ = project_send.try_send(ProjectCommand::Wake);
            }
            drop(pending);
        }
        if let Some(next) = frame {
            current = Some(next);
            dirty = true;
        }
        if !dirty {
            continue;
        }
        let Some(renderer) = renderer.as_mut() else {
            dirty = false;
            continue;
        };
        let Some(frame) = current.as_ref() else {
            dirty = false;
            continue;
        };
        let start = Instant::now();
        let valid = if let Some(preview) = &frame.preview {
            crate::preview::draw(renderer, preview, density)
        } else {
            renderer.draw(|canvas, _, _| {
                canvas.clear(skia_safe::Color::BLACK);
                if let Some(picture) = &frame.picture {
                    canvas.save();
                    canvas.scale((frame.density, frame.density));
                    canvas.draw_picture(picture, None, None);
                    canvas.restore();
                }
                None
            })
        };
        if !valid {
            renderer.resize(width, height);
        }
        dirty = !valid;
        if benchmark && valid && frames.len() < BENCHMARK_FRAMES {
            frames.push(start.elapsed().as_nanos());
            recordings.push(frame.record_ns);
            benchmark_submitted.store(frames.len(), Ordering::Release);
            if let Some(previous) = previous_frame.replace(start) {
                intervals.push(start.duration_since(previous).as_nanos());
            }
            if let Some(at) = frame.input_at {
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
    drop(renderer);
    drop(window);
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
    _this: JObject<'caller>,
    application: JObject<'caller>,
    config: JString<'caller>,
    cache: JString<'caller>,
    fonts: JString<'caller>,
) {
    unowned
        .with_env(|env| -> jni::errors::Result<()> {
            static APPLICATION: std::sync::OnceLock<jni::refs::Global<JObject<'static>>> =
                std::sync::OnceLock::new();
            let application = env.new_global_ref(application)?;
            let application = APPLICATION.get_or_init(|| application);
            let vm = env.get_java_vm()?.get_raw().cast();
            crate::localization::initialize_vm(env.get_java_vm()?);
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
pub extern "system" fn Java_dev_shrimply_app_NativeTimeline_create<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _this: JObject<'caller>,
    stress: jboolean,
    benchmark: jboolean,
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
            let (project, project_receive) = mpsc::sync_channel(8);
            let (timeline_send, timeline_receive) = mpsc::sync_channel(8);
            let (preview_send, preview_receive) = mpsc::sync_channel(8);
            let render = [timeline_send, preview_send];
            let input = Arc::new(Mutex::new(crate::input::Queue::default()));
            let lifecycle = SharedLifecycle::default();
            let latest: [Arc<Mutex<RenderSlot>>; 2] =
                std::array::from_fn(|_| Arc::new(Mutex::new(RenderSlot::default())));
            let benchmark_submitted = Arc::new(AtomicUsize::new(0));
            let render_threads = [timeline_receive, preview_receive]
                .into_iter()
                .enumerate()
                .map(|(index, render_receive)| {
                    let input = input.clone();
                    let latest = latest[index].clone();
                    let project = project.clone();
                    let benchmark_submitted = benchmark_submitted.clone();
                    thread::Builder::new()
                        .name(
                            if index == 0 {
                                "shrimply-timeline"
                            } else {
                                "shrimply-preview"
                            }
                            .into(),
                        )
                        .spawn(move || {
                            run_render(
                                render_receive,
                                project,
                                benchmark && index == 0,
                                input,
                                latest,
                                benchmark_submitted,
                                index == 1,
                            )
                        })
                        .expect("start Vulkan thread")
                })
                .collect();
            let project_thread = {
                let latest = latest.clone();
                let lifecycle = lifecycle.clone();
                let wake = project.clone();
                let render = render.clone();
                let input = input.clone();
                thread::Builder::new()
                    .name("shrimply-project".into())
                    .spawn(move || {
                        run_project(
                            ProjectWorker {
                                receive: project_receive,
                                wake,
                                render,
                                latest,
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
                        )
                    })
                    .expect("start project thread")
            };
            Ok(Box::into_raw(Box::new(Session {
                input,
                lifecycle,
                project,
                render,
                latest,
                render_threads,
                project_thread: Some(project_thread),
            })) as jlong)
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
        let (send, receive) = mpsc::sync_channel(0);
        session(handle)
            .render
            .get(usize::try_from(role).expect("nonnegative surface role"))
            .expect("known surface role")
            .send(RenderCommand::Surface(
                window,
                width as u32,
                height as u32,
                density,
                send,
            ))
            .expect("send native surface");
        receive.recv().expect("wait for native surface release");
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
    let slot = current.latest.get(role).expect("known surface role");
    slot.lock().expect("Android frame slot poisoned").resize = Some((width as u32, height as u32));
    let _ = current.render[role].try_send(RenderCommand::Wake);
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
    for render in &session.render {
        let (send, receive) = mpsc::sync_channel(0);
        render
            .send(RenderCommand::Active(active, send))
            .expect("send lifecycle");
        receive.recv().expect("wait for lifecycle");
    }
    let _ = session.project.try_send(ProjectCommand::Wake);
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_app_NativeTimeline_input(
    env: *mut JNIEnv,
    _this: jobject,
    handle: jlong,
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
        let Some(input) = crate::input::decode(&values) else {
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
        for render in &session.render {
            render.send(RenderCommand::Stop).expect("stop rendering");
        }
        for thread in session.render_threads.drain(..) {
            thread.join().expect("Vulkan thread failed");
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
