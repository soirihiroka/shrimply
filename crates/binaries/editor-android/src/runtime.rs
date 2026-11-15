//! One project and playback owner, independent of Compose, JNI and Vulkan.
use crate::project::ProjectBackend;
use serde_json::{Value, json};
use shrimply_math_core::{Time, time_from_frame};
use shrimply_player_core::{self as player, SharedPlayerState};
use shrimply_project_document::{Project, time_format};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
mod context;
mod context_media;
#[cfg(target_os = "android")]
mod export;
mod graph;
mod inspector;
pub(crate) mod palette;
mod preview;
#[cfg(target_os = "android")]
mod preview_image;
mod video_recording;

#[derive(Default)]
pub(crate) struct Lifecycle {
    pause: Option<Instant>,
    latest: Option<(bool, Instant)>,
}
pub(crate) type SharedLifecycle = Arc<Mutex<Lifecycle>>;
impl Lifecycle {
    pub fn observe(&mut self, active: bool, at: Instant) {
        if !active {
            self.pause = Some(self.pause.map_or(at, |previous| previous.min(at)));
        }
        self.latest = Some((active, at));
    }
}

pub(crate) struct Runtime {
    context_menu: Option<context::Menu>,
    context_serial: u64,
    timeline_service: Option<shrimply_inspector_document::timeline_service::Session>,
    video_recording: Option<video_recording::Recording>,
    inspect_serial: u64,
    pub backend: ProjectBackend,
    #[cfg(target_os = "android")]
    wake: Arc<dyn Fn() + Send + Sync>,
    #[cfg(target_os = "android")]
    pub preview_frame: Option<shrimply_preview_render_vulkan::worker::Frame>,
    #[cfg(target_os = "android")]
    preview_image: Option<preview_image::PreviewImage>,
    pub native_palette: Option<palette::NativePalette>,
    pub player: SharedPlayerState,
    pub timeline: Option<shrimply_timeline_skia::scene::Scene>,
    inspector: Option<shrimply_inspector_core::InspectorController>,
    inspector_document: Option<shrimply_inspector_document::InspectorDocument>,
    inspector_interaction: shrimply_inspector_document::remote::Interaction,
    inspector_fonts: shrimply_inspector_document::remote::Fonts,
    inspector_status: String,
    inspector_polled_at: std::time::Instant,
    pub framegraph: Option<shrimply_inspector_document::remote::graph::GraphSession>,
    pub graph_dirty: bool,
    graph_stamp: Option<(u64, Time)>,
    graph_pointer: shrimply_framegraph_skia::FrameGraphPointerPosition,
    preferences_ui: shrimply_inspector_document::preferences::Preferences,
    preview_toolbar: shrimply_preview_render_core::playback::toolbar::Toolbar,
    pub preview_interaction: shrimply_preview_interaction_skia::surface::Surface,
    preview_size: glam::Vec2,
    preview_context: Option<preview::Menu>,
    inspector_stamp: Option<(u64, Time, shrimply_inspector_core::InspectorTarget)>,
    pub inspector_visible: bool,
    #[cfg(target_os = "android")]
    export: Option<export::Export>,
    #[cfg(target_os = "android")]
    audio: Option<std::rc::Rc<shrimply_audio_engine::AudioPlayer>>,
    #[cfg(target_os = "android")]
    audio_levels: shrimply_audio_engine::SharedAudioLevels,
    #[cfg(target_os = "android")]
    pub media: Option<
        shrimply_preview_render_core::playback::PreviewMedia<
            shrimply_preview_render_vulkan::worker::Frame,
        >,
    >,
    status: String,
    pub error: Option<String>,
    lifecycle: SharedLifecycle,
    pub foreground: bool,
    pub generation: u64,
    action: Option<Value>,
    action_serial: u64,
}
impl Runtime {
    #[cfg(test)]
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        Self::with_lifecycle(wake, SharedLifecycle::default())
    }

    pub fn with_lifecycle(
        wake: impl Fn() + Send + Sync + 'static,
        lifecycle: SharedLifecycle,
    ) -> Result<Self, String> {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let notify = wake.clone();
        let backend = ProjectBackend::new(move || notify())?;
        let status =
            serde_json::to_string(&backend.snapshot()?).map_err(|error| error.to_string())?;
        Ok(Self {
            backend,
            #[cfg(target_os = "android")]
            wake,
            #[cfg(target_os = "android")]
            preview_frame: None,
            #[cfg(target_os = "android")]
            preview_image: None,
            native_palette: None,
            player: player::new(Time::ZERO, Project::default().fps),
            status,
            timeline: None,
            inspector: None,
            inspector_document: None,
            inspector_interaction: Default::default(),
            inspector_fonts: Default::default(),
            inspector_status: "null".into(),
            inspector_polled_at: std::time::Instant::now(),
            framegraph: None,
            graph_dirty: true,
            graph_stamp: None,
            graph_pointer: shrimply_framegraph_skia::FrameGraphPointerPosition {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            preferences_ui: Default::default(),
            preview_toolbar: Default::default(),
            preview_interaction: Default::default(),
            preview_size: glam::Vec2::ZERO,
            preview_context: None,
            inspector_stamp: None,
            inspector_visible: true,
            context_menu: None,
            context_serial: 0,
            timeline_service: None,
            video_recording: None,
            inspect_serial: 0,
            #[cfg(target_os = "android")]
            export: None,
            #[cfg(target_os = "android")]
            audio: None,
            #[cfg(target_os = "android")]
            audio_levels: Arc::new(shrimply_audio_engine::AudioLevels::default()),
            #[cfg(target_os = "android")]
            media: None,
            error: None,
            lifecycle,
            foreground: true,
            generation: 0,
            action: None,
            action_serial: 0,
        })
    }

    #[cfg(target_os = "android")]
    pub fn take_audio_peaks(&self) -> [f32; 2] {
        self.audio_levels.take_peaks()
    }

    /// Keep the earliest pause even when pause/resume both arrive during a long import.
    /// Consume again after backend I/O, before any clock refresh can replace its anchor.
    pub fn synchronize_lifecycle(&mut self) -> Result<bool, String> {
        let mut changed = false;
        let mut failure = None;
        loop {
            let pending = std::mem::take(
                &mut *self
                    .lifecycle
                    .lock()
                    .expect("Android lifecycle latch poisoned"),
            );
            if pending.latest.is_none() && pending.pause.is_none() {
                break;
            }
            changed = true;
            if let Some(at) = pending.pause {
                self.cancel_graph_edit()?;
                self.cancel_numeric()?;
                self.cancel_preview_edit()?;
                let transport = player::snapshot(&self.player);
                let capture = self.screen_recording_pending();
                if !capture {
                    player::set_playing_at(&self.player, false, at);
                }
                if transport.playing
                    && player::current_time(&self.player) > transport.duration
                    && !self
                        .timeline
                        .as_ref()
                        .is_some_and(|scene| scene.is_recording())
                {
                    player::seek_time(&self.player, transport.duration);
                }
                if let Some(scene) = self.timeline.as_mut() {
                    if capture {
                        scene.suspend_while_recording();
                    } else {
                        scene.suspend();
                    }
                }
                #[cfg(target_os = "android")]
                if let Some(media) = &self.media {
                    media.set_visible(false);
                }
                self.sync_cursor();
                self.foreground = false;
                if let Err(error) = self.backend.active(false) {
                    failure.get_or_insert(error);
                }
            }
            if let Some((active, _)) = pending.latest
                && active != self.foreground
            {
                self.foreground = active;
                #[cfg(target_os = "android")]
                if let Some(media) = &self.media {
                    media.set_visible(active);
                }
                if let Err(error) = self.backend.active(active) {
                    failure.get_or_insert(error);
                }
            }
        }
        if changed {
            self.status = serde_json::to_string(&self.backend.snapshot()?)
                .map_err(|error| error.to_string())?;
        }
        if let Some(error) = failure {
            self.error = Some(error.clone());
            Err(error)
        } else {
            Ok(changed)
        }
    }

    pub fn request(&mut self, request: &Value) -> Result<Value, String> {
        self.request_with_source(request, None)
    }

    pub fn request_with_source(
        &mut self,
        request: &Value,
        source: Option<std::fs::File>,
    ) -> Result<Value, String> {
        if self.synchronize_lifecycle()? {
            self.refresh(false);
        }
        self.backend.validate_target(request)?;
        let operation = request["op"]
            .as_str()
            .ok_or("request requires an operation")?;
        if matches!(
            operation,
            "open" | "create" | "close" | "undo" | "redo" | "play" | "pause" | "step" | "seek"
        ) {
            self.cancel_graph_edit()?;
            self.cancel_numeric()?;
            self.cancel_preview_edit()?;
        }
        if matches!(operation, "framegraph_open" | "inspector_visible") {
            self.finish_numeric()?;
        }
        if matches!(
            operation,
            "inspector"
                | "inspector_numeric"
                | "inspector_fonts"
                | "inspector_file"
                | "context_action"
                | "context_control"
                | "context_delete"
                | "context_clipboard"
                | "record_audio"
                | "record_video"
        ) {
            if self.preview_interaction.active() {
                self.cancel_preview_edit()?;
            }
            self.cancel_graph_edit()?;
        }
        if source.is_some()
            && !matches!(
                operation,
                "import_file" | "inspector_file" | "timeline_service_file"
            )
        {
            return Err("a source descriptor is only valid for a file import".into());
        }
        let mut result = match operation {
            "timeline_service_action" | "timeline_service_file" => {
                self.timeline_service_request(request, source)
            }
            "preview_image_dismiss" => {
                #[cfg(target_os = "android")]
                {
                    self.dismiss_preview_image(request)
                }
                #[cfg(not(target_os = "android"))]
                {
                    Err("Preview image delivery requires the native host".into())
                }
            }
            "framegraph_open" | "framegraph_close" | "framegraph_action" => {
                self.graph_request(request)
            }
            "context_action" | "context_control" | "context_delete" | "context_dismiss"
            | "context_clipboard" => self.context_request(request),
            "performance" | "performance_clear" | "performance_copy" => Ok(
                shrimply_inspector_document::remote::performance_request(operation),
            ),
            "export_schema"
            | "export_validate"
            | "export_video"
            | "caption_export_schema"
            | "export_captions"
            | "export_cancel"
            | "export_dismiss"
            | "export_saved" => {
                #[cfg(target_os = "android")]
                {
                    self.export_request(request)
                }
                #[cfg(not(target_os = "android"))]
                {
                    Err("Hardware export requires the native Android host".into())
                }
            }
            "native_palette" => {
                let palette = palette::NativePalette::from_request(request)?;
                palette.apply();
                self.native_palette = Some(palette);
                self.backend.snapshot()
            }
            "editor_layout" => {
                let layout = shrimply_preferences_core::set_editor_layout(
                    &self.backend.preferences,
                    &request["changes"],
                )?;
                Ok(layout.json())
            }
            "preferences_schema" | "preferences" => {
                let schema = if operation == "preferences" {
                    self.preferences_ui
                        .apply(&self.backend.preferences, request)?
                } else {
                    self.preferences_ui.snapshot(&self.backend.preferences)
                };
                let font = shrimply_preferences_core::snapshot(&self.backend.preferences)
                    .default_text_font_family;
                self.inspector = self
                    .inspector
                    .take()
                    .map(|controller| controller.with_default_text_font(font));
                self.inspector_stamp = None;
                Ok(schema)
            }
            "preview_context_action" | "preview_context_dismiss" => {
                self.preview_context_request(request)
            }
            "preview_guides" => self.preview_request(request),
            "preview_paint" => self.preview_paint_request(request),
            "inspector_visible" => {
                self.inspector_visible = request["visible"]
                    .as_bool()
                    .ok_or("inspector visibility requires a boolean")?;
                self.inspector_stamp = None;
                self.backend.snapshot()
            }
            "inspector_fonts" => self.apply_inspector_fonts(request),
            "inspector_resource" => self.inspector_resource(request),
            "inspector_numeric" => self.apply_inspector_numeric(request),
            "inspector_file" => self.apply_inspector_file(request, source),
            "inspector" => {
                let message = self.apply_inspector(request)?;
                let mut snapshot = self.backend.snapshot()?;
                if let Some(message) = message {
                    snapshot["message"] = json!(message);
                }
                Ok(snapshot)
            }
            "timeline_tool" => {
                let id = request["id"]
                    .as_str()
                    .ok_or("timeline tool requires an id")?;
                let tool = shrimply_timeline_skia::TimelineTool::ALL
                    .into_iter()
                    .find(|tool| tool.description().id == id)
                    .ok_or("unknown timeline tool")?;
                tool.activate(&shrimply_timeline_skia::TimelineTools::new(
                    self.backend.preferences.clone(),
                ));
                self.backend.snapshot()
            }
            "show_track_add" | "show_audio_record" => {
                if self.backend.project().is_none() {
                    return Err("open a project first".into());
                }
                self.action_serial = self
                    .action_serial
                    .checked_add(1)
                    .expect("Android UI action overflow");
                self.action = Some(
                    json!({"serial":self.action_serial,"service":if operation == "show_audio_record" { "audio_record" } else { "track_add" },"project_path":self.backend.path(),"kind":request["kind"],"track":request["track"],"track_id":request["track_id"],"position":request["position"],"bottom_sheet":request["bottom_sheet"],"x":request["x"],"y":request["y"],"import_targets":request["import_targets"]}),
                );
                self.backend.snapshot()
            }
            "record_audio" => {
                let key = {
                    let project = self.backend.project().ok_or("open a project first")?;
                    shrimply_timeline_edit::TrackKey {
                        kind: shrimply_timeline_edit::TrackKind::Audio,
                        track_index: crate::project::target_index(
                            &project,
                            shrimply_timeline_edit::TrackKind::Audio,
                            request,
                        )?,
                    }
                };
                let position = self.backend.position(request)?;
                self.timeline
                    .as_mut()
                    .ok_or("open a project first")?
                    .toggle_audio_recording_at(key, position)?;
                self.backend.snapshot()
            }
            "record_video" => {
                let key = {
                    let project = self.backend.project().ok_or("open a project first")?;
                    shrimply_timeline_edit::TrackKey {
                        kind: shrimply_timeline_edit::TrackKind::Video,
                        track_index: crate::project::target_index(
                            &project,
                            shrimply_timeline_edit::TrackKind::Video,
                            request,
                        )?,
                    }
                };
                let position = self.backend.position(request)?;
                self.timeline
                    .as_mut()
                    .ok_or("open a project first")?
                    .toggle_video_recording_at(key, position)?;
                self.backend.snapshot()
            }
            "video_recording_event" => self.video_recording_event(request),
            "play" | "pause" | "step" | "seek" => {
                if self.backend.project().is_none() {
                    return Err("open a project first".into());
                }
                let snapshot = player::snapshot(&self.player);
                match operation {
                    "play" => {
                        if !self.foreground {
                            return Err("resume the application before playing".into());
                        }
                        if snapshot.duration <= Time::ZERO {
                            return Err("add timeline content before playing".into());
                        }
                        if snapshot.position >= snapshot.duration {
                            player::seek_time(&self.player, Time::ZERO);
                        }
                        player::set_playing(&self.player, true);
                    }
                    "pause" => {
                        player::set_playing(&self.player, false);
                        if snapshot.playing
                            && player::current_time(&self.player) > snapshot.duration
                            && !self
                                .timeline
                                .as_ref()
                                .is_some_and(|scene| scene.is_recording())
                        {
                            player::seek_time(&self.player, snapshot.duration);
                        }
                    }
                    "step" | "seek" => {
                        let total = snapshot.duration.as_frame_ceil(snapshot.frame_rate);
                        let frame = if operation == "step" {
                            let step = request["frames"]
                                .as_i64()
                                .ok_or("frames must be a signed integer")?;
                            #[cfg(target_os = "android")]
                            if let Some(media) = &self.media {
                                use shrimply_preview_render_core::playback::StepDirection;
                                media.mark_step(if step < 0 {
                                    StepDirection::Backward
                                } else {
                                    StepDirection::Forward
                                });
                            }
                            snapshot
                                .position
                                .as_frame(snapshot.frame_rate)
                                .saturating_add_signed(step)
                        } else {
                            request["frame"]
                                .as_u64()
                                .ok_or("frame must be a nonnegative integer")?
                        }
                        .min(total);
                        let time = time_from_frame(frame, snapshot.frame_rate)
                            .ok_or("frame exceeds exact timeline range")?;
                        player::set_playing(&self.player, false);
                        player::seek_time(&self.player, time.clamp(Time::ZERO, snapshot.duration));
                    }
                    _ => unreachable!(),
                }
                self.sync_cursor();
                self.backend.snapshot()
            }
            _ => {
                self.cancel_graph_edit()?;
                self.cancel_numeric()?;
                if operation == "close" {
                    player::set_playing(&self.player, false);
                    if let Some(scene) = self.timeline.as_mut() {
                        scene.suspend();
                    }
                    self.sync_cursor();
                }
                let previous_path = self.backend.path().map(std::path::Path::to_path_buf);
                let previous_revision = self.backend.revision;
                let result = if let Some(source) = source {
                    self.backend.import_descriptor(request, source)
                } else {
                    self.backend.dispatch(request)
                };
                let lifecycle = self.synchronize_lifecycle();
                let reset = previous_path.as_deref() != self.backend.path();
                let model_changed = previous_revision != self.backend.revision;
                if reset {
                    self.generation = self
                        .generation
                        .checked_add(1)
                        .expect("Android project generation overflow");
                    self.action = None;
                }
                // Auxiliary failures (for example recent-history I/O) cannot hide
                // a canonical edit that already committed successfully.
                if reset || model_changed {
                    self.refresh(reset);
                }
                result.and_then(|value| lifecycle.map(|_| value))
            }
        };
        match self
            .backend
            .snapshot()
            .and_then(|value| serde_json::to_string(&value).map_err(|error| error.to_string()))
        {
            Ok(status) => self.status = status,
            Err(error) => {
                if result.is_ok() {
                    result = Err(error);
                }
            }
        }
        self.error = result.as_ref().err().cloned();
        result
    }

    /// The project backend accepts a caller-bounded cursor without scanning content.
    pub fn sync_cursor(&mut self) {
        let current = player::snapshot(&self.player);
        if current.playing
            && current.position >= current.duration
            && !self
                .timeline
                .as_ref()
                .is_some_and(|scene| scene.is_recording())
        {
            player::set_playing(&self.player, false);
            player::seek_time(&self.player, current.duration);
        }
        // The shared Scene permits paused editing beyond the media end (for
        // example comments). Persist that position without a host-side clamp.
        self.backend.cursor(player::current_time(&self.player));
    }

    fn refresh(&mut self, reset: bool) {
        if reset {
            self.discard_video_recording();
            self.framegraph = None;
            self.graph_stamp = None;
            self.graph_dirty = true;
            self.preview_interaction = Default::default();
            self.preview_toolbar = Default::default();
            self.preview_context = None;
        }
        let (duration, frame_rate, position) = {
            let project = self.backend.project();
            (
                project
                    .as_ref()
                    .map_or(Time::ZERO, |project| project.duration()),
                project
                    .as_ref()
                    .map_or(Project::default().fps, |project| project.fps),
                project
                    .and_then(|project| project.cursor_position)
                    .unwrap_or(Time::ZERO),
            )
        };
        if reset {
            if let Some(mut service) = self.timeline_service.take()
                && let Err(error) = service.cleanup(self.timeline.as_ref())
            {
                self.error = Some(error);
            }
            self.inspector = None;
            self.inspector_document = None;
            self.inspector_interaction = Default::default();
            self.inspector_fonts = Default::default();
            self.inspector_stamp = None;
            self.inspector_status = "null".into();
            #[cfg(target_os = "android")]
            {
                if let Some(media) = self.media.take() {
                    media.stop();
                }
                self.audio = None;
                self.audio_levels.take_peaks();
            }
            self.timeline = self.backend.shared_project().map(|project| {
                let selection = shrimply_timeline_edit::selection_state::new();
                let clipboard = shrimply_property_transfer::new_clipboard();
                self.inspector = Some(
                    shrimply_inspector_core::InspectorController::new(
                        project.clone(),
                        self.player.clone(),
                        selection.clone(),
                    )
                    .with_analysis_backend({
                        #[cfg(target_os = "android")]
                        {
                            shrimply_inspector_core::InspectorAnalysisBackend::default()
                                .camera(shrimply_preview_render_vulkan::analysis::camera)
                                .transparent_fill(
                                    shrimply_preview_render_vulkan::analysis::transparent_fill,
                                )
                                .visual_cache_images(
                                    shrimply_preview_render_vulkan::modifier_cache::bake,
                                )
                        }
                        #[cfg(not(target_os = "android"))]
                        {
                            Default::default()
                        }
                    })
                    .with_property_clipboard(clipboard.clone())
                    .with_default_text_font(
                        shrimply_preferences_core::snapshot(&self.backend.preferences)
                            .default_text_font_family,
                    ),
                );
                #[cfg(target_os = "android")]
                {
                    let audio = std::rc::Rc::new(
                        shrimply_audio_engine::AudioPlayer::new(
                            &project.borrow(),
                            self.audio_levels.clone(),
                        )
                        .expect("create shared audio transport"),
                    );
                    player::connect_audio_playback(&project, &self.player, &audio);
                    self.audio = Some(audio);
                }
                let performance =
                    shrimply_playback_performance::open(Arc::new(project.borrow().clone()));
                #[cfg(target_os = "android")]
                {
                    let wake = self.backend.wake.clone();
                    self.media = Some(shrimply_preview_render_core::playback::PreviewMedia::new(
                        project.clone(),
                        self.player.clone(),
                        performance.clone(),
                        self.backend.preferences.clone(),
                        move |project, config, observer| {
                            shrimply_preview_render_vulkan::worker::spawn_worker(
                                project,
                                config,
                                observer,
                                Some(wake),
                            )
                        },
                    ));
                }
                let mut scene = shrimply_timeline_skia::scene::Scene::new(
                    project,
                    self.player.clone(),
                    selection,
                    self.backend.preferences.clone(),
                    clipboard,
                    performance,
                );
                scene.set_recording_available(true, true);
                scene
            });
        }
        player::refresh_project(
            &self.player,
            player::ProjectChange {
                duration: Some(duration),
                frame_rate: Some(frame_rate),
                audio: true,
                audio_waveforms: true,
                audio_beats: true,
                video: true,
                captions: true,
                inspector: true,
                ..Default::default()
            },
        );
        if reset {
            player::set_playing(&self.player, false);
            player::seek_time(&self.player, position.max(Time::ZERO));
        }
        self.sync_cursor();
    }

    /// Timeline edits use the same canonical document and history on this worker.
    pub fn timeline_changed(&mut self) -> Result<(), String> {
        self.sync_cursor();
        self.backend.revision = self
            .backend
            .revision
            .checked_add(1)
            .expect("project revision overflow");
        self.status =
            serde_json::to_string(&self.backend.snapshot()?).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn changed(&mut self) -> Result<bool, String> {
        let before = self.synchronize_lifecycle()?;
        let revision = self.backend.revision;
        let changed = self.backend.changed();
        let after = self.synchronize_lifecycle()?;
        let changed = changed.map(|value| value || before || after);
        if !matches!(changed, Ok(false)) {
            if revision != self.backend.revision {
                self.refresh(false);
            }
            self.status = serde_json::to_string(&self.backend.snapshot()?)
                .map_err(|error| error.to_string())?;
            self.error = changed.as_ref().err().cloned();
        }
        #[cfg(target_os = "android")]
        if let Some(error) = self.audio.as_ref().and_then(|audio| audio.take_failure()) {
            player::set_playing(&self.player, false);
            self.error = Some(error);
            return Ok(true);
        }
        changed
    }

    #[cfg(test)]
    pub fn active(&mut self, active: bool, observed: Instant) -> Result<(), String> {
        self.lifecycle
            .lock()
            .expect("Android lifecycle latch poisoned")
            .observe(active, observed);
        self.synchronize_lifecycle()?;
        self.refresh(false);
        self.status =
            serde_json::to_string(&self.backend.snapshot()?).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[cfg(test)]
    pub fn seek(&mut self, position: Time) {
        player::seek_time(
            &self.player,
            position.clamp(Time::ZERO, player::snapshot(&self.player).duration),
        );
        self.sync_cursor();
    }

    pub fn tick(&mut self) -> bool {
        match self.synchronize_lifecycle() {
            Ok(true) => {
                self.refresh(false);
                return true;
            }
            Err(error) => {
                self.error = Some(error);
                return true;
            }
            Ok(false) => {}
        }
        let before = player::snapshot(&self.player);
        player::tick(&self.player);
        self.sync_cursor();
        player::snapshot(&self.player) != before
    }

    pub fn event(&mut self) -> String {
        self.update_inspector();
        #[cfg(target_os = "android")]
        let export = self.export.as_ref().map(|export| &export.state);
        #[cfg(not(target_os = "android"))]
        let export: Option<&Value> = None;
        let snapshot = player::snapshot(&self.player);
        let playback = json!({"position_frame":snapshot.position.as_frame(snapshot.frame_rate),
            "total_frames":snapshot.duration.as_frame_ceil(snapshot.frame_rate), "playing":snapshot.playing,
            "time_label":format!("{} / {}", time_format::playback_time(snapshot.position), time_format::playback_time(snapshot.duration))});
        let tools = shrimply_timeline_skia::TimelineTools::new(self.backend.preferences.clone());
        let state = tools.state();
        let tools: Vec<_> = shrimply_timeline_skia::TimelineTool::ALL.into_iter().map(|tool| {
            let description = tool.description();
            json!({"id":description.id,"icon":description.icon,"label":description.label,"section":description.section,"active":tool.active(state)})
        }).collect();
        #[cfg(target_os = "android")]
        let preview_image = self.preview_image_snapshot();
        #[cfg(not(target_os = "android"))]
        let preview_image = Value::Null;
        let framegraph = self
            .framegraph
            .as_ref()
            .map(|graph| graph.snapshot())
            .unwrap_or(Value::Null);
        // The project/recents payload changes only on project events, not every frame.
        format!(
            "{{\"status\":{},\"editor_layout\":{},\"playback\":{},\"preview_controls\":{},\"preview_context\":{},\"preview_image\":{},\"framegraph\":{},\"timeline_service\":{},\"error\":{},\"inspector\":{},\"video_recording\":{},\"export\":{},\"timeline\":{{\"tools_progress\":{},\"show_tools\":{},\"audio_monitor_progress\":{},\"show_audio_monitor\":{},\"ruler_height\":{},\"action\":{},\"tools\":{},\"context\":{},\"inspect_serial\":{}}}}}",
            self.status,
            shrimply_preferences_core::snapshot(&self.backend.preferences)
                .editor_layout
                .json(),
            playback,
            self.preview_controls(),
            self.preview_context_snapshot(),
            preview_image,
            framegraph,
            self.timeline_service
                .as_ref()
                .map_or(Value::Null, |service| service.snapshot()),
            json!(self.error),
            self.inspector_status,
            self.video_recording_snapshot(),
            json!(export),
            self.timeline
                .as_ref()
                .map_or(1.0, |scene| scene.tools_progress()),
            self.timeline
                .as_ref()
                .is_none_or(|scene| scene.show_tools()),
            self.timeline
                .as_ref()
                .map_or(1.0, |scene| scene.audio_monitor_progress()),
            self.timeline
                .as_ref()
                .is_none_or(|scene| scene.show_audio_monitor()),
            shrimply_timeline_paint_skia::RULER_HEIGHT,
            json!(self.action),
            json!(tools),
            self.context_menu
                .as_ref()
                .map(context::Menu::snapshot)
                .unwrap_or(Value::Null),
            self.inspect_serial
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playhead_cut_selected_uses_captured_touch_selection_and_saves_one_undo() {
        use shrimply_components_skia::input::{TouchChange, TouchPhase};
        use shrimply_timeline_skia::{items::TrackKind, scene::Event};
        let Some(root) = crate::project::tests::isolated_root(
            "runtime::tests::playhead_cut_selected_uses_captured_touch_selection_and_saves_one_undo",
            false,
        ) else {
            return;
        };
        let path = root.join("playhead-cut.shrimp");
        let mut runtime = Runtime::new(|| {}).unwrap();
        runtime
            .request(&json!({"op":"create","path":path,"name":"Playhead cut","fps":"30000/1001"}))
            .unwrap();
        runtime.request(&json!({"op":"track_add","kind":"comment","action":"Comment","text":"Unselected","duration_frames":90})).unwrap();
        runtime
            .request(
                &json!({"op":"track_add","kind":"video","action":"Shape","duration_frames":90}),
            )
            .unwrap();
        runtime.request(&json!({"op":"seek","frame":30})).unwrap();
        let project = runtime.backend.shared_project().unwrap();
        let mut recorder = skia_safe::PictureRecorder::new();
        let scene = runtime.timeline.as_mut().unwrap();
        scene.draw(
            recorder.begin_recording(skia_safe::Rect::from_wh(800.0, 320.0), false),
            glam::vec2(800.0, 320.0),
        );
        recorder.finish_recording_as_picture(None).unwrap();
        let view = scene.view();
        let x = view.timeline_x()
            + (player::current_time(&runtime.player).as_secs_f64() - view.scroll_seconds)
                / view.seconds_per_pixel;
        let row =
            shrimply_timeline_skia::items::row_for_track(&project.borrow(), TrackKind::Video, 0)
                .unwrap();
        let clip_point = glam::vec2(
            x as f32,
            (shrimply_timeline_paint_skia::RULER_HEIGHT
                + row as f64 * shrimply_timeline_paint_skia::TRACK_HEIGHT
                + shrimply_timeline_paint_skia::TRACK_HEIGHT / 2.0) as f32,
        );
        for phase in [TouchPhase::Begin, TouchPhase::End] {
            scene.event(Event::Touch(vec![TouchChange {
                id: 1,
                point: clip_point,
                phase,
            }]));
        }
        let point = glam::vec2(
            x as f32,
            (shrimply_timeline_paint_skia::PLAYHEAD_HANDLE_TOP
                + shrimply_timeline_paint_skia::PLAYHEAD_HANDLE_HEIGHT / 2.0) as f32,
        );
        scene.event(Event::ContextMenu { point, touch: true });
        let (point, touch) = scene
            .take_requests()
            .context_menu
            .expect("playhead context request");
        runtime.show_context_menu(point, touch);
        let menu = runtime.context_menu.as_ref().unwrap().snapshot();
        let cut = menu["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|section| section.as_array().unwrap())
            .find(|entry| entry["action"] == "split-selected")
            .unwrap();
        assert_eq!(
            cut["enabled"], true,
            "selected clip spans the captured playhead"
        );
        assert_eq!(menu["bottom_sheet"], true);
        // Opening a native sheet cancels its originating surface contact.
        runtime.timeline.as_mut().unwrap().event(Event::Cancel);
        runtime.request(&json!({"op":"seek","frame":60})).unwrap();
        let response = runtime.request(&json!({"op":"context_action","project_path":path,"serial":menu["serial"],"action":"split-selected"})).unwrap();
        assert!(
            response.get("clipboard").is_none(),
            "playhead cut never removes clips to the clipboard"
        );
        let cut_time = time_from_frame(30, project.borrow().fps).unwrap();
        {
            let project = project.borrow();
            let clips = &project.video_tracks[0].items;
            assert_eq!(clips.len(), 2);
            assert_eq!(clips[0].end, cut_time);
            assert_eq!(clips[1].start, cut_time);
            assert_eq!(project.comment_tracks[0].items.len(), 1);
        }
        shrimply_project_document::project::flush_history().unwrap();
        let saved = shrimply_project_document::load_project_snapshot(&path).unwrap();
        assert_eq!(saved.video_tracks[0].items.len(), 2);
        assert_eq!(saved.video_tracks[0].items[1].start, cut_time);
        runtime.request(&json!({"op":"undo"})).unwrap();
        assert_eq!(
            project.borrow().video_tracks[0].items.len(),
            1,
            "one undo restores the unsplit clip"
        );
        assert_eq!(project.borrow().comment_tracks[0].items.len(), 1);
        runtime.request(&json!({"op":"redo"})).unwrap();
        assert_eq!(project.borrow().video_tracks[0].items.len(), 2);
        runtime.request(&json!({"op":"undo"})).unwrap();
        // A seek within the first quarter-frame snaps to the clip boundary,
        // where the shared menu must disable splitting.
        runtime
            .timeline
            .as_mut()
            .unwrap()
            .prepare_context_menu(clip_point);
        let edge = Time {
            seconds: project.borrow().frame_step().seconds / shrimply_math_core::Fraction::from(4),
        };
        player::seek_time(&runtime.player, edge);
        assert_eq!(player::current_time(&runtime.player), Time::ZERO);
        let view = runtime.timeline.as_ref().unwrap().view();
        let point = glam::vec2(
            (view.timeline_x()
                + (player::current_time(&runtime.player).as_secs_f64() - view.scroll_seconds)
                    / view.seconds_per_pixel) as f32,
            point.y,
        );
        runtime.show_context_menu(point, true);
        let menu = runtime.context_menu.as_ref().unwrap().snapshot();
        let cut = menu["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|section| section.as_array().unwrap())
            .find(|entry| entry["action"] == "split-selected")
            .unwrap();
        assert_eq!(
            cut["enabled"], false,
            "frame-zero cuts must not be advertised as usable"
        );
        assert!(runtime.request(&json!({"op":"context_action","project_path":path,"serial":menu["serial"],"action":"split-selected"})).is_err());
        assert_eq!(project.borrow().video_tracks[0].items.len(), 1);
        runtime.request(&json!({"op":"close"})).unwrap();
    }

    #[test]
    fn exact_transport_and_project_lifecycle_share_the_saved_project() {
        let Some(root) = crate::project::tests::isolated_root(
            "runtime::tests::exact_transport_and_project_lifecycle_share_the_saved_project",
            false,
        ) else {
            return;
        };
        let path = root.join("transport.shrimp");
        let mut runtime = Runtime::new(|| {}).unwrap();
        runtime
            .request(&json!({"op":"create","path":path,"name":"Transport","fps":"30000/1001"}))
            .unwrap();
        runtime.request(&json!({"op":"track_add","kind":"comment","action":"Comment","text":"Actual saved item","start_frame":0,"duration_frames":90})).unwrap();
        runtime
            .request(&json!({"op":"track_add","kind":"video","action":"Shape","start_frame":0,"duration_frames":90}))
            .unwrap();
        runtime.request(&json!({"op":"settings_schema"})).unwrap();
        let event: Value = serde_json::from_str(&runtime.event()).unwrap();
        assert_eq!(event["status"]["project"]["name"], "Transport");
        assert_eq!(event["playback"]["total_frames"], 90);
        runtime.request(&json!({"op":"seek","frame":10})).unwrap();
        assert_eq!(
            player::current_time(&runtime.player),
            time_from_frame(10, runtime.backend.project().unwrap().fps).unwrap()
        );
        runtime.request(&json!({"op":"step","frames":-1})).unwrap();
        assert_eq!(
            player::snapshot(&runtime.player)
                .position
                .as_frame(player::snapshot(&runtime.player).frame_rate),
            9
        );
        runtime.request(&json!({"op":"play"})).unwrap();
        assert!(player::snapshot(&runtime.player).playing);
        runtime.active(false, std::time::Instant::now()).unwrap();
        assert!(!player::snapshot(&runtime.player).playing);
        let saved = shrimply_project_document::load_project_snapshot(&path).unwrap();
        assert_eq!(
            saved.cursor_position,
            Some(player::current_time(&runtime.player))
        );
        runtime.active(true, std::time::Instant::now()).unwrap();
        runtime.request(&json!({"op":"undo"})).unwrap();
        assert_eq!(player::snapshot(&runtime.player).duration, Time::ZERO);
        runtime.request(&json!({"op":"redo"})).unwrap();
        assert_eq!(
            runtime.backend.project().unwrap().comment_tracks[0].items[0].text,
            "Actual saved item"
        );
        runtime
            .request(&json!({"op":"seek","frame":u64::MAX}))
            .unwrap();
        assert_eq!(
            player::snapshot(&runtime.player).position,
            player::snapshot(&runtime.player).duration
        );
        runtime.request(&json!({"op":"play"})).unwrap();
        assert_eq!(player::current_time(&runtime.player), Time::ZERO);
        runtime.request(&json!({"op":"pause"})).unwrap();
        assert!(runtime.request(&json!({"op":"export_video"})).is_err());
        runtime.request(&json!({"op":"close"})).unwrap();
        assert!(runtime.backend.project().is_none());
        runtime.request(&json!({"op":"open","path":path})).unwrap();
        assert!(!player::snapshot(&runtime.player).playing);
        assert_eq!(
            runtime.backend.project().unwrap().comment_tracks[0].items[0].text,
            "Actual saved item"
        );
        runtime.request(&json!({"op":"close"})).unwrap();
    }
    #[test]
    fn lifecycle_retains_earliest_pause_across_delayed_resume_and_refresh() {
        let Some(root) = crate::project::tests::isolated_root(
            "runtime::tests::lifecycle_retains_earliest_pause_across_delayed_resume_and_refresh",
            false,
        ) else {
            return;
        };
        let lifecycle = SharedLifecycle::default();
        let mut runtime = Runtime::with_lifecycle(|| {}, lifecycle.clone()).unwrap();
        runtime.request(&json!({"op":"create","path":root.join("lifecycle.shrimp"),"name":"Lifecycle","fps":"30"})).unwrap();
        runtime
            .request(
                &json!({"op":"track_add","kind":"video","action":"Shape","duration_frames":300}),
            )
            .unwrap();
        let started = Instant::now() - std::time::Duration::from_secs(4);
        player::set_playing_at(&runtime.player, true, started);
        {
            let mut pending = lifecycle.lock().unwrap();
            pending.observe(false, started + std::time::Duration::from_secs(1));
            pending.observe(true, started + std::time::Duration::from_secs(2));
            pending.observe(false, started + std::time::Duration::from_secs(3));
            pending.observe(true, started + std::time::Duration::from_secs(4));
        }
        runtime.request(&json!({"op":"settings_schema"})).unwrap();
        assert!(runtime.foreground);
        assert!(!player::snapshot(&runtime.player).playing);
        assert_eq!(
            player::current_time(&runtime.player),
            time_from_frame(30, runtime.backend.project().unwrap().fps).unwrap()
        );
        assert!(!runtime.tick());
        let past = Instant::now() - std::time::Duration::from_secs(40);
        player::set_playing_at(&runtime.player, true, past);
        lifecycle
            .lock()
            .unwrap()
            .observe(false, past + std::time::Duration::from_secs(30));
        runtime.request(&json!({"op":"settings_schema"})).unwrap();
        let final_state = player::snapshot(&runtime.player);
        assert_eq!(final_state.position, final_state.duration);
        assert!(!final_state.playing);
        runtime.active(true, Instant::now()).unwrap();
        player::seek_time(&runtime.player, Time::ZERO);
        player::set_playing_at(&runtime.player, true, past);
        runtime.request(&json!({"op":"pause"})).unwrap();
        assert_eq!(player::current_time(&runtime.player), final_state.duration);
        runtime.request(&json!({"op":"close"})).unwrap();
    }
    #[test]
    fn auxiliary_failure_still_exposes_committed_settings_and_player_state() {
        let Some(root) = crate::project::tests::isolated_root(
            "runtime::tests::auxiliary_failure_still_exposes_committed_settings_and_player_state",
            false,
        ) else {
            return;
        };
        let mut runtime = Runtime::new(|| {}).unwrap();
        runtime.request(&json!({"op":"create","path":root.join("auxiliary.shrimp"),"name":"Original","fps":"30"})).unwrap();
        runtime
            .request(
                &json!({"op":"track_add","kind":"video","action":"Shape","duration_frames":90}),
            )
            .unwrap();
        let database = shrimply_recent_projects::settings_db_path();
        let saved = database.with_extension("saved");
        std::fs::rename(&database, &saved).unwrap();
        std::fs::create_dir(&database).unwrap();
        let result = runtime.request(&json!({"op":"settings","name":"Committed","fps":"60"}));
        let generation = runtime.generation;
        let destination = root.join("renamed.shrimp");
        let save_as = runtime.request(&json!({"op":"save_as","path":destination}));
        std::fs::remove_dir(&database).unwrap();
        std::fs::rename(&saved, &database).unwrap();
        assert!(result.is_err());
        assert!(save_as.is_err());
        assert_eq!(runtime.generation, generation + 1);
        assert_eq!(runtime.backend.path(), Some(destination.as_path()));
        let event: Value = serde_json::from_str(&runtime.event()).unwrap();
        assert_eq!(event["status"]["project"]["name"], "Committed");
        assert_eq!(event["playback"]["total_frames"], 180);
        assert!(event["error"].is_string());
        assert_eq!(
            player::snapshot(&runtime.player).frame_rate,
            shrimply_math_core::Fraction::from(60u64)
        );
        runtime.request(&json!({"op":"close"})).unwrap();
    }
}
