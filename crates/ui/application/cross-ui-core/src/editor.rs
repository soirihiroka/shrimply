use crate::project_save::ProjectFormat;
use shrimply_editing_runtime::attached;
pub use shrimply_editing_runtime::attached::{
    MatchVersion, OpenError, VersionMismatch, VersionPrompt, activate as activate_project_session,
    open as load_project_session,
};
use shrimply_editor_state::{player_state, preferences, preview_focus};
use shrimply_math_core::Fraction;
use shrimply_project_document::project::{self, CanvasSize, Project, ProjectValidationOutcome};
use shrimply_resource_pipeline::CancelToken;
pub use shrimply_timeline_edit::project_menu;
use shrimply_timeline_edit::selection_state;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

const KDENLIVE_READING: &str = "Reading and converting Kdenlive timeline…";
const OTIO_READING: &str = "Reading and converting OTIO timeline…";
const PROJECT_READING: &str = "Loading project…";
const PROJECT_WRITING: &str = "Writing Shrimply project…";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportKind {
    Kdenlive,
    Otio,
}

#[derive(Debug)]
pub enum LoadEvent {
    ConfirmTrust(shrimply_trust_core::Review),
    ConfirmVersion(Box<VersionPrompt>),
    ConfirmKdenlive,
    ChooseOtioSettings,
    Progress(&'static str),
    ConfirmFrameGridRepair,
    ChooseDestination {
        title: &'static str,
        suggested_name: String,
    },
    ImportWarnings(Vec<String>),
    Ready {
        path: PathBuf,
        project: Box<Project>,
    },
    Error {
        heading: &'static str,
        body: String,
    },
    Canceled,
    HandedOff,
}

pub enum VersionChoice {
    Close,
    Force,
    Match,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorTitle {
    pub text: String,
    pub save_error: Option<String>,
}

#[derive(Default)]
pub struct SessionUpdate {
    pub audio_playback_stopped: Option<String>,
    pub title: Option<EditorTitle>,
    pub has_collaborators: bool,
}

pub struct EditorSession {
    editing: RefCell<attached::Editor>,
    pub project: Rc<RefCell<Project>>,
    pub player_state: player_state::SharedPlayerState,
    pub playback_performance: shrimply_playback_performance::SharedCollector,
    pub selection_state: selection_state::SharedSelectionState,
    pub preview_focus: preview_focus::SharedPreviewFocus,
    pub preferences: preferences::SharedPreferences,
    pub audio_levels: shrimply_audio_engine::SharedAudioLevels,
    pub audio_player: Rc<shrimply_audio_engine::AudioPlayer>,
    pub property_clipboard: shrimply_property_transfer::SharedClipboard,
    asset_changes: async_channel::Receiver<shrimply_asset::AssetChange>,
    pending_cursor: Rc<Cell<Option<project::Time>>>,
    commit_status: Rc<RefCell<project::CommitStatus>>,
    project_metadata: RefCell<(String, Vec<String>)>,
    title_dirty: Rc<Cell<bool>>,
    sync_error: RefCell<Option<String>>,
}

impl EditorSession {
    /// Execute shared menu commands and return window-level requests to the host.
    pub fn activate_project_menu(
        &self,
        action: project_menu::Action,
    ) -> Result<Option<project_menu::Action>, String> {
        if let Some(edit) = action.edit() {
            let kind = {
                let mut project = self.project.borrow_mut();
                let kind = edit.apply(&mut project)?;
                project::commit_edit(&project, project_menu::Edit::DESCRIPTION);
                kind
            };
            player_state::refresh_project(
                &self.player_state,
                player_state::ProjectChange {
                    audio: kind == project::ItemKind::Audio,
                    video: kind == project::ItemKind::Video,
                    captions: kind == project::ItemKind::Caption,
                    inspector: true,
                    ..Default::default()
                },
            );
        } else if let Some(redo) = action.history() {
            change_history(
                &self.project,
                &self.player_state,
                if redo { project::redo } else { project::undo },
            );
        } else if action == project_menu::Action::Save {
            self.save()?;
        } else {
            return Ok(Some(action));
        }
        Ok(None)
    }

