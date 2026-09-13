#![cfg(windows)]

use ffmpeg::{format::Pixel, software::scaling};
use ffmpeg_next as ffmpeg;
use shrimply_math_core::Fraction;
use shrimply_project_document::project::{self, Time};
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::Instant;
use uuid::Uuid;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCapturePicker,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{E_ABORT, ERROR_CANCELLED, HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize};
use windows::Win32::UI::Shell::IInitializeWithWindow;
use windows::core::{IInspectable, Interface, Result as WindowsResult};
use windows_future::{AsyncStatus, IAsyncOperation};

const DEFAULT_FPS_NUMERATOR: u64 = 30;
const DEFAULT_FPS_DENOMINATOR: u64 = 1;
const BYTES_PER_PIXEL: usize = 4;
const WGC_FRAME_POOL_SIZE: i32 = 2;
const CAPTURE_WAIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);
const HUNDRED_NANOSECONDS_TO_NANOSECONDS: i64 = 100;
const NVENC_CONSTANT_QP: &str = "20";
const NVENC_B_FRAMES: usize = 0;
const NVENC_KEYFRAME_INTERVAL_SECONDS: u32 = 1;

pub struct ScreenRecording {
    controls: mpsc::Sender<CaptureControl>,
    events: mpsc::Receiver<ScreenRecordingEvent>,
    stopped: AtomicBool,
    selection: Mutex<Option<Selection>>,
    _winrt: WinrtApartment,
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

enum CaptureControl {
    Stop,
    Failed(String),
}

struct CapturedFrame {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    timestamp: Option<i64>,
}

struct Selection {
    operation: IAsyncOperation<GraphicsCaptureItem>,
    fps: Fraction,
    controls: mpsc::Sender<CaptureControl>,
    control_rx: mpsc::Receiver<CaptureControl>,
    events: mpsc::Sender<ScreenRecordingEvent>,
    final_path: PathBuf,
    temporary_path: PathBuf,
}

struct Readback {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    staging: Option<(D3D11_TEXTURE2D_DESC, ID3D11Texture2D)>,
}

struct VideoWriter {
    output: ffmpeg::format::context::Output,
    encoder: ffmpeg::codec::encoder::video::Encoder,
    scaler: scaling::Context,
    input_frame: ffmpeg::frame::Video,
    output_frame: ffmpeg::frame::Video,
    input_width: u32,
    input_height: u32,
    width: u32,
    height: u32,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    fps: Fraction,
    first_timestamp: Option<i64>,
    fallback_started_at: Instant,
    last_pts: Option<i64>,
    final_path: PathBuf,
    temporary_path: PathBuf,
}

impl ScreenRecording {
    pub fn start(fps: Fraction, native_window: usize) -> Result<Self, String> {
        if native_window == 0 {
            return Err("The Windows capture picker has no owner window".into());
        }
        let directory = project::project_directory().join("media/recordings");
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let name = Uuid::new_v4().to_string();
        let final_path = directory.join(format!("{name}.mp4"));
        let temporary_path = directory.join(format!("{name}.mp4.part"));
        let (control_tx, control_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let winrt = WinrtApartment::new()?;
        let picker = GraphicsCapturePicker::new().map_err(windows_error)?;
        let initialize: IInitializeWithWindow = picker.cast().map_err(windows_error)?;
        unsafe { initialize.Initialize(HWND(native_window as *mut _)) }.map_err(windows_error)?;
        let operation = picker.PickSingleItemAsync().map_err(windows_error)?;
        Ok(Self {
            controls: control_tx.clone(),
            events,
            stopped: AtomicBool::new(false),
            selection: Mutex::new(Some(Selection {
                operation,
                fps: valid_fps(fps),
                controls: control_tx,
                control_rx,
                events: event_tx,
                final_path,
                temporary_path,
            })),
            _winrt: winrt,
        })
    }

    pub fn stop(&self) {
        if !self.stopped.swap(true, Ordering::Relaxed) {
            if let Ok(selection) = self.selection.lock()
                && let Some(selection) = selection.as_ref()
            {
                let _ = selection.operation.Cancel();
            }
            let _ = self.controls.send(CaptureControl::Stop);
        }
    }

    pub fn try_event(&self) -> Result<ScreenRecordingEvent, mpsc::TryRecvError> {
        match self.events.try_recv() {
            Ok(event) => return Ok(event),
            Err(mpsc::TryRecvError::Empty) => {}
            Err(error) => return Err(error),
        }
        let mut selection = self
            .selection
            .lock()
            .map_err(|_| mpsc::TryRecvError::Disconnected)?;
        let Some(pending) = selection.as_ref() else {
            return Err(mpsc::TryRecvError::Empty);
        };
        match pending.operation.Status() {
            Ok(AsyncStatus::Started) => Err(mpsc::TryRecvError::Empty),
            Ok(AsyncStatus::Completed) => {
                let selection = selection.take().expect("capture selection exists");
                match selection.operation.GetResults() {
                    Ok(item) => {
                        thread::spawn(move || start_selected_capture(item, selection));
                        Err(mpsc::TryRecvError::Empty)
                    }
                    Err(error) if picker_cancelled(&error) => Ok(ScreenRecordingEvent::Cancelled),
                    Err(error) => Ok(ScreenRecordingEvent::Finished(Err(windows_error(error)))),
                }
            }
            Ok(AsyncStatus::Canceled) => {
                selection.take();
                Ok(ScreenRecordingEvent::Cancelled)
            }
            Ok(AsyncStatus::Error) => {
                let selection = selection.take().expect("capture selection exists");
                match selection.operation.GetResults() {
                    Err(error) if picker_cancelled(&error) => Ok(ScreenRecordingEvent::Cancelled),
                    Err(error) => Ok(ScreenRecordingEvent::Finished(Err(windows_error(error)))),
                    Ok(_) => Ok(ScreenRecordingEvent::Finished(Err(
                        "Windows capture picker reported an error without an error result".into(),
                    ))),
                }
            }
            Err(error) => {
                selection.take();
                Ok(ScreenRecordingEvent::Finished(Err(windows_error(error))))
            }
            Ok(status) => Ok(ScreenRecordingEvent::Finished(Err(format!(
                "Windows capture picker returned unknown status {}",
                status.0
            )))),
        }
    }
}

impl Drop for ScreenRecording {
    fn drop(&mut self) {
        self.stop();
    }
}

fn start_selected_capture(item: GraphicsCaptureItem, selection: Selection) {
    let Selection {
        operation,
        fps,
        controls,
        control_rx,
        events,
        final_path,
        temporary_path,
    } = selection;
    let cleanup_path = temporary_path.clone();
    let _winrt = match WinrtApartment::new() {
        Ok(apartment) => apartment,
        Err(error) => {
            let _ = events.send(ScreenRecordingEvent::Finished(Err(error)));
            return;
        }
    };
    drop(operation);
    if let Err(error) = record(
        item,
        fps,
        controls,
        control_rx,
        events.clone(),
        final_path,
        temporary_path,
    ) {
        remove_incomplete_recording(&cleanup_path);
        let _ = events.send(ScreenRecordingEvent::Finished(Err(error)));
    }
}

fn record(
    item: GraphicsCaptureItem,
    fps: Fraction,
    controls: mpsc::Sender<CaptureControl>,
    control_rx: mpsc::Receiver<CaptureControl>,
    events: mpsc::Sender<ScreenRecordingEvent>,
    final_path: PathBuf,
    temporary_path: PathBuf,
) -> Result<(), String> {
    ffmpeg::init().map_err(|error| error.to_string())?;
    let size = item.Size().map_err(windows_error)?;
    if size.Width <= 0 || size.Height <= 0 {
        return Err("Windows Graphics Capture selected an empty source".into());
    }
    let (device, context, capture_device) = create_device()?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &capture_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        WGC_FRAME_POOL_SIZE,
        size,
    )
    .map_err(windows_error)?;
    let session = pool.CreateCaptureSession(&item).map_err(windows_error)?;
    session
        .SetIsCursorCaptureEnabled(true)
        .map_err(windows_error)?;
    let readback = Arc::new(Mutex::new(Readback {
        device,
        context,
        staging: None,
    }));
    let (frame_tx, frame_rx) = mpsc::sync_channel(WGC_FRAME_POOL_SIZE as usize);
    let frame_controls = controls.clone();
    let frame_readback = readback.clone();
    let frame_device = capture_device.clone();
    let frame_pool_size = Arc::new(Mutex::new(size));
    let callback_pool_size = frame_pool_size.clone();
    let frame_token = pool
        .FrameArrived(
            &TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
                move |sender, _| {
                    let result = sender.ok().and_then(|pool| match pool.TryGetNextFrame() {
                        Ok(frame) => read_frame_and_resize(
                            pool,
                            &frame_device,
                            &callback_pool_size,
                            &frame_readback,
                            &frame,
                        ),
                        Err(error) if error.code().is_ok() => Ok(None),
                        Err(error) => Err(error),
                    });
                    match result {
                        Ok(Some(frame)) => {
                            let _ = frame_tx.try_send(frame);
                        }
                        Ok(None) => {}
                        Err(error) => {
                            let _ =
                                frame_controls.send(CaptureControl::Failed(windows_error(error)));
                        }
                    }
                    Ok(())
                },
            ),
        )
        .map_err(windows_error)?;
    let closed_controls = controls;
    let closed_token = item
        .Closed(
            &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                let _ = closed_controls.send(CaptureControl::Stop);
                Ok(())
            }),
        )
        .map_err(windows_error)?;
    session.StartCapture().map_err(windows_error)?;

    let mut writer = None;
    let recording_result = loop {
        match control_rx.try_recv() {
            Ok(CaptureControl::Stop) => {
                break finish_recording(writer.take(), &temporary_path);
            }
            Ok(CaptureControl::Failed(error)) => break Err(error),
            Err(mpsc::TryRecvError::Disconnected) => {
                break Err("Windows screen capture control channel closed".into());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        match frame_rx.recv_timeout(CAPTURE_WAIT_INTERVAL) {
            Ok(frame) => {
                if writer.is_none() {
                    let value = match VideoWriter::new(
                        frame.width,
                        frame.height,
                        fps,
                        final_path.clone(),
                        temporary_path.clone(),
                    ) {
                        Ok(value) => value,
                        Err(error) => break Err(error),
                    };
                    let _ = events.send(ScreenRecordingEvent::Ready {
                        width: value.width,
                        height: value.height,
                    });
                    writer = Some(value);
                }
                if let Err(error) = writer
                    .as_mut()
                    .expect("video writer exists")
                    .write_frame(frame)
                {
                    break Err(error);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err("Windows screen capture frame channel closed".into());
            }
        }
    };
    let _ = pool.RemoveFrameArrived(frame_token);
    let _ = item.RemoveClosed(closed_token);
    let _ = session.Close();
    let _ = pool.Close();
    if recording_result.is_err() {
        drop(writer.take());
        remove_incomplete_recording(&temporary_path);
    }
    let _ = events.send(ScreenRecordingEvent::Finished(recording_result));
    Ok(())
}

fn finish_recording(
    writer: Option<VideoWriter>,
    temporary_path: &PathBuf,
) -> Result<FinishedScreenRecording, String> {
    match writer {
        Some(writer) => writer.finish(),
        None => {
            remove_incomplete_recording(temporary_path);
            Err("Screen recording captured no video frames".into())
        }
    }
}

fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext, IDirect3DDevice), String> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(windows_error)?;
    let device = device.ok_or("D3D11 did not return a device")?;
    let context = context.ok_or("D3D11 did not return an immediate context")?;
    let dxgi: IDXGIDevice = device.cast().map_err(windows_error)?;
    let capture_device: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .and_then(|value| value.cast())
        .map_err(windows_error)?;
    Ok((device, context, capture_device))
}

