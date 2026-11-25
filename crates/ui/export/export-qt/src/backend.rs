use core::pin::Pin;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QUrl};
use serde_json::Value;
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_editor_state::preferences::{self, SharedPreferences};
use shrimply_export_core::video::settings;
use shrimply_export_core::{caption::ytt, json, output, project, time_format};
use shrimply_export_cuda as video;
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
        #[qproperty(QString, video_form, cxx_name = "videoForm")]
        type ExportBackend = super::ExportBackendRust;

        #[qinvokable]
        #[cxx_name = "resetVideo"]
        fn reset_video(self: Pin<&mut ExportBackend>);
        #[qinvokable]
        #[cxx_name = "setVideoField"]
        fn set_video_field(self: Pin<&mut ExportBackend>, id: &QString, value: &QString);
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
    video_form: QString,
    video_values: Value,
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
            video_form: QString::default(),
            video_values: Value::Null,
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
        self.as_mut().refresh_video_form(None);
    }

    pub fn set_video_field(mut self: Pin<&mut Self>, id: &QString, value: &QString) {
        let id = id.to_string();
        let mut values = self.as_ref().rust().video_values.clone();
        if let Some(field) = values.get_mut(id.as_str())
            && let Ok(value) = serde_json::from_str(&value.to_string())
        {
            *field = value;
            self.as_mut().refresh_video_form(Some(&values));
        }
    }

    fn refresh_video_form(mut self: Pin<&mut Self>, values: Option<&Value>) {
        let context = context();
        let schema =
            settings::schema_for(&context.project.borrow(), video::settings_backend(), values);
        self.as_mut().rust_mut().get_mut().video_values = schema["values"].clone();
        self.set_video_form(QString::from(schema.to_string()));
    }

    pub fn start_video(mut self: Pin<&mut Self>) -> bool {
        if self.busy {
            return false;
        }
        let context = context();
        let project = context.project.borrow().clone();
        let preferences = preferences::snapshot(&context.preferences);
        let mut settings = match video::parse_settings(
            &project,
            PathBuf::new(),
            &self.as_ref().rust().video_values,
            video::RenderResourceConfig {
                maximum_temporal_decoders: preferences.temporal_decoder_pool_size as usize,
                gpu_host_memory_gib: preferences.gpu_host_memory_gib,
            },
        ) {
            Ok(settings) => settings,
            Err(error) => {
                self.as_mut().failed(
                    shrimply_i18n_qt::text("Export failed"),
                    QString::from(error),
                );
                return false;
            }
        };
        let extension = video::extension_for_container(settings.container);
        let Some(path) = choose_path(
            &output::default_filename(&project, extension),
            "Export Video",
            video_filter(extension),
            extension,
        ) else {
            return false;
        };
        settings.path = output::ensure_extension(path, extension);
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
        match shrimply_components_qt::desktop_open::prepare(&path, None) {
            Ok(shrimply_components_qt::desktop_open::Action::Open(path)) => self
                .as_mut()
                .open_path(QUrl::from_local_file(&QString::from(
                    path.to_string_lossy().as_ref(),
                ))),
            Ok(shrimply_components_qt::desktop_open::Action::FocusRevealed(_)) => {}
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
    let url = shrimply_application_qt::save_file_dialog(
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

fn progress_fraction(current: u64, total: u64) -> f64 {
    if total == 0 {
        1.0
    } else {
        current as f64 / total as f64
    }
    .clamp(0.0, 1.0)
}
