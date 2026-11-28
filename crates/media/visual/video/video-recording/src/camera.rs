use super::{VideoRecordingEvent, remove_incomplete_recording, writer};
use ffmpeg::{ffi, format, frame, software};
use ffmpeg_next as ffmpeg;
use shrimply_math_core::Fraction;
use std::ffi::{CStr, CString};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

fn input_format() -> Result<*const ffi::AVInputFormat, String> {
    ffmpeg::init().map_err(|error| error.to_string())?;
    let name = if cfg!(windows) { c"dshow" } else { c"v4l2" };
    let format = unsafe { ffi::av_find_input_format(name.as_ptr()) };
    if format.is_null() {
        return Err("Camera capture is unavailable".into());
    }
    Ok(format)
}

pub fn devices() -> Vec<(String, String)> {
    enumerate().unwrap_or_else(|error| {
        tracing::warn!(%error, "Could not list cameras");
        Vec::new()
    })
}

#[cfg(target_os = "linux")]
fn enumerate() -> Result<Vec<(String, String)>, String> {
    let mut list = std::ptr::null_mut();
    let status = unsafe {
        ffi::avdevice_list_input_sources(
            input_format()?,
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut list,
        )
    };
    let rows = if status > 0 && !list.is_null() {
        unsafe {
            std::slice::from_raw_parts((*list).devices, status as usize)
                .iter()
                .filter_map(|&device| {
                    let device = device.as_ref()?;
                    let name = CStr::from_ptr(device.device_name).to_string_lossy();
                    let label = if device.device_description.is_null() {
                        name.clone()
                    } else {
                        CStr::from_ptr(device.device_description).to_string_lossy()
                    };
                    Some((name.into_owned(), label.into_owned()))
                })
                .collect()
        }
    } else {
        Vec::new()
    };
    unsafe { ffi::avdevice_free_list_devices(&mut list) };
    if status < 0 {
        return Err(ffmpeg::Error::from(status).to_string());
    }
    Ok(rows)
}

#[cfg(windows)]
fn enumerate() -> Result<Vec<(String, String)>, String> {
    use windows::Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        Media::{
            DirectShow::ICreateDevEnum,
            MediaFoundation::{CLSID_SystemDeviceEnum, CLSID_VideoInputDeviceCategory},
        },
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize, CreateBindCtx, StructuredStorage::IPropertyBag,
            },
            Variant::VARIANT,
        },
    };
    use windows::core::{BSTR, w};

    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if initialized != RPC_E_CHANGED_MODE {
            initialized.ok().map_err(|error| error.to_string())?;
        }
        let result: windows::core::Result<_> = (|| {
            let devices: ICreateDevEnum =
                CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX_INPROC_SERVER)?;
            let mut cameras = None;
            devices.CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut cameras, 0)?;
            let Some(cameras) = cameras else {
                return Ok(Vec::new());
            };
            let context = CreateBindCtx(0)?;
            let mut rows = Vec::new();
            loop {
                let mut next = [None];
                cameras.Next(&mut next, None).ok()?;
                let [Some(camera)] = next else {
                    break;
                };
                let display = camera.GetDisplayName(&context, None)?;
                let name = display.to_string();
                CoTaskMemFree(Some(display.0.cast()));
                let Ok(name) = name else {
                    continue;
                };
                let name = name.replace(':', "_");
                let label = (|| {
                    let properties: IPropertyBag = camera.BindToStorage(&context, None).ok()?;
                    let mut value = VARIANT::default();
                    properties.Read(w!("FriendlyName"), &mut value, None).ok()?;
                    Some(BSTR::try_from(&value).ok()?.to_string())
                })()
                .unwrap_or_else(|| name.clone());
                rows.push((name, label));
            }
            Ok(rows)
        })();
        if initialized.is_ok() {
            CoUninitialize();
        }
        result.map_err(|error| error.to_string())
    }
}

pub struct CameraRecording {
    stopped: Arc<AtomicBool>,
    events: mpsc::Receiver<VideoRecordingEvent>,
}

