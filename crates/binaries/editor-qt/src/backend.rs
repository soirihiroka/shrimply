use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList, QUrl};
use shrimply_components_qt::about;
use shrimply_cross_ui_core::editor::project_menu::{self, Action as ProjectAction};
use shrimply_cross_ui_core::editor::{EditorSession, LoadEvent, ProjectLoader, VersionChoice};
use shrimply_editor_state::player_state;
use shrimply_inspector_document::preferences::Preferences;
use shrimply_math_core::{Fraction, frame_count, frame_index, time_from_signed_frame};
use shrimply_preview_render_core::playback::toolbar as preview_toolbar;
use shrimply_project_document::project::{self, CanvasSize};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
#[cfg(windows)]
use std::sync::mpsc::TryRecvError;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("gpu_surface.h");
        #[namespace = "shrimply"]
        fn force_opengl();
        #[namespace = "shrimply"]
        fn fixed_font_family() -> QString;
        #[namespace = "shrimply"]
        fn register_gpu_surfaces();

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
        #[qproperty(bool, ready)]
        #[qproperty(QString, loading_text, cxx_name = "loadingText")]
        #[qproperty(QString, project_title, cxx_name = "projectTitle")]
        #[qproperty(bool, playing)]
        #[qproperty(bool, preview_controls_visible, cxx_name = "previewControlsVisible")]
        #[qproperty(bool, inspector_visible, cxx_name = "inspectorVisible")]
        #[qproperty(bool, timeline_visible, cxx_name = "timelineVisible")]
        #[qproperty(f32, inspector_width, cxx_name = "inspectorWidth")]
        #[qproperty(f32, timeline_fraction, cxx_name = "timelineFraction")]
        #[qproperty(i64, position_frame, cxx_name = "positionFrame")]
        #[qproperty(i64, duration_frame, cxx_name = "durationFrame")]
        #[qproperty(QString, time_label, cxx_name = "timeLabel")]
        #[qproperty(QString, frame_rate_label, cxx_name = "frameRateLabel")]
        #[qproperty(QString, playback_speed_label, cxx_name = "playbackSpeedLabel")]
        #[qproperty(QString, fixed_font_family, cxx_name = "fixedFontFamily")]
        type EditorBackend = super::EditorBackendRust;

        #[qinvokable]
        #[cxx_name = "toggleInspector"]
        fn toggle_inspector(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "toggleTimeline"]
        fn toggle_timeline(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "saveInspectorWidth"]
        fn save_inspector_width(self: Pin<&mut EditorBackend>, width: f32);
        #[qinvokable]
        #[cxx_name = "saveTimelineFraction"]
        fn save_timeline_fraction(self: Pin<&mut EditorBackend>, leading: f32, trailing: f32);
        #[qinvokable]
        #[cxx_name = "previewContextLabels"]
        fn preview_context_labels(self: &EditorBackend) -> QStringList;
        #[qinvokable]
        #[cxx_name = "previewContextChecked"]
        fn preview_context_checked(self: &EditorBackend, index: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "activatePreviewContext"]
        fn activate_preview_context(self: Pin<&mut EditorBackend>, index: i32) -> i32;
        #[qinvokable]
        #[cxx_name = "showPreviewImageSaveDialog"]
        fn show_preview_image_save_dialog(self: Pin<&mut EditorBackend>) -> QUrl;
        #[qinvokable]
        #[cxx_name = "reportPreviewImageError"]
        fn report_preview_image_error(self: Pin<&mut EditorBackend>, error: &QString);
        #[qinvokable]
        #[cxx_name = "previewContextBottomSheet"]
        fn preview_context_bottom_sheet(self: &EditorBackend, touch: bool) -> bool;
        #[qinvokable]
        fn begin(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        fn poll(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "confirmTrust"]
        fn confirm_trust(self: Pin<&mut EditorBackend>, choice: i32);
        #[qinvokable]
        #[cxx_name = "confirmKdenlive"]
        fn confirm_kdenlive(self: Pin<&mut EditorBackend>, convert: bool);
        #[qinvokable]
        #[cxx_name = "chooseOtio"]
        fn choose_otio(
            self: Pin<&mut EditorBackend>,
            accepted: bool,
            width: i32,
            height: i32,
            fps_numerator: i32,
            fps_denominator: i32,
        );
        #[qinvokable]
        #[cxx_name = "confirmRepair"]
        fn confirm_repair(self: Pin<&mut EditorBackend>, repair: bool);
        #[qinvokable]
        #[cxx_name = "confirmVersion"]
        fn confirm_version(self: Pin<&mut EditorBackend>, choice: i32);
        #[qinvokable]
        #[cxx_name = "chooseDestination"]
        fn choose_destination(self: Pin<&mut EditorBackend>, accepted: bool, url: &QUrl);
        #[qinvokable]
        #[cxx_name = "acknowledgeWarnings"]
        fn acknowledge_warnings(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "resolveLock"]
        #[qinvokable]
        #[cxx_name = "togglePlaying"]
        fn toggle_playing(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "stepFrame"]
        fn step_frame(self: Pin<&mut EditorBackend>, delta: i32);
        #[qinvokable]
        #[cxx_name = "seekFrame"]
        fn seek_frame(self: Pin<&mut EditorBackend>, frame: i64);
        #[qinvokable]
        #[cxx_name = "projectMenu"]
        fn project_menu(self: &EditorBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "activateProjectMenu"]
        fn activate_project_menu(self: Pin<&mut EditorBackend>, id: &QString);
        #[qinvokable]
        fn save(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "showSaveAsDialog"]
        fn show_save_as_dialog(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "showOpenFileDialog"]
        fn show_open_file_dialog(
            self: Pin<&mut EditorBackend>,
            initial_path: &QString,
            title: &QString,
            filter: &QString,
        ) -> QUrl;
        #[qinvokable]
        #[cxx_name = "showFileSaveDialog"]
        fn show_file_save_dialog(
            self: Pin<&mut EditorBackend>,
            suggested_path: &QString,
            title: &QString,
            filter: &QString,
            default_suffix: &QString,
        ) -> QUrl;
        #[qinvokable]
        fn undo(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        fn redo(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "splitPaneExtent"]
        fn split_pane_extent(self: &EditorBackend, total: f32, divider: f32, fraction: f32) -> f32;
        #[qinvokable]
        #[cxx_name = "inspectorOverlay"]
        fn inspector_overlay(self: Pin<&mut EditorBackend>, width: f64) -> bool;
        #[qinvokable]
        #[cxx_name = "transportStacked"]
        fn transport_stacked(self: Pin<&mut EditorBackend>, width: f64) -> bool;
        #[qinvokable]
        #[cxx_name = "previewMinimumWidth"]
        fn preview_minimum_width(self: Pin<&mut EditorBackend>) -> f64;
        #[qinvokable]
        fn translate(self: Pin<&mut EditorBackend>, text: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "applicationVersion"]
        fn application_version(self: Pin<&mut EditorBackend>) -> QString;
        #[qinvokable]
        #[cxx_name = "creditHeadings"]
        fn credit_headings(self: &EditorBackend) -> QStringList;
        #[qinvokable]
        #[cxx_name = "creditEntries"]
        fn credit_entries(self: &EditorBackend, index: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "aboutLinkLabels"]
        fn about_link_labels(self: &EditorBackend) -> QStringList;
        #[qinvokable]
        #[cxx_name = "aboutLinkUrl"]
        fn about_link_url(self: &EditorBackend, index: i32) -> QUrl;
        #[qinvokable]
        #[cxx_name = "licenseText"]
        fn license_text(self: Pin<&mut EditorBackend>) -> QString;
        #[qinvokable]
        #[cxx_name = "settingsDocument"]
        fn settings_document(self: &EditorBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "applySettings"]
        fn apply_settings(self: &EditorBackend, command: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "settingsPath"]
        fn settings_path(self: &EditorBackend, url: &QUrl) -> QString;
        #[qinvokable]
        #[cxx_name = "revealSettingsPath"]
        fn reveal_settings_path(self: &EditorBackend, path: &QString) -> QUrl;

        #[qsignal]
        #[cxx_name = "projectPreferencesRequested"]
        fn project_preferences_requested(self: Pin<&mut EditorBackend>);
        #[qsignal]
        #[cxx_name = "projectAboutRequested"]
        fn project_about_requested(self: Pin<&mut EditorBackend>);
        #[qsignal]
        #[cxx_name = "requestTrust"]
        fn request_trust(
            self: Pin<&mut EditorBackend>,
            explanation: QString,
            details: QString,
            files: i32,
            folders: i32,
        );
        #[qsignal]
        #[cxx_name = "requestKdenlive"]
        fn request_kdenlive(self: Pin<&mut EditorBackend>);
        #[qsignal]
        #[cxx_name = "requestOtio"]
        fn request_otio(self: Pin<&mut EditorBackend>);
        #[qsignal]
        #[cxx_name = "requestRepair"]
        fn request_repair(self: Pin<&mut EditorBackend>);
        #[qsignal]
        #[cxx_name = "requestDestination"]
        fn request_destination(
            self: Pin<&mut EditorBackend>,
            title: QString,
            suggested_name: QString,
        );
        #[qsignal]
        #[cxx_name = "requestWarnings"]
        fn request_warnings(self: Pin<&mut EditorBackend>, body: QString);
        #[qsignal]
        #[cxx_name = "requestVersion"]
        fn request_version(self: Pin<&mut EditorBackend>, body: QString, match_label: QString);
        #[qsignal]
        #[cxx_name = "showError"]
        fn show_error(self: Pin<&mut EditorBackend>, heading: QString, body: QString);
        #[qsignal]
        #[cxx_name = "showPlaybackError"]
        fn show_playback_error(self: Pin<&mut EditorBackend>, body: QString);
        #[qsignal]
        fn canceled(self: Pin<&mut EditorBackend>, exit_code: i32);
    }

    impl cxx_qt::Initialize for EditorBackend {}
    impl cxx_qt::Threading for EditorBackend {}
}

