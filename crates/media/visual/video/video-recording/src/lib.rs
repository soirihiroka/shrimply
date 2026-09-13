use hashbrown::HashMap;
use std::cell::RefCell;
use std::fs;
use std::future::{Future, poll_fn};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::pin::pin;
use std::rc::Rc;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc,
};
use std::task::Poll;
use std::thread;
use std::time::Instant;

use ffmpeg::{format::Pixel, software::scaling};
use ffmpeg_next as ffmpeg;
use gio::prelude::{DBusProxyExt, UnixFDListExtManual};
use glib::variant::{Handle, ObjectPath, ToVariant};
use pipewire as pw;
use pw::spa;
use pw::spa::buffer::meta::{MetaHeader, MetaHeaderFlags};
use pw::spa::param::format::{MediaSubtype, MediaType};
use pw::spa::pod::Pod;
use shrimply_math_core::Fraction;
use shrimply_project_document::project::{self, Time};
use uuid::Uuid;

const PORTAL_DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST_INTERFACE: &str = "org.freedesktop.portal.ScreenCast";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
const SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";
const MONITOR_SOURCE: u32 = 1;
const WINDOW_SOURCE: u32 = 2;
const RECORDING_SOURCES: u32 = MONITOR_SOURCE | WINDOW_SOURCE;
const CURSOR_HIDDEN: u32 = 1;
const CURSOR_EMBEDDED: u32 = 2;
const DEFAULT_FPS_NUMERATOR: u64 = 30;
const DEFAULT_FPS_DENOMINATOR: u64 = 1;
const BYTES_PER_PIXEL: usize = 4;
const NVENC_CONSTANT_QP: &str = "20";
const NVENC_B_FRAMES: usize = 0;
const NVENC_KEYFRAME_INTERVAL_SECONDS: u32 = 1;

static PORTAL_TOKEN: AtomicU32 = AtomicU32::new(1);

pub struct ScreenRecording {
    cancel: async_channel::Sender<()>,
    commands: Arc<Mutex<Option<pw::channel::Sender<RecordingCommand>>>>,
    events: mpsc::Receiver<ScreenRecordingEvent>,
    stopped: AtomicBool,
}

pub enum ScreenRecordingEvent {
    Ready { width: u32, height: u32 },
    Cancelled,
    Finished(Result<FinishedScreenRecording, String>),
}

pub struct FinishedScreenRecording {
    path: Option<PathBuf>,
    pub duration: Time,
    pub width: u32,
    pub height: u32,
}

impl FinishedScreenRecording {
    pub fn into_path(mut self) -> PathBuf {
        self.path.take().expect("recording file is owned")
    }
}

impl Drop for FinishedScreenRecording {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            remove_incomplete_recording(&path);
        }
    }
}

#[derive(Default)]
struct PortalState {
    connection: Option<gio::DBusConnection>,
    request_path: Option<String>,
    session_path: Option<String>,
}

enum RecordingCommand {
    Stop,
}

struct PortalFailure {
    message: String,
    cancelled: bool,
}

struct PipeWireTarget {
    node_id: u32,
    serial: Option<u64>,
}

struct CaptureState {
    writer: Option<VideoWriter>,
    error: Option<String>,
    events: mpsc::Sender<ScreenRecordingEvent>,
    final_path: PathBuf,
    temporary_path: PathBuf,
    fps: Fraction,
}

#[derive(Clone, Copy, PartialEq)]
struct CaptureFormat {
    pixel: Pixel,
    width: u32,
    height: u32,
    has_alpha: bool,
}

struct VideoWriter {
    output: ffmpeg::format::context::Output,
    rgb_encoder: ffmpeg::codec::encoder::video::Encoder,
    alpha_encoder: ffmpeg::codec::encoder::video::Encoder,
    rgb_scaler: scaling::Context,
    alpha_scaler: scaling::Context,
    input_frame: ffmpeg::frame::Video,
    alpha_input_frame: ffmpeg::frame::Video,
    rgb_output_frame: ffmpeg::frame::Video,
    alpha_output_frame: ffmpeg::frame::Video,
    input: CaptureFormat,
    width: u32,
    height: u32,
    rgb_stream_index: usize,
    alpha_stream_index: usize,
    rgb_stream_time_base: ffmpeg::Rational,
    alpha_stream_time_base: ffmpeg::Rational,
    fps: Fraction,
    first_timestamp: Option<i64>,
    fallback_started_at: Instant,
    last_pts: Option<i64>,
    final_path: PathBuf,
    temporary_path: PathBuf,
}

