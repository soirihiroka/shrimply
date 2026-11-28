use super::{Event, RecordingDelegate};
use block2::RcBlock;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, DefinedClass, Message, define_class, msg_send};
use objc2_av_foundation::{
    AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceDiscoverySession, AVCaptureDeviceInput,
    AVCaptureDevicePosition, AVCaptureDeviceTypeBuiltInWideAngleCamera,
    AVCaptureDeviceTypeContinuityCamera, AVCaptureDeviceTypeExternal, AVCaptureFileOutput,
    AVCaptureFileOutputRecordingDelegate, AVCaptureMovieFileOutput, AVCaptureSession,
    AVMediaTypeVideo,
};
use objc2_core_media::CMVideoFormatDescriptionGetDimensions;
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSString, NSURL};
use std::sync::{atomic::Ordering, mpsc};

pub(in crate::macos::canvas) fn devices() -> Vec<(String, String)> {
    unsafe {
        let kinds = NSArray::from_slice(&[
            AVCaptureDeviceTypeBuiltInWideAngleCamera,
            AVCaptureDeviceTypeExternal,
            AVCaptureDeviceTypeContinuityCamera,
        ]);
        AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
            &kinds,
            AVMediaTypeVideo,
            AVCaptureDevicePosition::Unspecified,
        )
        .devices()
        .iter()
        .map(|device| {
            (
                device.uniqueID().to_string(),
                device.localizedName().to_string(),
            )
        })
        .collect()
    }
}

enum Command {
    Stop,
    Permission(bool),
    Finished,
}

struct DelegateIvars {
    recording: Retained<RecordingDelegate>,
    commands: mpsc::Sender<Command>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[ivars = DelegateIvars]
    struct CameraDelegate;

    unsafe impl NSObjectProtocol for CameraDelegate {}

    unsafe impl AVCaptureFileOutputRecordingDelegate for CameraDelegate {
        #[unsafe(method(captureOutput:didStartRecordingToOutputFileAtURL:fromConnections:))]
        unsafe fn did_start(
            &self,
            _output: &AVCaptureFileOutput,
            _url: &NSURL,
            _connections: &NSArray<AVCaptureConnection>,
        ) {
            self.ivars().recording.started();
        }

        #[unsafe(method(captureOutput:didFinishRecordingToOutputFileAtURL:fromConnections:error:))]
        unsafe fn did_finish(
            &self,
            output: &AVCaptureFileOutput,
            _url: &NSURL,
            _connections: &NSArray<AVCaptureConnection>,
            error: Option<&NSError>,
        ) {
            if let Some(error) = error {
                self.ivars()
                    .recording
                    .fail(error.localizedDescription().to_string());
            } else {
                self.ivars()
                    .recording
                    .finish_duration(unsafe { output.recordedDuration() });
            }
            let _ = self.ivars().commands.send(Command::Finished);
        }
    }
);

pub(in crate::macos::canvas) struct CameraRecording {
    stop: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
}

impl CameraRecording {
    pub(super) fn start(source: String) -> Result<Self, String> {
        let (recording, events) = RecordingDelegate::new("mov")?;
        let (stop, commands) = mpsc::channel();
        let control = stop.clone();
        std::thread::Builder::new()
            .name("camera-recording".into())
            .spawn(move || {
                autoreleasepool(|_| {
                    if let Err(error) = record(&source, &recording, control, commands) {
                        recording.fail(error);
                    }
                });
            })
            .map_err(|error| error.to_string())?;
        Ok(Self { stop, events })
    }

    pub(super) fn stop(&self) {
        let _ = self.stop.send(Command::Stop);
    }

    pub(super) fn try_event(&self) -> Result<Event, mpsc::TryRecvError> {
        self.events.try_recv()
    }
}

impl Drop for CameraRecording {
    fn drop(&mut self) {
        self.stop();
    }
}

fn record(
    source: &str,
    recording: &RecordingDelegate,
    control: mpsc::Sender<Command>,
    commands: mpsc::Receiver<Command>,
) -> Result<(), String> {
    let permission = control.clone();
    let callback = RcBlock::new(move |allowed: Bool| {
        let _ = permission.send(Command::Permission(allowed.as_bool()));
    });
    unsafe {
        let video = AVMediaTypeVideo.ok_or("Camera capture is unavailable")?;
        AVCaptureDevice::requestAccessForMediaType_completionHandler(video, &callback);
    }
    match commands.recv() {
        Ok(Command::Permission(true)) => {}
        Ok(Command::Permission(false)) => return Err("Camera access was denied".into()),
        _ => {
            recording.cancel();
            return Ok(());
        }
    }
    unsafe {
        let device = AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(source))
            .ok_or("The selected camera is unavailable")?;
        let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
            .map_err(|error| error.localizedDescription().to_string())?;
        let session = AVCaptureSession::new();
        let output = AVCaptureMovieFileOutput::new();
        if !session.canAddInput(&input) || !session.canAddOutput(&output) {
            return Err("The selected camera cannot record video".into());
        }
        session.addInput(&input);
        session.addOutput(&output);
        let url = NSURL::from_file_path(&recording.ivars().path)
            .ok_or("Could not create the camera recording file URL")?;
        let delegate = CameraDelegate::alloc().set_ivars(DelegateIvars {
            recording: recording.retain(),
            commands: control,
        });
        let delegate: Retained<CameraDelegate> = msg_send![super(delegate), init];
        session.startRunning();
        if !session.isRunning() {
            return Err("Could not start the camera".into());
        }
        if matches!(commands.try_recv(), Ok(Command::Stop)) {
            session.stopRunning();
            recording.cancel();
            return Ok(());
        }
        let dimensions =
            CMVideoFormatDescriptionGetDimensions(&device.activeFormat().formatDescription());
        recording
            .ivars()
            .width
            .store(dimensions.width.max(0) as u32, Ordering::Relaxed);
        recording
            .ivars()
            .height
            .store(dimensions.height.max(0) as u32, Ordering::Relaxed);
        output.startRecordingToOutputFileURL_recordingDelegate(
            &url,
            ProtocolObject::from_ref(&*delegate),
        );
        let mut stopping = false;
        while let Ok(command) = commands.recv() {
            match command {
                Command::Stop if !stopping => {
                    output.stopRecording();
                    stopping = true;
                }
                Command::Finished => break,
                _ => {}
            }
        }
        session.stopRunning();
    }
    Ok(())
}