pub struct EditorBackendRust {
    ready: bool,
    loading_text: QString,
    project_title: QString,
    playing: bool,
    preview_controls_visible: bool,
    inspector_visible: bool,
    timeline_visible: bool,
    inspector_width: f32,
    timeline_fraction: f32,
    position_frame: i64,
    duration_frame: i64,
    time_label: QString,
    frame_rate_label: QString,
    playback_speed_label: QString,
    fixed_font_family: QString,
    trust_requests: Receiver<shrimply_trust_core::Request>,
    pending_trust: Option<shrimply_trust_core::Request>,
    trust_open: bool,
    #[cfg(windows)]
    cuda_preflight: Option<(PathBuf, Receiver<Result<(), String>>)>,
    loader: Option<ProjectLoader>,
    session: Option<Pin<Box<EditorSession>>>,
    settings: RefCell<Preferences>,
    launcher_preferences:
        std::cell::OnceCell<shrimply_editor_state::preferences::SharedPreferences>,
}

impl cxx_qt::Initialize for qobject::EditorBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl Default for EditorBackendRust {
    fn default() -> Self {
        let layout = shrimply_editor_state::preferences::EditorLayout::default();
        Self {
            ready: false,
            loading_text: QString::from(if cfg!(windows) {
                "Compiling CUDA kernels…"
            } else {
                "Loading project…"
            }),
            project_title: QString::from("Shrimply"),
            playing: false,
            preview_controls_visible: true,
            inspector_visible: layout.inspector_visible,
            timeline_visible: layout.timeline_visible,
            inspector_width: layout.inspector_width,
            timeline_fraction: layout.timeline_fraction,
            position_frame: 0,
            duration_frame: 0,
            time_label: QString::default(),
            frame_rate_label: QString::from("--"),
            playback_speed_label: QString::from("x1"),
            fixed_font_family: qobject::fixed_font_family(),
            trust_requests: shrimply_trust_core::interactive_requests(),
            pending_trust: None,
            trust_open: false,
            #[cfg(windows)]
            cuda_preflight: None,
            loader: None,
            session: None,
            settings: RefCell::default(),
            launcher_preferences: std::cell::OnceCell::new(),
        }
    }
}

