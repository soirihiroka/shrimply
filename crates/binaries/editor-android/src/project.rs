//! Android project commands and document-provider checkpoint notifications.
//! Invoke this service on one native worker thread, never the Android UI thread.
mod watch;
use serde_json::{Value, json};
#[cfg(target_os = "android")]
use shrimply_editing_runtime::{
    attached::{self, VersionMismatch, VersionPrompt},
    embedded,
};
use shrimply_math_core::Time;
use shrimply_project_document::{
    self as document, Project,
    creation::{NewProject, apply_project_settings},
};
#[cfg(target_os = "android")]
use shrimply_project_service::{SharedWorkspace, catalog::ProjectEntry};
use shrimply_timeline_edit::project_menu::{self, Action};
use shrimply_timeline_edit::selection_state;
use std::{
    cell::{Ref, RefCell},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};
pub(crate) use watch::MirrorListener;

pub(crate) struct ProjectBackend {
    project: Option<Rc<RefCell<Project>>>,
    #[cfg(target_os = "android")]
    owner: Option<embedded::Lease>,
    #[cfg(target_os = "android")]
    remote: bool,
    #[cfg(target_os = "android")]
    pub(crate) forced_version: Option<VersionMismatch>,
    path: Option<PathBuf>,
    watcher: Option<watch::Watch>,
    mirror_root: Option<String>,
    mirror_listener: Option<MirrorListener>,
    pub selection: selection_state::SharedSelectionState,
    pub(crate) wake: Arc<dyn Fn() + Send + Sync>,
    fingerprint: Option<String>,
    status: Rc<RefCell<document::CommitStatus>>,
    error: Option<String>,
    active: bool,
    pub revision: u64,
    view_dirty: bool,
    pub preferences: shrimply_preferences_core::SharedPreferences,
}
impl ProjectBackend {
    pub fn new(
        wake: impl Fn() + Send + Sync + 'static,
        mirror_listener: Option<MirrorListener>,
    ) -> Result<Self, String> {
        let status = Rc::new(RefCell::new(document::CommitStatus::Idle));
        let observed = status.clone();
        document::connect_commit_status(move |value| *observed.borrow_mut() = value);
        Ok(Self {
            project: None,
            #[cfg(target_os = "android")]
            owner: None,
            #[cfg(target_os = "android")]
            remote: false,
            #[cfg(target_os = "android")]
            forced_version: None,
            path: None,
            watcher: None,
            mirror_root: None,
            mirror_listener,
            selection: selection_state::new(),
            wake: Arc::new(wake),
            fingerprint: None,
            status,
            error: None,
            active: true,
            revision: 0,
            view_dirty: false,
            preferences: shrimply_preferences_core::open_with_defaults(),
        })
    }

    pub fn project(&self) -> Option<Ref<'_, Project>> {
        self.project.as_ref().map(|project| project.borrow())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn shared_project(&self) -> Option<Rc<RefCell<Project>>> {
        self.project.clone()
    }

    /// The shared playback owner supplies an already bounded timeline position.
    /// This hot path must not scan or clone the immutable document.
    pub fn cursor(&mut self, position: Time) {
        if let Some(project) = &self.project {
            let mut project = project.borrow_mut();
            let next = Some(position.max(Time::ZERO));
            self.view_dirty |= project.cursor_position != next;
            project.cursor_position = next;
        }
    }

    pub fn validate_target(&self, request: &Value) -> Result<(), String> {
        if let Some(expected) = request.get("project_path") {
            let expected = Path::new(expected.as_str().ok_or("project_path must be a string")?);
            if self.path.as_deref() != Some(expected) {
                return Err("the active project changed while this action was open; select its destination again".into());
            }
        }
        Ok(())
    }