    pub fn new(project: Project) -> Result<Self, String> {
        let playback_performance = shrimply_playback_performance::open(Arc::new(project.clone()));
        let project = Rc::new(RefCell::new(project));
        project.borrow().watch_assets()?;
        let (duration, frame_rate, cursor_position) = {
            let project = project.borrow();
            (project.duration(), project.fps, project.cursor_position)
        };
        let player_state = player_state::new(duration, frame_rate);
        if let Some(position) = cursor_position {
            player_state::seek_time(&player_state, position.max(project::Time::ZERO));
        }
        let pending_cursor = Rc::new(Cell::new(None));
        let cursor_target = pending_cursor.clone();
        let cursor_player = player_state.clone();
        player_state::connect_named(&player_state, "persist project cursor", move |event| {
            if matches!(event, player_state::PlayerEvent::State(_)) {
                cursor_target.set(Some(
                    player_state::snapshot(&cursor_player)
                        .position
                        .max(project::Time::ZERO),
                ));
            }
        });
        let watched_project = project.clone();
        player_state::connect_named(&player_state, "watch project assets", move |event| {
            if matches!(event, player_state::PlayerEvent::Project(_))
                && let Err(error) = watched_project.borrow().watch_assets()
            {
                tracing::error!(%error, "could not watch project assets");
            }
        });
        let preferences = preferences::open_with_defaults();
        let preference = preferences::snapshot(&preferences);
        shrimply_blender_core::set_binary(preference.blender_binary);
        shrimply_audio_engine::pneuma::set_server_url(&preference.compute_server_url);
        let audio_levels = Arc::new(shrimply_audio_engine::AudioLevels::default());
        let audio_player = Rc::new(shrimply_audio_engine::AudioPlayer::new(
            &project.borrow(),
            audio_levels.clone(),
        )?);
        player_state::connect_audio_playback(&project, &player_state, &audio_player);
        let commit_status = Rc::new(RefCell::new(project::CommitStatus::Idle));
        let current_commit_status = commit_status.clone();
        let title_dirty = Rc::new(Cell::new(true));
        let changed_title = title_dirty.clone();
        project::connect_commit_status(move |status| {
            *current_commit_status.borrow_mut() = status;
            changed_title.set(true);
        });
        let project_metadata = {
            let project = project.borrow();
            RefCell::new((project.name.clone(), project.tags.clone()))
        };
        let selection_state = selection_state::new();
        let editing = RefCell::new(attached::Editor::new(
            &project::active_project_path(),
            project.clone(),
            player_state.clone(),
            selection_state.clone(),
        )?);
        Ok(Self {
            editing,
            project,
            player_state,
            playback_performance,
            selection_state,
            preview_focus: preview_focus::new(),
            preferences,
            audio_levels,
            audio_player,
            property_clipboard: shrimply_property_transfer::new_clipboard(),
            asset_changes: shrimply_asset::subscribe(),
            pending_cursor,
            commit_status,
            project_metadata,
            title_dirty,
            sync_error: RefCell::new(None),
        })
    }

    pub fn poll(&self) -> SessionUpdate {
        {
            let editing = self.editing.borrow();
            editing.poll();
            let error = editing.sync_error();
            let mut current_error = self.sync_error.borrow_mut();
            if *current_error != error {
                *current_error = error;
                self.title_dirty.set(true);
            }
        }
        project::poll_commit_status();
        player_state::tick(&self.player_state);
        let audio_playback_stopped = self.audio_player.take_failure();
        if audio_playback_stopped.is_some() {
            player_state::set_playing(&self.player_state, false);
        }
        while let Ok(change) = self.asset_changes.try_recv() {
            let (audio, video) = {
                let project = self.project.borrow();
                (
                    project.uses_audio_asset(&change.path),
                    project.uses_video_asset(&change.path),
                )
            };
            if audio || video {
                tracing::info!(
                    path = %change.path.display(),
                    revision = change.revision,
                    audio,
                    video,
                    "project asset changed"
                );
                player_state::refresh_project(
                    &self.player_state,
                    player_state::ProjectChange {
                        audio,
                        audio_beats: audio,
                        audio_waveforms: audio,
                        video,
                        inspector: true,
                        ..Default::default()
                    },
                );
            }
        }
        {
            let project = self.project.borrow();
            let mut metadata = self.project_metadata.borrow_mut();
            if metadata.0 != project.name || metadata.1 != project.tags {
                if let Err(error) = shrimply_project_service::catalog::register(
                    &project::active_project_path(),
                    &project,
                ) {
                    tracing::warn!(%error, "could not register project");
                }
                if metadata.0 != project.name {
                    self.title_dirty.set(true);
                }
                *metadata = (project.name.clone(), project.tags.clone());
            }
        }
        if let Some(position) = self.pending_cursor.take() {
            let mut project = self.project.borrow_mut();
            if project.cursor_position != Some(position) {
                project.cursor_position = Some(position);
                project::save_view_state(&project);
            }
        }
        SessionUpdate {
            audio_playback_stopped,
            title: self.title_dirty.replace(false).then(|| self.title()),
            has_collaborators: !player_state::collaborators(&self.player_state).is_empty(),
        }
    }