fn read_frame(
    readback: &Mutex<Readback>,
    frame: &windows::Graphics::Capture::Direct3D11CaptureFrame,
) -> WindowsResult<CapturedFrame> {
    let result = read_frame_inner(readback, frame);
    let _ = frame.Close();
    result
}

fn read_frame_and_resize(
    pool: &Direct3D11CaptureFramePool,
    device: &IDirect3DDevice,
    pool_size: &Mutex<SizeInt32>,
    readback: &Mutex<Readback>,
    frame: &windows::Graphics::Capture::Direct3D11CaptureFrame,
) -> WindowsResult<Option<CapturedFrame>> {
    let content = frame.ContentSize()?;
    let current = *pool_size.lock().map_err(|_| E_ABORT)?;
    if content.Width == current.Width && content.Height == current.Height {
        return read_frame(readback, frame).map(Some);
    }
    let captured = if content.Width > 0
        && content.Height > 0
        && content.Width <= current.Width
        && content.Height <= current.Height
    {
        read_frame(readback, frame).map(Some)
    } else {
        let _ = frame.Close();
        Ok(None)
    };
    if content.Width > 0 && content.Height > 0 {
        pool.Recreate(
            device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            WGC_FRAME_POOL_SIZE,
            content,
        )?;
        *pool_size.lock().map_err(|_| E_ABORT)? = content;
    }
    captured
}