    pub fn dispatch(&mut self, request: &Value) -> Result<Value, String> {
        self.validate_target(request)?;
        let operation = operation(request)?;
        match operation {
            "status" => {}
            "create" => {
                if self.project.is_some() {
                    return Err("close the current project before creating another project".into());
                }
                let project = NewProject::from_fields(request)?;
                #[cfg(target_os = "android")]
                if let Some(server) = request["server"]
                    .as_str()
                    .filter(|server| !server.is_empty())
                {
                    let old = shrimply_preferences_core::snapshot(&self.preferences);
                    if !old.project_server_urls.iter().any(|url| url == server) {
                        return Err("Choose a server from Project Servers".into());
                    }
                    let project = project.into_document();
                    let name = document::creation::filename_stem(&project.name);
                    let directory = Path::new(string(request, "directory")?);
                    let path =
                        SharedWorkspace::managed_path_in(directory, &format!("{name}.shrimp"))?;
                    SharedWorkspace::create(&path, server, &project)?;
                    self.open(&path, None)?;
                    return self.snapshot();
                }
                let path = if let Some(path) = request["path"].as_str() {
                    project.write(PathBuf::from(path), false)?
                } else {
                    project.write_in(Path::new(string(request, "directory")?))?
                };
                self.open(&path, None)?;
            }
            "settings_schema" => return Ok(settings_schema()),
            "settings" => {
                self.edit("Change project settings", |project| {
                    apply_project_settings(project, request)?;
                    if request["repair_frame_grid"] == true {
                        project.repair_frame_grid();
                    }
                    Ok(())
                })?;
                self.register_project()?;
            }
            "open" => {
                let path = Path::new(string(request, "path")?);
                #[cfg(target_os = "android")]
                let forced = if let Some(workspace) = SharedWorkspace::open(path)? {
                    match check_server_version(&workspace.link().server, request) {
                        Ok(forced) => forced,
                        Err(prompt) => return Ok(prompt),
                    }
                } else {
                    None
                };
                self.open(path, request["mirror_root"].as_str().map(Path::new))?;
                #[cfg(target_os = "android")]
                {
                    self.forced_version = forced;
                }
            }
            #[cfg(target_os = "android")]
            "open_remote" => {
                if self.project.is_some() {
                    return Err("close the current project before opening another project".into());
                }
                let entry: ProjectEntry = serde_json::from_value(request["project"].clone())
                    .map_err(|error| error.to_string())?;
                let link = entry
                    .remote
                    .as_ref()
                    .ok_or("project has no remote location")?;
                if !shrimply_preferences_core::snapshot(&self.preferences)
                    .project_server_urls
                    .contains(&link.server)
                {
                    return Err("Choose a server from Project Servers".into());
                }
                let forced = match check_server_version(&link.server, request) {
                    Ok(forced) => forced,
                    Err(prompt) => return Ok(prompt),
                };
                let path = entry.open(Some(Path::new(string(request, "directory")?)))?;
                self.open(&path, None)?;
                self.forced_version = forced;
            }
            #[cfg(target_os = "android")]
            "delete_project" => {
                if self.project.is_some() {
                    return Err("Close the open project before deleting from the catalog".into());
                }
                let entry: ProjectEntry = serde_json::from_value(request["project"].clone())
                    .map_err(|error| error.to_string())?;
                entry.delete()?;
            }
            "close" => self.close()?,
            "save" => self.save()?,
            "save_as" => {
                self.save()?;
                let path = std::path::absolute(string(request, "path")?)
                    .map_err(|error| error.to_string())?;
                self.save_as(path)?;
            }
            "export" => {
                self.save()?;
                #[cfg(target_os = "android")]
                if self.remote {
                    let path = Path::new(string(request, "path")?);
                    let canonical = attached::load(self.path().expect("open project"))?;
                    attached::save_copy(path, &canonical)?;
                    return self.snapshot();
                }
                let directory = self
                    .path
                    .as_ref()
                    .expect("open project path")
                    .parent()
                    .expect("project directory");
                for asset in self.required()?.assets() {
                    let source = asset.path().canonicalize().map_err(|error| {
                        format!(
                            "cannot export missing asset {}: {error}",
                            asset.path().display()
                        )
                    })?;
                    if !source.starts_with(directory) {
                        return Err(format!(
                            "cannot export {} outside the project folder; import or relocate it into the project before exporting",
                            source.display()
                        ));
                    }
                }
                if let Some(path) = request.get("path") {
                    let path =
                        std::path::absolute(path.as_str().ok_or("export path must be a string")?)
                            .map_err(|error| error.to_string())?;
                    if path.parent() != self.path.as_ref().and_then(|path| path.parent()) {
                        return Err("export the entire project folder, including media; a project file alone in another directory would break relative asset paths".into());
                    }
                    document::create_new_project_file(&path, &*self.required()?)?;
                }
            }
            "undo" | "redo" => {
                let redo = operation == "redo";
                self.reload()?;
                let mut candidate = self.required()?.clone();
                if document::change_history_saved(&mut candidate, redo)? {
                    *self.project.as_ref().expect("open project").borrow_mut() = candidate;
                    self.observe_saved()?;
                    self.revision += 1;
                }
            }
            "project_menu" => {
                let action = menu_action(request)?;
                if let Some(edit) = action.edit() {
                    self.edit(project_menu::Edit::DESCRIPTION, |project| {
                        edit.apply(project)
                    })?;
                } else {
                    if action == Action::Save {
                        self.save()?;
                    }
                    let effect =
                        menu_effect(action).ok_or("project menu callback is unavailable")?;
                    return Ok(json!({"effect":effect}));
                }
            }
            "toggle_track" => {
                let kind = track_kind(string(request, "kind")?)?;
                self.edit("Toggle track", |project| {
                    let key = shrimply_timeline_edit::TrackKey {
                        kind,
                        track_index: target_index(project, kind, request)?,
                    };
                    let address =
                        shrimply_timeline_edit::selection_state::track_address(project, key)
                            .ok_or("track does not exist")?;
                    let enabled = project.track(&address).expect("resolved track").enabled();
                    shrimply_timeline_edit::edit::set_track_enabled(project, &address, !enabled)
                        .ok_or_else(|| "track does not exist".into())
                })?;
            }
            "track_add" => {
                use shrimply_timeline_edit::{TrackAddMenuEntry, track_add_menu};
                let kind = track_kind(string(request, "kind")?)?;
                let token = string(request, "action")?;
                let action = track_add_menu(kind)
                    .iter()
                    .find_map(|entry| match entry {
                        TrackAddMenuEntry::Action(action) if format!("{action:?}") == token => {
                            Some(*action)
                        }
                        _ => None,
                    })
                    .ok_or("this action is not available for the selected track kind")?;
                self.generated(request, kind, action)?;
            }
            "set_cursor" => {
                let frame = request["frame"]
                    .as_u64()
                    .ok_or("frame must be a nonnegative integer")?;
                let position = shrimply_math_core::time_from_frame(frame, self.required()?.fps)
                    .ok_or("frame is outside the exact timeline range")?;
                let duration = self.required()?.duration();
                self.cursor(position.min(duration));
            }
            "import_file" => return self.import_file(request, None),
            "reload" => {
                self.reload()?;
            }
            operation => return Err(format!("unknown Android project operation '{operation}'")),
        }
        self.snapshot()
    }

