use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_core_media::CMTime;
use objc2_foundation::{MainThreadMarker, NSError, NSObject, NSObjectProtocol, NSURL};
use objc2_screen_capture_kit::{
    SCContentFilter, SCContentSharingPicker, SCContentSharingPickerConfiguration,
    SCContentSharingPickerMode, SCContentSharingPickerObserver, SCRecordingOutput,
    SCRecordingOutputConfiguration, SCRecordingOutputDelegate, SCShareableContent, SCStream,
    SCStreamConfiguration,
};
use shrimply_timeline_skia::recording::{FinishedVideoRecording, VideoRecordingEvent as Event};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;

use super::{CanvasView, Content};

#[path = "camera_recording.rs"]
pub(super) mod camera;

const VIDEO_DIMENSION_ALIGNMENT: u32 = 2;

struct RecordingDelegateIvars {
    events: mpsc::Sender<Event>,
    path: PathBuf,
    width: AtomicU32,
    height: AtomicU32,
    terminal: AtomicBool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[ivars = RecordingDelegateIvars]
    struct RecordingDelegate;

    unsafe impl NSObjectProtocol for RecordingDelegate {}

    unsafe impl SCRecordingOutputDelegate for RecordingDelegate {
        #[unsafe(method(recordingOutputDidStartRecording:))]
        unsafe fn recording_started(&self, _output: &SCRecordingOutput) {
            self.started();
        }

        #[unsafe(method(recordingOutput:didFailWithError:))]
        unsafe fn recording_failed(&self, _output: &SCRecordingOutput, error: &NSError) {
            self.fail(error.localizedDescription().to_string());
        }

        #[unsafe(method(recordingOutputDidFinishRecording:))]
        unsafe fn recording_finished(&self, output: &SCRecordingOutput) {
            self.finish_duration(unsafe { output.recordedDuration() });
        }
    }
);

impl RecordingDelegate {
    fn new(extension: &str) -> Result<(Retained<Self>, mpsc::Receiver<Event>), String> {
        let directory = shrimply_project_document::project::recording_directory()?;
        let (events, receiver) = mpsc::channel();
        let recording = Self::alloc().set_ivars(RecordingDelegateIvars {
            events,
            path: directory.join(format!("{}.{extension}", uuid::Uuid::new_v4())),
            width: AtomicU32::new(0),
            height: AtomicU32::new(0),
            terminal: AtomicBool::new(false),
        });
        Ok((unsafe { msg_send![super(recording), init] }, receiver))
    }

    fn started(&self) {
        let _ = self.ivars().events.send(Event::Ready {
            width: self.ivars().width.load(Ordering::Relaxed),
            height: self.ivars().height.load(Ordering::Relaxed),
        });
    }

    fn finish_duration(&self, duration: CMTime) {
        if duration.value <= 0 || duration.timescale <= 0 {
            self.fail("The recording contains no video".into());
            return;
        }
        self.finish(FinishedVideoRecording::new(
            self.ivars().path.clone(),
            shrimply_project_document::project::Time::from_fraction(
                duration.value,
                i64::from(duration.timescale),
            ),
            self.ivars().width.load(Ordering::Relaxed),
            self.ivars().height.load(Ordering::Relaxed),
            None,
        ));
    }

    fn fail(&self, error: String) {
        if self.ivars().terminal.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = std::fs::remove_file(&self.ivars().path);
        let _ = self.ivars().events.send(Event::Finished(Err(error)));
    }

    fn finish(&self, finished: FinishedVideoRecording) {
        if self.ivars().terminal.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.ivars().events.send(Event::Finished(Ok(finished)));
    }

    fn cancel(&self) {
        if self.ivars().terminal.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = std::fs::remove_file(&self.ivars().path);
        let _ = self.ivars().events.send(Event::Cancelled);
    }
}

struct CaptureObjects {
    stream: Retained<SCStream>,
    _output: Retained<SCRecordingOutput>,
}

struct PickerDelegateIvars {
    fps: shrimply_math_core::Fraction,
    capture: RefCell<Option<CaptureObjects>>,
    recording: Retained<RecordingDelegate>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PickerDelegateIvars]
    struct PickerDelegate;

    unsafe impl NSObjectProtocol for PickerDelegate {}

    unsafe impl SCContentSharingPickerObserver for PickerDelegate {
        #[unsafe(method(contentSharingPicker:didCancelForStream:))]
        unsafe fn picker_cancelled(
            &self,
            picker: &SCContentSharingPicker,
            _stream: Option<&SCStream>,
        ) {
            self.detach_picker(picker);
            self.ivars().recording.cancel();
        }

        #[unsafe(method(contentSharingPicker:didUpdateWithFilter:forStream:))]
        unsafe fn picker_selected(
            &self,
            picker: &SCContentSharingPicker,
            filter: &SCContentFilter,
            _stream: Option<&SCStream>,
        ) {
            self.detach_picker(picker);
            if let Err(error) = self.start_capture(filter) {
                self.ivars().recording.fail(error);
            }
        }

        #[unsafe(method(contentSharingPickerStartDidFailWithError:))]
        unsafe fn picker_failed(&self, error: &NSError) {
            self.ivars()
                .recording
                .fail(error.localizedDescription().to_string());
        }
    }
);