impl ScreenRecording {
    pub fn start(fps: Fraction) -> Result<Self, String> {
        let directory = project::project_directory().join("media/recordings");
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let name = Uuid::new_v4().to_string();
        let final_path = directory.join(format!("{name}.mp4"));
        let temporary_path = directory.join(format!("{name}.mp4.part"));
        let (cancel, cancelled) = async_channel::bounded(1);
        let commands = Arc::new(Mutex::new(None));
        let (event_tx, events) = mpsc::channel();

        let worker_commands = commands.clone();
        thread::spawn(move || {
            let context = glib::MainContext::new();
            context.block_on(run_portal(
                Rc::new(RefCell::new(PortalState::default())),
                cancelled,
                worker_commands,
                event_tx,
                final_path,
                temporary_path,
                valid_fps(fps),
            ));
        });

        Ok(Self {
            cancel,
            commands,
            events,
            stopped: AtomicBool::new(false),
        })
    }

    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::Relaxed) {
            return;
        }
        let _ = self.cancel.try_send(());
        if let Some(commands) = self.commands.lock().ok().and_then(|value| value.clone()) {
            let _ = commands.send(RecordingCommand::Stop);
        }
    }

    pub fn try_event(&self) -> Result<ScreenRecordingEvent, mpsc::TryRecvError> {
        self.events.try_recv()
    }
}