    pub fn title(&self) -> EditorTitle {
        let project = self.project.borrow();
        let status = self.commit_status.borrow();
        if !matches!(*status, project::CommitStatus::SaveFailed(_))
            && let Some(error) = self.sync_error.borrow().as_ref()
        {
            return editor_title(
                &project.name,
                &project::CommitStatus::SaveFailed(error.clone()),
            );
        }
        editor_title(&project.name, &status)
    }

    pub fn save(&self) -> Result<(), String> {
        project::save()
    }

    pub fn save_as(&self, path: PathBuf) -> Result<PathBuf, String> {
        let path = ProjectFormat::from_path(&path).normalize_path(path);
        let current = project::active_project_path();
        if project::normalized_project_path(&path) == project::normalized_project_path(&current) {
            self.save()?;
            return Ok(current);
        }
        project::flush_history()?;
        let source = self.project.borrow().clone();
        attached::save_copy(&path, &source)?;
        let mut candidate = load_project_session(&path, &[], None)
            .map_err(|error| error.to_string())?
            .expect("opening a new copy cannot relaunch the editor");
        candidate.copy_view_state_from(&source);
        let editing = attached::Editor::new(
            &path,
            self.project.clone(),
            self.player_state.clone(),
            self.selection_state.clone(),
        )?;
        project::shutdown_history()?;
        activate_project_session(&path, &candidate);
        *self.project.borrow_mut() = candidate;
        self.editing.replace(editing);
        player_state::set_collaborators(&self.player_state, Vec::new());
        let project = self.project.borrow();
        if let Err(error) = shrimply_project_service::catalog::register(&path, &project) {
            tracing::warn!(%error, "could not register project");
        }
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                inspector: true,
                ..Default::default()
            },
        );
        Ok(path)
    }
}

pub use shrimply_editor_state::player_state::change_history;

pub fn suggested_save_as_path() -> PathBuf {
    let current = project::active_project_path();
    let extension = ProjectFormat::from_path(&current).extension();
    let name = current
        .file_stem()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name} copy.{extension}"))
        .unwrap_or_else(|| format!("project copy.{extension}"));
    current.with_file_name(name)
}

fn editor_title(project_name: &str, status: &project::CommitStatus) -> EditorTitle {
    let text = match status {
        project::CommitStatus::InProgress(action) => shrimply_i18n::text_args(
            "%{project} — %{action}",
            &[
                ("project", project_name.to_owned()),
                ("action", shrimply_i18n::text(action).into_owned()),
            ],
        ),
        project::CommitStatus::SavePending => shrimply_i18n::text_args(
            "%{project} — Unsaved",
            &[("project", project_name.to_owned())],
        ),
        project::CommitStatus::Saving => shrimply_i18n::text_args(
            "%{project} — Saving",
            &[("project", project_name.to_owned())],
        ),
        project::CommitStatus::SaveFailed(_) => shrimply_i18n::text_args(
            "%{project} — Unsaved — Save failed",
            &[("project", project_name.to_owned())],
        ),
        project::CommitStatus::Idle => project_name.to_string(),
    };
    EditorTitle {
        text,
        save_error: match status {
            project::CommitStatus::SaveFailed(error) => Some(error.clone()),
            _ => None,
        },
    }
}

impl Drop for EditorSession {
    fn drop(&mut self) {
        self.audio_player.stop();
    }
}

enum SaveKind {
    Repair,
    Import(Vec<String>),
}

