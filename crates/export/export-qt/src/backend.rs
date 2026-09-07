use core::pin::Pin;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList, QUrl};
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_export_core::{caption::ytt, json, output, project, time_format, video};
use shrimply_math_core::Fraction;
use shrimply_state::preferences::{self, SharedPreferences};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Context {
    project: Rc<RefCell<project::Project>>,
    preferences: SharedPreferences,
}

thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

pub fn install(session: &EditorSession) {
    CONTEXT.with_borrow_mut(|context| {
        assert!(context.is_none(), "Qt export is already installed");
        *context = Some(Context {
            project: session.project.clone(),
            preferences: session.preferences.clone(),
        });
    });
}

fn context() -> Context {
    CONTEXT.with_borrow(|context| {
        context
            .clone()
            .expect("Qt export requires a ready editor session")
    })
}

enum JobEvent {
    VideoProgress(video::ExportProgress),
    Finished(Result<Completion, String>),
}

enum Completion {
    Video { path: PathBuf, elapsed: Duration },
    Captions(Vec<PathBuf>),
    Json(PathBuf),
}

struct ExportJob {
    receiver: Receiver<JobEvent>,
    cancelled: Arc<AtomicBool>,
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, busy)]
        #[qproperty(QString, status)]
        #[qproperty(QString, progress_text, cxx_name = "progressText")]
        #[qproperty(f64, progress)]
        #[qproperty(bool, progress_determinate, cxx_name = "progressDeterminate")]
        #[qproperty(QStringList, frame_rate_labels, cxx_name = "frameRateLabels")]
        #[qproperty(i32, video_codec, cxx_name = "videoCodec")]
        #[qproperty(i32, container)]
        #[qproperty(i32, frame_rate_index, cxx_name = "frameRateIndex")]
        #[qproperty(i32, background_alpha, cxx_name = "backgroundAlpha")]
        #[qproperty(i32, rate_control, cxx_name = "rateControl")]
        #[qproperty(i32, bitrate_kbps, cxx_name = "bitrateKbps")]
        #[qproperty(i32, max_bitrate_kbps, cxx_name = "maxBitrateKbps")]
        #[qproperty(i32, target_quality, cxx_name = "targetQuality")]
        #[qproperty(i32, constant_qp, cxx_name = "constantQp")]
        #[qproperty(i32, keyframe_interval_seconds, cxx_name = "keyframeIntervalSeconds")]
        #[qproperty(i32, preset)]
        #[qproperty(i32, tuning)]
        #[qproperty(i32, multipass)]
        #[qproperty(i32, profile)]
        #[qproperty(bool, look_ahead, cxx_name = "lookAhead")]
        #[qproperty(bool, adaptive_quantization, cxx_name = "adaptiveQuantization")]
        #[qproperty(i32, b_frames, cxx_name = "bFrames")]
        #[qproperty(bool, b_frame_as_reference, cxx_name = "bFrameAsReference")]
        #[qproperty(i32, audio_encoder, cxx_name = "audioEncoder")]
        #[qproperty(i32, audio_sample_rate, cxx_name = "audioSampleRate")]
        #[qproperty(i32, audio_bitrate_kbps, cxx_name = "audioBitrateKbps")]
        type ExportBackend = super::ExportBackendRust;

        #[qinvokable]
        #[cxx_name = "resetVideo"]
        fn reset_video(self: Pin<&mut ExportBackend>);
        #[qinvokable]
        #[cxx_name = "startVideo"]
        fn start_video(self: Pin<&mut ExportBackend>) -> bool;
        #[qinvokable]
        #[cxx_name = "startCaptions"]
        fn start_captions(self: Pin<&mut ExportBackend>, separate: bool) -> bool;
        #[qinvokable]
        #[cxx_name = "startJson"]
        fn start_json(self: Pin<&mut ExportBackend>) -> bool;
        #[qinvokable]
        fn cancel(self: Pin<&mut ExportBackend>);
        #[qinvokable]
        fn poll(self: Pin<&mut ExportBackend>);
        #[qinvokable]
        fn translate(self: &ExportBackend, key: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "revealLastOutput"]
        fn reveal_last_output(self: Pin<&mut ExportBackend>);

        #[qsignal]
        fn succeeded(self: Pin<&mut ExportBackend>, title: QString);
        #[qsignal]
        fn failed(self: Pin<&mut ExportBackend>, heading: QString, body: QString);
        #[qsignal]
        fn canceled(self: Pin<&mut ExportBackend>);
        #[qsignal]
        #[cxx_name = "openPath"]
        fn open_path(self: Pin<&mut ExportBackend>, url: QUrl);
    }

    impl cxx_qt::Initialize for ExportBackend {}
}

