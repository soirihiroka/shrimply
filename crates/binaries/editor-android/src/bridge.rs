use crate::{TimelineState, math, vulkan::Renderer};
use jni::sys::{JNIEnv, jboolean, jclass, jfloat, jfloatArray, jint, jlong, jobject};
use ndk::native_window::NativeWindow;
use std::{
    sync::mpsc::{self, SyncSender},
    thread::{self, JoinHandle},
    time::Instant,
};

const ANDROID_LOG_INFO: i32 = 4;
const INPUT_STRIDE: usize = 6;
const MAX_BATCH: usize = INPUT_STRIDE * 32;
const BENCHMARK_FRAMES: usize = 1200;
const FRAME_BUDGET_NS: u128 = 1_000_000_000 / 60;

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

enum Command {
    Surface(Option<NativeWindow>, u32, u32, f32, SyncSender<()>),
    Resize(u32, u32, SyncSender<()>),
    Active(bool, SyncSender<()>),
    Input(Vec<f32>, Instant),
    Stop,
}
struct Session {
    send: SyncSender<Command>,
    thread: Option<JoinHandle<()>>,
}

/// Lifecycle calls acknowledge only after native window access has finished.
fn run(receive: mpsc::Receiver<Command>, stress: bool, benchmark: bool) {
    shrimply_cross_ui_theme::set_dark(true);
    let mut state = TimelineState::new(stress);
    let mut renderer: Option<Renderer> = None;
    let mut window = None;
    let (mut width, mut height, mut density) = (0, 0, 1.0);
    let mut active = true;
    let mut dirty = false;
    let mut frames = Vec::with_capacity(BENCHMARK_FRAMES);
    let mut latency = Vec::with_capacity(BENCHMARK_FRAMES);
    let mut intervals = Vec::with_capacity(BENCHMARK_FRAMES);
    let mut previous_frame: Option<Instant> = None;
    let initial_rss = rss_kib();
    let (mut min_rss, mut max_rss) = (initial_rss, initial_rss);
    let mut input_at = None;
    'render: loop {
        let mut command =
            if dirty || (benchmark && frames.len() < BENCHMARK_FRAMES && renderer.is_some()) {
                receive.try_recv().ok()
            } else {
                Some(receive.recv().expect("Android session sender disappeared"))
            };
        while let Some(next) = command {
            match next {
                Command::Surface(next, w, h, d, ack) => {
                    drop(renderer.take());
                    window = next;
                    (width, height, density) = (w, h, d);
                    if width > 0 && height > 0 {
                        math::clamp_viewport(
                            &mut state,
                            f64::from(width) / f64::from(density),
                            f64::from(height) / f64::from(density),
                        );
                    }
                    if active && width > 0 && height > 0 {
                        renderer = window
                            .as_ref()
                            .map(|window| Renderer::new(window, width, height));
                    }
                    dirty = renderer.is_some();
                    ack.send(()).expect("surface acknowledgment");
                }
                Command::Resize(w, h, ack) => {
                    (width, height) = (w, h);
                    if width > 0 && height > 0 {
                        math::clamp_viewport(
                            &mut state,
                            f64::from(width) / f64::from(density),
                            f64::from(height) / f64::from(density),
                        );
                    }
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
                    ack.send(()).expect("resize acknowledgment");
                }
                Command::Active(next, ack) => {
                    active = next;
                    if !active {
                        drop(renderer.take());
                    } else if renderer.is_none() && width > 0 && height > 0 {
                        renderer = window
                            .as_ref()
                            .map(|window| Renderer::new(window, width, height));
                    }
                    dirty = renderer.is_some();
                    ack.send(()).expect("lifecycle acknowledgment");
                }
                Command::Input(batch, at) => {
                    for values in batch.chunks_exact(INPUT_STRIDE) {
                        math::gesture(
                            &mut state,
                            values,
                            f64::from(width) / f64::from(density),
                            f64::from(height) / f64::from(density),
                        );
                    }
                    input_at.get_or_insert(at);
                    dirty = true;
                }
                Command::Stop => break 'render,
            }
            command = receive.try_recv().ok();
        }
        if !(dirty || benchmark && frames.len() < BENCHMARK_FRAMES) {
            continue;
        }
        let Some(renderer) = renderer.as_mut() else {
            dirty = false;
            continue;
        };
        if benchmark && frames.len() < BENCHMARK_FRAMES {
            let direction = if frames.len() / 120 % 2 == 0 {
                -1.0
            } else {
                1.0
            };
            math::gesture(
                &mut state,
                &[0.0, 400.0, 80.0, direction * 3.0, 0.0, 1.0],
                f64::from(width) / f64::from(density),
                f64::from(height) / f64::from(density),
            );
        }
        let start = Instant::now();
        let valid = renderer.draw(|canvas, w, h| state.draw(canvas, w, h, density));
        if !valid {
            renderer.resize(width, height);
        }
        dirty = !valid;
        if benchmark && frames.len() < BENCHMARK_FRAMES {
            frames.push(start.elapsed().as_nanos());
            if let Some(previous) = previous_frame.replace(start) {
                intervals.push(start.duration_since(previous).as_nanos());
            }
            if let Some(at) = input_at.take() {
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
                frames.sort_unstable();
                latency.sort_unstable();
                intervals.sort_unstable();
                log(format!(
                    "benchmark stress={stress} frames={} submit_p50_us={} submit_p95_us={} missed_60hz_intervals={} input_samples={} input_p50_us={} input_p95_us={} rss_initial_kib={} rss_final_kib={} rss_min_kib={} rss_max_kib={} (submission/acquire/present wall time; input through submission)",
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
pub extern "system" fn Java_dev_shrimply_tablet_NativeTimeline_create(
    _env: *mut JNIEnv,
    _class: jclass,
    stress: jboolean,
    benchmark: jboolean,
) -> jlong {
    static PANIC_LOG: std::sync::Once = std::sync::Once::new();
    PANIC_LOG.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            log(format!("native timeline panic: {info}"));
            std::process::abort();
        }))
    });
    let (send, receive) = mpsc::sync_channel(8);
    let thread = thread::Builder::new()
        .name("shrimply-timeline".into())
        .spawn(move || run(receive, stress, benchmark))
        .expect("start timeline thread");
    Box::into_raw(Box::new(Session {
        send,
        thread: Some(thread),
    })) as jlong
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_tablet_NativeTimeline_surface(
    env: *mut JNIEnv,
    _class: jclass,
    handle: jlong,
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
            .send
            .send(Command::Surface(
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
pub unsafe extern "system" fn Java_dev_shrimply_tablet_NativeTimeline_resize(
    _env: *mut JNIEnv,
    _class: jclass,
    handle: jlong,
    width: jint,
    height: jint,
) {
    assert!(width >= 0 && height >= 0);
    let (send, receive) = mpsc::sync_channel(0);
    unsafe {
        session(handle)
            .send
            .send(Command::Resize(width as u32, height as u32, send))
            .expect("send resize");
    }
    receive.recv().expect("wait for resize");
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_tablet_NativeTimeline_active(
    _env: *mut JNIEnv,
    _class: jclass,
    handle: jlong,
    active: jboolean,
) {
    let (send, receive) = mpsc::sync_channel(0);
    unsafe {
        session(handle)
            .send
            .send(Command::Active(active, send))
            .expect("send lifecycle");
    }
    receive.recv().expect("wait for lifecycle");
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_tablet_NativeTimeline_input(
    env: *mut JNIEnv,
    _class: jclass,
    handle: jlong,
    array: jfloatArray,
) {
    unsafe {
        let length = ((**env).v1_1.GetArrayLength)(env, array) as usize;
        assert!(
            length > 0 && length <= MAX_BATCH && length.is_multiple_of(INPUT_STRIDE),
            "invalid timeline input batch size"
        );
        let mut values = vec![0.0; length];
        ((**env).v1_1.GetFloatArrayRegion)(env, array, 0, length as jint, values.as_mut_ptr());
        assert!(!((**env).v1_2.ExceptionCheck)(env), "read input array");
        session(handle)
            .send
            .send(Command::Input(values, Instant::now()))
            .expect("send timeline input");
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_shrimply_tablet_NativeTimeline_destroy(
    _env: *mut JNIEnv,
    _class: jclass,
    handle: jlong,
) {
    unsafe {
        let mut session = Box::from_raw(handle as *mut Session);
        session
            .send
            .send(Command::Stop)
            .expect("stop timeline thread");
        session
            .thread
            .take()
            .expect("timeline thread")
            .join()
            .expect("timeline rendering thread failed");
    }
}