impl CameraRecording {
    pub fn start(source: &str, fps: Fraction) -> Result<Self, String> {
        let source = if cfg!(windows) {
            format!("video={source}")
        } else {
            source.into()
        };
        let source = CString::new(source).map_err(|error| error.to_string())?;
        let (send, events) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stopped);
        std::thread::Builder::new()
            .name("camera-recording".into())
            .spawn(move || {
                let result = record(&source, fps, &worker_stop, &send);
                let _ = send.send(match result {
                    Ok(Some(recording)) => VideoRecordingEvent::Finished(Ok(recording)),
                    Ok(None) => VideoRecordingEvent::Cancelled,
                    Err(error) => VideoRecordingEvent::Finished(Err(error)),
                });
            })
            .map_err(|error| error.to_string())?;
        Ok(Self { stopped, events })
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    pub fn try_event(&self) -> Result<VideoRecordingEvent, mpsc::TryRecvError> {
        self.events.try_recv()
    }
}

impl Drop for CameraRecording {
    fn drop(&mut self) {
        self.stop();
    }
}

fn record(
    source: &CStr,
    fps: Fraction,
    stopped: &AtomicBool,
    events: &mpsc::Sender<VideoRecordingEvent>,
) -> Result<Option<super::FinishedVideoRecording>, String> {
    if stopped.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let input_format = input_format()?;
    let mut input = unsafe {
        let mut context = ffi::avformat_alloc_context();
        if context.is_null() {
            return Err("Could not allocate the camera input".into());
        }
        (*context).flags |= ffi::AVFMT_FLAG_NONBLOCK;
        let status = ffi::avformat_open_input(
            &mut context,
            source.as_ptr(),
            input_format,
            std::ptr::null_mut(),
        );
        if status < 0 {
            return Err(format!(
                "Could not open camera: {}",
                ffmpeg::Error::from(status)
            ));
        }
        format::context::Input::wrap(context)
    };
    let video = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or("The camera has no video input")?;
    let index = video.index();
    let mut decoder = ffmpeg::codec::Context::from_parameters(video.parameters())
        .and_then(|context| context.decoder().video())
        .map_err(|error| error.to_string())?;
    let (path, temporary) = writer::recording_paths()?;
    let result = (|| {
        let mut recording: Option<writer::VideoWriter> = None;
        let mut scaler: Option<software::scaling::Context> = None;
        let mut packet = ffmpeg::Packet::empty();
        let mut captured = frame::Video::empty();
        let mut converted = frame::Video::empty();
        while !stopped.load(Ordering::Relaxed) {
            match packet.read(&mut input) {
                Ok(()) => {}
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                Err(error) => return Err(error.to_string()),
            }
            if packet.stream() != index {
                continue;
            }
            decoder
                .send_packet(&packet)
                .map_err(|error| error.to_string())?;
            while !stopped.load(Ordering::Relaxed) {
                match decoder.receive_frame(&mut captured) {
                    Ok(()) => {}
                    Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => break,
                    Err(error) => return Err(error.to_string()),
                }
                let scaler = match &mut scaler {
                    Some(scaler) => scaler,
                    entry @ None => {
                        let width = captured.width();
                        let height = captured.height();
                        let pixel = format::Pixel::BGRA;
                        let color = writer::CaptureFormat {
                            pixel,
                            width,
                            height,
                            alpha: None,
                        };
                        let writer = writer::VideoWriter::new(
                            color,
                            writer::valid_fps(fps),
                            path.clone(),
                            temporary.clone(),
                        )?;
                        let ready = VideoRecordingEvent::Ready {
                            width: writer.width,
                            height: writer.height,
                        };
                        recording = Some(writer);
                        let _ = events.send(ready);
                        entry.insert(
                            software::scaling::Context::get(
                                captured.format(),
                                width,
                                height,
                                pixel,
                                width,
                                height,
                                software::scaling::Flags::BILINEAR,
                            )
                            .map_err(|error| error.to_string())?,
                        )
                    }
                };
                scaler
                    .run(&captured, &mut converted)
                    .map_err(|error| error.to_string())?;
                let stride =
                    i32::try_from(converted.stride(0)).map_err(|_| "Camera frame is too wide")?;
                recording.as_mut().expect("camera writer").write_frame(
                    converted.data(0),
                    stride,
                    None,
                )?;
            }
        }
        recording.map(writer::VideoWriter::finish).transpose()
    })();
    if result.is_err() {
        remove_incomplete_recording(&temporary);
    }
    result
}
