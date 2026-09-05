use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QUrl};
use shrimply_cross_ui_core::launcher::Launcher;
use std::path::{Path, PathBuf};
use std::process::Child;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("shrimply-launcher-qt/include/icon_theme.h");
        #[namespace = "shrimply"]
        fn set_breeze_icon_fallback();

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(i32, recent_count, cxx_name = "recentCount")]
        #[qproperty(i32, preset_count, cxx_name = "presetCount")]
        #[qproperty(i32, frame_rate_count, cxx_name = "frameRateCount")]
        type LauncherBackend = super::LauncherBackendRust;

        #[qinvokable]
        fn text(self: &LauncherBackend, key: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "recentName"]
        fn recent_name(self: &LauncherBackend, index: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "recentPath"]
        fn recent_path(self: &LauncherBackend, index: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "recentLastEdited"]
        fn recent_last_edited(self: &LauncherBackend, index: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "presetLabel"]
        fn preset_label(self: &LauncherBackend, index: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "frameRateLabel"]
        fn frame_rate_label(self: &LauncherBackend, index: i32) -> QString;

        #[qinvokable]
        #[cxx_name = "projectFileFilter"]
        fn project_file_filter(self: &LauncherBackend) -> QString;

        #[qinvokable]
        #[cxx_name = "setSearch"]
        fn set_search(self: Pin<&mut LauncherBackend>, query: &QString);

        #[qinvokable]
        #[cxx_name = "clearHistory"]
        fn clear_history(self: Pin<&mut LauncherBackend>);

        #[qinvokable]
        #[cxx_name = "removeRecent"]
        fn remove_recent(self: Pin<&mut LauncherBackend>, index: i32);

        #[qinvokable]
        #[cxx_name = "showRecent"]
        fn show_recent(self: Pin<&mut LauncherBackend>, index: i32);

        #[qinvokable]
        #[cxx_name = "openRecent"]
        fn open_recent(self: Pin<&mut LauncherBackend>, index: i32);

        #[qinvokable]
        #[cxx_name = "openProject"]
        fn open_project(self: Pin<&mut LauncherBackend>, url: &QUrl);

        #[qinvokable]
        #[cxx_name = "chooseProject"]
        fn choose_project(self: Pin<&mut LauncherBackend>);

        #[qinvokable]
        #[cxx_name = "presetWidth"]
        fn preset_width(self: &LauncherBackend, index: i32) -> i32;

        #[qinvokable]
        #[cxx_name = "presetHeight"]
        fn preset_height(self: &LauncherBackend, index: i32) -> i32;

        #[qinvokable]
        #[cxx_name = "presetFrameRate"]
        fn preset_frame_rate(self: &LauncherBackend, index: i32) -> i32;

        #[qinvokable]
        #[cxx_name = "chooseProjectDestination"]
        fn choose_project_destination(self: &LauncherBackend, name: &QString) -> QUrl;

        #[qinvokable]
        #[cxx_name = "requestCreateProject"]
        fn request_create_project(
            self: Pin<&mut LauncherBackend>,
            name: &QString,
            width: i32,
            height: i32,
            frame_rate_index: i32,
            url: &QUrl,
        );

        #[qsignal]
        #[cxx_name = "showError"]
        fn show_error(self: Pin<&mut LauncherBackend>, heading: QString, body: QString);

        #[qsignal]
        #[cxx_name = "editorStarted"]
        fn editor_started(self: Pin<&mut LauncherBackend>);

        #[qsignal]
        #[cxx_name = "editorFinished"]
        fn editor_finished(self: Pin<&mut LauncherBackend>);

        #[qsignal]
        #[cxx_name = "openDirectory"]
        fn open_directory(self: Pin<&mut LauncherBackend>, url: QUrl, after_reveal: bool);
    }

    impl cxx_qt::Initialize for LauncherBackend {}
    impl cxx_qt::Threading for LauncherBackend {}
}

pub struct LauncherBackendRust {
    recent_count: i32,
    preset_count: i32,
    frame_rate_count: i32,
    core: Launcher,
}

impl Default for LauncherBackendRust {
    fn default() -> Self {
        Self {
            recent_count: 0,
            preset_count: count(shrimply_cross_ui_core::launcher::preset_labels().len()),
            frame_rate_count: count(shrimply_cross_ui_core::launcher::frame_rate_labels().len()),
            core: Launcher::default(),
        }
    }
}