fn read_frame_inner(
    readback: &Mutex<Readback>,
    frame: &windows::Graphics::Capture::Direct3D11CaptureFrame,
) -> WindowsResult<CapturedFrame> {
    let content = frame.ContentSize()?;
    let width = u32::try_from(content.Width).map_err(|_| E_ABORT)?;
    let height = u32::try_from(content.Height).map_err(|_| E_ABORT)?;
    if width == 0 || height == 0 {
        return Err(E_ABORT.into());
    }
    let surface = frame.Surface()?;
    let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
    let source: ID3D11Texture2D = unsafe { access.GetInterface()? };
    let mut source_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { source.GetDesc(&mut source_desc) };
    if width > source_desc.Width || height > source_desc.Height {
        return Err(E_ABORT.into());
    }
    let mut readback = readback.lock().map_err(|_| E_ABORT)?;
    let needs_staging = readback.staging.as_ref().is_none_or(|(desc, _)| {
        desc.Width != source_desc.Width || desc.Height != source_desc.Height
    });
    if needs_staging {
        let mut desc = source_desc;
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        unsafe {
            readback
                .device
                .CreateTexture2D(&desc, None, Some(&mut staging))?
        };
        readback.staging = Some((
            desc,
            staging.ok_or_else(|| windows::core::Error::from(E_ABORT))?,
        ));
    }
    let staging = readback
        .staging
        .as_ref()
        .expect("staging texture exists")
        .1
        .clone();
    unsafe { readback.context.CopyResource(&staging, &source) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        readback
            .context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?
    };
    let row_bytes = width as usize * BYTES_PER_PIXEL;
    let row_pitch = mapped.RowPitch as usize;
    let mut bytes = vec![0; row_bytes * height as usize];
    let source = unsafe {
        std::slice::from_raw_parts(mapped.pData.cast::<u8>(), row_pitch * height as usize)
    };
    for row in 0..height as usize {
        bytes[row * row_bytes..(row + 1) * row_bytes]
            .copy_from_slice(&source[row * row_pitch..row * row_pitch + row_bytes]);
    }
    unsafe { readback.context.Unmap(&staging, 0) };
    let timestamp = frame.SystemRelativeTime().ok().map(|time| {
        time.Duration
            .saturating_mul(HUNDRED_NANOSECONDS_TO_NANOSECONDS)
    });
    Ok(CapturedFrame {
        bytes,
        width,
        height,
        timestamp,
    })
}

