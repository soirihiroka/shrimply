use shrimply_components_skia::{
    canvas::{Vec2, vec2},
    cursor::SoftwareCursor,
};
use shrimply_editor_state::{
    player_state::{self, SharedPlayerState},
    preferences::SharedPreferences,
};
use shrimply_math_color::Color;
use shrimply_playback_performance as playback_performance;
#[cfg(target_os = "linux")]
use shrimply_pointer_lock_wayland::WaylandPointerLock;
use shrimply_project_document::project::Project;
use shrimply_surface_gl_skia::TimelineRenderer;
use shrimply_timeline_edit::selection_state::SharedSelectionState;
pub use shrimply_timeline_skia::scene::PointerButton as ToolkitPointerButton;
use shrimply_timeline_skia::{
    ContextMenu, ContextMenuAction, ContextMenuControl, ContextMenuRequest, CursorTool,
    DragCollisionMode, TimelineTools, ToolState, TrackAddAction, TrackAddOutcome,
    VideoFrameSelection,
    drawing::row_screen_y,
    items,
    metrics::*,
    scene::{
        Event, Frame, KeyAction, PointerButton, Scene, TimelineModifiers, TrackAddMenuRequest,
        TrackDeletion,
    },
    track_controls::track_label_button_y,
    view::{TimelineCursor, TimelineScrollEvent, TimelineScrollInput},
};
pub use shrimply_visual_cuda::compositor::RgbaVideoFrame as RenderedVideoFrame;
#[cfg(target_os = "linux")]
use std::ffi::c_void;
use std::{cell::RefCell, path::PathBuf, rc::Rc};

pub struct TrackAddMenuPresentation {
    pub kind: shrimply_timeline_edit::TrackKind,
    pub x: f32,
    pub y: f32,
}

/// Qt owns the GL context and native pointer capture; Scene owns timeline behavior.
pub struct ToolkitTimeline {
    scene: Scene,
    renderer: TimelineRenderer,
    project: Rc<RefCell<Project>>,
    player: SharedPlayerState,
    selection: SharedSelectionState,
    tools: TimelineTools,
    context_menu: ContextMenu,
    track_add_request: Option<TrackAddMenuRequest>,
    track_add_presentation: Option<TrackAddMenuPresentation>,
    deletion: Option<TrackDeletion>,
    #[cfg(target_os = "linux")]
    pointer_lock: Option<WaylandPointerLock>,
    pointer_lock_origin: Option<Vec2>,
}

impl ToolkitTimeline {
    pub fn new(
        project: Rc<RefCell<Project>>,
        player: SharedPlayerState,
        performance: playback_performance::SharedCollector,
        selection: SharedSelectionState,
        preferences: SharedPreferences,
        clipboard: shrimply_property_transfer::SharedClipboard,
    ) -> Self {
        let mut scene = Scene::new(
            project.clone(),
            player.clone(),
            selection.clone(),
            preferences.clone(),
            clipboard,
            performance,
        );
        scene.set_stabilization_handler(shrimply_visual_cuda::video_stabilization::request);
        Self {
            scene,
            renderer: TimelineRenderer::new(),
            project,
            player,
            selection,
            tools: TimelineTools::new(preferences),
            context_menu: ContextMenu::default(),
            track_add_request: None,
            track_add_presentation: None,
            deletion: None,
            #[cfg(target_os = "linux")]
            pointer_lock: None,
            pointer_lock_origin: None,
        }
    }

    pub fn render(
        &mut self,
        width: u32,
        height: u32,
        scale: f32,
        accent_color: Color,
    ) -> Result<(), String> {
        self.poll_pointer_lock();
        let painter = self.renderer.begin_frame(
            glam::UVec2::new(width.max(1), height.max(1)),
            scale,
            shrimply_cross_ui_theme::current().view_bg,
        )?;
        self.scene.draw_frame(
            painter.canvas(),
            vec2(width as f32 / scale, height as f32 / scale),
            Frame {
                before_seek: None,
                accent_color,
                active_audio_recording_key: None,
                active_video_recording_key: None,
                live_recording: None,
                live_video_recording: None,
            },
        );
        self.renderer.end_frame()?;
        let requests = self.scene.take_requests();
        if requests.pause_playback {
            player_state::set_playing(&self.player, false);
        }
        if let Some(key) = requests.audio_record {
            self.scene.toggle_audio_recording(key)?;
        }
        if requests.video_record.is_some() {
            return Err("Screen recording is unavailable in the Qt timeline".into());
        }
        if let Some(request) = requests.track_add {
            let row = items::row_for_track(
                &self.project.borrow(),
                request.key.kind,
                request.key.track_index,
            )
            .ok_or("Add menu track no longer exists")?;
            self.track_add_presentation = Some(TrackAddMenuPresentation {
                kind: request.key.kind,
                x: TRACK_LABEL_ADD_X as f32,
                y: track_label_button_y(row_screen_y(row, self.scene.view())) as f32,
            });
            self.track_add_request = Some(request);
        }
        Ok(())
    }

