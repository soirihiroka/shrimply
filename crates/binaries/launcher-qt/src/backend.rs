use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList, QUrl};
use shrimply_cross_ui_core::launcher::{self, Launcher};
use std::path::{Path, PathBuf};
use std::process::Child;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("shrimply-launcher-qt/include/startup_error.h");
        #[namespace = "shrimply"]
        fn show_startup_error(heading: &QString, body: &QString);

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
        #[qproperty(QStringList, projects)]
        #[qproperty(i32, preset_count, cxx_name = "presetCount")]
        #[qproperty(i32, frame_rate_count, cxx_name = "frameRateCount")]
        type LauncherBackend = super::LauncherBackendRust;

        #[qinvokable]
        fn text(self: &LauncherBackend, key: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "projectIsRemote"]
        fn project_is_remote(self: &LauncherBackend, index: i32) -> bool;

        #[qinvokable]
        #[cxx_name = "projectServers"]
        fn project_servers(self: &LauncherBackend, include_local: bool) -> QStringList;

        #[qinvokable]
        #[cxx_name = "updateProjectServer"]
        fn update_project_server(
            self: Pin<&mut LauncherBackend>,
            previous: &QString,
            url: &QString,
        ) -> bool;

        #[qinvokable]
        #[cxx_name = "createRemoteProject"]
        fn create_remote_project(
            self: Pin<&mut LauncherBackend>,
            server: &QString,
            name: &QString,
            width: i32,
            height: i32,
            frame_rate_index: i32,
        );

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
        #[cxx_name = "openListedProject"]
        fn open_listed_project(self: Pin<&mut LauncherBackend>, index: i32);

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

    }

    impl cxx_qt::Initialize for LauncherBackend {}
    impl cxx_qt::Threading for LauncherBackend {}
}

pub struct LauncherBackendRust {
    projects: QStringList,
    preset_count: i32,
    frame_rate_count: i32,
    core: Launcher,
}

impl Default for LauncherBackendRust {
    fn default() -> Self {
        Self {
            projects: QStringList::default(),
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
        if let Ok(projects) = launcher::local_projects() {
            self.as_mut().rust_mut().core.set_projects(projects);
            self.as_mut().update_projects();
        }
        self.reload_projects();
    }
}

impl qobject::LauncherBackend {
    pub fn text(&self, key: &QString) -> QString {
        shrimply_i18n_qt::text(&key.to_string())
    }

    pub fn project_is_remote(&self, index: i32) -> bool {
        index_of(index)
            .and_then(|index| self.core.project(index))
            .is_some_and(|project| project.remote.is_some())
    }

    pub fn project_servers(&self, include_local: bool) -> QStringList {
        include_local
            .then(|| shrimply_i18n_qt::text("Local"))
            .into_iter()
            .chain(launcher::project_servers().into_iter().map(QString::from))
            .collect()
    }

    pub fn update_project_server(self: Pin<&mut Self>, previous: &QString, url: &QString) -> bool {
        let previous = previous.to_string();
        let url = url.to_string();
        match launcher::update_project_servers(
            (!previous.is_empty()).then_some(previous.as_str()),
            (!url.is_empty()).then_some(url.as_str()),
        ) {
            Ok(()) => {
                self.reload_projects();
                true
            }
            Err(error) => {
                self.emit_error("Could not update project server", &error);
                false
            }
        }
    }

    pub fn create_remote_project(
        self: Pin<&mut Self>,
        server: &QString,
        name: &QString,
        width: i32,
        height: i32,
        frame_rate_index: i32,
    ) {
        let server = server.to_string();
        let name = name.to_string();
        self.open_remote("Could not create project", move || {
            let (canvas, fps) =
                launcher::project_settings_from_values(width, height, frame_rate_index)?;
            launcher::create_remote_project(&server, &name, canvas, fps, &[])
        });
    }

    fn open_remote(
        self: Pin<&mut Self>,
        heading: &'static str,
        operation: impl FnOnce() -> Result<PathBuf, String> + Send + 'static,
    ) {
        let qt_thread = self.qt_thread();
        std::thread::spawn(move || {
            let result = operation();
            let _ = qt_thread.queue(move |backend| match result {
                Ok(path) => backend.start_editor(launcher::launch_qt_editor(&path)),
                Err(error) => backend.emit_error(heading, &error),
            });
        });
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
        self.as_mut().rust_mut().core.set_search(&query.to_string());
        self.update_projects();
    }

    pub fn open_listed_project(self: Pin<&mut Self>, index: i32) {
        if let Some(project) = index_of(index)
            .and_then(|index| self.core.project(index))
            .cloned()
        {
            self.open_remote("Could not open project", move || project.open(None));
        }
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
        let selected = shrimply_application_qt::open_file_dialog(
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
        shrimply_application_qt::save_file_dialog(
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

    fn reload_projects(mut self: Pin<&mut Self>) {
        let load = self.as_mut().rust_mut().core.begin_project_load();
        let qt_thread = self.qt_thread();
        std::thread::spawn(move || {
            let projects = launcher::load_projects();
            let _ = qt_thread.queue(move |mut backend| {
                let result = backend
                    .as_mut()
                    .rust_mut()
                    .core
                    .finish_project_load(load, projects);
                match result {
                    Ok(true) => backend.update_projects(),
                    Ok(false) => (),
                    Err(error) => backend.emit_error("Could not load projects", &error),
                }
            });
        });
    }

    fn update_projects(self: Pin<&mut Self>) {
        let projects = (0..self.core.project_count())
            .filter_map(|index| self.core.project(index))
            .map(|project| {
                QString::from(match &project.remote {
                    Some(remote) => format!("{}\n{}", project.name, remote.server),
                    None => project.name.clone(),
                })
            })
            .collect();
        self.set_projects(projects);
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