    fn required(&self) -> Result<Ref<'_, Project>, String> {
        self.project().ok_or_else(|| "open a project first".into())
    }

    fn generated(
        &mut self,
        request: &Value,
        kind: shrimply_timeline_edit::TrackKind,
        action: shrimply_timeline_edit::TrackAddAction,
    ) -> Result<(), String> {
        use shrimply_timeline_edit::{TrackAddAction, TrackAddSettings, TrackKey};
        if action == TrackAddAction::Import {
            return Err("choose a source file before adding imported media".into());
        }
        let text = if action == TrackAddAction::Comment {
            Some(string(request, "text")?)
        } else {
            None
        };
        let (start, end) = self.interval(request)?;
        let preferences = shrimply_preferences_core::snapshot(&self.preferences);
        let item = self.edit("Add timeline item", |project| {
            let key = TrackKey {
                kind,
                track_index: target_index(project, kind, request)?,
            };
            if key.track_index == 0 && selection_state::track_address(project, key).is_none() {
                shrimply_timeline_edit::edit::create_track(
                    project,
                    &document::SequenceScopeId::root(),
                    kind,
                    true,
                )?;
            }
            let settings = TrackAddSettings {
                default_visual_duration: end.saturating_sub(start),
                default_text_font_family: &preferences.default_text_font_family,
            };
            let item = shrimply_timeline_edit::insert_track_add(project, key, action, start, settings, text)?;
            item.ok_or_else(|| "the destination is occupied or the track no longer exists; choose a gap before adding an item".to_owned())
        })?;
        selection_state::select_item_address(&self.selection, &*self.required()?, item);
        Ok(())
    }

    pub fn position(&self, request: &Value) -> Result<Time, String> {
        let project = self.required()?;
        if let Some(position) = request.get("position").filter(|value| !value.is_null()) {
            let position: Time = serde_json::from_value(position.clone())
                .map_err(|error| format!("invalid insertion position: {error}"))?;
            if !shrimply_math_core::fraction_is_finite(position.seconds) || position < Time::ZERO {
                return Err("insertion position must be finite and nonnegative".into());
            }
            return Ok(position);
        }
        let start = request
            .get("start_frame")
            .map(|value| {
                value
                    .as_u64()
                    .ok_or("start_frame must be a nonnegative integer")
            })
            .transpose()?
            .map(|frame| {
                shrimply_math_core::time_from_frame(frame, project.fps)
                    .ok_or("start_frame exceeds timeline range")
            })
            .transpose()?
            .unwrap_or(project.cursor_position.unwrap_or(Time::ZERO));
        Ok(start)
    }

    fn interval(&self, request: &Value) -> Result<(Time, Time), String> {
        let project = self.required()?;
        let start = self.position(request)?;
        let duration = request
            .get("duration_frames")
            .map(|value| {
                value
                    .as_u64()
                    .ok_or("duration_frames must be a positive integer")
            })
            .transpose()?
            .map(|frame| {
                shrimply_math_core::time_from_frame(frame, project.fps)
                    .ok_or("duration_frames exceeds timeline range")
            })
            .transpose()?
            .unwrap_or_else(|| {
                shrimply_preferences_core::snapshot(&self.preferences).default_visual_duration
            });
        if duration <= Time::ZERO {
            return Err("item duration must be positive".into());
        }
        Ok((start, start.saturating_add(duration)))
    }

    fn edit<T>(
        &mut self,
        message: &str,
        update: impl FnOnce(&mut Project) -> Result<T, String>,
    ) -> Result<T, String> {
        // The shared transaction persists before advancing the shared undo cursor
        // or exposing the new model. Failed edits retain the previous project.
        // An external save may precede its queued native watch notification.
        self.reload()?;
        let mut candidate = self.required()?.clone();
        let value = update(&mut candidate)?;
        candidate.ensure_ids();
        document::commit_edit_saved(&candidate, message)?;
        *self.project.as_ref().expect("open project").borrow_mut() = candidate;
        self.observe_saved()?;
        self.revision += 1;
        Ok(value)
    }

    fn save_as(&mut self, path: PathBuf) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            if self.path.as_ref() == Some(&path) {
                return Ok(());
            }
            let old_path = self.path.clone().ok_or("open a project first")?;
            let mirror = self.mirror_root.clone();
            let forced_version = self.forced_version.clone();
            attached::save_copy(&path, &*self.required()?)?;
            self.close()?;
            let new_mirror = mirror
                .as_deref()
                .map(Path::new)
                .filter(|root| path.starts_with(root));
            if let Err(error) = self.open(&path, new_mirror) {
                self.open(&old_path, mirror.as_deref().map(Path::new))?;
                self.forced_version = forced_version;
                return Err(error);
            }
            Ok(())
        }
        #[cfg(not(target_os = "android"))]
        {
            document::save_as(&path)?;
            self.mirror_root = self
                .mirror_root
                .take()
                .filter(|root| path.starts_with(root));
            self.path = Some(path);
            self.observe_saved()?;
            self.watch()?;
            self.register_project()
        }
    }

    fn open(&mut self, path: &Path, mirror_root: Option<&Path>) -> Result<(), String> {
        if self.project.is_some() {
            return Err("close the current project before opening another project".into());
        }
        let path = path
            .canonicalize()
            .map_err(|error| format!("could not open {}: {error}", path.display()))?;
        let mirror_root = mirror_root
            .map(Path::canonicalize)
            .transpose()
            .map_err(|error| error.to_string())?;
        if mirror_root
            .as_ref()
            .is_some_and(|root| !root.is_dir() || !path.starts_with(root))
        {
            return Err("project is outside the selected source folder".into());
        }
        let mirror_root = mirror_root
            .map(|root| {
                root.into_os_string()
                    .into_string()
                    .map_err(|_| "the mirror root must be a UTF-8 path")
            })
            .transpose()?;
        if mirror_root.is_some() && self.mirror_listener.is_none() {
            return Err("the document-provider mirror has no storage listener".into());
        }
        #[cfg(target_os = "android")]
        let project = {
            let remote = SharedWorkspace::open(&path)?.is_some();
            let owner = embedded::Lease::open(&path)?;
            let project = attached::load(&path)?;
            attached::activate(&path, &project);
            self.owner = Some(owner);
            self.remote = remote;
            project
        };
        #[cfg(not(target_os = "android"))]
        let project = {
            let prepared = document::prepare_project(&path).map_err(|error| match error {
                document::ProjectLoadError::LockedByOtherInstance { owner } => {
                    format!("project is already open in process {}", owner.pid())
                }
                document::ProjectLoadError::Other(message) => message,
            })?;
            document::activate_project(prepared)
        };
        self.project = Some(Rc::new(RefCell::new(project)));
        self.path = Some(path);
        self.mirror_root = mirror_root;
        self.revision += 1;
        if let Err(error) = self
            .observe_saved()
            .and_then(|_| self.watch())
            .and_then(|_| self.register_project())
        {
            self.close()?;
            return Err(error);
        }
        // Catch an external save between the canonical read and watcher setup.
        // Once watching, later changes remain queued even if this read races.
        if let Err(error) = self.reload() {
            self.error = Some(error);
        }
        Ok(())
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.required()?;
        self.reload()?;
        if self.view_dirty {
            document::save_view_state(&*self.required()?);
        }
        document::flush_history()?;
        #[cfg(target_os = "android")]
        attached::flush_checkpoint(self.path.as_ref().expect("open project has a path"))?;
        self.observe_saved()?;
        document::poll_commit_status();
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), String> {
        if self.project.is_none() {
            return Ok(());
        }
        self.save()?;
        document::shutdown_history()?;
        self.watcher = None;
        self.mirror_root = None;
        #[cfg(target_os = "android")]
        {
            self.owner = None;
            self.remote = false;
            self.forced_version = None;
        }
        #[cfg(not(target_os = "android"))]
        document::release_project_lock(self.path.as_ref().expect("open project has a path"));
        self.project = None;
        self.selection = selection_state::new();
        self.path = None;
        self.fingerprint = None;
        self.error = None;
        self.view_dirty = false;
        self.revision += 1;
        Ok(())
    }

    fn observe_saved(&mut self) -> Result<(), String> {
        #[cfg(not(target_os = "android"))]
        {
            self.fingerprint = Some(
                document::saved_project_fingerprint()
                    .ok_or("the active document has no canonical I/O receipt")?,
            );
        }
        self.error = None;
        self.view_dirty = false;
        document::poll_commit_status();
        Ok(())
    }

    fn register_project(&self) -> Result<(), String> {
        let project = self.required()?;
        shrimply_project_service::catalog::register(
            self.path.as_ref().expect("open path"),
            &project,
        )
    }

    fn watch(&mut self) -> Result<(), String> {
        self.watcher = None;
        if (self.active || self.mirror_root.is_some())
            && let Some(path) = &self.path
        {
            self.watcher = Some(watch::Watch::new(
                path,
                self.mirror_root
                    .as_deref()
                    .zip(self.mirror_listener.clone()),
                self.wake.clone(),
            )?);
        }
        Ok(())
    }

    pub fn changed(&mut self) -> Result<bool, String> {
        let flags = self.watcher.as_ref().map_or(0, watch::Watch::take);
        let mut changed = false;
        if flags & watch::PROJECT != 0 {
            match self.reload() {
                Ok(_) => changed = true,
                Err(message) => {
                    self.error = Some(message.clone());
                    return Err(message);
                }
            }
        }
        if flags & watch::ASSETS != 0 && self.project.is_some() {
            self.revision += 1;
            changed = true;
        }
        Ok(changed)
    }

    fn reload(&mut self) -> Result<bool, String> {
        let path = self.path.as_ref().ok_or("open a project first")?.clone();
        let current = document::checkpoint_fingerprint(&path)?;
        if self.fingerprint.as_ref() == Some(&current) {
            return Ok(false);
        }
        #[cfg(target_os = "android")]
        let (candidate, loaded_fingerprint) = {
            let started = std::time::Instant::now();
            let Some(mut candidate) = attached::reload_checkpoint(&path)? else {
                self.fingerprint = Some(current);
                return Ok(false);
            };
            candidate.copy_view_state_from(&*self.required()?);
            document::adopt_attached_snapshot(&candidate, started)?;
            (candidate, current)
        };
        #[cfg(not(target_os = "android"))]
        let (candidate, loaded_fingerprint) = {
            let Some((mut candidate, loaded_fingerprint)) =
                document::changed_checkpoint(&path, &current)?
            else {
                self.fingerprint = Some(current);
                return Ok(false);
            };
            if self.view_dirty {
                let project = self.required()?;
                candidate.cursor_position = project
                    .cursor_position
                    .map(|cursor| cursor.clamp(Time::ZERO, candidate.duration()));
                candidate.timeline_zoom = project.timeline_zoom;
            }
            document::adopt_project_snapshot(&candidate, &loaded_fingerprint)?;
            (candidate, loaded_fingerprint)
        };
        *self.project.as_ref().expect("open project").borrow_mut() = candidate;
        self.fingerprint = Some(loaded_fingerprint);
        self.error = None;
        self.revision += 1;
        self.register_project()?;
        Ok(true)
    }

    pub fn active(&mut self, active: bool) -> Result<(), String> {
        if !active && self.project.is_some() {
            self.save()?;
        }
        self.active = active;
        if self.mirror_root.is_none() {
            self.watch()?;
        }
        if active && self.project.is_some() {
            self.reload()?;
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Value, String> {
        document::poll_commit_status();
        #[cfg(target_os = "android")]
        let remote = self.remote;
        #[cfg(not(target_os = "android"))]
        let remote = false;
        let project = self.project().map(|project| json!({
            "path":self.path, "remote":remote, "name":project.name, "tags":project.tags,
            "canvas_width":project.canvas_size.width, "canvas_height":project.canvas_size.height,
            "fps":project.fps.to_string(), "duration_frames":project.duration().as_frame_ceil(project.fps),
            "cursor_frame":project.cursor_position.unwrap_or(Time::ZERO).as_frame(project.fps),
            "track_count":project.comment_tracks.len()+project.caption_tracks.len()+project.video_tracks.len()+project.audio_tracks.len(),
            "visual_clip_count":project.video_tracks.iter().map(|track|track.items.len()).sum::<usize>(),
        }));
        let (state, save_error) = match &*self.status.borrow() {
            document::CommitStatus::SavePending => ("pending", None),
            document::CommitStatus::Saving | document::CommitStatus::InProgress(_) => {
                ("saving", None)
            }
            document::CommitStatus::SaveFailed(message) => ("failed", Some(message.clone())),
            document::CommitStatus::Idle => ("saved", None),
        };
        let can_undo = project.is_some() && document::can_undo();
        let can_redo = project.is_some() && document::can_redo();
        let menu = project.as_ref().map(|_| {
            project_menu::document(|action| menu_effect(action).is_some(), can_undo, can_redo)
        });
        Ok(json!({"project":project,"project_menu":menu,
            "project_servers":shrimply_preferences_core::snapshot(&self.preferences).project_server_urls,
            "save_state":state,"save_error":save_error,"watch_error":self.error,"revision":self.revision}))
    }

    pub fn stage_picked_file(
        &self,
        request: &Value,
        source: Option<fs::File>,
    ) -> Result<shrimply_media_import::StagedSource, String> {
        let source_path = Path::new(
            request["path"]
                .as_str()
                .ok_or("file selection requires a path")?,
        );
        let name = request["name"]
            .as_str()
            .or_else(|| source_path.file_name().and_then(|name| name.to_str()))
            .ok_or("file selection requires a name")?;
        let directory = self
            .path()
            .ok_or("open a project first")?
            .parent()
            .expect("project directory");
        shrimply_media_import::stage_source(source_path, source, name, directory)
    }

    pub fn import_file(
        &mut self,
        request: &Value,
        source_file: Option<fs::File>,
    ) -> Result<Value, String> {
        use shrimply_timeline_edit::TrackKind;
        let source = Path::new(string(request, "path")?);
        let name = request
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| source.file_name().and_then(|name| name.to_str()))
            .ok_or("imported file needs a name")?;
        if Path::new(name).file_name().and_then(|name| name.to_str()) != Some(name) {
            return Err("imported file name must not contain a directory".into());
        }
        let file_kind =
            shrimply_media_import::file_kind(Path::new(name)).ok_or("unsupported file type")?;
        let remux = file_kind == shrimply_media_import::FileKind::RemuxToMp4;
        let destinations: Vec<&Value> = if let Some(targets) = request
            .get("import_targets")
            .filter(|value| !value.is_null())
        {
            targets
                .as_array()
                .filter(|targets| !targets.is_empty())
                .ok_or("import targets must be a nonempty list")?
                .iter()
                .collect()
        } else {
            request.get("kind").map(|_| request).into_iter().collect()
        };
        for target in &destinations {
            let kind = track_kind(string(target, "kind")?)?;
            let index = target_index(&*self.required()?, kind, target)?;
            if index > 0
                && shrimply_timeline_edit::selection_state::track_address(
                    &*self.required()?,
                    shrimply_timeline_edit::TrackKey {
                        kind,
                        track_index: index,
                    },
                )
                .is_none()
            {
                return Err("the destination track no longer exists".into());
            }
        }
        let (start, end) = self.interval(request)?;
        if remux && request["remux_confirmed"] != true {
            let mut prompt = remux_confirmation();
            prompt["position"] = json!(start);
            return Ok(prompt);
        }
        let mut staged = self.stage_picked_file(request, source_file)?;
        let mut inspected_snapshot = None;
        let project_path = self.path.as_ref().expect("open project path").clone();
        let result = (|| {
            let original =
                inspected_snapshot.insert(shrimply_asset::Asset::new(&staged.path).snapshot()?);
            if remux {
                let output = shrimply_media_import::remux_to_mp4(&staged.path, Default::default())?;
                original.verify_current()?;
                let inspected = shrimply_asset::Asset::new(output.path()).snapshot()?;
                // Only the descriptor copy is ours. A pre-existing project source is retained.
                self.discard_staged(&staged, Some(original))?;
                staged = shrimply_media_import::StagedSource {
                    path: output.path().to_owned(),
                    created: true,
                };
                inspected_snapshot = Some(inspected);
                output.keep();
            }
            self.edit("Import file", |project| {
                use shrimply_editing_api::{imports, preview};
                let mut targets = Vec::new();
                for target in &destinations {
                    let kind = track_kind(string(target, "kind")?)?;
                    if kind == TrackKind::Comment {
                        return Err("media destination must be caption, video, or audio".into());
                    }
                    let key = shrimply_timeline_edit::TrackKey {
                        kind,
                        track_index: target_index(project, kind, target)?,
                    };
                    if key.track_index == 0
                        && shrimply_timeline_edit::selection_state::track_address(project, key)
                            .is_none()
                    {
                        shrimply_timeline_edit::edit::create_track(
                            project,
                            &document::SequenceScopeId::root(),
                            kind,
                            true,
                        )?;
                    }
                    let address =
                        shrimply_timeline_edit::selection_state::track_address(project, key)
                            .ok_or("the destination track no longer exists")?;
                    targets.push(address);
                }
                let collision = if targets.is_empty() { "new_track" } else { "reject" };
                let frame = start.as_frame(project.fps);
                let request = preview::edit_request("insert_files", json!({
                    "files":[{"source":staged.path,"targets":targets}], "frame":frame, "link":true, "collision":collision,
                }))?.expect("insert_files is a shared edit operation");
                let mut prepared = imports::prepare(
                    project.clone(),
                    &request,
                    frame,
                    document::SequenceScopeId::root(),
                    end.saturating_sub(start),
                    &project_path,
                )?;
                prepared.promote()?;
                *project = prepared.project;
                Ok(())
            })
        })();
        if let Err(error) = &result {
            self.discard_staged(&staged, inspected_snapshot.as_ref())
                .map_err(|cleanup| format!("{error}; {cleanup}"))?;
        }
        result?;
        self.snapshot()
    }

    pub fn discard_staged(
        &self,
        staged: &shrimply_media_import::StagedSource,
        snapshot: Option<&shrimply_asset::AssetSnapshot>,
    ) -> Result<(), String> {
        if !staged.created {
            return Ok(());
        }
        // A save can fail after commit, or another writer can replace the file.
        // Only the unreferenced, unchanged file owned by this operation is ours.
        if self
            .required()?
            .assets()
            .iter()
            .any(|asset| asset.path() == staged.path)
            || snapshot.is_some_and(|snapshot| snapshot.verify_current().is_err())
        {
            return Err(
                "imported path was retained because it is now referenced or changed externally"
                    .into(),
            );
        }
        fs::remove_file(&staged.path).map_err(|error| {
            format!(
                "could not remove failed import {}: {error}",
                staged.path.display()
            )
        })
    }
}