impl qobject::EditorBackend {
    pub fn begin(self: Pin<&mut Self>) {
        assert!(
            self.loader.is_none(),
            "Qt editor project loader already started"
        );
        let path = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .expect("Shrimply editor requires a project path");
        #[cfg(windows)]
        {
            let mut this = self;
            assert!(
                this.cuda_preflight.is_none(),
                "Qt editor CUDA preflight already started"
            );
            this.as_mut()
                .set_loading_text(QString::from("Compiling CUDA kernels…"));
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(shrimply_visual_cuda::gpu::preflight());
            });
            this.as_mut().rust_mut().get_mut().cuda_preflight = Some((path, receiver));
        }
        #[cfg(not(windows))]
        self.start_project_loader(path);
    }

    fn start_project_loader(mut self: Pin<&mut Self>, path: PathBuf) {
        let mut loader = ProjectLoader::new(path);
        let event = loader.begin();
        self.as_mut().rust_mut().get_mut().loader = Some(loader);
        self.handle_event(event);
    }

    pub fn poll(mut self: Pin<&mut Self>) {
        #[cfg(windows)]
        self.as_mut().poll_cuda_preflight();
        for error in shrimply_trust_core::poll_edits() {
            self.as_mut().emit_error("Could not change source", &error);
        }
        if !self.trust_open
            && let Ok(request) = self.trust_requests.try_recv()
        {
            let review = request.review.clone();
            self.as_mut().rust_mut().get_mut().pending_trust = Some(request);
            self.as_mut().show_trust(&review);
        }

        let session_update = self
            .as_ref()
            .rust()
            .session
            .as_deref()
            .map(EditorSession::poll)
            .unwrap_or_default();
        if let Some(error) = session_update.audio_playback_stopped {
            self.as_mut().show_playback_error(QString::from(error));
        }
        if let Some(notice) = session_update.version_notice {
            self.as_mut().show_error(
                shrimply_i18n_qt::text("Project Server Changed"),
                QString::from(notice),
            );
        }
        if let Some(title) = session_update.title {
            self.as_mut()
                .set_project_title(QString::from(title.text.as_str()));
        }
        let event = self
            .as_mut()
            .rust_mut()
            .get_mut()
            .loader
            .as_mut()
            .and_then(ProjectLoader::poll);
        if let Some(event) = event {
            self.as_mut().handle_event(event);
        }
        self.as_mut().update_player_properties();
    }

    #[cfg(windows)]
    fn poll_cuda_preflight(mut self: Pin<&mut Self>) {
        let result = self
            .as_ref()
            .rust()
            .cuda_preflight
            .as_ref()
            .map(|(_, receiver)| receiver.try_recv());
        let result = match result {
            Some(Ok(result)) => result,
            Some(Err(TryRecvError::Disconnected)) => {
                Err("CUDA kernel preflight worker stopped unexpectedly".into())
            }
            Some(Err(TryRecvError::Empty)) | None => return,
        };
        let (path, _) = self
            .as_mut()
            .rust_mut()
            .get_mut()
            .cuda_preflight
            .take()
            .expect("CUDA preflight exists");
        match result {
            Ok(()) => {
                self.as_mut()
                    .set_loading_text(QString::from("Loading project…"));
                self.start_project_loader(path);
            }
            Err(error) => self
                .as_mut()
                .emit_error("Could not initialize CUDA", &error),
        }
    }

    fn show_trust(mut self: Pin<&mut Self>, review: &shrimply_trust_core::Review) {
        self.as_mut().rust_mut().get_mut().trust_open = true;
        self.request_trust(
            QString::from(shrimply_trust_core::EXPLANATION),
            QString::from(review.details()),
            i32::try_from(review.files.len()).expect("too many executable files"),
            i32::try_from(review.folders.len()).expect("too many executable folders"),
        );
    }

    pub fn confirm_trust(mut self: Pin<&mut Self>, choice: i32) {
        let kind = match choice {
            1 => Some(shrimply_trust_core::Kind::File),
            2 => Some(shrimply_trust_core::Kind::Folder),
            _ => None,
        };
        self.as_mut().rust_mut().get_mut().trust_open = false;
        if let Some(request) = self.as_mut().rust_mut().get_mut().pending_trust.take() {
            let result = kind
                .ok_or_else(|| "Trust approval was canceled".to_string())
                .and_then(|kind| request.review.approve(kind));
            let _ = request.response.send(result);
        } else {
            let event = self.as_mut().loader_mut().confirm_trust(kind);
            self.handle_event(event);
        }
    }

    pub fn confirm_kdenlive(mut self: Pin<&mut Self>, convert: bool) {
        let event = self.as_mut().loader_mut().confirm_kdenlive(convert);
        self.as_mut().handle_event(event);
    }

    pub fn choose_otio(
        mut self: Pin<&mut Self>,
        accepted: bool,
        width: i32,
        height: i32,
        fps_numerator: i32,
        fps_denominator: i32,
    ) {
        let settings = accepted.then(|| {
            let width = u32::try_from(width).expect("OTIO width must be positive");
            let height = u32::try_from(height).expect("OTIO height must be positive");
            assert!(
                fps_numerator > 0 && fps_denominator > 0,
                "invalid OTIO frame rate"
            );
            (
                CanvasSize { width, height },
                Fraction::new(fps_numerator as u64, fps_denominator as u64),
            )
        });
        let event = self.as_mut().loader_mut().choose_otio_settings(settings);
        self.as_mut().handle_event(event);
    }

    pub fn confirm_repair(mut self: Pin<&mut Self>, repair: bool) {
        let event = self.as_mut().loader_mut().confirm_frame_grid_repair(repair);
        self.as_mut().handle_event(event);
    }

    pub fn confirm_version(mut self: Pin<&mut Self>, choice: i32) {
        let choice = match choice {
            1 => VersionChoice::Force,
            2 => VersionChoice::Match,
            _ => VersionChoice::Close,
        };
        let event = self.as_mut().loader_mut().confirm_version(choice);
        self.handle_event(event);
    }

    pub fn choose_destination(mut self: Pin<&mut Self>, accepted: bool, url: &QUrl) {
        let destination = accepted.then(|| local_path(url)).flatten();
        let event = self.as_mut().loader_mut().choose_destination(destination);
        self.as_mut().handle_event(event);
    }

    pub fn acknowledge_warnings(mut self: Pin<&mut Self>) {
        let event = self.as_mut().loader_mut().acknowledge_warnings();
        self.as_mut().handle_event(event);
    }

    pub fn toggle_playing(self: Pin<&mut Self>) {
        if let Some(session) = self.session.as_deref() {
            player_state::toggle_playing(&session.player_state);
        }
    }

    pub fn step_frame(self: Pin<&mut Self>, delta: i32) {
        let Some(session) = self.session.as_deref() else {
            return;
        };
        let snapshot = player_state::snapshot(&session.player_state);
        let current = frame_index(snapshot.position, snapshot.frame_rate).unwrap_or(0);
        let target = current.saturating_add(i64::from(delta)).max(0);
        if let Some(position) = time_from_signed_frame(target, snapshot.frame_rate) {
            shrimply_preview_qt::mark_preview_step(delta);
            player_state::seek_time(&session.player_state, position);
        }
    }

    pub fn seek_frame(self: Pin<&mut Self>, frame: i64) {
        let Some(session) = self.session.as_deref() else {
            return;
        };
        let fps = player_state::snapshot(&session.player_state).frame_rate;
        if let Some(position) = time_from_signed_frame(frame.max(0), fps) {
            player_state::seek_time(&session.player_state, position);
        }
    }

    pub fn project_menu(&self) -> QString {
        let document = project_menu::document(
            |action| Self::project_host_action(action).is_some(),
            project::can_undo(),
            project::can_redo(),
        );
        QString::from(document.to_string().as_str())
    }

    pub fn activate_project_menu(self: Pin<&mut Self>, id: &QString) {
        let Some(action) = ProjectAction::from_id(&id.to_string()) else {
            return;
        };
        self.run_project_menu(action);
    }

    fn project_host_action(action: ProjectAction) -> Option<fn(Pin<&mut Self>)> {
        Some(match action {
            ProjectAction::SaveAs => Self::show_save_as_dialog,
            ProjectAction::Preferences => Self::project_preferences_requested,
            ProjectAction::RestartService => Self::restart_local_service,
            ProjectAction::About => Self::project_about_requested,
            _ => return None,
        })
    }

    fn run_project_menu(self: Pin<&mut Self>, action: ProjectAction) {
        let Some(session) = self.session.as_deref() else {
            return;
        };
        match session.activate_project_menu(action) {
            Ok(Some(action)) => {
                if let Some(activate) = Self::project_host_action(action) {
                    activate(self);
                }
            }
            Ok(None) => {}
            Err(error) => self.emit_error(action.label(), &error),
        }
    }

    fn restart_local_service(self: Pin<&mut Self>) {
        let path = project::active_project_path();
        let qt_thread = self.qt_thread();
        std::thread::spawn(move || {
            if let Err(error) = shrimply_cross_ui_core::launcher::restart_local_service(&path) {
                let _ = qt_thread.queue(move |backend| {
                    backend.emit_error("Could not restart the project service", &error)
                });
            }
        });
    }

    pub fn save(self: Pin<&mut Self>) {
        self.run_project_menu(ProjectAction::Save);
    }

    pub fn show_save_as_dialog(mut self: Pin<&mut Self>) {
        let path = shrimply_cross_ui_core::editor::suggested_save_as_path();
        let suggested_url = QUrl::from_local_file(&QString::from(path.to_string_lossy().as_ref()));
        tracing::debug!(suggested = %path.display(), "opening Qt Save As dialog");
        let url = shrimply_application_qt::save_file_dialog(
            &suggested_url,
            &shrimply_i18n_qt::text("Save Project As"),
            &shrimply_i18n_qt::text("Shrimply projects (*.shrimp)"),
            &QString::from("shrimp"),
        );
        if url.is_empty() {
            tracing::debug!("Qt Save As dialog canceled");
            return;
        }
        self.as_mut().save_as_url(&url);
    }

    pub fn show_open_file_dialog(
        self: Pin<&mut Self>,
        initial_path: &QString,
        title: &QString,
        filter: &QString,
    ) -> QUrl {
        shrimply_application_qt::open_file_dialog(
            &QUrl::from_local_file(initial_path),
            title,
            filter,
        )
    }

    pub fn show_file_save_dialog(
        self: Pin<&mut Self>,
        suggested_path: &QString,
        title: &QString,
        filter: &QString,
        default_suffix: &QString,
    ) -> QUrl {
        shrimply_application_qt::save_file_dialog(
            &QUrl::from_local_file(suggested_path),
            title,
            filter,
            default_suffix,
        )
    }

    fn save_as_url(self: Pin<&mut Self>, url: &QUrl) {
        let Some(path) = local_path(url) else {
            tracing::error!("Qt Save As returned a non-local destination");
            self.emit_error(
                "Could not save project",
                "The selected location does not have a local path.",
            );
            return;
        };
        tracing::debug!(path = %path.display(), "Qt Save As destination accepted");
        let Some(session) = self.session.as_deref() else {
            return;
        };
        if let Err(error) = session.save_as(path) {
            tracing::error!(%error, "Qt Save As failed");
            self.emit_error("Could not save project", &error);
        }
    }

    pub fn undo(self: Pin<&mut Self>) {
        self.run_project_menu(ProjectAction::Undo);
    }

    pub fn redo(self: Pin<&mut Self>) {
        self.run_project_menu(ProjectAction::Redo);
    }

    pub fn inspector_overlay(self: Pin<&mut Self>, width: f64) -> bool {
        shrimply_components_qt::layout::inspector_overlay(width as f32)
    }

    pub fn transport_stacked(self: Pin<&mut Self>, width: f64) -> bool {
        shrimply_components_qt::layout::transport_stacked(width as f32)
    }

    pub fn preview_minimum_width(self: Pin<&mut Self>) -> f64 {
        f64::from(shrimply_components_qt::layout::MIN_PREVIEW_WIDTH)
    }

    pub fn translate(self: Pin<&mut Self>, text: &QString) -> QString {
        shrimply_i18n_qt::text(&text.to_string())
    }

    pub fn application_version(self: Pin<&mut Self>) -> QString {
        QString::from(env!("CARGO_PKG_VERSION"))
    }

    pub fn credit_headings(&self) -> QStringList {
        about::CREDIT_SECTIONS
            .iter()
            .map(|(heading, _)| shrimply_i18n_qt::text(heading))
            .collect()
    }

    pub fn credit_entries(&self, index: i32) -> QString {
        about::CREDIT_SECTIONS
            .get(index as usize)
            .map(|(_, entries)| QString::from(&entries.join("\n")))
            .unwrap_or_default()
    }

    pub fn about_link_labels(&self) -> QStringList {
        about::LINKS
            .iter()
            .map(|(label, _)| shrimply_i18n_qt::text(label))
            .collect()
    }

    pub fn about_link_url(&self, index: i32) -> QUrl {
        about::LINKS
            .get(index as usize)
            .map(|&(_, url)| QUrl::from(url))
            .unwrap_or_default()
    }

    pub fn license_text(self: Pin<&mut Self>) -> QString {
        QString::from(include_str!("../../../../LICENSE"))
    }

    pub fn settings_document(&self) -> QString {
        QString::from(
            self.settings
                .borrow_mut()
                .snapshot(self.settings_store())
                .to_string(),
        )
    }

    pub fn apply_settings(&self, command: &QString) -> QString {
        let mut settings = self.settings.borrow_mut();
        let result = serde_json::from_str(&command.to_string())
            .map_err(|error| error.to_string())
            .and_then(|command| settings.apply(self.settings_store(), &command));
        let document = match result {
            Ok(document) => document,
            Err(error) => serde_json::json!({"error":error}),
        };
        QString::from(document.to_string())
    }

    pub fn settings_path(&self, url: &QUrl) -> QString {
        url.to_local_file_or_default()
    }

    pub fn reveal_settings_path(&self, path: &QString) -> QUrl {
        use shrimply_cross_ui_core::desktop_open::{Action, prepare};
        match prepare(&PathBuf::from(path.to_string()), None) {
            Ok(Action::Open(target) | Action::FocusRevealed(target)) => {
                QUrl::from_local_file(&QString::from(target.to_string_lossy().as_ref()))
            }
            Err(_) => QUrl::default(),
        }
    }

    fn settings_store(&self) -> &shrimply_editor_state::preferences::SharedPreferences {
        self.session.as_deref().map_or_else(
            || {
                self.launcher_preferences
                    .get_or_init(shrimply_editor_state::preferences::open_with_defaults)
            },
            |session| &session.preferences,
        )
    }

    fn loader_mut(self: Pin<&mut Self>) -> &mut ProjectLoader {
        self.rust_mut()
            .get_mut()
            .loader
            .as_mut()
            .expect("Qt editor project loader is not active")
    }

    fn handle_event(mut self: Pin<&mut Self>, event: LoadEvent) {
        match event {
            LoadEvent::ConfirmVersion(prompt) => {
                let label = prompt
                    .action
                    .as_ref()
                    .map_or_else(QString::default, |action| {
                        shrimply_i18n_qt::text(action.label())
                    });
                self.as_mut()
                    .request_version(QString::from(prompt.to_string()), label);
            }
            LoadEvent::ConfirmTrust(review) => self.as_mut().show_trust(&review),
            LoadEvent::ConfirmKdenlive => self.as_mut().request_kdenlive(),
            LoadEvent::ChooseOtioSettings => self.as_mut().request_otio(),
            LoadEvent::Progress(text) => self.set_loading_text(QString::from(text)),
            LoadEvent::ConfirmFrameGridRepair => self.as_mut().request_repair(),
            LoadEvent::ChooseDestination {
                title,
                suggested_name,
            } => self
                .as_mut()
                .request_destination(shrimply_i18n_qt::text(title), QString::from(suggested_name)),
            LoadEvent::ImportWarnings(warnings) => self
                .as_mut()
                .request_warnings(QString::from(warnings.join("\n"))),
            LoadEvent::Ready {
                project, forced, ..
            } => {
                ffmpeg_next::init().expect("FFmpeg should initialize");
                ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Error);
                let session = EditorSession::new(*project, &forced)
                    .unwrap_or_else(|error| panic!("could not initialize Qt editor: {error}"));
                shrimply_preview_qt::install(&session).unwrap_or_else(|error| {
                    panic!("could not initialize Qt GPU surfaces: {error}")
                });
                shrimply_inspector_qt::install(&session);
                shrimply_export_qt::install(&session);
                self.as_mut().rust_mut().get_mut().session = Some(Box::pin(session));
                self.as_mut().set_ready(true);
                self.as_mut().update_player_properties();
            }
            LoadEvent::Error { heading, body } => self.emit_error(heading, &body),
            LoadEvent::HandedOff => self.as_mut().canceled(0),
            LoadEvent::Canceled => self.as_mut().canceled(i32::from(
                shrimply_cross_ui_core::launcher::EDITOR_OPEN_CANCELED_EXIT_CODE,
            )),
        }
    }

    pub fn preview_context_labels(&self) -> QStringList {
        self.session
            .as_deref()
            .map(|session| {
                preview_toolbar::context_menu(&session.preferences, true)
                    .into_iter()
                    .map(|item| QString::from(item.label))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn preview_context_checked(&self, index: i32) -> i32 {
        self.session
            .as_deref()
            .and_then(|session| {
                preview_toolbar::context_menu(&session.preferences, true)
                    .get(usize::try_from(index).ok()?)
                    .and_then(|item| item.checked)
            })
            .map_or(-1, i32::from)
    }

    pub fn activate_preview_context(mut self: Pin<&mut Self>, index: i32) -> i32 {
        let request = self.session.as_deref().and_then(|session| {
            preview_toolbar::context_menu(&session.preferences, true)
                .get(usize::try_from(index).expect("preview menu index is nonnegative"))
                .and_then(|item| item.action.activate(&session.preferences))
        });
        self.as_mut().update_player_properties();
        match request {
            Some(preview_toolbar::ContextRequest::CopyImage) => 1,
            Some(preview_toolbar::ContextRequest::SaveImage) => 2,
            None => 0,
        }
    }

    pub fn show_preview_image_save_dialog(self: Pin<&mut Self>) -> QUrl {
        let session = self
            .session
            .as_deref()
            .expect("preview requires an editor session");
        let title = preview_toolbar::context_menu(&session.preferences, true)
            .into_iter()
            .find(|item| item.action == preview_toolbar::ContextAction::SaveImage)
            .expect("preview save service has a shared title")
            .label;
        let path = shrimply_editor_state::preferences::preview_image_folder(&session.preferences)
            .unwrap_or_default()
            .join("preview.png");
        let preferences = session.preferences.clone();
        let destination = self.show_file_save_dialog(
            &QString::from(path.to_string_lossy().as_ref()),
            &QString::from(title),
            &shrimply_i18n_qt::text("PNG image (*.png)"),
            &QString::from("png"),
        );
        if let Some(path) = local_path(&destination)
            && let Some(folder) = path.parent()
        {
            shrimply_editor_state::preferences::set_preview_image_folder(&preferences, folder);
        }
        destination
    }

    pub fn report_preview_image_error(self: Pin<&mut Self>, error: &QString) {
        let error = error.to_string();
        self.emit_error(
            "Could not save preview image",
            if error.is_empty() {
                "Could not capture the preview image."
            } else {
                &error
            },
        );
    }

    pub fn preview_context_bottom_sheet(&self, touch: bool) -> bool {
        shrimply_components_qt::layout::menu_uses_bottom_sheet(if touch {
            "touch"
        } else {
            "pointer"
        })
    }

    fn toggle_inspector(self: Pin<&mut Self>) {
        self.update_editor_layout(|layout| layout.inspector_visible = !layout.inspector_visible);
    }

    fn toggle_timeline(self: Pin<&mut Self>) {
        self.update_editor_layout(|layout| layout.timeline_visible = !layout.timeline_visible);
    }

    fn save_inspector_width(self: Pin<&mut Self>, width: f32) {
        self.update_editor_layout(|layout| layout.inspector_width = width);
    }

    fn save_timeline_fraction(self: Pin<&mut Self>, leading: f32, trailing: f32) {
        if let Some(fraction) = shrimply_components_qt::math::split_pane_fraction(leading, trailing)
        {
            self.update_editor_layout(|layout| layout.timeline_fraction = fraction);
        }
    }

    fn split_pane_extent(&self, total: f32, divider: f32, fraction: f32) -> f32 {
        shrimply_components_qt::math::split_pane_extent(total, divider, fraction)
    }

    fn update_editor_layout(
        mut self: Pin<&mut Self>,
        update: impl FnOnce(&mut shrimply_editor_state::preferences::EditorLayout),
    ) {
        let this = self.as_ref();
        let Some(session) = this.rust().session.as_deref() else {
            return;
        };
        let result =
            shrimply_editor_state::preferences::update_editor_layout(&session.preferences, update);
        if let Err(error) = result {
            self.as_mut()
                .emit_error("Could not save preferences", &error);
        }
        self.update_player_properties();
    }

    fn update_player_properties(mut self: Pin<&mut Self>) {
        let this = self.as_ref();
        let Some(session) = this.rust().session.as_deref() else {
            return;
        };
        let snapshot = player_state::snapshot(&session.player_state);
        let playing = snapshot.playing;
        let preferences = shrimply_editor_state::preferences::snapshot(&session.preferences);
        let preview_controls_visible = preferences.preview_controls_visible;
        let layout = preferences.editor_layout;
        let position_frame = frame_index(snapshot.position, snapshot.frame_rate).unwrap_or(0);
        let duration_frame = frame_count(snapshot.duration, snapshot.frame_rate)
            .and_then(|frame| i64::try_from(frame).ok())
            .unwrap_or(i64::MAX);
        let time_label = QString::from(shrimply_preview_runtime_cuda::playback_time_label(
            snapshot.position,
            snapshot.duration,
        ));
        let frame_rate_label = QString::from(shrimply_preview_qt::preview_frame_rate_label());
        let playback_speed_label = QString::from(
            shrimply_preview_runtime_cuda::playback_speed_label(snapshot.playback_speed),
        );
        self.as_mut().set_playing(playing);
        self.as_mut()
            .set_preview_controls_visible(preview_controls_visible);
        self.as_mut()
            .set_inspector_visible(layout.inspector_visible);
        self.as_mut().set_timeline_visible(layout.timeline_visible);
        self.as_mut().set_inspector_width(layout.inspector_width);
        self.as_mut()
            .set_timeline_fraction(layout.timeline_fraction);
        self.as_mut().set_position_frame(position_frame);
        self.as_mut().set_duration_frame(duration_frame);
        self.as_mut().set_time_label(time_label);
        self.as_mut().set_frame_rate_label(frame_rate_label);
        self.as_mut().set_playback_speed_label(playback_speed_label);
    }

    fn emit_error(mut self: Pin<&mut Self>, heading: &str, body: &str) {
        self.as_mut()
            .show_error(shrimply_i18n_qt::text(heading), QString::from(body));
    }
}

fn local_path(url: &QUrl) -> Option<PathBuf> {
    url.is_local_file()
        .then(|| PathBuf::from(url.to_local_file_or_default().to_string()))
}
