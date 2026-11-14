//! The Android adapter owns the same validated Project and history as the desktop.
//! Invoke this service on one native worker thread, never the Android UI thread.
mod watch;
use serde_json::{Value, json};
use shrimply_math_core::Time;
use shrimply_project_document::{self as document, Project};
use std::{
    cell::{Ref, RefCell},
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

pub(crate) struct ProjectBackend {
    project: Option<Rc<RefCell<Project>>>,
    path: Option<PathBuf>,
    watcher: Option<watch::Watch>,
    pub(crate) wake: Arc<dyn Fn() + Send + Sync>,
    fingerprint: Option<String>,
    status: Rc<RefCell<document::CommitStatus>>,
    error: Option<String>,
    active: bool,
    pub revision: u64,
    view_dirty: bool,
    recents: Vec<shrimply_recent_projects::RecentProject>,
    pub preferences: shrimply_preferences_core::SharedPreferences,
}
impl ProjectBackend {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        let status = Rc::new(RefCell::new(document::CommitStatus::Idle));
        let observed = status.clone();
        document::connect_commit_status(move |value| *observed.borrow_mut() = value);
        Ok(Self {
            project: None,
            path: None,
            watcher: None,
            wake: Arc::new(wake),
            fingerprint: None,
            status,
            error: None,
            active: true,
            revision: 0,
            view_dirty: false,
            recents: shrimply_recent_projects::load()?,
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
        match string(request, "op")? {
            "status" | "recents" => {}
            "import_probe" => {
                shrimply_media_import::file_kind(Path::new(string(request, "name")?))
                    .ok_or("unsupported file type")?;
                return Ok(json!({}));
            }
            "create" => {
                if self.project.is_some() {
                    return Err("close the current project before creating another project".into());
                }
                let path = PathBuf::from(string(request, "path")?);
                string(request, "name")?;
                let mut project = Project::default();
                apply_settings(&mut project, request)?;
                project.validate()?;
                fs::create_dir_all(path.parent().ok_or("project path requires a directory")?)
                    .map_err(|error| format!("could not create project directory: {error}"))?;
                document::create_new_project_file(&path, &project)?;
                self.open(&path)?;
            }
            "settings_schema" => return Ok(settings_schema()),
            "settings" => {
                self.edit("Change project settings", |project| {
                    apply_settings(project, request)?;
                    if request["repair_frame_grid"] == true {
                        project.repair_frame_grid();
                    }
                    Ok(())
                })?;
                self.touch_recent()?;
            }
            "open" => self.open(Path::new(string(request, "path")?))?,
            "close" => self.close()?,
            "save" => self.save()?,
            "save_as" => {
                self.save()?;
                let path = std::path::absolute(string(request, "path")?)
                    .map_err(|error| error.to_string())?;
                document::save_as(&path)?;
                self.path = Some(path);
                self.observe_saved()?;
                self.watch()?;
                self.touch_recent()?;
            }
            "export" => {
                self.save()?;
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
                let redo = request["op"] == "redo";
                self.reload()?;
                let mut candidate = self.required()?.clone();
                if document::change_history_saved(&mut candidate, redo)? {
                    *self.project.as_ref().expect("open project").borrow_mut() = candidate;
                    self.observe_saved()?;
                    self.revision += 1;
                }
            }
            "add_track" => {
                let kind = track_kind(string(request, "kind")?)?;
                self.edit("Add track", |project| {
                    shrimply_timeline_edit::edit::create_track(
                        project,
                        &document::SequenceScopeId::root(),
                        kind,
                        true,
                    )?;
                    Ok(())
                })?;
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
            "remove_recent" => {
                shrimply_recent_projects::remove(Path::new(string(request, "path")?))?;
                self.recents = shrimply_recent_projects::load()?;
            }
            "clear_recents" => {
                shrimply_recent_projects::clear()?;
                self.recents.clear();
            }
            "import_file" => self.import_file(request, None)?,
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
            Some(string(request, "text")?.to_owned())
        } else {
            None
        };
        let (start, end) = self.interval(request)?;
        let preferences = shrimply_preferences_core::snapshot(&self.preferences);
        self.edit("Add timeline item", |project| {
            let index = target_index(project, kind, request)?;
            if index == 0 && shrimply_timeline_edit::selection_state::track_address(project, TrackKey { kind, track_index: index }).is_none() {
                shrimply_timeline_edit::edit::create_track(project, &document::SequenceScopeId::root(), kind, true)?;
            }
            let settings = TrackAddSettings { default_visual_duration:end.saturating_sub(start), default_text_font_family:&preferences.default_text_font_family };
            let (address, mut item) = shrimply_timeline_edit::prepare_track_add(project, TrackKey {kind,track_index:index}, action, start, settings)?.ok_or("the destination is occupied or the track no longer exists; choose a gap before adding an item")?;
            if let document::ProjectItem::Comment(comment) = &mut item { comment.text = text.expect("comment action requires text"); }
            project.insert_item(&address, item).ok_or("the destination changed while adding the item")?;
            Ok(())
        })
    }

    fn interval(&self, request: &Value) -> Result<(Time, Time), String> {
        let project = self.required()?;
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

    fn edit(
        &mut self,
        message: &str,
        update: impl FnOnce(&mut Project) -> Result<(), String>,
    ) -> Result<(), String> {
        // The shared transaction persists before advancing the shared undo cursor
        // or exposing the new model. Failed edits retain the previous project.
        // An external save may precede its queued native watch notification.
        self.reload()?;
        let mut candidate = self.required()?.clone();
        update(&mut candidate)?;
        candidate.ensure_ids();
        document::commit_edit_saved(&candidate, message)?;
        *self.project.as_ref().expect("open project").borrow_mut() = candidate;
        self.observe_saved()?;
        self.revision += 1;
        Ok(())
    }

    fn open(&mut self, path: &Path) -> Result<(), String> {
        if self.project.is_some() {
            return Err("close the current project before opening another project".into());
        }
        let path = path
            .canonicalize()
            .map_err(|error| format!("could not open {}: {error}", path.display()))?;
        let prepared = document::prepare_project(&path).map_err(|error| match error {
            document::ProjectLoadError::LockedByOtherInstance { owner } => {
                format!("project is already open in process {}", owner.pid())
            }
            document::ProjectLoadError::Other(message) => message,
        })?;
        let project = document::activate_project(prepared);
        self.project = Some(Rc::new(RefCell::new(project)));
        self.path = Some(path);
        self.revision += 1;
        if let Err(error) = self
            .observe_saved()
            .and_then(|_| self.watch())
            .and_then(|_| self.touch_recent())
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
        document::release_project_lock(self.path.as_ref().expect("open project has a path"));
        self.project = None;
        self.path = None;
        self.fingerprint = None;
        self.error = None;
        self.view_dirty = false;
        self.revision += 1;
        Ok(())
    }

    fn observe_saved(&mut self) -> Result<(), String> {
        self.fingerprint = Some(
            document::saved_project_fingerprint()
                .ok_or("the active document has no canonical I/O receipt")?,
        );
        self.error = None;
        self.view_dirty = false;
        document::poll_commit_status();
        Ok(())
    }

    fn touch_recent(&mut self) -> Result<(), String> {
        let project = self.required()?;
        shrimply_recent_projects::touch(
            self.path.as_ref().expect("open path"),
            &project.name,
            &project.tags,
        )?;
        drop(project);
        self.recents = shrimply_recent_projects::load()?;
        Ok(())
    }

    fn watch(&mut self) -> Result<(), String> {
        self.watcher = None;
        if self.active
            && let Some(path) = &self.path
        {
            self.watcher = Some(watch::Watch::new(path, self.wake.clone())?);
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
        let current = fingerprint(&path)?;
        if self.fingerprint.as_ref() == Some(&current) {
            return Ok(false);
        }
        // Shared Scene edits commit through the same document history. Its I/O
        // receipt distinguishes our own completed save from an external edit.
        if document::saved_project_fingerprint().as_ref() == Some(&current) {
            self.fingerprint = Some(current);
            return Ok(false);
        }
        // Validate before touching the live document or its history worker. A
        // partially written/broken external project keeps the last good state.
        let (mut candidate, loaded_fingerprint) =
            document::load_project_snapshot_with_fingerprint(&path)?;
        if fingerprint(&path)? != loaded_fingerprint {
            return Err(
                "project changed again while loading; waiting for its next complete save".into(),
            );
        }
        if self.view_dirty {
            let project = self.required()?;
            candidate.cursor_position = project
                .cursor_position
                .map(|cursor| cursor.clamp(Time::ZERO, candidate.duration()));
            candidate.timeline_zoom = project.timeline_zoom;
        }
        document::adopt_project_snapshot(&candidate, &loaded_fingerprint)?;
        *self.project.as_ref().expect("open project").borrow_mut() = candidate;
        self.fingerprint = Some(loaded_fingerprint);
        self.error = None;
        self.revision += 1;
        self.touch_recent()?;
        Ok(true)
    }

    pub fn active(&mut self, active: bool) -> Result<(), String> {
        if !active && self.project.is_some() {
            self.save()?;
        }
        self.active = active;
        self.watch()?;
        if active && self.project.is_some() {
            self.reload()?;
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Value, String> {
        document::poll_commit_status();
        let project = self.project().map(|project| json!({
            "path":self.path, "name":project.name, "tags":project.tags,
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
        Ok(
            json!({"project":project,"can_undo":self.project.is_some()&&document::can_undo(),"can_redo":self.project.is_some()&&document::can_redo(),"save_state":state,"save_error":save_error,"watch_error":self.error,"revision":self.revision,"recents":self.recents}),
        )
    }

    pub fn import_descriptor(
        &mut self,
        request: &Value,
        source: fs::File,
    ) -> Result<Value, String> {
        self.validate_target(request)?;
        self.import_file(request, Some(source))?;
        self.snapshot()
    }

    fn import_file(
        &mut self,
        request: &Value,
        source_file: Option<fs::File>,
    ) -> Result<(), String> {
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
        shrimply_media_import::file_kind(Path::new(name)).ok_or("unsupported file type")?;
        if let Some(kind) = request.get("kind").and_then(Value::as_str) {
            let kind = track_kind(kind)?;
            let index = target_index(&*self.required()?, kind, request)?;
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
        let project_directory = self
            .path
            .as_ref()
            .ok_or("open a project first")?
            .parent()
            .expect("project directory");
        // Reusing a project-local source preserves its sibling imports/data.
        // Provider descriptors outside that tree become owned media files.
        let canonical_source = if source_file.is_none() {
            source.canonicalize().ok()
        } else {
            None
        };
        let directory = canonical_source
            .as_ref()
            .filter(|source| source.starts_with(project_directory))
            .and_then(|source| source.parent())
            .map(Path::to_owned)
            .unwrap_or_else(|| project_directory.join("media"));
        fs::create_dir_all(&directory)
            .map_err(|error| format!("could not create project media directory: {error}"))?;
        let destination = directory.join(name);
        let already_staged = canonical_source
            .as_ref()
            .is_some_and(|source| destination.canonicalize().ok().as_ref() == Some(source));
        if !already_staged {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&destination)
                .map_err(|error| {
                    format!(
                        "could not create imported asset {}: {error}",
                        destination.display()
                    )
                })?;
            let copied = source_file
                .map(Ok)
                .unwrap_or_else(|| fs::File::open(source))
                .and_then(|mut source| std::io::copy(&mut source, &mut file))
                .and_then(|_| file.sync_all());
            if let Err(error) = copied {
                drop(file);
                fs::remove_file(&destination).map_err(|cleanup| {
                    format!(
                        "could not save imported asset: {error}; could not remove {}: {cleanup}",
                        destination.display()
                    )
                })?;
                return Err(format!("could not save imported asset: {error}"));
            }
        }
        let mut inspected_snapshot = None;
        let project_path = self.path.as_ref().expect("open project path").clone();
        let result = (|| {
            inspected_snapshot = Some(shrimply_asset::Asset::new(&destination).snapshot()?);
            self.edit("Import file", |project| {
                use shrimply_editing_api::{imports, preview};
                let mut targets = Vec::new();
                if let Some(kind) = request.get("kind").and_then(Value::as_str) {
                    let kind = track_kind(kind)?;
                    if kind == TrackKind::Comment {
                        return Err("media destination must be caption, video, or audio".into());
                    }
                    let key = shrimply_timeline_edit::TrackKey {
                        kind,
                        track_index: target_index(project, kind, request)?,
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
                    targets.push(shrimply_editing_api::query::protocol_track_address(
                        &address,
                    ));
                }
                let collision = if targets.is_empty() { "new_track" } else { "reject" };
                let frame = start.as_frame(project.fps);
                let request = preview::edit_request("insert_files", json!({
                    "files":[{"source":destination,"targets":targets}], "frame":frame, "link":true, "collision":collision,
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
        if let Err(error) = &result
            && !already_staged
        {
            // Reload can discover a newer external project during the import.
            // Never roll back an asset that it now references, or a replacement
            // written by somebody else after we inspected our owned source.
            if self
                .required()?
                .assets()
                .iter()
                .any(|asset| asset.path() == destination)
                || inspected_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.verify_current().is_err())
            {
                return Err(format!(
                    "{error}; imported path was retained because it is now referenced or changed externally"
                ));
            }
            fs::remove_file(&destination).map_err(|cleanup| {
                format!(
                    "{error}; could not remove failed import {}: {cleanup}",
                    destination.display()
                )
            })?;
        }
        result
    }
}

impl Drop for ProjectBackend {
    fn drop(&mut self) {
        self.close()
            .expect("flush and close Android project before releasing its owner");
    }
}

fn string<'a>(request: &'a Value, key: &str) -> Result<&'a str, String> {
    request[key]
        .as_str()
        .ok_or_else(|| format!("'{key}' must be a string"))
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

fn apply_settings(project: &mut Project, request: &Value) -> Result<(), String> {
    if let Some(name) = request.get("name") {
        let name = name.as_str().ok_or("project name must be a string")?.trim();
        if name.is_empty() {
            return Err("project name cannot be empty".into());
        }
        project.name = name.into();
    }
    let frame_rate = request
        .get("fps")
        .map(|value| {
            value
                .as_str()
                .ok_or("fps must be an exact string, for example 30 or 30000/1001")?
                .parse()
                .map_err(|_| "invalid exact project frame rate")
        })
        .transpose()?
        .unwrap_or(project.fps);
    let mut canvas_size = project.canvas_size;
    for (key, dimension) in [
        ("canvas_width", &mut canvas_size.width),
        ("canvas_height", &mut canvas_size.height),
    ] {
        if let Some(value) = request.get(key) {
            *dimension = u32::try_from(
                value
                    .as_u64()
                    .ok_or_else(|| format!("{key} must be a positive integer"))?,
            )
            .map_err(|_| format!("{key} is too large"))?;
        }
    }
    project.set_video_settings(canvas_size, frame_rate)?;
    if let Some(value) = request.get("tags") {
        let tags: Vec<String> =
            serde_json::from_value(value.clone()).map_err(|_| "tags must be a list of strings")?;
        project.tags = document::normalize_tags(&tags);
    }
    Ok(())
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
        "frame_rates":document::COMMON_FRAME_RATES.iter().map(|rate|json!({"label":rate.label,"value":rate.value.to_string()})).collect::<Vec<_>>()
    })
}

fn fingerprint(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let mut hash = shrimply_resource_pipeline::fingerprint::Writer::default();
    std::io::copy(&mut file, &mut hash)
        .map_err(|error| format!("could not read project: {error}"))?;
    Ok(hash.finish())
}

#[cfg(test)]
pub(crate) mod tests;
