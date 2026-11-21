use crate::{
    VideoRecordingEvent, remove_incomplete_recording,
    writer::{BYTES_PER_PIXEL, CaptureFormat, VideoWriter, recording_paths, valid_fps},
};

use ffmpeg::format::Pixel;
use ffmpeg_next as ffmpeg;
use shrimply_math_core::Fraction;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;

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

const WGC_FRAME_POOL_SIZE: i32 = 2;
const CAPTURE_WAIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);
const HUNDRED_NANOSECONDS_TO_NANOSECONDS: i64 = 100;
pub struct ScreenRecording {
    controls: mpsc::Sender<CaptureControl>,
    events: mpsc::Receiver<VideoRecordingEvent>,
    stopped: AtomicBool,
    selection: Mutex<Option<Selection>>,
    _winrt: WinrtApartment,
}

enum CaptureControl {
    Stop,
    Failed(String),
}

struct CapturedFrame {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    stride: i32,
    timestamp: Option<i64>,
}

struct Selection {
    operation: IAsyncOperation<GraphicsCaptureItem>,
    fps: Fraction,
    controls: mpsc::Sender<CaptureControl>,
    control_rx: mpsc::Receiver<CaptureControl>,
    events: mpsc::Sender<VideoRecordingEvent>,
    final_path: PathBuf,
    temporary_path: PathBuf,
}

struct Readback {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    staging: Option<(D3D11_TEXTURE2D_DESC, ID3D11Texture2D)>,
}

impl ScreenRecording {
    pub fn start(fps: Fraction, native_window: usize) -> Result<Self, String> {
        if native_window == 0 {
            return Err("The Windows capture picker has no owner window".into());
        }
        let (final_path, temporary_path) = recording_paths()?;
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

    pub fn try_event(&self) -> Result<VideoRecordingEvent, mpsc::TryRecvError> {
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
                    Err(error) if picker_cancelled(&error) => Ok(VideoRecordingEvent::Cancelled),
                    Err(error) => Ok(VideoRecordingEvent::Finished(Err(windows_error(error)))),
                }
            }
            Ok(AsyncStatus::Canceled) => {
                selection.take();
                Ok(VideoRecordingEvent::Cancelled)
            }
            Ok(AsyncStatus::Error) => {
                let selection = selection.take().expect("capture selection exists");
                match selection.operation.GetResults() {
                    Err(error) if picker_cancelled(&error) => Ok(VideoRecordingEvent::Cancelled),
                    Err(error) => Ok(VideoRecordingEvent::Finished(Err(windows_error(error)))),
                    Ok(_) => Ok(VideoRecordingEvent::Finished(Err(
                        "Windows capture picker reported an error without an error result".into(),
                    ))),
                }
            }
            Err(error) => {
                selection.take();
                Ok(VideoRecordingEvent::Finished(Err(windows_error(error))))
            }
            Ok(status) => Ok(VideoRecordingEvent::Finished(Err(format!(
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
            let _ = events.send(VideoRecordingEvent::Finished(Err(error)));
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
        let _ = events.send(VideoRecordingEvent::Finished(Err(error)));
    }
}

fn record(
    item: GraphicsCaptureItem,
    fps: Fraction,
    controls: mpsc::Sender<CaptureControl>,
    control_rx: mpsc::Receiver<CaptureControl>,
    events: mpsc::Sender<VideoRecordingEvent>,
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
    let frame_pool_size = Arc::new(Mutex::new(size));
    let callback_pool_size = frame_pool_size.clone();
    let frame_token = pool
        .FrameArrived(
            &TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
                move |sender, _| {
                    let result = sender.ok().and_then(|pool| match pool.TryGetNextFrame() {
                        Ok(frame) => read_frame_and_resize(
                            pool,
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

    let mut writer: Option<VideoWriter> = None;
    let recording_result = loop {
        match control_rx.try_recv() {
            Ok(CaptureControl::Stop) => {
                break writer
                    .take()
                    .ok_or_else(|| "Screen recording captured no video frames".to_string())
                    .and_then(VideoWriter::finish);
            }
            Ok(CaptureControl::Failed(error)) => break Err(error),
            Err(mpsc::TryRecvError::Disconnected) => {
                break Err("Windows screen capture control channel closed".into());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        match frame_rx.recv_timeout(CAPTURE_WAIT_INTERVAL) {
            Ok(frame) => {
                let input = CaptureFormat {
                    width: frame.width,
                    height: frame.height,
                    pixel: Pixel::BGRA,
                    alpha: None,
                };
                if writer.is_none() {
                    let value = match VideoWriter::new(
                        input,
                        fps,
                        final_path.clone(),
                        temporary_path.clone(),
                    ) {
                        Ok(value) => value,
                        Err(error) => break Err(error),
                    };
                    let _ = events.send(VideoRecordingEvent::Ready {
                        width: value.width,
                        height: value.height,
                    });
                    writer = Some(value);
                }
                let writer = writer.as_mut().expect("video writer exists");
                if let Err(error) = writer
                    .configure_input(input)
                    .and_then(|()| writer.write_frame(&frame.bytes, frame.stride, frame.timestamp))
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
    let _ = events.send(VideoRecordingEvent::Finished(recording_result));
    Ok(())
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
    let capture_device = create_capture_device(&device).map_err(windows_error)?;
    Ok((device, context, capture_device))
}

fn create_capture_device(device: &ID3D11Device) -> WindowsResult<IDirect3DDevice> {
    let dxgi: IDXGIDevice = device.cast()?;
    unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }.and_then(|value| value.cast())
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
        let device = {
            let readback = readback.lock().map_err(|_| E_ABORT)?;
            create_capture_device(&readback.device)?
        };
        pool.Recreate(
            &device,
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
        stride: i32::try_from(row_bytes).map_err(|_| E_ABORT)?,
        timestamp,
    })
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