pub struct ExportBackendRust {
    busy: bool,
    status: QString,
    progress_text: QString,
    progress: f64,
    progress_determinate: bool,
    frame_rate_labels: QStringList,
    video_codec: i32,
    container: i32,
    frame_rate_index: i32,
    background_alpha: i32,
    rate_control: i32,
    bitrate_kbps: i32,
    max_bitrate_kbps: i32,
    target_quality: i32,
    constant_qp: i32,
    keyframe_interval_seconds: i32,
    preset: i32,
    tuning: i32,
    multipass: i32,
    profile: i32,
    look_ahead: bool,
    adaptive_quantization: bool,
    b_frames: i32,
    b_frame_as_reference: bool,
    audio_encoder: i32,
    audio_sample_rate: i32,
    audio_bitrate_kbps: i32,
    frame_rates: Vec<Fraction>,
    job: Option<ExportJob>,
    last_output: Option<PathBuf>,
}

impl Default for ExportBackendRust {
    fn default() -> Self {
        Self {
            busy: false,
            status: shrimply_i18n_qt::text("Preparing"),
            progress_text: shrimply_i18n_qt::text("Preparing"),
            progress: 0.0,
            progress_determinate: false,
            frame_rate_labels: QStringList::default(),
            video_codec: 1,
            container: 0,
            frame_rate_index: 0,
            background_alpha: 0,
            rate_control: 0,
            bitrate_kbps: 10_000,
            max_bitrate_kbps: 10_000,
            target_quality: 20,
            constant_qp: 20,
            keyframe_interval_seconds: 0,
            preset: 5,
            tuning: 1,
            multipass: 1,
            profile: 0,
            look_ahead: true,
            adaptive_quantization: true,
            b_frames: 2,
            b_frame_as_reference: false,
            audio_encoder: 0,
            audio_sample_rate: 1,
            audio_bitrate_kbps: 192,
            frame_rates: Vec::new(),
            job: None,
            last_output: None,
        }
    }
}

impl cxx_qt::Initialize for qobject::ExportBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::ExportBackend {
    pub fn reset_video(mut self: Pin<&mut Self>) {
        assert!(!self.busy, "cannot reset settings during an export");
        let project_fps = context().project.borrow().fps;
        let mut rates = project::COMMON_FRAME_RATES
            .iter()
            .map(|rate| (rate.value, rate.label.to_string()))
            .collect::<Vec<_>>();
        if !rates.iter().any(|(rate, _)| *rate == project_fps) {
            rates.push((project_fps, project::fraction_as_label(project_fps)));
        }
        let selected = rates
            .iter()
            .position(|(rate, _)| *rate == project_fps)
            .expect("project frame rate must be available for export");
        self.as_mut().rust_mut().get_mut().frame_rates =
            rates.iter().map(|(rate, _)| *rate).collect();
        self.as_mut().set_frame_rate_labels(
            rates
                .into_iter()
                .map(|(_, label)| QString::from(label))
                .collect(),
        );
        self.as_mut().set_frame_rate_index(
            i32::try_from(selected).expect("frame rate index exceeds Qt model capacity"),
        );
        self.as_mut().set_video_codec(1);
        self.as_mut().set_container(0);
        self.as_mut().set_background_alpha(0);
        self.as_mut().set_rate_control(0);
        self.as_mut().set_bitrate_kbps(10_000);
        self.as_mut().set_max_bitrate_kbps(10_000);
        self.as_mut().set_target_quality(20);
        self.as_mut().set_constant_qp(20);
        self.as_mut().set_keyframe_interval_seconds(0);
        self.as_mut().set_preset(5);
        self.as_mut().set_tuning(1);
        self.as_mut().set_multipass(1);
        self.as_mut().set_profile(0);
        self.as_mut().set_look_ahead(true);
        self.as_mut().set_adaptive_quantization(true);
        self.as_mut().set_b_frames(2);
        self.as_mut().set_b_frame_as_reference(false);
        self.as_mut().set_audio_encoder(0);
        self.as_mut().set_audio_sample_rate(1);
        self.as_mut().set_audio_bitrate_kbps(192);
    }

