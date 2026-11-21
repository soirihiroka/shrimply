use crate::{
    VideoRecordingEvent, remove_incomplete_recording,
    writer::{CaptureFormat, VideoWriter, recording_paths, valid_fps},
};
use hashbrown::HashMap;
use std::cell::RefCell;

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

use ffmpeg::format::Pixel;
use ffmpeg_next as ffmpeg;
use gio::prelude::{DBusProxyExt, UnixFDListExtManual};
use glib::variant::{Handle, ObjectPath, ToVariant};
use pipewire as pw;
use pw::spa;
use pw::spa::buffer::meta::{MetaHeader, MetaHeaderFlags};
use pw::spa::param::format::{MediaSubtype, MediaType};
use pw::spa::pod::Pod;
use shrimply_math_core::Fraction;

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
static PORTAL_TOKEN: AtomicU32 = AtomicU32::new(1);

pub struct ScreenRecording {
    cancel: async_channel::Sender<()>,
    commands: Arc<Mutex<Option<pw::channel::Sender<RecordingCommand>>>>,
    events: mpsc::Receiver<VideoRecordingEvent>,
    stopped: AtomicBool,
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
    events: mpsc::Sender<VideoRecordingEvent>,
    final_path: PathBuf,
    temporary_path: PathBuf,
    fps: Fraction,
}

impl ScreenRecording {
    pub fn start(fps: Fraction) -> Result<Self, String> {
        let (final_path, temporary_path) = recording_paths()?;
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

    pub fn try_event(&self) -> Result<VideoRecordingEvent, mpsc::TryRecvError> {
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
    events: mpsc::Sender<VideoRecordingEvent>,
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
            let _ = events.send(VideoRecordingEvent::Cancelled);
        }
        Err(error) => {
            let _ = events.send(VideoRecordingEvent::Finished(Err(error.message)));
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
    events: mpsc::Sender<VideoRecordingEvent>,
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
        let _ = events.send(VideoRecordingEvent::Finished(Err(error)));
    }
}

fn record_pipewire_inner(
    fd: OwnedFd,
    target: PipeWireTarget,
    command_rx: pw::channel::Receiver<RecordingCommand>,
    events: mpsc::Sender<VideoRecordingEvent>,
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
        return Err(error);
    }
    let writer = state
        .writer
        .take()
        .ok_or_else(|| "Screen capture ended before receiving a video format".to_string())?;
    let finished = writer.finish()?;
    let _ = state
        .events
        .send(VideoRecordingEvent::Finished(Ok(finished)));
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
    let ready = VideoRecordingEvent::Ready {
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
        alpha: Some(has_alpha),
    })
}

fn fps_parts(fps: Fraction) -> (u32, u32) {
    (
        shrimply_math_core::fraction_numerator(fps).clamp(1, i64::from(u32::MAX)) as u32,
        shrimply_math_core::fraction_denominator(fps).clamp(1, i64::from(u32::MAX)) as u32,
    )
}