impl Drop for ScreenRecording {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn run_portal(
    state: Rc<RefCell<PortalState>>,
    cancelled: async_channel::Receiver<()>,
    commands: Arc<Mutex<Option<pw::channel::Sender<RecordingCommand>>>>,
    events: mpsc::Sender<ScreenRecordingEvent>,
    final_path: PathBuf,
    temporary_path: PathBuf,
    fps: Fraction,
) {
    let result = open_portal_stream(state.clone(), &cancelled).await;
    match result {
        Ok((fd, target)) => {
            let (command_tx, command_rx) = pw::channel::channel();
            if let Ok(mut slot) = commands.lock() {
                *slot = Some(command_tx.clone());
            }
            thread::spawn(move || {
                record_pipewire(
                    fd,
                    target,
                    command_rx,
                    events,
                    final_path,
                    temporary_path,
                    fps,
                )
            });
            let _ = cancelled.recv().await;
            let _ = command_tx.send(RecordingCommand::Stop);
        }
        Err(error) if error.cancelled => {
            let _ = events.send(ScreenRecordingEvent::Cancelled);
        }
        Err(error) => {
            let _ = events.send(ScreenRecordingEvent::Finished(Err(error.message)));
        }
    }
    let mut state = state.borrow_mut();
    if let Some(path) = state.request_path.take() {
        close_portal_object(state.connection.as_ref(), &path, REQUEST_INTERFACE);
    }
    if let Some(path) = state.session_path.take() {
        close_portal_object(state.connection.as_ref(), &path, SESSION_INTERFACE);
    }
}

async fn open_portal_stream(
    state: Rc<RefCell<PortalState>>,
    cancelled: &async_channel::Receiver<()>,
) -> Result<(OwnedFd, PipeWireTarget), PortalFailure> {
    let proxy = until_cancelled(
        gio::DBusProxy::for_bus_future(
            gio::BusType::Session,
            gio::DBusProxyFlags::NONE,
            None,
            PORTAL_DESTINATION,
            PORTAL_PATH,
            SCREENCAST_INTERFACE,
        ),
        cancelled,
    )
    .await
    .ok_or_else(portal_cancelled)?
    .map_err(portal_error)?;
    let connection = proxy.connection();
    state.borrow_mut().connection = Some(connection);

    let available_sources = proxy
        .cached_property("AvailableSourceTypes")
        .and_then(|value| value.get::<u32>())
        .unwrap_or_default();
    let recording_sources = available_sources & RECORDING_SOURCES;
    if recording_sources == 0 {
        return Err(PortalFailure {
            message: "The desktop portal does not support screen or application capture"
                .to_string(),
            cancelled: false,
        });
    }

    let create_token = token("request");
    let create_options = glib::VariantDict::new(None);
    create_options.insert_value("handle_token", &create_token.to_variant());
    create_options.insert_value("session_handle_token", &token("session").to_variant());
    let create_response = portal_request(
        &proxy,
        &state,
        cancelled,
        "CreateSession",
        &create_token,
        glib::Variant::tuple_from_iter([create_options.end()]),
    )
    .await?;
    let create_results = response_results(&create_response)?;
    let session_path = create_results
        .get("session_handle")
        .and_then(|value| value.get::<String>())
        .ok_or_else(|| PortalFailure {
            message: "The portal did not return a screen-cast session".to_string(),
            cancelled: false,
        })?;
    state.borrow_mut().session_path = Some(session_path.clone());
    let session = ObjectPath::try_from(session_path.as_str()).map_err(|error| PortalFailure {
        message: error.to_string(),
        cancelled: false,
    })?;

    let cursor_modes = proxy
        .cached_property("AvailableCursorModes")
        .and_then(|value| value.get::<u32>())
        .unwrap_or_default();
    let cursor_mode = if cursor_modes & CURSOR_EMBEDDED != 0 {
        CURSOR_EMBEDDED
    } else {
        CURSOR_HIDDEN
    };
    let select_token = token("request");
    let select_options = glib::VariantDict::new(None);
    select_options.insert_value("handle_token", &select_token.to_variant());
    select_options.insert_value("types", &recording_sources.to_variant());
    select_options.insert_value("multiple", &false.to_variant());
    select_options.insert_value("cursor_mode", &cursor_mode.to_variant());
    portal_request(
        &proxy,
        &state,
        cancelled,
        "SelectSources",
        &select_token,
        glib::Variant::tuple_from_iter([session.clone().to_variant(), select_options.end()]),
    )
    .await?;

    let start_token = token("request");
    let start_options = glib::VariantDict::new(None);
    start_options.insert_value("handle_token", &start_token.to_variant());
    let start_response = portal_request(
        &proxy,
        &state,
        cancelled,
        "Start",
        &start_token,
        glib::Variant::tuple_from_iter([
            session.clone().to_variant(),
            "".to_variant(),
            start_options.end(),
        ]),
    )
    .await?;
    let start_results = response_results(&start_response)?;
    let streams = start_results
        .get("streams")
        .and_then(streams_from_variant)
        .ok_or_else(|| PortalFailure {
            message: "The portal did not return a PipeWire stream".to_string(),
            cancelled: false,
        })?;
    let (node_id, properties) = streams.last().ok_or_else(|| PortalFailure {
        message: "The portal returned an empty PipeWire stream list".to_string(),
        cancelled: false,
    })?;
    if streams.len() != 1 {
        tracing::warn!(
            streams = streams.len(),
            "portal returned multiple streams; using the last"
        );
    }
    let target = PipeWireTarget {
        node_id: *node_id,
        serial: properties
            .get("pipewire-serial")
            .and_then(|value| value.get::<u64>()),
    };

    let (result, fd_list) = until_cancelled(
        proxy.call_with_unix_fd_list_future(
            "OpenPipeWireRemote",
            Some(&glib::Variant::tuple_from_iter([
                session.to_variant(),
                glib::VariantDict::new(None).end(),
            ])),
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::UnixFDList>,
        ),
        cancelled,
    )
    .await
    .ok_or_else(portal_cancelled)?
    .map_err(portal_error)?;
    let (handle,) = result.get::<(Handle,)>().ok_or_else(|| PortalFailure {
        message: "The portal returned an invalid PipeWire file descriptor".to_string(),
        cancelled: false,
    })?;
    let fd = fd_list
        .ok_or_else(|| PortalFailure {
            message: "The portal returned no PipeWire file descriptor list".to_string(),
            cancelled: false,
        })?
        .get(handle.0)
        .map_err(portal_error)?;
    Ok((fd, target))
}

async fn portal_request(
    proxy: &gio::DBusProxy,
    state: &Rc<RefCell<PortalState>>,
    cancelled: &async_channel::Receiver<()>,
    method: &str,
    request_token: &str,
    parameters: glib::Variant,
) -> Result<glib::Variant, PortalFailure> {
    let connection = proxy.connection();
    let sender = connection
        .unique_name()
        .ok_or_else(|| PortalFailure {
            message: "The D-Bus connection has no unique name".to_string(),
            cancelled: false,
        })?
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{request_token}");
    state.borrow_mut().request_path = Some(path.clone());
    let (tx, rx) = async_channel::bounded(1);
    let subscription = connection.subscribe_to_signal(
        Some(PORTAL_DESTINATION),
        Some(REQUEST_INTERFACE),
        Some("Response"),
        Some(&path),
        None,
        gio::DBusSignalFlags::NO_MATCH_RULE,
        move |signal| {
            let _ = tx.try_send(signal.parameters.clone());
        },
    );
    let response = async {
        until_cancelled(
            proxy.call_future(method, Some(&parameters), gio::DBusCallFlags::NONE, -1),
            cancelled,
        )
        .await
        .ok_or_else(portal_cancelled)?
        .map_err(portal_error)?;
        until_cancelled(rx.recv(), cancelled)
            .await
            .ok_or_else(portal_cancelled)?
            .map_err(|error| PortalFailure {
                message: error.to_string(),
                cancelled: false,
            })
    }
    .await;
    drop(subscription);
    let response = response?;
    if state.borrow().request_path.as_deref() == Some(path.as_str()) {
        state.borrow_mut().request_path = None;
    }
    let code = response
        .child_value(0)
        .get::<u32>()
        .ok_or_else(|| PortalFailure {
            message: "The portal returned an invalid response".to_string(),
            cancelled: false,
        })?;
    match code {
        0 => Ok(response),
        1 => Err(PortalFailure {
            message: "Screen or application selection was cancelled".to_string(),
            cancelled: true,
        }),
        _ => Err(PortalFailure {
            message: format!("The screen-cast portal rejected {method}"),
            cancelled: false,
        }),
    }
}

async fn until_cancelled<F: Future>(
    future: F,
    cancelled: &async_channel::Receiver<()>,
) -> Option<F::Output> {
    let mut future = pin!(future);
    let mut cancellation = pin!(cancelled.recv());
    poll_fn(|context| {
        if let Poll::Ready(value) = future.as_mut().poll(context) {
            return Poll::Ready(Some(value));
        }
        if cancellation.as_mut().poll(context).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

fn portal_cancelled() -> PortalFailure {
    PortalFailure {
        message: "Screen or application selection was cancelled".into(),
        cancelled: true,
    }
}

fn response_results(
    response: &glib::Variant,
) -> Result<HashMap<String, glib::Variant>, PortalFailure> {
    variant_map(&response.child_value(1)).ok_or_else(|| PortalFailure {
        message: "The portal returned invalid response data".to_string(),
        cancelled: false,
    })
}

fn variant_map(variant: &glib::Variant) -> Option<HashMap<String, glib::Variant>> {
    if !variant.is_type(glib::VariantTy::VARDICT) {
        return None;
    }
    variant
        .iter()
        .map(|entry| {
            Some((
                entry.child_value(0).get::<String>()?,
                entry.child_value(1).get::<glib::Variant>()?,
            ))
        })
        .collect()
}

fn streams_from_variant(
    variant: &glib::Variant,
) -> Option<Vec<(u32, HashMap<String, glib::Variant>)>> {
    if !variant.is_container() {
        return None;
    }
    variant
        .iter()
        .map(|stream| {
            Some((
                stream.child_value(0).get::<u32>()?,
                variant_map(&stream.child_value(1))?,
            ))
        })
        .collect()
}

fn token(prefix: &str) -> String {
    format!(
        "shrimply_{prefix}_{}",
        PORTAL_TOKEN.fetch_add(1, Ordering::Relaxed)
    )
}

fn portal_error(error: glib::Error) -> PortalFailure {
    PortalFailure {
        cancelled: error.matches(gio::IOErrorEnum::Cancelled),
        message: error.to_string(),
    }
}

fn close_portal_object(connection: Option<&gio::DBusConnection>, path: &str, interface: &str) {
    let Some(connection) = connection else {
        return;
    };
    connection.call(
        Some(PORTAL_DESTINATION),
        path,
        interface,
        "Close",
        None,
        None,
        gio::DBusCallFlags::NONE,
        -1,
        None::<&gio::Cancellable>,
        |_| {},
    );
}

fn record_pipewire(
    fd: OwnedFd,
    target: PipeWireTarget,
    command_rx: pw::channel::Receiver<RecordingCommand>,
    events: mpsc::Sender<ScreenRecordingEvent>,
    final_path: PathBuf,
    temporary_path: PathBuf,
    fps: Fraction,
) {
    let cleanup_path = temporary_path.clone();
    if let Err(error) = record_pipewire_inner(
        fd,
        target,
        command_rx,
        events.clone(),
        final_path,
        temporary_path,
        fps,
    ) {
        remove_incomplete_recording(&cleanup_path);
        let _ = events.send(ScreenRecordingEvent::Finished(Err(error)));
    }
}

fn record_pipewire_inner(
    fd: OwnedFd,
    target: PipeWireTarget,
    command_rx: pw::channel::Receiver<RecordingCommand>,
    events: mpsc::Sender<ScreenRecordingEvent>,
    final_path: PathBuf,
    temporary_path: PathBuf,
    fps: Fraction,
) -> Result<(), String> {
    pw::init();
    ffmpeg::init().map_err(|error| error.to_string())?;
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|error| error.to_string())?;
    let context =
        pw::context::ContextRc::new(&mainloop, None).map_err(|error| error.to_string())?;
    let core = context
        .connect_fd_rc(fd, None)
        .map_err(|error| error.to_string())?;
    let mut properties = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Video",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Screen",
    };
    let node_id = if let Some(serial) = target.serial {
        properties.insert(*pw::keys::TARGET_OBJECT, serial.to_string());
        None
    } else {
        Some(target.node_id)
    };
    let stream = pw::stream::StreamBox::new(&core, "Shrimply screen recording", properties)
        .map_err(|error| error.to_string())?;
    let state = Rc::new(RefCell::new(CaptureState {
        writer: None,
        error: None,
        events,
        final_path,
        temporary_path,
        fps,
    }));

    let command_mainloop = mainloop.clone();
    let _commands = command_rx.attach(mainloop.loop_(), move |command| match command {
        RecordingCommand::Stop => command_mainloop.quit(),
    });
    let error_mainloop = mainloop.clone();
    let format_mainloop = mainloop.clone();
    let process_mainloop = mainloop.clone();
    let _listener = stream
        .add_local_listener_with_user_data(state.clone())
        .state_changed(move |_, state, _, new| {
            if let pw::stream::StreamState::Error(error) = new {
                state.borrow_mut().error = Some(format!("PipeWire stream failed: {error}"));
                error_mainloop.quit();
            }
        })
        .param_changed(move |_, state, id, parameter| {
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Some(parameter) = parameter else {
                return;
            };
            let result = configure_format(&mut state.borrow_mut(), parameter);
            if let Err(error) = result {
                state.borrow_mut().error = Some(error);
                format_mainloop.quit();
            }
        })
        .process(move |stream, state| {
            let result = process_frame(stream, &mut state.borrow_mut());
            if let Err(error) = result {
                state.borrow_mut().error = Some(error);
                process_mainloop.quit();
            }
        })
        .register()
        .map_err(|error| error.to_string())?;

    let fraction = fps_parts(fps);
    let format = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::BGRA,
            spa::param::video::VideoFormat::BGRA,
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::RGBx
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction {
                num: fraction.0,
                denom: fraction.1
            },
            spa::utils::Fraction { num: 0, denom: 1 },
            spa::utils::Fraction {
                num: fraction.0,
                denom: fraction.1
            }
        )
    );
    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(format),
    )
    .map_err(|error| error.to_string())?
    .0
    .into_inner();
    let mut parameters = [Pod::from_bytes(&bytes).ok_or("Could not build PipeWire video format")?];
    stream
        .connect(
            spa::utils::Direction::Input,
            node_id,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut parameters,
        )
        .map_err(|error| error.to_string())?;
    mainloop.run();

    let mut state = state.borrow_mut();
    if let Some(error) = state.error.take() {
        if let Some(writer) = state.writer.take() {
            writer.abort();
        }
        return Err(error);
    }
    let writer = state
        .writer
        .take()
        .ok_or_else(|| "Screen capture ended before receiving a video format".to_string())?;
    let result = writer.finish();
    if result.is_err() {
        remove_incomplete_recording(&state.temporary_path);
    }
    let _ = state.events.send(ScreenRecordingEvent::Finished(result));
    Ok(())
}

