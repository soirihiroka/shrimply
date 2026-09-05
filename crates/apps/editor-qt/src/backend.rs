use core::pin::Pin;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QUrl};
use shrimply_cross_ui_core::editor::{EditorSession, LoadEvent, ProjectLoader};
use shrimply_math_core::{Fraction, frame_count, frame_index, time_from_signed_frame};
use shrimply_project::project::{self, CanvasSize};
use shrimply_state::player_state;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

const GIB_BYTES: f64 = 1024.0 * 1024.0 * 1024.0;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("gpu_surface.h");
        #[namespace = "shrimply"]
        fn force_opengl();
        #[namespace = "shrimply"]
        fn configure_icons();
        #[namespace = "shrimply"]
        fn fixed_font_family() -> QString;
        #[namespace = "shrimply"]
        fn register_gpu_surfaces();

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
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
        #[qproperty(i64, position_frame, cxx_name = "positionFrame")]
        #[qproperty(i64, duration_frame, cxx_name = "durationFrame")]
        #[qproperty(QString, time_label, cxx_name = "timeLabel")]
        #[qproperty(QString, frame_rate_label, cxx_name = "frameRateLabel")]
        #[qproperty(QString, playback_speed_label, cxx_name = "playbackSpeedLabel")]
        #[qproperty(QString, fixed_font_family, cxx_name = "fixedFontFamily")]
        type EditorBackend = super::EditorBackendRust;

        #[qinvokable]
        fn begin(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        fn poll(self: Pin<&mut EditorBackend>);
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
        #[cxx_name = "chooseDestination"]
        fn choose_destination(self: Pin<&mut EditorBackend>, accepted: bool, url: &QUrl);
        #[qinvokable]
        #[cxx_name = "acknowledgeWarnings"]
        fn acknowledge_warnings(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "resolveLock"]
        fn resolve_lock(self: Pin<&mut EditorBackend>, action: i32);
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
        fn translate(self: Pin<&mut EditorBackend>, text: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "applicationVersion"]
        fn application_version(self: Pin<&mut EditorBackend>) -> QString;
        #[qinvokable]
        #[cxx_name = "licenseText"]
        fn license_text(self: Pin<&mut EditorBackend>) -> QString;
        #[qinvokable]
        #[cxx_name = "preferenceValue"]
        fn preference_value(self: Pin<&mut EditorBackend>, key: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "preferenceMinimum"]
        fn preference_minimum(self: Pin<&mut EditorBackend>, key: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "preferenceMaximum"]
        fn preference_maximum(self: Pin<&mut EditorBackend>, key: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "preferenceStep"]
        fn preference_step(self: Pin<&mut EditorBackend>, key: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "preferenceScale"]
        fn preference_scale(self: Pin<&mut EditorBackend>, key: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "setPreferenceValue"]
        fn set_preference_value(self: Pin<&mut EditorBackend>, key: &QString, value: &QString);
        #[qinvokable]
        #[cxx_name = "preferenceServerCount"]
        fn preference_server_count(self: Pin<&mut EditorBackend>) -> i32;
        #[qinvokable]
        #[cxx_name = "preferenceServerUrl"]
        fn preference_server_url(self: Pin<&mut EditorBackend>, index: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "preferenceSelectedServer"]
        fn preference_selected_server(self: Pin<&mut EditorBackend>) -> i32;
        #[qinvokable]
        #[cxx_name = "addPreferenceServer"]
        fn add_preference_server(self: Pin<&mut EditorBackend>, value: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "editPreferenceServer"]
        fn edit_preference_server(
            self: Pin<&mut EditorBackend>,
            index: i32,
            value: &QString,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "removePreferenceServer"]
        fn remove_preference_server(self: Pin<&mut EditorBackend>, index: i32);
        #[qinvokable]
        #[cxx_name = "selectPreferenceServer"]
        fn select_preference_server(self: Pin<&mut EditorBackend>, index: i32);
        #[qinvokable]
        #[cxx_name = "choosePreferenceBlenderBinary"]
        fn choose_preference_blender_binary(self: Pin<&mut EditorBackend>) -> bool;
        #[qinvokable]
        #[cxx_name = "clearPreferenceBlenderBinary"]
        fn clear_preference_blender_binary(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "refreshPreferenceServerStatus"]
        fn refresh_preference_server_status(self: Pin<&mut EditorBackend>);
        #[qinvokable]
        #[cxx_name = "preferenceServerDetail"]
        fn preference_server_detail(self: Pin<&mut EditorBackend>, key: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "preferenceServerDeviceCount"]
        fn preference_server_device_count(self: Pin<&mut EditorBackend>) -> i32;
        #[qinvokable]
        #[cxx_name = "preferenceServerDeviceLabel"]
        fn preference_server_device_label(self: Pin<&mut EditorBackend>, index: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "preferenceServerSelectedDevice"]
        fn preference_server_selected_device(self: Pin<&mut EditorBackend>) -> i32;
        #[qinvokable]
        #[cxx_name = "selectPreferenceServerDevice"]
        fn select_preference_server_device(self: Pin<&mut EditorBackend>, index: i32);

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
        #[cxx_name = "requestLock"]
        fn request_lock(self: Pin<&mut EditorBackend>, pid: i64);
        #[qsignal]
        #[cxx_name = "showError"]
        fn show_error(self: Pin<&mut EditorBackend>, heading: QString, body: QString);
        #[qsignal]
        #[cxx_name = "showPlaybackError"]
        fn show_playback_error(self: Pin<&mut EditorBackend>, body: QString);
        #[qsignal]
        #[cxx_name = "preferenceBlenderFinished"]
        fn preference_blender_finished(self: Pin<&mut EditorBackend>, error: QString);
        #[qsignal]
        #[cxx_name = "preferenceServerStatusChanged"]
        fn preference_server_status_changed(self: Pin<&mut EditorBackend>, error: QString);
        #[qsignal]
        fn canceled(self: Pin<&mut EditorBackend>);
    }

    impl cxx_qt::Initialize for EditorBackend {}
}