impl VideoWriter {
    fn new(
        input_width: u32,
        input_height: u32,
        fps: Fraction,
        final_path: PathBuf,
        temporary_path: PathBuf,
    ) -> Result<Self, String> {
        let width = input_width & !1;
        let height = input_height & !1;
        if width == 0 || height == 0 {
            return Err("Windows screen capture frame is too small".into());
        }
        let mut output =
            ffmpeg::format::output_as(&temporary_path, "mp4").map_err(|error| error.to_string())?;
        let global_header = output
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
        let (fps_numerator, fps_denominator) = fps_parts_i32(fps)?;
        let time_base = ffmpeg::Rational(fps_denominator, fps_numerator);
        let frame_rate = ffmpeg::Rational(fps_numerator, fps_denominator);
        let encoder = open_hevc_encoder(width, height, time_base, frame_rate, global_header)?;
        let stream_index = {
            let mut stream = output
                .add_stream_with(encoder.as_ref())
                .map_err(|error| error.to_string())?;
            stream.set_time_base(time_base);
            stream.set_rate(frame_rate);
            stream.set_avg_frame_rate(frame_rate);
            stream.index()
        };
        output.write_header().map_err(|error| error.to_string())?;
        let stream_time_base = output
            .stream(stream_index)
            .ok_or("MP4 video stream disappeared")?
            .time_base();
        let scaler = screen_scaler(input_width, input_height, width, height)?;
        Ok(Self {
            output,
            encoder,
            scaler,
            input_frame: ffmpeg::frame::Video::new(Pixel::BGRA, input_width, input_height),
            output_frame: ffmpeg::frame::Video::new(Pixel::YUV420P, width, height),
            input_width,
            input_height,
            width,
            height,
            stream_index,
            stream_time_base,
            fps,
            first_timestamp: None,
            fallback_started_at: Instant::now(),
            last_pts: None,
            final_path,
            temporary_path,
        })
    }