fn configure_format(state: &mut CaptureState, parameter: &Pod) -> Result<(), String> {
    let (media_type, media_subtype) =
        spa::param::format_utils::parse_format(parameter).map_err(|error| error.to_string())?;
    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
        return Err("PipeWire negotiated a non-video stream".to_string());
    }
    let mut info = spa::param::video::VideoInfoRaw::new();
    info.parse(parameter).map_err(|error| error.to_string())?;
    let width = info.size().width;
    let height = info.size().height;
    if width < 2 || height < 2 {
        return Err(format!(
            "PipeWire negotiated an invalid video size {width}x{height}"
        ));
    }
    let input = capture_format(info.format(), width, height)?;
    if let Some(writer) = state.writer.as_mut() {
        writer.configure_input(input)?;
        return Ok(());
    }
    let writer = VideoWriter::new(
        input,
        state.fps,
        state.final_path.clone(),
        state.temporary_path.clone(),
    )?;
    let ready = ScreenRecordingEvent::Ready {
        width: writer.width,
        height: writer.height,
    };
    state.writer = Some(writer);
    let _ = state.events.send(ready);
    Ok(())
}

fn process_frame(stream: &pw::stream::Stream, state: &mut CaptureState) -> Result<(), String> {
    let Some(writer) = state.writer.as_mut() else {
        return Ok(());
    };
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return Ok(());
    };
    let header = buffer.find_meta::<MetaHeader>();
    if header.is_some_and(|header| header.flags().contains(MetaHeaderFlags::CORRUPTED)) {
        return Ok(());
    }
    let timestamp = header.map(MetaHeader::pts).filter(|value| *value >= 0);
    let data = buffer
        .datas_mut()
        .first_mut()
        .ok_or_else(|| "PipeWire returned a video buffer without data".to_string())?;
    if data
        .chunk()
        .flags()
        .contains(spa::buffer::ChunkFlags::CORRUPTED)
        || data.chunk().size() == 0
    {
        return Ok(());
    }
    let offset = data.chunk().offset() as usize;
    let size = data.chunk().size() as usize;
    let stride = data.chunk().stride();
    let data_type = data.type_();
    let bytes = data
        .data()
        .ok_or_else(|| format!("PipeWire returned an unmapped {data_type:?} video buffer"))?;
    let end = offset
        .checked_add(size)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "PipeWire returned video data outside its mapped buffer".to_string())?;
    writer.write_frame(&bytes[offset..end], stride, timestamp)
}

