use core::pin::Pin;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QUrl};
use serde_json::Value;
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_editor_state::preferences::{self, SharedPreferences};
use shrimply_export_core::{caption::ytt, json, output, project, time_format};
use shrimply_export_core::{
    job::{Event, Job, Outcome},
    video::{progress, settings},
};
use shrimply_export_cuda as video;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
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

enum Completion {
    Video { path: PathBuf, elapsed: Duration },
    Captions(Vec<PathBuf>),
    Json(PathBuf),
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
    job: Option<Job<Completion>>,
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
        self.as_mut().begin_job(
            Job::spawn(
                move |cancelled, progress| {
                    let started = Instant::now();
                    let path = settings.path.clone();
                    video::export_project(project, settings, cancelled, progress).map(|()| {
                        Completion::Video {
                            path,
                            elapsed: started.elapsed(),
                        }
                    })
                },
                || {},
            ),
            "Preparing",
        )
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
        self.as_mut().begin_job(
            Job::spawn(
                move |_, _| ytt::export(&project, &path, mode).map(Completion::Captions),
                || {},
            ),
            "Exporting Captions",
        )
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
        self.as_mut().begin_job(
            Job::spawn(
                move |_, _| json::export(&project, &path).map(|()| Completion::Json(path)),
                || {},
            ),
            "Exporting JSON",
        )
    }

    pub fn cancel(self: Pin<&mut Self>) {
        if let Some(job) = self.as_ref().rust().job.as_ref() {
            job.cancel();
        }
    }

    pub fn poll(mut self: Pin<&mut Self>) {
        let (events, cancelling) = match self.as_mut().rust_mut().get_mut().job.as_mut() {
            Some(job) => (job.poll(), job.is_cancelling()),
            None => return,
        };
        for event in events {
            match event {
                Event::Started => {}
                Event::Progress(progress) if !cancelling => self.as_mut().show_progress(progress),
                Event::Progress(_) => {}
                Event::Cancelling => self
                    .as_mut()
                    .set_status(shrimply_i18n_qt::text("Canceling")),
                Event::Finished(outcome) => self.as_mut().finish(outcome),
            }
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
        job: Result<Job<Completion>, String>,
        status: &str,
    ) -> bool {
        let job = match job {
            Ok(job) => job,
            Err(error) => {
                self.as_mut().failed(
                    shrimply_i18n_qt::text("Could not export"),
                    QString::from(error),
                );
                return false;
            }
        };
        self.as_mut().rust_mut().get_mut().job = Some(job);
        self.as_mut().set_progress(0.0);
        self.as_mut().set_progress_determinate(false);
        self.as_mut().set_status(shrimply_i18n_qt::text(status));
        self.as_mut()
            .set_progress_text(shrimply_i18n_qt::text(status));
        self.as_mut().set_busy(true);
        true
    }

    fn show_progress(mut self: Pin<&mut Self>, event: video::ExportProgress) {
        let event = progress::presentation(event);
        self.as_mut()
            .set_status(shrimply_i18n_qt::text(event.label));
        self.as_mut().set_progress_text(shrimply_i18n_qt::text_args(
            event.message,
            &event.arguments.into_iter().collect::<Vec<_>>(),
        ));
        let fraction = event.fraction;
        self.as_mut().set_progress_determinate(fraction.is_some());
        if let Some(fraction) = fraction {
            self.as_mut().set_progress(fraction);
        }
    }

    fn finish(mut self: Pin<&mut Self>, result: Outcome<Completion>) {
        self.as_mut().rust_mut().get_mut().job = None;
        self.as_mut().set_busy(false);
        match result {
            Outcome::Complete(Completion::Video { path, elapsed }) => {
                self.as_mut().rust_mut().get_mut().last_output = Some(path);
                self.as_mut().succeeded(shrimply_i18n_qt::text_args(
                    "Video exported in %{duration}",
                    &[("duration", time_format::human_duration(elapsed))],
                ));
            }
            Outcome::Complete(Completion::Captions(paths)) => {
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
            Outcome::Complete(Completion::Json(path)) => {
                self.as_mut().rust_mut().get_mut().last_output = Some(path);
                self.as_mut()
                    .succeeded(shrimply_i18n_qt::text("JSON exported"));
            }
            Outcome::Cancelled => self.as_mut().canceled(),
            Outcome::Failed(error) => self.as_mut().failed(
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