    pub fn take_track_add_menu(&mut self) -> Option<TrackAddMenuPresentation> {
        self.track_add_presentation.take()
    }
    pub fn activate_track_add_action(&mut self, action: TrackAddAction) -> Result<bool, String> {
        let request = self
            .track_add_request
            .as_ref()
            .ok_or("Track add menu is no longer active")?;
        self.scene
            .activate_track_add(request.key, action)
            .map(|outcome| outcome != TrackAddOutcome::Unchanged)
    }
    pub fn import_track_file(&mut self, path: PathBuf) -> Result<(), String> {
        let request = self
            .track_add_request
            .as_ref()
            .ok_or("Track add menu is no longer active")?;
        self.scene.import_track_file(path, &request.import_targets)
    }
    pub fn take_error(&mut self) -> Option<String> {
        self.scene.take_error()
    }

    fn poll_pointer_lock(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some((x, y)) = self
            .pointer_lock
            .as_mut()
            .and_then(WaylandPointerLock::poll)
        {
            self.scene.event(Event::RelativeMotion {
                delta: vec2(x as f32, y as f32),
            });
        }
    }
    pub fn relative_motion(&mut self, x: f32, y: f32) {
        self.scene
            .event(Event::RelativeMotion { delta: vec2(x, y) });
    }
    pub fn pointer_move(&mut self, x: f32, y: f32, ctrl: bool, shift: bool) {
        self.scene.event(Event::Motion {
            point: vec2(x, y),
            modifiers: TimelineModifiers { ctrl, shift },
        });
    }
    pub fn pointer_cursor(&self) -> TimelineCursor {
        self.scene.pointer_cursor()
    }
    pub fn pointer_leave(&mut self) {
        self.scene.event(Event::Leave);
    }
    pub fn pointer_press(
        &mut self,
        button: PointerButton,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) {
        self.scene.event(Event::Press {
            point: vec2(x, y),
            double: false,
            modifiers: TimelineModifiers { ctrl, shift },
            button,
        });
    }
    pub fn pointer_release(
        &mut self,
        button: PointerButton,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) {
        self.scene.event(Event::Release {
            point: vec2(x, y),
            modifiers: TimelineModifiers { ctrl, shift },
            button,
        });
    }
    /// # Safety
    /// Pointers must belong to the live Wayland connection and remain valid until capture ends.
    #[cfg(target_os = "linux")]
    pub unsafe fn begin_pointer_lock(
        &mut self,
        display: *mut c_void,
        surface: *mut c_void,
        seat: *mut c_void,
        cursor: SoftwareCursor,
    ) -> bool {
        if self.pointer_lock.is_some() {
            return true;
        }
        let Some(lock) = (unsafe { WaylandPointerLock::new(display, surface, seat) }) else {
            return false;
        };
        let position = self.scene.pointer_state().position.unwrap_or(Vec2::ZERO);
        self.scene.begin_relative_pointer(position, cursor);
        self.pointer_lock = Some(lock);
        self.pointer_lock_origin = Some(position);
        true
    }
    #[cfg(windows)]
    pub fn begin_pointer_lock(&mut self, cursor: SoftwareCursor) -> bool {
        if self.pointer_lock_origin.is_some() {
            return true;
        }
        let position = self.scene.pointer_state().position.unwrap_or(Vec2::ZERO);
        self.scene.begin_relative_pointer(position, cursor);
        self.pointer_lock_origin = Some(position);
        true
    }
    pub fn end_pointer_lock(&mut self, ctrl: bool, shift: bool) {
        self.poll_pointer_lock();
        #[cfg(target_os = "linux")]
        let Some(mut lock) = self.pointer_lock.take() else {
            return;
        };
        let origin = self
            .pointer_lock_origin
            .take()
            .expect("Pointer lock must have an origin");
        let cursor = self
            .scene
            .end_relative_pointer()
            .expect("Pointer lock must own a software cursor");
        #[cfg(target_os = "linux")]
        lock.restore_cursor_with_offset(
            f64::from(cursor.x - origin.x),
            f64::from(cursor.y - origin.y),
        );
        #[cfg(windows)]
        let _ = (origin, cursor);
        #[cfg(target_os = "linux")]
        drop(lock);
        if let Some(point) = self.scene.pointer_state().position {
            self.scene.event(Event::Release {
                point,
                button: PointerButton::Middle,
                modifiers: TimelineModifiers { ctrl, shift },
            });
        }
    }
    pub fn scroll(&mut self, dx: f32, dy: f32, ctrl: bool, shift: bool) {
        self.scene
            .event(Event::Modifiers(TimelineModifiers { ctrl, shift }));
        self.scene.event(Event::Scroll(TimelineScrollEvent {
            delta: vec2(
                dx * SCROLL_PIXELS_PER_STEP as f32,
                dy * SCROLL_PIXELS_PER_STEP as f32,
            ),
            ctrl,
            pointer: self.scene.pointer_state().position,
            input: TimelineScrollInput::Wheel,
        }));
    }
    pub fn key_action(&mut self, action: KeyAction) -> Result<Option<ContextMenuRequest>, String> {
        let request = self.scene.key_action(action)?;
        if matches!(request, Some(ContextMenuRequest::DeleteTracks { .. })) {
            self.deletion = Some(self.scene.track_deletion());
        }
        Ok(request)
    }
    pub fn tool_state(&self) -> ToolState {
        self.tools.state()
    }
    pub fn set_magnet(&self, enabled: bool) {
        self.tools.set_magnet(enabled);
    }
    pub fn set_beat_grid(&self, enabled: bool) {
        self.tools.set_beat_grid(enabled);
    }
    pub fn set_cursor_tool(&self, cursor: CursorTool) {
        self.tools.set_cursor(cursor);
    }
    pub fn set_drag_collision_mode(&self, mode: DragCollisionMode) {
        self.tools.set_drag_collision(mode);
    }
    pub fn prepare_context_menu(&mut self, x: f32, y: f32) {
        self.context_menu = self.scene.prepare_context_menu(vec2(x, y));
        self.deletion = None;
    }
    pub fn context_menu(&self) -> &ContextMenu {
        &self.context_menu
    }
    pub fn set_context_menu_control(
        &mut self,
        control: ContextMenuControl,
        value: f64,
    ) -> Result<(), String> {
        self.scene.set_context_menu_control(control, value)
    }
    pub fn activate_context_menu_action(
        &mut self,
        action: ContextMenuAction,
    ) -> Result<Option<ContextMenuRequest>, String> {
        let request = self.scene.activate_context_menu_action(action)?;
        if matches!(request, Some(ContextMenuRequest::DeleteFoldedTrack { .. })) {
            self.deletion = Some(self.scene.track_deletion());
        }
        Ok(request)
    }
    pub fn render_context_video_frame(
        &self,
        selection: VideoFrameSelection,
    ) -> Result<RenderedVideoFrame, String> {
        let (project, position, ids) =
            shrimply_timeline_skia::video_selection::prepare_selected_video_frame(
                &self.project,
                &self.player,
                &self.selection,
                selection,
            );
        shrimply_visual_cuda::compositor::render_items_rgba(project, position, &ids)
    }
    pub fn context_file_path(&self) -> Option<PathBuf> {
        self.scene.context_file_path()
    }
    pub fn delete_context_folded_track(&mut self) -> Result<(), String> {
        let deletion = self.deletion.take().ok_or("No track deletion is pending")?;
        self.scene.confirm_delete_selected_tracks(deletion)
    }
    pub fn paste_context_clipboard_text(&mut self, text: String) -> Result<(), String> {
        if text == shrimply_timeline_skia::TIMELINE_CLIPBOARD_MARKER {
            self.scene.paste_context_clipboard()
        } else if self.scene.insert_external_text(text, None) {
            Ok(())
        } else {
            Err("Could not insert clipboard text".into())
        }
    }
    pub fn destroy(&mut self) {
        self.end_pointer_lock(false, false);
        self.scene.suspend();
        self.renderer.destroy();
    }
}