    fn write_frame(&mut self, frame: CapturedFrame) -> Result<(), String> {
        if frame.width != self.input_width || frame.height != self.input_height {
            self.scaler = screen_scaler(frame.width, frame.height, self.width, self.height)?;
            self.input_frame = ffmpeg::frame::Video::new(Pixel::BGRA, frame.width, frame.height);
            self.input_width = frame.width;
            self.input_height = frame.height;
        }
        let row_bytes = frame.width as usize * BYTES_PER_PIXEL;
        let input_stride = self.input_frame.stride(0);
        if frame.bytes.len() != row_bytes * frame.height as usize {
            return Err("Windows screen capture returned an incomplete frame".into());
        }
        let input = self.input_frame.data_mut(0);
        for row in 0..frame.height as usize {
            input[row * input_stride..row * input_stride + row_bytes]
                .copy_from_slice(&frame.bytes[row * row_bytes..(row + 1) * row_bytes]);
        }
        let elapsed_nanos = match frame.timestamp {
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
            .ok_or("Invalid recording frame rate")?;
        if self.last_pts.is_some_and(|last| pts <= last) {
            return Ok(());
        }
        self.scaler
            .run(&self.input_frame, &mut self.output_frame)
            .map_err(|error| error.to_string())?;
        self.output_frame.set_pts(Some(pts));
        self.encoder
            .send_frame(&self.output_frame)
            .map_err(|error| error.to_string())?;
        self.write_packets()?;
        self.last_pts = Some(pts);
        Ok(())
    }

    fn write_packets(&mut self) -> Result<(), String> {
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(self.stream_index);
                    packet.rescale_ts(self.encoder.time_base(), self.stream_time_base);
                    packet
                        .write_interleaved(&mut self.output)
                        .map_err(|error| error.to_string())?;
                }
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {
                    return Ok(());
                }
                Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(error) => return Err(error.to_string()),
            }
        }
    }

    fn finish(mut self) -> Result<FinishedScreenRecording, String> {
        let Some(last_pts) = self.last_pts else {
            remove_incomplete_recording(&self.temporary_path);
            return Err("Screen recording captured no video frames".into());
        };
        let elapsed = self
            .fallback_started_at
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let final_pts = shrimply_math_core::frame_index(Time::from_nanos(elapsed), self.fps)
            .ok_or("Invalid recording frame rate")?
            .max(last_pts);
        if final_pts > last_pts {
            self.output_frame.set_pts(Some(final_pts));
            self.encoder
                .send_frame(&self.output_frame)
                .map_err(|error| error.to_string())?;
            self.write_packets()?;
        }
        self.encoder.send_eof().map_err(|error| error.to_string())?;
        self.write_packets()?;
        self.output
            .write_trailer()
            .map_err(|error| error.to_string())?;
        drop(self.output);
        fs::rename(&self.temporary_path, &self.final_path).map_err(|error| error.to_string())?;
        let numerator = project::fraction_numerator(self.fps);
        let denominator = project::fraction_denominator(self.fps);
        Ok(FinishedScreenRecording {
            path: Some(self.final_path.clone()),
            duration: Time::from_fraction(
                final_pts.saturating_add(1).saturating_mul(denominator),
                numerator,
            ),
            width: self.width,
            height: self.height,
        })
    }
}

fn screen_scaler(
    input_width: u32,
    input_height: u32,
    width: u32,
    height: u32,
) -> Result<scaling::Context, String> {
    scaling::Context::get(
        Pixel::BGRA,
        input_width,
        input_height,
        Pixel::YUV420P,
        width,
        height,
        scaling::Flags::BILINEAR,
    )
    .map_err(|error| error.to_string())
}

fn open_hevc_encoder(
    width: u32,
    height: u32,
    time_base: ffmpeg::Rational,
    frame_rate: ffmpeg::Rational,
    global_header: bool,
) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
    let codec = ffmpeg::codec::encoder::find_by_name("hevc_nvenc")
        .ok_or("FFmpeg encoder hevc_nvenc was not found")?;
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

fn fps_parts_i32(fps: Fraction) -> Result<(i32, i32), String> {
    let numerator = project::fraction_numerator(fps);
    let denominator = project::fraction_denominator(fps);
    Ok((
        i32::try_from(numerator).map_err(|_| "Recording frame-rate numerator is too large")?,
        i32::try_from(denominator).map_err(|_| "Recording frame-rate denominator is too large")?,
    ))
}

fn valid_fps(fps: Fraction) -> Fraction {
    if project::fraction_numerator(fps) > 0 && project::fraction_denominator(fps) > 0 {
        fps
    } else {
        Fraction::new_raw(DEFAULT_FPS_NUMERATOR, DEFAULT_FPS_DENOMINATOR)
    }
}

fn remove_incomplete_recording(path: &PathBuf) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), %error, "Could not remove abandoned screen recording");
    }
}

fn windows_error(error: windows::core::Error) -> String {
    error.to_string()
}

fn picker_cancelled(error: &windows::core::Error) -> bool {
    error.code().is_ok()
        || error.code() == E_ABORT
        || error.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0)
}

struct WinrtApartment(std::marker::PhantomData<Rc<()>>);

impl WinrtApartment {
    fn new() -> Result<Self, String> {
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }
            .map(|()| Self(std::marker::PhantomData))
            .map_err(windows_error)
    }
}

impl Drop for WinrtApartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}