    pub fn start_video(mut self: Pin<&mut Self>) -> bool {
        if self.busy {
            return false;
        }
        let context = context();
        let project = context.project.borrow().clone();
        let extension =
            video::extension_for_container(container_for(self.video_codec, self.container));
        let path = choose_path(
            &output::default_filename(&project, extension),
            "Export Video",
            video_filter(extension),
            extension,
        );
        let Some(path) = path else {
            return false;
        };
        let settings = self.video_settings(
            output::ensure_extension(path, extension),
            &preferences::snapshot(&context.preferences),
        );
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        thread::spawn(move || {
            let started = Instant::now();
            let path = settings.path.clone();
            let progress_sender = sender.clone();
            let result = video::export_project(project, settings, worker_cancelled, move |event| {
                let _ = progress_sender.send(JobEvent::VideoProgress(event));
            })
            .map(|()| Completion::Video {
                path,
                elapsed: started.elapsed(),
            });
            let _ = sender.send(JobEvent::Finished(result));
        });
        self.as_mut().begin_job(receiver, cancelled, "Preparing");
        true
    }

    pub fn start_captions(mut self: Pin<&mut Self>, separate: bool) -> bool {
        if self.busy {
            return false;
        }
        let project = context().project.borrow().clone();
        let path = choose_path(
            &output::default_filename(&project, "ytt"),
            "Export YouTube Captions",
            "YouTube captions (*.ytt)",
            "ytt",
        );
        let Some(path) = path else {
            return false;
        };
        let path = output::ensure_extension(path, "ytt");
        let mode = if separate {
            ytt::ExportMode::Separate
        } else {
            ytt::ExportMode::Merge
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        thread::spawn(move || {
            let result = ytt::export(&project, &path, mode).map(Completion::Captions);
            let _ = sender.send(JobEvent::Finished(result));
        });
        self.as_mut()
            .begin_job(receiver, cancelled, "Exporting Captions");
        true
    }

    pub fn start_json(mut self: Pin<&mut Self>) -> bool {
        if self.busy {
            return false;
        }
        let project = context().project.borrow().clone();
        let path = choose_path(
            &output::default_filename(&project, "json"),
            "Export JSON",
            "JSON files (*.json)",
            "json",
        );
        let Some(path) = path else {
            return false;
        };
        let path = output::ensure_extension(path, "json");
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        thread::spawn(move || {
            let result = json::export(&project, &path).map(|()| Completion::Json(path));
            let _ = sender.send(JobEvent::Finished(result));
        });
        self.as_mut()
            .begin_job(receiver, cancelled, "Exporting JSON");
        true
    }

    pub fn cancel(mut self: Pin<&mut Self>) {
        if let Some(job) = self.as_ref().rust().job.as_ref() {
            job.cancelled.store(true, Ordering::Relaxed);
            self.as_mut()
                .set_status(shrimply_i18n_qt::text("Canceling"));
        }
    }

    pub fn poll(mut self: Pin<&mut Self>) {
        let mut latest_progress = None;
        let mut finished = None;
        loop {
            let event = {
                let this = self.as_ref();
                let Some(job) = this.rust().job.as_ref() else {
                    return;
                };
                job.receiver.try_recv()
            };
            match event {
                Ok(JobEvent::VideoProgress(event)) => latest_progress = Some(event),
                Ok(JobEvent::Finished(result)) => {
                    finished = Some(result);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = Some(Err(
                        "The export worker stopped before reporting a result.".to_string()
                    ));
                    break;
                }
            }
        }
        if let Some(event) = latest_progress {
            self.as_mut().show_progress(event);
        }
        if let Some(result) = finished {
            self.as_mut().finish(result);
        }
    }

    pub fn translate(&self, key: &QString) -> QString {
        shrimply_i18n_qt::text(&key.to_string())
    }

    pub fn reveal_last_output(mut self: Pin<&mut Self>) {
        let Some(path) = self.as_ref().rust().last_output.clone() else {
            return;
        };
        match shrimply_qt_components::desktop_open::prepare(&path, None) {
            Ok(shrimply_qt_components::desktop_open::Action::Open(path)) => self
                .as_mut()
                .open_path(QUrl::from_local_file(&QString::from(
                    path.to_string_lossy().as_ref(),
                ))),
            Ok(shrimply_qt_components::desktop_open::Action::FocusRevealed(_)) => {}
            Err(error) => self.as_mut().failed(
                shrimply_i18n_qt::text("Could not show exported file"),
                QString::from(error),
            ),
        }
    }

    fn begin_job(
        mut self: Pin<&mut Self>,
        receiver: Receiver<JobEvent>,
        cancelled: Arc<AtomicBool>,
        status: &'static str,
    ) {
        self.as_mut().rust_mut().get_mut().job = Some(ExportJob {
            receiver,
            cancelled,
        });
        self.as_mut().set_progress(0.0);
        self.as_mut().set_progress_determinate(false);
        self.as_mut().set_status(shrimply_i18n_qt::text(status));
        self.as_mut()
            .set_progress_text(shrimply_i18n_qt::text(status));
        self.as_mut().set_busy(true);
    }

    fn video_settings(
        &self,
        path: PathBuf,
        preferences: &preferences::PreferencesSnapshot,
    ) -> video::ExportSettings {
        let fps = *self
            .frame_rates
            .get(usize::try_from(self.frame_rate_index).expect("negative frame rate index"))
            .expect("Qt selected an unavailable frame rate");
        video::ExportSettings {
            path,
            video_codec: video_codec(self.video_codec),
            container: container_for(self.video_codec, self.container),
            fps,
            background_alpha: if self.video_codec == 2 {
                u8::try_from(self.background_alpha).expect("background alpha is out of range")
            } else {
                u8::MAX
            },
            rate_control: rate_control(self.rate_control),
            constant_qp: as_u32(self.constant_qp, "constant QP"),
            bitrate_kbps: as_u32(self.bitrate_kbps, "video bitrate"),
            max_bitrate_kbps: as_u32(self.max_bitrate_kbps, "maximum video bitrate"),
            target_quality: as_u32(self.target_quality, "target quality"),
            keyframe_interval_seconds: as_u32(self.keyframe_interval_seconds, "keyframe interval"),
            preset: preset(self.preset),
            tuning: tuning(self.tuning),
            multipass: multipass(self.multipass),
            profile: profile(self.profile),
            look_ahead: self.look_ahead,
            adaptive_quantization: self.adaptive_quantization,
            b_frames: as_u32(self.b_frames, "B frames"),
            b_frame_as_reference: self.b_frame_as_reference,
            audio_encoder: audio_encoder(self.audio_encoder),
            audio_sample_rate: match self.audio_sample_rate {
                0 => 44_100,
                1 => 48_000,
                2 => 96_000,
                value => panic!("unknown audio sample rate index {value}"),
            },
            audio_bitrate_kbps: as_u32(self.audio_bitrate_kbps, "audio bitrate"),
            maximum_temporal_decoders: preferences.temporal_decoder_pool_size as usize,
            gpu_host_memory_gib: preferences.gpu_host_memory_gib,
        }
    }

    fn show_progress(mut self: Pin<&mut Self>, event: video::ExportProgress) {
        match event {
            video::ExportProgress::MixingAudio {
                current_frame,
                total_frames,
            } => self.as_mut().set_count_progress(
                "Preparing audio",
                "Preparing audio (%{percent}%)",
                current_frame,
                total_frames,
            ),
            video::ExportProgress::SettingUp(message) => {
                self.as_mut().set_status(shrimply_i18n_qt::text(message));
                self.as_mut()
                    .set_progress_text(shrimply_i18n_qt::text(message));
                self.as_mut().set_progress_determinate(false);
            }
            video::ExportProgress::EncodingAudio {
                current_frame,
                total_frames,
            } => self.as_mut().set_count_progress(
                "Encoding audio",
                "Encoding audio (%{percent}%)",
                current_frame,
                total_frames,
            ),
            video::ExportProgress::EncodingVideo {
                current_frame,
                total_frames,
                fps_milli,
            } => {
                let fraction = progress_fraction(current_frame, total_frames);
                let mut arguments = vec![
                    ("current", current_frame.to_string()),
                    ("total", total_frames.to_string()),
                    ("percent", format!("{:.0}", fraction * 100.0)),
                ];
                let key = if fps_milli > 0 {
                    arguments.push((
                        "fps",
                        format!("{}.{}", fps_milli / 1_000, fps_milli % 1_000 / 100),
                    ));
                    arguments.push((
                        "eta",
                        time_format::human_duration(
                            shrimply_math_media::duration_for_frames_at_millifps(
                                total_frames.saturating_sub(current_frame),
                                fps_milli,
                            ),
                        ),
                    ));
                    "%{current} of %{total} frames (%{percent}%) - %{fps} fps - %{eta} left"
                } else {
                    "%{current} of %{total} frames (%{percent}%)"
                };
                self.as_mut()
                    .set_status(shrimply_i18n_qt::text("Rendering frames"));
                self.as_mut()
                    .set_progress_text(shrimply_i18n_qt::text_args(key, &arguments));
                self.as_mut().set_progress(fraction);
                self.as_mut().set_progress_determinate(true);
            }
            video::ExportProgress::Finalizing => {
                self.as_mut()
                    .set_status(shrimply_i18n_qt::text("Finishing file"));
                self.as_mut()
                    .set_progress_text(shrimply_i18n_qt::text("Finishing"));
                self.as_mut().set_progress_determinate(false);
            }
        }
    }

    fn set_count_progress(
        mut self: Pin<&mut Self>,
        status: &'static str,
        text: &'static str,
        current: u64,
        total: u64,
    ) {
        let fraction = progress_fraction(current, total);
        self.as_mut().set_status(shrimply_i18n_qt::text(status));
        self.as_mut().set_progress_text(shrimply_i18n_qt::text_args(
            text,
            &[("percent", format!("{:.0}", fraction * 100.0))],
        ));
        self.as_mut().set_progress(fraction);
        self.as_mut().set_progress_determinate(true);
    }

    fn finish(mut self: Pin<&mut Self>, result: Result<Completion, String>) {
        let was_cancelled = self
            .as_ref()
            .rust()
            .job
            .as_ref()
            .is_some_and(|job| job.cancelled.load(Ordering::Relaxed));
        self.as_mut().rust_mut().get_mut().job = None;
        self.as_mut().set_busy(false);
        match result {
            Ok(Completion::Video { path, elapsed }) if !was_cancelled => {
                self.as_mut().rust_mut().get_mut().last_output = Some(path);
                self.as_mut().succeeded(shrimply_i18n_qt::text_args(
                    "Video exported in %{duration}",
                    &[("duration", time_format::human_duration(elapsed))],
                ));
            }
            Ok(Completion::Captions(paths)) if !was_cancelled => {
                let Some(path) = paths.first().cloned() else {
                    self.as_mut().failed(
                        shrimply_i18n_qt::text("Could not export captions"),
                        shrimply_i18n_qt::text("No caption files were exported."),
                    );
                    return;
                };
                let title = if paths.len() == 1 {
                    shrimply_i18n_qt::text("Captions exported")
                } else {
                    shrimply_i18n_qt::text_args(
                        "%{count} caption files exported",
                        &[("count", paths.len().to_string())],
                    )
                };
                self.as_mut().rust_mut().get_mut().last_output = Some(path);
                self.as_mut().succeeded(title);
            }
            Ok(Completion::Json(path)) if !was_cancelled => {
                self.as_mut().rust_mut().get_mut().last_output = Some(path);
                self.as_mut()
                    .succeeded(shrimply_i18n_qt::text("JSON exported"));
            }
            Ok(_) => self.as_mut().canceled(),
            Err(_) if was_cancelled => self.as_mut().canceled(),
            Err(error) => self.as_mut().failed(
                shrimply_i18n_qt::text("Could not export"),
                QString::from(error),
            ),
        }
    }
}

fn choose_path(name: &str, title: &str, filter: &str, suffix: &str) -> Option<PathBuf> {
    let url = shrimply_qt_helpers::save_file_dialog(
        &QUrl::from_local_file(&QString::from(name)),
        &shrimply_i18n_qt::text(title),
        &shrimply_i18n_qt::text(filter),
        &QString::from(suffix),
    );
    (url.is_local_file() && !url.is_empty())
        .then(|| PathBuf::from(url.to_local_file_or_default().to_string()))
}

fn video_filter(extension: &str) -> &'static str {
    match extension {
        "mp4" => "MP4 video (*.mp4)",
        "mkv" => "Matroska video (*.mkv)",
        "gif" => "GIF image (*.gif)",
        _ => unreachable!("unsupported video extension"),
    }
}