impl VideoWriter {
    fn new(
        input: CaptureFormat,
        fps: Fraction,
        final_path: PathBuf,
        temporary_path: PathBuf,
    ) -> Result<Self, String> {
        let width = input.width & !1;
        let height = input.height & !1;
        let mut output =
            ffmpeg::format::output_as(&temporary_path, "mp4").map_err(|error| error.to_string())?;
        let global_header = output
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
        let (fps_numerator, fps_denominator) = fps_parts_i32(fps)?;
        let time_base = ffmpeg::Rational(fps_denominator, fps_numerator);
        let frame_rate = ffmpeg::Rational(fps_numerator, fps_denominator);
        let rgb_encoder = open_hevc_encoder(width, height, time_base, frame_rate, global_header)?;
        let alpha_encoder = open_hevc_encoder(width, height, time_base, frame_rate, global_header)?;
        let rgb_stream_index = {
            let mut stream = output
                .add_stream_with(rgb_encoder.as_ref())
                .map_err(|error| error.to_string())?;
            stream.set_time_base(time_base);
            stream.set_rate(frame_rate);
            stream.set_avg_frame_rate(frame_rate);
            let mut metadata = ffmpeg::Dictionary::new();
            metadata.set("title", "RGB");
            stream.set_metadata(metadata);
            stream.index()
        };
        let alpha_stream_index = {
            let mut stream = output
                .add_stream_with(alpha_encoder.as_ref())
                .map_err(|error| error.to_string())?;
            stream.set_time_base(time_base);
            stream.set_rate(frame_rate);
            stream.set_avg_frame_rate(frame_rate);
            let mut metadata = ffmpeg::Dictionary::new();
            metadata.set("title", "Alpha");
            stream.set_metadata(metadata);
            stream.index()
        };
        output.write_header().map_err(|error| error.to_string())?;
        let rgb_stream_time_base = output
            .stream(rgb_stream_index)
            .ok_or_else(|| "MP4 RGB stream disappeared".to_string())?
            .time_base();
        let alpha_stream_time_base = output
            .stream(alpha_stream_index)
            .ok_or_else(|| "MP4 alpha stream disappeared".to_string())?
            .time_base();
        let rgb_scaler = scaling::Context::get(
            input.pixel,
            input.width,
            input.height,
            Pixel::YUV420P,
            width,
            height,
            scaling::Flags::BILINEAR,
        )
        .map_err(|error| error.to_string())?;
        let alpha_scaler = scaling::Context::get(
            Pixel::GRAY8,
            input.width,
            input.height,
            Pixel::YUV420P,
            width,
            height,
            scaling::Flags::BILINEAR,
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            output,
            rgb_encoder,
            alpha_encoder,
            rgb_scaler,
            alpha_scaler,
            input_frame: ffmpeg::frame::Video::new(input.pixel, input.width, input.height),
            alpha_input_frame: ffmpeg::frame::Video::new(Pixel::GRAY8, input.width, input.height),
            rgb_output_frame: ffmpeg::frame::Video::new(Pixel::YUV420P, width, height),
            alpha_output_frame: ffmpeg::frame::Video::new(Pixel::YUV420P, width, height),
            input,
            width,
            height,
            rgb_stream_index,
            alpha_stream_index,
            rgb_stream_time_base,
            alpha_stream_time_base,
            fps,
            first_timestamp: None,
            fallback_started_at: Instant::now(),
            last_pts: None,
            final_path,
            temporary_path,
        })
    }

    fn configure_input(&mut self, input: CaptureFormat) -> Result<(), String> {
        if input == self.input {
            return Ok(());
        }
        self.rgb_scaler = scaling::Context::get(
            input.pixel,
            input.width,
            input.height,
            Pixel::YUV420P,
            self.width,
            self.height,
            scaling::Flags::BILINEAR,
        )
        .map_err(|error| error.to_string())?;
        self.alpha_scaler = scaling::Context::get(
            Pixel::GRAY8,
            input.width,
            input.height,
            Pixel::YUV420P,
            self.width,
            self.height,
            scaling::Flags::BILINEAR,
        )
        .map_err(|error| error.to_string())?;
        self.input_frame = ffmpeg::frame::Video::new(input.pixel, input.width, input.height);
        self.alpha_input_frame = ffmpeg::frame::Video::new(Pixel::GRAY8, input.width, input.height);
        self.input = input;
        Ok(())
    }

    fn write_frame(
        &mut self,
        bytes: &[u8],
        stride: i32,
        timestamp: Option<i64>,
    ) -> Result<(), String> {
        let row_bytes = self.input.width as usize * BYTES_PER_PIXEL;
        let bottom_up = stride < 0;
        let stride = stride.unsigned_abs() as usize;
        let required = stride
            .saturating_mul(self.input.height.saturating_sub(1) as usize)
            .saturating_add(row_bytes);
        if stride < row_bytes || bytes.len() < required {
            return Err(format!(
                "PipeWire video buffer is too small: bytes={} stride={stride} size={}x{}",
                bytes.len(),
                self.input.width,
                self.input.height
            ));
        }
        let rgb_stride = self.input_frame.stride(0);
        let alpha_stride = self.alpha_input_frame.stride(0);
        let rgb = self.input_frame.data_mut(0);
        let alpha = self.alpha_input_frame.data_mut(0);
        for row in 0..self.input.height as usize {
            let source_row = if bottom_up {
                self.input.height as usize - row - 1
            } else {
                row
            };
            let source_start = source_row * stride;
            let rgb_start = row * rgb_stride;
            rgb[rgb_start..rgb_start + row_bytes]
                .copy_from_slice(&bytes[source_start..source_start + row_bytes]);
            let alpha_start = row * alpha_stride;
            let alpha_row = &mut alpha[alpha_start..alpha_start + self.input.width as usize];
            if self.input.has_alpha {
                for (column, value) in alpha_row.iter_mut().enumerate() {
                    *value = bytes[source_start + column * BYTES_PER_PIXEL + 3];
                }
            } else {
                alpha_row.fill(u8::MAX);
            }
        }
        let elapsed_nanos = match timestamp {
            Some(timestamp) => {
                let first = *self.first_timestamp.get_or_insert(timestamp);
                timestamp.saturating_sub(first) as u64
            }
            None => self
                .fallback_started_at
                .elapsed()
                .as_nanos()
                .min(u128::from(u64::MAX)) as u64,
        };
        let pts = shrimply_math_core::frame_index(Time::from_nanos(elapsed_nanos), self.fps)
            .ok_or_else(|| "Invalid recording frame rate".to_string())?;
        if self.last_pts.is_some_and(|last| pts <= last) {
            return Ok(());
        }
        self.rgb_scaler
            .run(&self.input_frame, &mut self.rgb_output_frame)
            .map_err(|error| error.to_string())?;
        self.alpha_scaler
            .run(&self.alpha_input_frame, &mut self.alpha_output_frame)
            .map_err(|error| error.to_string())?;
        self.rgb_output_frame.set_pts(Some(pts));
        self.alpha_output_frame.set_pts(Some(pts));
        self.rgb_encoder
            .send_frame(&self.rgb_output_frame)
            .map_err(|error| error.to_string())?;
        self.alpha_encoder
            .send_frame(&self.alpha_output_frame)
            .map_err(|error| error.to_string())?;
        self.write_packets()?;
        self.last_pts = Some(pts);
        Ok(())
    }

    fn write_packets(&mut self) -> Result<(), String> {
        write_encoder_packets(
            &mut self.rgb_encoder,
            self.rgb_stream_index,
            self.rgb_stream_time_base,
            &mut self.output,
        )?;
        write_encoder_packets(
            &mut self.alpha_encoder,
            self.alpha_stream_index,
            self.alpha_stream_time_base,
            &mut self.output,
        )
    }

    fn finish(mut self) -> Result<FinishedScreenRecording, String> {
        let Some(last_pts) = self.last_pts else {
            self.abort();
            return Err("Screen recording captured no video frames".to_string());
        };
        self.rgb_encoder
            .send_eof()
            .map_err(|error| error.to_string())?;
        self.alpha_encoder
            .send_eof()
            .map_err(|error| error.to_string())?;
        self.write_packets()?;
        self.output
            .write_trailer()
            .map_err(|error| error.to_string())?;
        fs::rename(&self.temporary_path, &self.final_path).map_err(|error| error.to_string())?;
        let numerator = project::fraction_numerator(self.fps);
        let denominator = project::fraction_denominator(self.fps);
        let duration = Time::from_fraction(
            last_pts.saturating_add(1).saturating_mul(denominator),
            numerator,
        );
        Ok(FinishedScreenRecording {
            path: Some(self.final_path.clone()),
            duration,
            width: self.width,
            height: self.height,
        })
    }

    fn abort(self) {
        drop(self.output);
        remove_incomplete_recording(&self.temporary_path);
    }
}