pub struct EditorBackendRust {
    ready: bool,
    loading_text: QString,
    project_title: QString,
    playing: bool,
    position_frame: i64,
    duration_frame: i64,
    time_label: QString,
    frame_rate_label: QString,
    playback_speed_label: QString,
    fixed_font_family: QString,
    loader: Option<ProjectLoader>,
    session: Option<Pin<Box<EditorSession>>>,
    pending_lock_pid: Option<u32>,
    blender_probe: Option<Receiver<Result<PathBuf, String>>>,
    server_request: Option<Receiver<Result<shrimply_preferences_qt::ServerStatus, String>>>,
    server_status: Option<shrimply_preferences_qt::ServerStatus>,
}

impl cxx_qt::Initialize for qobject::EditorBackend {
    fn initialize(self: Pin<&mut Self>) {}
}

impl Default for EditorBackendRust {
    fn default() -> Self {
        Self {
            ready: false,
            loading_text: QString::from("Loading project…"),
            project_title: QString::from("Shrimply"),
            playing: false,
            position_frame: 0,
            duration_frame: 0,
            time_label: QString::default(),
            frame_rate_label: QString::from("--"),
            playback_speed_label: QString::from("x1"),
            fixed_font_family: qobject::fixed_font_family(),
            loader: None,
            session: None,
            pending_lock_pid: None,
            blender_probe: None,
            server_request: None,
            server_status: None,
        }
    }
}

impl qobject::EditorBackend {
    pub fn begin(mut self: Pin<&mut Self>) {
        assert!(
            self.loader.is_none(),
            "Qt editor project loader already started"
        );
        let path = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .expect("shrimply-editor-qt requires a project path");
        let mut loader = ProjectLoader::new(path);
        let event = loader.begin();
        self.as_mut().rust_mut().get_mut().loader = Some(loader);
        self.handle_event(event);
    }