fn video_codec(index: i32) -> video::ExportVideoCodec {
    match index {
        0 => video::ExportVideoCodec::H264,
        1 => video::ExportVideoCodec::H265,
        2 => video::ExportVideoCodec::Gif,
        value => panic!("unknown video codec index {value}"),
    }
}

fn container_for(codec: i32, container: i32) -> video::ExportContainer {
    if codec == 2 {
        video::ExportContainer::Gif
    } else {
        match container {
            0 => video::ExportContainer::Mp4,
            1 => video::ExportContainer::Mkv,
            value => panic!("unknown video container index {value}"),
        }
    }
}

fn rate_control(index: i32) -> video::ExportRateControl {
    match index {
        0 => video::ExportRateControl::ConstantQp,
        1 => video::ExportRateControl::ConstantBitrate,
        2 => video::ExportRateControl::VariableBitrate,
        3 => video::ExportRateControl::VariableBitrateTargetQuality,
        4 => video::ExportRateControl::Lossless,
        value => panic!("unknown rate control index {value}"),
    }
}

fn preset(index: i32) -> video::ExportPreset {
    match index {
        0 => video::ExportPreset::P1,
        1 => video::ExportPreset::P2,
        2 => video::ExportPreset::P3,
        3 => video::ExportPreset::P4,
        4 => video::ExportPreset::P5,
        5 => video::ExportPreset::P6,
        6 => video::ExportPreset::P7,
        value => panic!("unknown preset index {value}"),
    }
}