impl PickerDelegate {
    fn detach_picker(&self, picker: &SCContentSharingPicker) {
        unsafe {
            picker.removeObserver(ProtocolObject::from_ref(self));
            picker.setActive(false);
        }
    }

    fn start_capture(&self, filter: &SCContentFilter) -> Result<(), String> {
        let info = unsafe { SCShareableContent::infoForFilter(filter) };
        let rect = unsafe { info.contentRect() };
        let scale = f64::from(unsafe { info.pointPixelScale() });
        let width = aligned_dimension(rect.size.width * scale)?;
        let height = aligned_dimension(rect.size.height * scale)?;
        self.ivars()
            .recording
            .ivars()
            .width
            .store(width, Ordering::Relaxed);
        self.ivars()
            .recording
            .ivars()
            .height
            .store(height, Ordering::Relaxed);

        let fps_numerator = i32::try_from(shrimply_math_core::fraction_numerator(self.ivars().fps))
            .map_err(|_| "Project frame-rate numerator is too large for ScreenCaptureKit")?;
        let fps_denominator = shrimply_math_core::fraction_denominator(self.ivars().fps);
        if fps_numerator <= 0 || fps_denominator <= 0 {
            return Err("Project frame rate must be positive".into());
        }
        let configuration = unsafe { SCStreamConfiguration::new() };
        unsafe {
            configuration.setWidth(width as usize);
            configuration.setHeight(height as usize);
            configuration.setMinimumFrameInterval(CMTime::new(fps_denominator, fps_numerator));
            configuration.setShowsCursor(true);
        }
        let output_configuration = unsafe { SCRecordingOutputConfiguration::new() };
        let url = NSURL::from_file_path(&self.ivars().recording.ivars().path)
            .ok_or("Could not create the screen-recording file URL")?;
        unsafe { output_configuration.setOutputURL(&url) };
        let output = unsafe {
            SCRecordingOutput::initWithConfiguration_delegate(
                SCRecordingOutput::alloc(),
                &output_configuration,
                ProtocolObject::from_ref(&*self.ivars().recording),
            )
        };
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                filter,
                &configuration,
                None,
            )
        };
        unsafe { stream.addRecordingOutput_error(&output) }
            .map_err(|error| error.localizedDescription().to_string())?;
        self.ivars().capture.replace(Some(CaptureObjects {
            stream: stream.clone(),
            _output: output,
        }));
        let recording = self.ivars().recording.clone();
        let completion = RcBlock::new(move |error: *mut NSError| {
            if let Some(error) = unsafe { error.as_ref() } {
                recording.fail(error.localizedDescription().to_string());
            }
        });
        unsafe { stream.startCaptureWithCompletionHandler(Some(&completion)) };
        Ok(())
    }
}

fn aligned_dimension(value: f64) -> Result<u32, String> {
    if !value.is_finite() || value < f64::from(VIDEO_DIMENSION_ALIGNMENT) {
        return Err("The selected screen or application has an invalid size".into());
    }
    let value = value.round();
    if value > f64::from(u32::MAX) {
        return Err("The selected screen or application is too large".into());
    }
    let value = value as u32;
    Ok(value - value % VIDEO_DIMENSION_ALIGNMENT)
}

pub struct ScreenRecording {
    picker: Retained<SCContentSharingPicker>,
    delegate: Retained<PickerDelegate>,
    events: mpsc::Receiver<Event>,
    stopping: Cell<bool>,
}

impl ScreenRecording {
    pub fn start(fps: shrimply_math_core::Fraction, mtm: MainThreadMarker) -> Result<Self, String> {
        let (recording, events) = RecordingDelegate::new("mp4")?;
        let delegate = PickerDelegate::alloc(mtm).set_ivars(PickerDelegateIvars {
            fps,
            capture: RefCell::new(None),
            recording,
        });
        let delegate: Retained<PickerDelegate> = unsafe { msg_send![super(delegate), init] };
        let picker = unsafe { SCContentSharingPicker::sharedPicker() };
        let configuration = unsafe { SCContentSharingPickerConfiguration::new() };
        unsafe {
            configuration.setAllowedPickerModes(
                SCContentSharingPickerMode::SingleWindow
                    | SCContentSharingPickerMode::SingleApplication
                    | SCContentSharingPickerMode::SingleDisplay,
            );
            configuration.setAllowsChangingSelectedContent(false);
            picker.setDefaultConfiguration(&configuration);
            picker.addObserver(ProtocolObject::from_ref(&*delegate));
            picker.setActive(true);
            picker.present();
        }
        Ok(Self {
            picker,
            delegate,
            events,
            stopping: Cell::new(false),
        })
    }