impl cxx_qt::Initialize for qobject::LauncherBackend {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_preset_count(count(
            shrimply_cross_ui_core::launcher::preset_labels().len(),
        ));
        self.as_mut().set_frame_rate_count(count(
            shrimply_cross_ui_core::launcher::frame_rate_labels().len(),
        ));
        self.as_mut().reload_recents();
        tracing::info!(
            recent_count = self.recent_count(),
            preset_count = self.preset_count(),
            frame_rate_count = self.frame_rate_count(),
            first_recent = %self.recent_name(0),
            first_preset = %self.preset_label(0),
            first_frame_rate = %self.frame_rate_label(0),
            "Qt launcher models initialized"
        );
    }
}

impl qobject::LauncherBackend {
    pub fn text(&self, key: &QString) -> QString {
        shrimply_i18n_qt::text(&key.to_string())
    }

    pub fn recent_name(&self, index: i32) -> QString {
        let name = index_of(index)
            .and_then(|index| self.core.recent_name(index))
            .map(QString::from)
            .unwrap_or_default();
        tracing::debug!(index, name = %name, "Qt recent name requested");
        name
    }

    pub fn recent_path(&self, index: i32) -> QString {
        index_of(index)
            .and_then(|index| self.core.recent_path(index))
            .map(|path| QString::from(path.to_string_lossy().as_ref()))
            .unwrap_or_default()
    }

    pub fn recent_last_edited(&self, index: i32) -> QString {
        index_of(index)
            .and_then(|index| self.core.recent_last_edited(index))
            .map(|date| shrimply_i18n_qt::text_args("Last edited %{date}", &[("date", date)]))
            .unwrap_or_else(|| shrimply_i18n_qt::text("Last edited time unavailable"))
    }

    pub fn preset_label(&self, index: i32) -> QString {
        index_of(index)
            .and_then(|index| {
                shrimply_cross_ui_core::launcher::preset_labels()
                    .get(index)
                    .copied()
            })
            .map(shrimply_i18n_qt::text)
            .unwrap_or_default()
    }

    pub fn frame_rate_label(&self, index: i32) -> QString {
        index_of(index)
            .and_then(|index| {
                shrimply_cross_ui_core::launcher::frame_rate_labels()
                    .get(index)
                    .copied()
            })
            .map(QString::from)
            .unwrap_or_default()
    }

    pub fn project_file_filter(&self) -> QString {
        let label = shrimply_i18n_qt::text("Shrimply projects").to_string();
        QString::from(shrimply_cross_ui_core::launcher::project_file_name_filter(
            &label,
        ))
    }

    pub fn set_search(mut self: Pin<&mut Self>, query: &QString) {
        let result = self.as_mut().rust_mut().core.set_search(query.to_string());
        self.finish_core_action(result, "Could not load recent projects");
    }

    pub fn clear_history(mut self: Pin<&mut Self>) {
        let result = self.as_mut().rust_mut().core.clear_history();
        self.finish_core_action(result, "Could not clear recent projects");
    }

    pub fn remove_recent(mut self: Pin<&mut Self>, index: i32) {
        let Some(index) = index_of(index) else {
            return;
        };
        let result = self.as_mut().rust_mut().core.remove_recent(index);
        self.finish_core_action(result, "Could not remove recent project");
    }

    pub fn show_recent(self: Pin<&mut Self>, index: i32) {
        let Some(index) = index_of(index) else {
            return;
        };
        match self.core.desktop_action(index) {
            Ok(shrimply_cross_ui_core::desktop_open::Action::Open(path)) => {
                self.open_directory(local_url(&path), false);
            }
            Ok(shrimply_cross_ui_core::desktop_open::Action::FocusRevealed(path)) => {
                self.open_directory(local_url(&path), true);
            }
            Err(error) => self.emit_error("Could not show project file", &error),
        }
    }

    pub fn open_recent(self: Pin<&mut Self>, index: i32) {
        let Some(index) = index_of(index) else {
            return;
        };
        let editor = self
            .core
            .recent_path(index)
            .ok_or_else(|| "Recent project does not exist.".to_string())
            .and_then(shrimply_cross_ui_core::launcher::launch_qt_editor);
        self.start_editor(editor);
    }

    pub fn open_project(self: Pin<&mut Self>, url: &QUrl) {
        let Some(path) = local_path(url) else {
            self.emit_error(
                "Could not open project",
                "The selected file does not have a local path.",
            );
            return;
        };
        let editor = shrimply_cross_ui_core::launcher::launch_qt_editor(&path);
        self.start_editor(editor);
    }