fn open_hevc_encoder(
    width: u32,
    height: u32,
    time_base: ffmpeg::Rational,
    frame_rate: ffmpeg::Rational,
    global_header: bool,
) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
    let codec = ffmpeg::codec::encoder::find_by_name("hevc_nvenc")
        .ok_or_else(|| "FFmpeg encoder hevc_nvenc was not found".to_string())?;
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| error.to_string())?;
    encoder.set_width(width);
    encoder.set_height(height);
    encoder.set_format(Pixel::YUV420P);
    encoder.set_time_base(time_base);
    encoder.set_frame_rate(Some(frame_rate));
    encoder.set_max_b_frames(NVENC_B_FRAMES);
    encoder.set_gop(
        ((u128::from(NVENC_KEYFRAME_INTERVAL_SECONDS) * frame_rate.0 as u128)
            / frame_rate.1 as u128)
            .max(1)
            .min(u128::from(u32::MAX)) as u32,
    );
    if global_header {
        unsafe {
            (*encoder.as_mut_ptr()).flags |= ffmpeg::sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    let mut options = ffmpeg::Dictionary::new();
    options.set("preset", "p3");
    options.set("tune", "ll");
    options.set("profile", "main");
    options.set("rc", "constqp");
    options.set("qp", NVENC_CONSTANT_QP);
    options.set("bf", &NVENC_B_FRAMES.to_string());
    options.set("spatial-aq", "1");
    options.set("temporal-aq", "0");
    options.set("zerolatency", "1");
    options.set("delay", "0");
    encoder
        .open_as_with(codec, options)
        .map_err(|error| format!("Could not open hevc_nvenc: {error}"))
}

fn write_encoder_packets(
    encoder: &mut ffmpeg::codec::encoder::video::Encoder,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    output: &mut ffmpeg::format::context::Output,
) -> Result<(), String> {
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                packet.set_stream(stream_index);
                packet.rescale_ts(encoder.time_base(), stream_time_base);
                packet
                    .write_interleaved(output)
                    .map_err(|error| error.to_string())?;
            }
            Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => return Ok(()),
            Err(ffmpeg::Error::Eof) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn capture_format(
    format: spa::param::video::VideoFormat,
    width: u32,
    height: u32,
) -> Result<CaptureFormat, String> {
    let (pixel, has_alpha) = match format {
        spa::param::video::VideoFormat::BGRx => (Pixel::BGRZ, false),
        spa::param::video::VideoFormat::RGBx => (Pixel::RGBZ, false),
        spa::param::video::VideoFormat::BGRA => (Pixel::BGRA, true),
        spa::param::video::VideoFormat::RGBA => (Pixel::RGBA, true),
        other => return Err(format!("Unsupported PipeWire video format {other:?}")),
    };
    Ok(CaptureFormat {
        pixel,
        width,
        height,
        has_alpha,
    })
}

fn valid_fps(fps: Fraction) -> Fraction {
    if project::fraction_numerator(fps) > 0 && project::fraction_denominator(fps) > 0 {
        fps
    } else {
        Fraction::new_raw(DEFAULT_FPS_NUMERATOR, DEFAULT_FPS_DENOMINATOR)
    }
}

fn fps_parts(fps: Fraction) -> (u32, u32) {
    (
        project::fraction_numerator(fps).clamp(1, i64::from(u32::MAX)) as u32,
        project::fraction_denominator(fps).clamp(1, i64::from(u32::MAX)) as u32,
    )
}

fn fps_parts_i32(fps: Fraction) -> Result<(i32, i32), String> {
    let numerator = project::fraction_numerator(fps);
    let denominator = project::fraction_denominator(fps);
    if numerator <= 0
        || denominator <= 0
        || numerator > i64::from(i32::MAX)
        || denominator > i64::from(i32::MAX)
    {
        return Err("Recording frame rate is outside FFmpeg's supported range".to_string());
    }
    Ok((numerator as i32, denominator as i32))
}

fn remove_incomplete_recording(path: &std::path::Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "could not remove incomplete recording")
        }
    }
}