pub(super) fn remux_confirmation() -> Value {
    let prompt = shrimply_timeline_skia::external_content::remux_prompt();
    json!({"confirmation": {"title": prompt.title, "body": prompt.body,
        "confirm_label": prompt.confirm, "cancel_label": prompt.cancel}})
}

impl Drop for ProjectBackend {
    fn drop(&mut self) {
        self.close()
            .expect("flush and close Android project before releasing its owner");
    }
}

#[cfg(target_os = "android")]
fn check_server_version(server: &str, request: &Value) -> Result<Option<VersionMismatch>, Value> {
    let forced = &request["force_version"];
    let Some(prompt) = VersionPrompt::remote(server) else {
        if forced["server"] == server
            && let Ok(service) = serde_json::from_value(forced["version"].clone())
        {
            return Ok(Some(VersionMismatch::Remote {
                server: server.to_owned(),
                service,
            }));
        }
        return Ok(None);
    };
    let VersionMismatch::Remote { server, service } = &prompt.mismatch else {
        unreachable!("remote probe returns a remote mismatch");
    };
    let identity = json!({ "server": server, "version": service });
    if *forced == identity {
        Ok(Some(prompt.mismatch))
    } else {
        Err(json!({
            "version_mismatch": { "body": prompt.to_string(), "identity": identity }
        }))
    }
}