    pub fn choose_project(mut self: Pin<&mut Self>) {
        let selected = shrimply_qt_helpers::open_file_dialog(
            &QUrl::default(),
            &shrimply_i18n_qt::text("Open Project"),
            &self.project_file_filter(),
        );
        if !selected.is_empty() {
            self.as_mut().open_project(&selected);
        }
    }

    pub fn preset_width(&self, index: i32) -> i32 {
        index_of(index)
            .map(shrimply_cross_ui_core::launcher::preset_width)
            .and_then(|width| i32::try_from(width).ok())
            .unwrap_or_default()
    }

    pub fn preset_height(&self, index: i32) -> i32 {
        index_of(index)
            .map(shrimply_cross_ui_core::launcher::preset_height)
            .and_then(|height| i32::try_from(height).ok())
            .unwrap_or_default()
    }

    pub fn preset_frame_rate(&self, index: i32) -> i32 {
        index_of(index)
            .map(shrimply_cross_ui_core::launcher::preset_frame_rate)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or_default()
    }

    pub fn choose_project_destination(&self, name: &QString) -> QUrl {
        let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let suggested = local_url(&shrimply_cross_ui_core::launcher::default_project_path(
            &directory,
            &name.to_string(),
        ));
        shrimply_qt_helpers::save_file_dialog(
            &suggested,
            &shrimply_i18n_qt::text("Create Project"),
            &self.project_file_filter(),
            &QString::from("shrimp"),
        )
    }

    pub fn request_create_project(
        self: Pin<&mut Self>,
        name: &QString,
        width: i32,
        height: i32,
        frame_rate_index: i32,
        url: &QUrl,
    ) {
        let Some(path) = local_path(url) else {
            self.emit_error(
                "Could not create project",
                "The selected location does not have a local path.",
            );
            return;
        };
        let path = match shrimply_cross_ui_core::launcher::create_project_from_values(
            path,
            &name.to_string(),
            width,
            height,
            frame_rate_index,
        ) {
            Ok(path) => path,
            Err(error) => {
                self.emit_error("Could not create project", &error);
                return;
            }
        };
        let editor = shrimply_cross_ui_core::launcher::launch_qt_editor(&path);
        self.start_editor(editor);
    }

    fn reload_recents(mut self: Pin<&mut Self>) {
        let result = self.as_mut().rust_mut().core.reload();
        self.finish_core_action(result, "Could not load recent projects");
    }

    fn finish_core_action(mut self: Pin<&mut Self>, result: Result<(), String>, heading: &str) {
        if let Err(error) = result {
            self.as_mut().emit_error(heading, &error);
        }
        let count = i32::try_from(self.core.recent_count()).unwrap_or(i32::MAX);
        self.set_recent_count(count);
    }

    fn emit_error(mut self: Pin<&mut Self>, heading: &str, body: &str) {
        self.as_mut()
            .show_error(shrimply_i18n_qt::text(heading), QString::from(body));
    }

    fn start_editor(mut self: Pin<&mut Self>, editor: Result<Child, String>) {
        let mut editor = match editor {
            Ok(editor) => editor,
            Err(error) => {
                self.emit_error("Could not start editor", &error);
                return;
            }
        };
        let qt_thread = self.qt_thread();
        self.as_mut().editor_started();
        std::thread::spawn(move || match editor.wait() {
            Ok(status) if status.success() => {
                qt_thread
                    .queue(|backend| backend.editor_finished())
                    .expect("Qt launcher should still exist while the editor is running");
            }
            Ok(status) => {
                eprintln!("editor exited with {status}");
                std::process::exit(status.code().unwrap_or(1).clamp(1, 255));
            }
            Err(error) => {
                eprintln!("could not wait for editor: {error}");
                std::process::exit(1);
            }
        });
    }
}

fn index_of(index: i32) -> Option<usize> {
    usize::try_from(index).ok()
}

fn count(len: usize) -> i32 {
    i32::try_from(len).unwrap_or(i32::MAX)
}

fn local_path(url: &QUrl) -> Option<PathBuf> {
    url.is_local_file()
        .then(|| PathBuf::from(url.to_local_file_or_default().to_string()))
}

fn local_url(path: &Path) -> QUrl {
    QUrl::from_local_file(&QString::from(path.to_string_lossy().as_ref()))
}