    pub fn stop(&self) {
        if self.stopping.replace(true) {
            return;
        }
        if let Some(capture) = self.delegate.ivars().capture.borrow().as_ref() {
            let recording = self.delegate.ivars().recording.clone();
            let completion = RcBlock::new(move |error: *mut NSError| {
                if let Some(error) = unsafe { error.as_ref() } {
                    recording.fail(error.localizedDescription().to_string());
                }
            });
            unsafe {
                capture
                    .stream
                    .stopCaptureWithCompletionHandler(Some(&completion));
            }
        } else {
            self.delegate.detach_picker(&self.picker);
            self.delegate.ivars().recording.cancel();
        }
    }

    pub fn try_event(&self) -> Result<Event, mpsc::TryRecvError> {
        self.events.try_recv()
    }
}

impl Drop for ScreenRecording {
    fn drop(&mut self) {
        self.stop();
        unsafe {
            self.picker
                .removeObserver(ProtocolObject::from_ref(&*self.delegate));
        }
    }
}

pub(super) enum VideoRecording {
    Screen(ScreenRecording),
    Camera(camera::CameraRecording),
}

impl VideoRecording {
    fn stop(&self) {
        match self {
            Self::Screen(recording) => recording.stop(),
            Self::Camera(recording) => recording.stop(),
        }
    }

    fn try_event(&self) -> Result<Event, mpsc::TryRecvError> {
        match self {
            Self::Screen(recording) => recording.try_event(),
            Self::Camera(recording) => recording.try_event(),
        }
    }
}

impl CanvasView {
    pub(super) fn update_video_recording(&self) -> Result<(), String> {
        let mut failure = None;
        loop {
            let event = match self.ivars().video_recording.borrow().as_ref() {
                Some(recording) => match recording.try_event() {
                    Ok(event) => Some(event),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => Some(Event::Finished(Err(
                        "ScreenCaptureKit stopped without a final event".into(),
                    ))),
                },
                None => None,
            };
            let Some(event) = event else {
                break;
            };
            let terminal = matches!(event, Event::Cancelled | Event::Finished(_));
            let result = {
                let mut content = self.ivars().content.borrow_mut();
                match &mut *content {
                    Content::Timeline(scene) => scene.handle_video_recording_event(event),
                    Content::Preview(_) | Content::Meter(_) => Ok(()),
                }
            };
            if terminal {
                self.ivars().video_recording.borrow_mut().take();
            }
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
            if terminal {
                break;
            }
        }

        loop {
            let command = {
                let mut content = self.ivars().content.borrow_mut();
                match &mut *content {
                    Content::Timeline(scene) => scene.take_video_recording_command(),
                    Content::Preview(_) | Content::Meter(_) => None,
                }
            };
            let Some(command) = command else {
                break;
            };
            match command {
                shrimply_timeline_skia::recording::VideoRecordingCommand::Start { fps } => {
                    if self.ivars().video_recording.borrow().is_some() {
                        failure.get_or_insert_with(|| "A video recording is already active".into());
                        continue;
                    }
                    let source = match &*self.ivars().content.borrow() {
                        Content::Timeline(scene) => scene.video_source().map(str::to_owned),
                        _ => None,
                    };
                    let started = match source {
                        Some(source) => {
                            camera::CameraRecording::start(source).map(VideoRecording::Camera)
                        }
                        None => ScreenRecording::start(fps, self.mtm()).map(VideoRecording::Screen),
                    };
                    match started {
                        Ok(recording) => {
                            self.ivars().video_recording.replace(Some(recording));
                        }
                        Err(error) => {
                            let result = {
                                let mut content = self.ivars().content.borrow_mut();
                                match &mut *content {
                                    Content::Timeline(scene) => scene
                                        .handle_video_recording_event(Event::Finished(Err(error))),
                                    Content::Preview(_) | Content::Meter(_) => Ok(()),
                                }
                            };
                            if let Err(error) = result {
                                failure.get_or_insert(error);
                            }
                        }
                    }
                }
                shrimply_timeline_skia::recording::VideoRecordingCommand::Stop => {
                    if let Some(recording) = self.ivars().video_recording.borrow().as_ref() {
                        recording.stop();
                    }
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