fn string<'a>(request: &'a Value, key: &str) -> Result<&'a str, String> {
    request[key]
        .as_str()
        .ok_or_else(|| format!("'{key}' must be a string"))
}

pub(super) fn operation(request: &Value) -> Result<&str, String> {
    let operation = string(request, "op")?;
    if operation == "project_menu" {
        let action = menu_action(request)?;
        if action.history().is_some() {
            return Ok(action.id());
        }
    }
    Ok(operation)
}

fn menu_effect(action: Action) -> Option<&'static str> {
    Some(match action {
        Action::Save => "flush",
        Action::Preferences => "show_preferences",
        Action::Import => "pick_media",
        Action::Close => "close_workspace",
        _ => return None,
    })
}

fn menu_action(request: &Value) -> Result<Action, String> {
    Action::from_id(string(request, "action")?).ok_or_else(|| "unknown project menu action".into())
}

fn track_kind(name: &str) -> Result<shrimply_timeline_edit::TrackKind, String> {
    use shrimply_timeline_edit::TrackKind;
    match name {
        "comment" => Ok(TrackKind::Comment),
        "caption" => Ok(TrackKind::Caption),
        "video" => Ok(TrackKind::Video),
        "audio" => Ok(TrackKind::Audio),
        _ => Err("track kind must be comment, caption, video, or audio".into()),
    }
}