fn tuning(index: i32) -> video::ExportTuning {
    match index {
        0 => video::ExportTuning::UltraHighQuality,
        1 => video::ExportTuning::HighQuality,
        2 => video::ExportTuning::LowLatency,
        3 => video::ExportTuning::UltraLowLatency,
        value => panic!("unknown tuning index {value}"),
    }
}

fn multipass(index: i32) -> video::ExportMultipass {
    match index {
        0 => video::ExportMultipass::SinglePass,
        1 => video::ExportMultipass::QuarterResolution,
        2 => video::ExportMultipass::FullResolution,
        value => panic!("unknown multipass index {value}"),
    }
}

fn profile(index: i32) -> video::ExportProfile {
    match index {
        0 => video::ExportProfile::Main,
        1 => video::ExportProfile::Main10,
        value => panic!("unknown profile index {value}"),
    }
}

fn audio_encoder(index: i32) -> video::ExportAudioEncoder {
    match index {
        0 => video::ExportAudioEncoder::Aac,
        1 => video::ExportAudioEncoder::Opus,
        value => panic!("unknown audio encoder index {value}"),
    }
}

fn as_u32(value: i32, name: &str) -> u32 {
    u32::try_from(value).unwrap_or_else(|_| panic!("{name} must not be negative"))
}

fn progress_fraction(current: u64, total: u64) -> f64 {
    if total == 0 {
        1.0
    } else {
        current as f64 / total as f64
    }
    .clamp(0.0, 1.0)
}