    pub fn poll(mut self: Pin<&mut Self>) {
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
        self.as_mut().poll_preference_tasks();
        self.as_mut().update_player_properties();
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

    pub fn choose_destination(mut self: Pin<&mut Self>, accepted: bool, url: &QUrl) {
        let destination = accepted.then(|| local_path(url)).flatten();
        let event = self.as_mut().loader_mut().choose_destination(destination);
        self.as_mut().handle_event(event);
    }

    pub fn acknowledge_warnings(mut self: Pin<&mut Self>) {
        let event = self.as_mut().loader_mut().acknowledge_warnings();
        self.as_mut().handle_event(event);
    }

    pub fn resolve_lock(mut self: Pin<&mut Self>, action: i32) {
        let pid = self
            .as_mut()
            .rust_mut()
            .get_mut()
            .pending_lock_pid
            .take()
            .expect("Qt lock response without a pending lock");
        let event = match action {
            1 => self.as_mut().loader_mut().retry_locked_project(false, pid),
            2 => self.as_mut().loader_mut().retry_locked_project(true, pid),
            _ => self.as_mut().loader_mut().cancel(),
        };
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

    pub fn save(self: Pin<&mut Self>) {
        let Some(session) = self.session.as_deref() else {
            return;
        };
        if let Err(error) = session.save() {
            tracing::error!(%error, "Qt project save failed");
            self.emit_error("Could not save project", &error);
        }
    }

    pub fn show_save_as_dialog(mut self: Pin<&mut Self>) {
        let path = shrimply_cross_ui_core::editor::suggested_save_as_path();
        let suggested_url = QUrl::from_local_file(&QString::from(path.to_string_lossy().as_ref()));
        tracing::debug!(suggested = %path.display(), "opening Qt Save As dialog");
        let url = shrimply_qt_helpers::save_file_dialog(
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
        shrimply_qt_helpers::open_file_dialog(&QUrl::from_local_file(initial_path), title, filter)
    }

    pub fn show_file_save_dialog(
        self: Pin<&mut Self>,
        suggested_path: &QString,
        title: &QString,
        filter: &QString,
        default_suffix: &QString,
    ) -> QUrl {
        shrimply_qt_helpers::save_file_dialog(
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
        self.history_action(project::undo);
    }

    pub fn redo(self: Pin<&mut Self>) {
        self.history_action(project::redo);
    }

    pub fn translate(self: Pin<&mut Self>, text: &QString) -> QString {
        shrimply_i18n_qt::text(&text.to_string())
    }

    pub fn application_version(self: Pin<&mut Self>) -> QString {
        QString::from(env!("CARGO_PKG_VERSION"))
    }

    pub fn license_text(self: Pin<&mut Self>) -> QString {
        QString::from(include_str!("../../../../LICENSE"))
    }

    pub fn preference_value(self: Pin<&mut Self>, key: &QString) -> QString {
        self.preference_connector().value(key)
    }

    pub fn preference_minimum(self: Pin<&mut Self>, key: &QString) -> i32 {
        integer_range_value(self.preference_connector(), key, |range| range.minimum)
    }

    pub fn preference_maximum(self: Pin<&mut Self>, key: &QString) -> i32 {
        integer_range_value(self.preference_connector(), key, |range| range.maximum)
    }

    pub fn preference_step(self: Pin<&mut Self>, key: &QString) -> i32 {
        integer_range_value(self.preference_connector(), key, |range| range.step)
    }

    pub fn preference_scale(self: Pin<&mut Self>, key: &QString) -> i32 {
        integer_range_value(self.preference_connector(), key, |range| range.scale)
    }

    pub fn set_preference_value(self: Pin<&mut Self>, key: &QString, value: &QString) {
        self.preference_connector()
            .set_value(key, value)
            .expect("Qt supplied an invalid preference value");
    }

    pub fn preference_server_count(self: Pin<&mut Self>) -> i32 {
        i32::try_from(self.preference_connector().compute_servers().0.len())
            .expect("compute server count exceeds Qt model capacity")
    }

    pub fn preference_server_url(self: Pin<&mut Self>, index: i32) -> QString {
        let index = usize::try_from(index).expect("compute server index must be positive");
        QString::from(
            self.preference_connector()
                .compute_servers()
                .0
                .get(index)
                .expect("Qt requested an unavailable compute server")
                .as_str(),
        )
    }

    pub fn preference_selected_server(self: Pin<&mut Self>) -> i32 {
        i32::try_from(self.preference_connector().compute_servers().1)
            .expect("selected compute server index exceeds Qt model capacity")
    }

    pub fn add_preference_server(self: Pin<&mut Self>, value: &QString) -> QString {
        result_message(
            self.preference_connector()
                .add_compute_server(&value.to_string()),
        )
    }

    pub fn edit_preference_server(self: Pin<&mut Self>, index: i32, value: &QString) -> QString {
        let result = usize::try_from(index)
            .map_err(|_| "Server selection is no longer available")
            .and_then(|index| {
                self.preference_connector()
                    .edit_compute_server(index, &value.to_string())
            });
        result_message(result)
    }

    pub fn remove_preference_server(self: Pin<&mut Self>, index: i32) {
        if let Ok(index) = usize::try_from(index) {
            self.preference_connector().remove_compute_server(index);
        }
    }

    pub fn select_preference_server(self: Pin<&mut Self>, index: i32) {
        if let Ok(index) = usize::try_from(index) {
            self.preference_connector().select_compute_server(index);
        }
    }

    pub fn choose_preference_blender_binary(mut self: Pin<&mut Self>) -> bool {
        let current = self
            .as_mut()
            .preference_connector()
            .value(&QString::from("blender-binary"));
        let initial_url = QUrl::from_local_file(&current);
        let url = shrimply_qt_helpers::open_file_dialog(
            &initial_url,
            &shrimply_i18n_qt::text("Choose Blender Binary"),
            &shrimply_i18n_qt::text("All files (*)"),
        );
        let Some(path) = local_path(&url) else {
            return false;
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(shrimply_preferences_qt::Connector::validate_blender_binary(
                &path,
            ));
        });
        self.as_mut().rust_mut().get_mut().blender_probe = Some(receiver);
        true
    }

    pub fn clear_preference_blender_binary(self: Pin<&mut Self>) {
        self.preference_connector().clear_blender_binary();
    }

    pub fn refresh_preference_server_status(mut self: Pin<&mut Self>) {
        let url = self
            .as_mut()
            .preference_connector()
            .selected_compute_server();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(shrimply_preferences_qt::Connector::compute_server_status(
                &url,
            ));
        });
        let state = self.as_mut().rust_mut().get_mut();
        state.server_status = None;
        state.server_request = Some(receiver);
    }

    pub fn preference_server_detail(self: Pin<&mut Self>, key: &QString) -> QString {
        let this = self.as_ref();
        let Some(status) = this.rust().server_status.as_ref() else {
            return QString::default();
        };
        QString::from(server_detail(status, &key.to_string()))
    }

    pub fn preference_server_device_count(self: Pin<&mut Self>) -> i32 {
        self.as_ref()
            .rust()
            .server_status
            .as_ref()
            .map_or(0, |status| {
                i32::try_from(status.torch.devices.len())
                    .expect("compute device count exceeds Qt model capacity")
            })
    }

    pub fn preference_server_device_label(self: Pin<&mut Self>, index: i32) -> QString {
        let index = usize::try_from(index).expect("compute device index must be positive");
        let this = self.as_ref();
        let device = this
            .rust()
            .server_status
            .as_ref()
            .and_then(|status| status.torch.devices.get(index))
            .expect("Qt requested an unavailable compute device");
        QString::from(device.total_memory_bytes.map_or_else(
            || device.name.clone(),
            |bytes| format!("{} · {:.1} GiB", device.name, bytes as f64 / GIB_BYTES),
        ))
    }

    pub fn preference_server_selected_device(self: Pin<&mut Self>) -> i32 {
        self.as_ref()
            .rust()
            .server_status
            .as_ref()
            .and_then(|status| {
                status
                    .torch
                    .devices
                    .iter()
                    .position(|device| device.id == status.torch.selected_device)
            })
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(-1)
    }

    pub fn select_preference_server_device(mut self: Pin<&mut Self>, index: i32) {
        let index = usize::try_from(index).expect("compute device index must be positive");
        let device = self
            .as_ref()
            .rust()
            .server_status
            .as_ref()
            .and_then(|status| status.torch.devices.get(index))
            .expect("Qt selected an unavailable compute device")
            .id
            .clone();
        let url = self
            .as_mut()
            .preference_connector()
            .selected_compute_server();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(shrimply_preferences_qt::Connector::select_compute_device(
                &url, &device,
            ));
        });
        self.as_mut().rust_mut().get_mut().server_request = Some(receiver);
    }

    fn poll_preference_tasks(mut self: Pin<&mut Self>) {
        let blender = self
            .as_ref()
            .rust()
            .blender_probe
            .as_ref()
            .map(Receiver::try_recv);
        match blender {
            Some(Ok(Ok(path))) => {
                self.as_mut().rust_mut().get_mut().blender_probe = None;
                self.as_mut()
                    .preference_connector()
                    .apply_blender_binary(path);
                self.as_mut()
                    .preference_blender_finished(QString::default());
            }
            Some(Ok(Err(error))) => {
                self.as_mut().rust_mut().get_mut().blender_probe = None;
                self.as_mut()
                    .preference_blender_finished(QString::from(error));
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.as_mut().rust_mut().get_mut().blender_probe = None;
                self.as_mut().preference_blender_finished(QString::from(
                    "Blender validation stopped unexpectedly",
                ));
            }
            None | Some(Err(TryRecvError::Empty)) => {}
        }

        let server = self
            .as_ref()
            .rust()
            .server_request
            .as_ref()
            .map(Receiver::try_recv);
        match server {
            Some(Ok(Ok(status))) => {
                let state = self.as_mut().rust_mut().get_mut();
                state.server_request = None;
                state.server_status = Some(status);
                self.as_mut()
                    .preference_server_status_changed(QString::default());
            }
            Some(Ok(Err(error))) => {
                self.as_mut().rust_mut().get_mut().server_request = None;
                self.as_mut()
                    .preference_server_status_changed(QString::from(error));
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.as_mut().rust_mut().get_mut().server_request = None;
                self.as_mut()
                    .preference_server_status_changed(QString::from(
                        "Compute server request stopped unexpectedly",
                    ));
            }
            None | Some(Err(TryRecvError::Empty)) => {}
        }
    }

    fn history_action(self: Pin<&mut Self>, action: fn(&mut project::Project) -> bool) {
        let Some(session) = self.session.as_deref() else {
            return;
        };
        if action(&mut session.project.borrow_mut()) {
            let duration = session.project.borrow().duration();
            player_state::refresh_project(
                &session.player_state,
                player_state::ProjectChange {
                    duration: Some(duration),
                    audio: true,
                    audio_beats: true,
                    audio_waveforms: true,
                    video: true,
                    captions: true,
                    inspector: true,
                    ..Default::default()
                },
            );
        }
    }

    fn preference_connector(self: Pin<&mut Self>) -> shrimply_preferences_qt::Connector {
        let preferences = self
            .as_ref()
            .rust()
            .session
            .as_deref()
            .expect("preferences require a ready editor session")
            .preferences
            .clone();
        shrimply_preferences_qt::Connector::new(preferences)
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
            LoadEvent::LockedByOtherInstance(pid) => {
                self.as_mut().rust_mut().get_mut().pending_lock_pid = Some(pid);
                self.as_mut().request_lock(i64::from(pid));
            }
            LoadEvent::Ready { path, project } => {
                if let Err(error) = shrimply_support::recent_projects::touch(&path, &project.name) {
                    tracing::warn!(%error, "could not update recent projects");
                }
                ffmpeg_next::init().expect("FFmpeg should initialize");
                ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Error);
                let session = EditorSession::new(*project)
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
            LoadEvent::Canceled => self.as_mut().canceled(),
        }
    }

    fn update_player_properties(mut self: Pin<&mut Self>) {
        let this = self.as_ref();
        let Some(session) = this.rust().session.as_deref() else {
            return;
        };
        let snapshot = player_state::snapshot(&session.player_state);
        let playing = snapshot.playing;
        let position_frame = frame_index(snapshot.position, snapshot.frame_rate).unwrap_or(0);
        let duration_frame = frame_count(snapshot.duration, snapshot.frame_rate)
            .and_then(|frame| i64::try_from(frame).ok())
            .unwrap_or(i64::MAX);
        let time_label = QString::from(shrimply_preview_runtime::playback_time_label(
            snapshot.position,
            snapshot.duration,
        ));
        let frame_rate_label = QString::from(shrimply_preview_qt::preview_frame_rate_label());
        let playback_speed_label = QString::from(shrimply_preview_runtime::playback_speed_label(
            snapshot.playback_speed,
        ));
        self.as_mut().set_playing(playing);
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

fn integer_range_value(
    connector: shrimply_preferences_qt::Connector,
    key: &QString,
    value: impl FnOnce(shrimply_preferences_qt::IntegerRange) -> i64,
) -> i32 {
    i32::try_from(value(connector.integer_range(key)))
        .expect("preference range exceeds Qt integer capacity")
}

fn result_message(result: Result<(), &'static str>) -> QString {
    result.map_or_else(QString::from, |()| QString::default())
}

fn server_detail(status: &shrimply_preferences_qt::ServerStatus, key: &str) -> String {
    match key {
        "version" => {
            if status.server.git_short_hash.is_empty() {
                status.server.version.clone()
            } else {
                format!(
                    "{} ({})",
                    status.server.version, status.server.git_short_hash
                )
            }
        }
        "protocol" => format!("{}.{}", status.protocol.major, status.protocol.minor),
        "torch" => status.torch.version.clone(),
        "cuda" => match (&status.torch.cuda_runtime, status.torch.cuda_available) {
            (Some(runtime), true) => format!("{runtime} · Available"),
            (None, true) => "Available".to_string(),
            (_, false) => "Unavailable".to_string(),
        },
        "jobs" => format!(
            "{} queued · {} active",
            status.compute.queued_jobs, status.compute.active_jobs
        ),
        "reservations" => format!(
            "RAM {:.1} GiB · VRAM {:.1} GiB",
            status.compute.reserved_ram_bytes as f64 / GIB_BYTES,
            status.compute.reserved_vram_bytes as f64 / GIB_BYTES
        ),
        "workers" => status
            .compute
            .workers
            .iter()
            .map(|worker| {
                format!(
                    "{} · {} · {} ×{}",
                    worker.service, worker.model, worker.state, worker.copies
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "features" => status.capabilities.join(", "),
        _ => panic!("Qt requested an unknown compute server detail: {key}"),
    }
}

fn local_path(url: &QUrl) -> Option<PathBuf> {
    url.is_local_file()
        .then(|| PathBuf::from(url.to_local_file_or_default().to_string()))
}