pub(super) fn target_index(
    project: &Project,
    kind: shrimply_timeline_edit::TrackKind,
    request: &Value,
) -> Result<usize, String> {
    if let Some(id) = request.get("track_id") {
        let id: uuid::Uuid = id
            .as_str()
            .ok_or("track_id must be a UUID string")?
            .parse()
            .map_err(|_| "track_id must be a valid UUID")?;
        let index = project.root_track_index(kind, id);
        return index.ok_or_else(|| {
            "the selected track no longer exists; choose its destination again".into()
        });
    }
    usize::try_from(
        request
            .get("track")
            .map(|value| value.as_u64().ok_or("track must be a nonnegative index"))
            .transpose()?
            .unwrap_or(0),
    )
    .map_err(|_| "track index is too large".into())
}

fn settings_schema() -> Value {
    let defaults = Project::default();
    json!({
        "defaults":{"name":defaults.name,"canvas_width":defaults.canvas_size.width,"canvas_height":defaults.canvas_size.height,"fps":defaults.fps.to_string(),"tags":defaults.tags},
        "min_canvas_dimension":document::MIN_CANVAS_DIMENSION,
        "max_canvas_dimension":document::MAX_CANVAS_DIMENSION,
        "presets":document::PROJECT_PRESETS.iter().map(|preset|json!({"label":preset.label,"canvas_width":preset.canvas_size.width,"canvas_height":preset.canvas_size.height,"fps":preset.fps.to_string()})).collect::<Vec<_>>(),
        "track_add":([("comment",shrimply_timeline_edit::TrackKind::Comment),("caption",shrimply_timeline_edit::TrackKind::Caption),("video",shrimply_timeline_edit::TrackKind::Video),("audio",shrimply_timeline_edit::TrackKind::Audio)].into_iter().map(|(kind_name,kind)| {
            let entries = shrimply_timeline_edit::track_add_menu(kind).iter().map(|entry|match entry {
                shrimply_timeline_edit::TrackAddMenuEntry::Separator => json!({"separator":true}),
                shrimply_timeline_edit::TrackAddMenuEntry::Action(action) => json!({"action":format!("{action:?}"),"label":action.label(kind),"icon":action.icon(),"needs_file":*action==shrimply_timeline_edit::TrackAddAction::Import,"needs_text":*action==shrimply_timeline_edit::TrackAddAction::Comment}),
            }).collect::<Vec<_>>();
            (kind_name,entries)
        }).collect::<std::collections::BTreeMap<_,_>>()),
        "frame_rates":document::project::frame_rate_choices(defaults.fps).into_iter().map(|(value,label)|json!({"label":label,"value":value.to_string()})).collect::<Vec<_>>()
    })
}

#[cfg(test)]
pub(crate) mod tests;