enum Pending {
    Load(Result<Option<ProjectValidationOutcome>, OpenError>),
    Import {
        kind: ImportKind,
        source: PathBuf,
        result: Result<(project::ProjectValidationOutcome, Vec<String>), String>,
    },
    Save {
        result: Result<PathBuf, String>,
        error_heading: &'static str,
        save_kind: SaveKind,
    },
}

enum State {
    Idle,
    WaitingForTrust {
        project: Project,
        review: shrimply_trust_core::Review,
    },
    WaitingForKdenlive,
    WaitingForOtioSettings,
    WaitingForVersion(Box<VersionPrompt>),
    Working(Receiver<Pending>),
    WaitingForRepair {
        source: PathBuf,
        project: Project,
        error_heading: &'static str,
        save_kind: SaveKind,
    },
    WaitingForDestination {
        project: Project,
        error_heading: &'static str,
        save_kind: SaveKind,
    },
    WaitingForWarnings {
        path: PathBuf,
    },
    Finished,
}

pub struct ProjectLoader {
    path: PathBuf,
    state: State,
    cancellation: CancelToken,
    forced_versions: Vec<VersionMismatch>,
}

impl Drop for ProjectLoader {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl ProjectLoader {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: State::Idle,
            cancellation: CancelToken::default(),
            forced_versions: Vec::new(),
        }
    }

    pub fn begin(&mut self) -> LoadEvent {
        assert!(
            matches!(self.state, State::Idle),
            "project loader already started"
        );
        if has_extension(&self.path, "kdenlive") {
            self.state = State::WaitingForKdenlive;
            LoadEvent::ConfirmKdenlive
        } else if has_extension(&self.path, "otio") {
            self.state = State::WaitingForOtioSettings;
            LoadEvent::ChooseOtioSettings
        } else {
            self.start_native_load(None);
            LoadEvent::Progress(PROJECT_READING)
        }
    }

    pub fn confirm_trust(&mut self, kind: Option<shrimply_trust_core::Kind>) -> LoadEvent {
        let State::WaitingForTrust { project, review } =
            std::mem::replace(&mut self.state, State::Finished)
        else {
            panic!("trust response without a pending confirmation");
        };
        let Some(kind) = kind else {
            return LoadEvent::Canceled;
        };
        if let Err(body) = review.approve(kind) {
            return LoadEvent::Error {
                heading: "Could not save trust",
                body,
            };
        }
        self.finish_native_load(Ok(Some(ProjectValidationOutcome::Valid(project))))
    }

    pub fn confirm_kdenlive(&mut self, convert: bool) -> LoadEvent {
        assert!(
            matches!(self.state, State::WaitingForKdenlive),
            "Kdenlive response without a pending confirmation"
        );
        if !convert {
            self.state = State::Finished;
            return LoadEvent::Canceled;
        }
        self.start_import(ImportKind::Kdenlive, |source| {
            shrimply_kdenlive_parser::from_file(source).map_err(|error| error.to_string())
        })
    }

    pub fn choose_otio_settings(&mut self, settings: Option<(CanvasSize, Fraction)>) -> LoadEvent {
        assert!(
            matches!(self.state, State::WaitingForOtioSettings),
            "OTIO settings without a pending request"
        );
        let Some((canvas_size, fps)) = settings else {
            self.state = State::Finished;
            return LoadEvent::Canceled;
        };
        self.start_import(ImportKind::Otio, move |source| {
            shrimply_otio_parser::from_file(source, canvas_size, fps)
        })
    }

    fn start_import(
        &mut self,
        kind: ImportKind,
        parse: impl FnOnce(&Path) -> Result<project::InterchangeDocument, String> + Send + 'static,
    ) -> LoadEvent {
        let source = self.path.clone();
        self.start_worker(move || {
            let result = parse(&source).and_then(|import| {
                let project = project::from_json_value_with_frame_grid_repair(import.project)?;
                Ok((project, import.warnings))
            });
            Pending::Import {
                kind,
                source,
                result,
            }
        });
        LoadEvent::Progress(match kind {
            ImportKind::Kdenlive => KDENLIVE_READING,
            ImportKind::Otio => OTIO_READING,
        })
    }

    pub fn confirm_frame_grid_repair(&mut self, repair: bool) -> LoadEvent {
        let state = std::mem::replace(&mut self.state, State::Finished);
        let State::WaitingForRepair {
            source,
            project,
            error_heading,
            save_kind,
        } = state
        else {
            panic!("repair response without a pending confirmation");
        };
        if !repair {
            return LoadEvent::Canceled;
        }
        let title = if matches!(&save_kind, SaveKind::Repair) {
            "Save Fixed Project"
        } else {
            match error_heading {
                "Could not import Kdenlive project" => "Import Kdenlive as Shrimply Project",
                _ => "Import OTIO as Shrimply Project",
            }
        };
        let suggested_name = fixed_project_filename(&source);
        self.state = State::WaitingForDestination {
            project,
            error_heading,
            save_kind,
        };
        LoadEvent::ChooseDestination {
            title,
            suggested_name,
        }
    }

    pub fn choose_destination(&mut self, destination: Option<PathBuf>) -> LoadEvent {
        let state = std::mem::replace(&mut self.state, State::Finished);
        let State::WaitingForDestination {
            project,
            error_heading,
            save_kind,
        } = state
        else {
            panic!("destination response without a pending request");
        };
        let Some(mut destination) = destination else {
            return LoadEvent::Canceled;
        };
        if !has_extension(&destination, "shrimp") {
            destination.set_extension("shrimp");
        }
        let cancellation = self.cancellation.clone();
        self.start_worker(move || {
            let result = match &save_kind {
                SaveKind::Import(_) => shrimply_media_import::save_interchange_project(
                    &destination,
                    project,
                    cancellation,
                ),
                SaveKind::Repair => project::create_project_file(&destination, &project),
            }
            .map(|()| destination);
            Pending::Save {
                result,
                error_heading,
                save_kind,
            }
        });
        LoadEvent::Progress(PROJECT_WRITING)
    }

    pub fn acknowledge_warnings(&mut self) -> LoadEvent {
        let state = std::mem::replace(&mut self.state, State::Finished);
        let State::WaitingForWarnings { path } = state else {
            panic!("warning acknowledgement without pending warnings");
        };
        self.path = path;
        self.start_native_load(None);
        LoadEvent::Progress(PROJECT_READING)
    }

    pub fn poll(&mut self) -> Option<LoadEvent> {
        let result = match &self.state {
            State::Working(receiver) => match receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    self.state = State::Finished;
                    return Some(LoadEvent::Error {
                        heading: "Could not open project",
                        body: "The project worker stopped unexpectedly.".to_string(),
                    });
                }
            },
            _ => return None,
        };
        Some(match result {
            Pending::Load(result) => self.finish_native_load(result),
            Pending::Import {
                kind,
                source,
                result,
            } => self.finish_import(kind, source, result),
            Pending::Save {
                result,
                error_heading,
                save_kind,
            } => self.finish_save(result, error_heading, save_kind),
        })
    }

    pub fn confirm_version(&mut self, choice: VersionChoice) -> LoadEvent {
        let State::WaitingForVersion(prompt) = std::mem::replace(&mut self.state, State::Finished)
        else {
            panic!("project loader is not waiting for a version choice");
        };
        let action = match choice {
            VersionChoice::Close => return LoadEvent::Canceled,
            VersionChoice::Force => {
                self.forced_versions.push(prompt.mismatch);
                None
            }
            VersionChoice::Match => Some(prompt.action.expect("version match action is available")),
        };
        self.start_native_load(action);
        LoadEvent::Progress(PROJECT_READING)
    }

    fn start_native_load(&mut self, action: Option<MatchVersion>) {
        let path = self.path.clone();
        let forced = self.forced_versions.clone();
        self.start_worker(move || {
            let result = match load_project_session(&path, &forced, action) {
                Ok(project) => Ok(project.map(ProjectValidationOutcome::Valid)),
                Err(error @ OpenError::Failed(_)) => {
                    match project::read_project_for_editor(&path) {
                        Ok(outcome @ ProjectValidationOutcome::FrameGridRepair(_)) => {
                            Ok(Some(outcome))
                        }
                        _ => Err(error),
                    }
                }
                Err(error) => Err(error),
            };
            Pending::Load(result)
        });
    }

    fn start_worker(&mut self, work: impl FnOnce() -> Pending + Send + 'static) {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let _ = sender.send(work());
        });
        self.state = State::Working(receiver);
    }

    fn finish_native_load(
        &mut self,
        result: Result<Option<ProjectValidationOutcome>, OpenError>,
    ) -> LoadEvent {
        match result {
            Ok(Some(ProjectValidationOutcome::Valid(project))) => {
                let review = match shrimply_trust_core::Review::new(project.executable_sources()) {
                    Ok(review) => review,
                    Err(body) => {
                        self.state = State::Finished;
                        return LoadEvent::Error {
                            heading: "Could not check source trust",
                            body,
                        };
                    }
                };
                if !review.files.is_empty() {
                    self.state = State::WaitingForTrust {
                        project,
                        review: review.clone(),
                    };
                    return LoadEvent::ConfirmTrust(review);
                }
                let path = self.path.clone();
                activate_project_session(&path, &project);
                self.state = State::Finished;
                LoadEvent::Ready {
                    path,
                    project: Box::new(project),
                }
            }
            Ok(Some(ProjectValidationOutcome::FrameGridRepair(project))) => {
                self.state = State::WaitingForRepair {
                    source: self.path.clone(),
                    project,
                    error_heading: "Could not fix project",
                    save_kind: SaveKind::Repair,
                };
                LoadEvent::ConfirmFrameGridRepair
            }
            Ok(None) => {
                self.state = State::Finished;
                LoadEvent::HandedOff
            }
            Err(OpenError::Version(prompt)) => {
                self.state = State::WaitingForVersion(prompt.clone());
                LoadEvent::ConfirmVersion(prompt)
            }
            Err(OpenError::Failed(body)) => {
                self.state = State::Finished;
                LoadEvent::Error {
                    heading: "Could not open project",
                    body,
                }
            }
        }
    }

    fn finish_import(
        &mut self,
        kind: ImportKind,
        source: PathBuf,
        result: Result<(project::ProjectValidationOutcome, Vec<String>), String>,
    ) -> LoadEvent {
        let error_heading = match kind {
            ImportKind::Kdenlive => "Could not import Kdenlive project",
            ImportKind::Otio => "Could not import OTIO",
        };
        let (outcome, warnings) = match result {
            Ok(result) => result,
            Err(body) => {
                self.state = State::Finished;
                return LoadEvent::Error {
                    heading: error_heading,
                    body,
                };
            }
        };
        let save_kind = SaveKind::Import(match kind {
            ImportKind::Otio => warnings,
            ImportKind::Kdenlive => Vec::new(),
        });
        match outcome {
            project::ProjectValidationOutcome::Valid(project) => {
                self.state = State::WaitingForDestination {
                    project,
                    error_heading,
                    save_kind,
                };
                LoadEvent::ChooseDestination {
                    title: match kind {
                        ImportKind::Kdenlive => "Import Kdenlive as Shrimply Project",
                        ImportKind::Otio => "Import OTIO as Shrimply Project",
                    },
                    suggested_name: imported_project_filename(&source),
                }
            }
            project::ProjectValidationOutcome::FrameGridRepair(project) => {
                self.state = State::WaitingForRepair {
                    source,
                    project,
                    error_heading,
                    save_kind,
                };
                LoadEvent::ConfirmFrameGridRepair
            }
        }
    }

    fn finish_save(
        &mut self,
        result: Result<PathBuf, String>,
        error_heading: &'static str,
        save_kind: SaveKind,
    ) -> LoadEvent {
        match result {
            Ok(path) => match save_kind {
                SaveKind::Import(warnings) if !warnings.is_empty() => {
                    self.state = State::WaitingForWarnings { path };
                    LoadEvent::ImportWarnings(warnings)
                }
                SaveKind::Repair | SaveKind::Import(_) => {
                    self.path = path;
                    self.start_native_load(None);
                    LoadEvent::Progress(PROJECT_READING)
                }
            },
            Err(body) => {
                self.state = State::Finished;
                LoadEvent::Error {
                    heading: error_heading,
                    body,
                }
            }
        }
    }
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn imported_project_filename(source: &Path) -> String {
    source
        .file_stem()
        .map(|name| format!("{}.shrimp", name.to_string_lossy()))
        .unwrap_or_else(|| "imported.shrimp".to_string())
}

fn fixed_project_filename(source: &Path) -> String {
    let stem = source
        .file_stem()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "project".into());
    let timestamp = glib::DateTime::now_local()
        .and_then(|time| time.format("%Y-%m-%d_%H-%M-%S"))
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "unknown-time".to_string());
    format!("{stem}_{timestamp}-fix.shrimp")
}
