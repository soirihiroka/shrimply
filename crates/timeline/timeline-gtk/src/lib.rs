use hashbrown::HashMap;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub use shrimply_audio as audio;
pub use shrimply_gtk_components::{desktop_open, playback_shortcuts, skia_font, skia_system_font};
pub use shrimply_math_color::Color;
pub use shrimply_math_core::Fraction;
pub use shrimply_project::{caption, project, time_format, timeline_search};
pub use shrimply_state::player_state;
pub use shrimply_timeline::selection_state;
pub use shrimply_transcription as transcription;

pub mod preferences {
    pub use shrimply_state::preferences as store;
}

pub mod export {
    pub use shrimply_export_core::audio;
    pub use shrimply_gtk_components::export_feedback::show_export_finished_for_widget;
}

use shrimply_math_media as math;
use shrimply_playback_performance as playback_performance;
use shrimply_video_recording as video_recording;

use crate::audio::SharedAudioLevels;
use crate::audio::beat::{self, BeatMap};
use crate::audio::waveform::{self, WaveformMap};
use crate::player_state::SharedPlayerState;
use crate::preferences::store as preferences_store;
use crate::project::{
    AudioItem, CaptionItem, FontFamily, Project, RepeatStrategy, Time, Transform, TransitionSide,
    VideoItem, VideoItemContent, VideoSampleMethod, default_playback_speed,
    generated_item_keyframe_span, generated_item_natural_end_position, generated_item_natural_span,
    media_item_natural_end_position, media_natural_end_interval, media_real_span,
    scaled_time_delta, video_natural_end_interval,
};
use crate::selection_state::SharedSelectionState;
use adw::prelude::*;
use gtk::glib;
use renderer::{Align2, FontId, Rect, Stroke, StrokeKind, Vec2, vec2};
use shrimply_core::timeline_value::{TimelineBool, TimelineValue};
use shrimply_cross_ui_theme as theme;
pub use shrimply_timeline_core::{
    ContextItemKind, ContextMenu, ContextMenuAction, ContextMenuControl, ContextMenuEntry,
    ContextMenuItem, ContextMenuRequest, CursorTool, DragCollisionMode, FoldedItemMenuContext,
    ItemMenuContext, TimelineTools, ToolState, TrackAddAction, TrackAddMenuEntry, TrackMenuContext,
    VideoFrameSelection, track_add_menu,
};
use shrimply_timeline::{TrackGap, TrackKey};

mod beat_grid;
mod audio_meter_gtk;
mod native_menu;
mod clipboard;
mod context_menu;
mod drawing;
pub use shrimply_gtk_components::cursor;
mod caption_tts;
mod drag_and_drop;
mod external_content;
mod folded_sequence;
mod frame;
mod geometry;
pub mod import;
mod interaction;
mod items;
mod recording;
mod runtime;
pub use shrimply_gtk_components::canvas as renderer;
pub(crate) mod ruler;
mod setup;
mod silence;
mod snapping;
mod timeline_operation;
mod toolkit_context_menu;
mod track_controls;
mod view;

use drawing::{
    TimelineInput, active_virtual_tracks, draw_timeline, item_rect, row_screen_y, row_y,
};
use frame::timeline_gtk;
use geometry::*;
use recording::{
    ensure_recording_duration, finish_audio_recording, handle_audio_recording,
    handle_video_recording, live_recording_draw,
};
use renderer::{TimelinePainter, TimelineRenderer};
use runtime::*;
use setup::*;
use track_controls::{
    for_each_visible_track_row, show_track_add_menu, timeline_sidebar, toggle_track_enabled,
    toggle_track_enabled_core, track_button_at, track_enabled, track_label_action_at,
    track_label_button_y, visible_row_range,
};
use view::*;

use items::{
    DragIndicator, DragPreviewStatus, DraggedGroup, ItemKey, ResizeDrag, TimelineClipboard,
    TrackKind, fitted_transition_durations, is_item_dragged, is_item_selected, resize_item_times,
    row_for_track, target_item_times, target_track_index, transition_durations,
};

const TRACK_LABEL_BUTTON_SIZE: f64 = 26.0;
const TRACK_INDEX_COLUMN_WIDTH: f64 = 40.0;
const TRACK_LABEL_PADDING_X: f64 = 14.0;
const TRACK_LABEL_BUTTON_START_X: f64 = TRACK_INDEX_COLUMN_WIDTH + TRACK_LABEL_PADDING_X;
const TRACK_LABEL_BUTTON_GAP: f64 = 6.0;
const TRACK_LABEL_BUTTON_STRIDE: f64 = TRACK_LABEL_BUTTON_SIZE + TRACK_LABEL_BUTTON_GAP;
const TRACK_LABEL_TOGGLE_X: f64 = TRACK_LABEL_BUTTON_START_X;
const TRACK_LABEL_ADD_X: f64 = TRACK_LABEL_BUTTON_START_X + TRACK_LABEL_BUTTON_STRIDE;
const TRACK_LABEL_RECORD_X: f64 = TRACK_LABEL_BUTTON_START_X + TRACK_LABEL_BUTTON_STRIDE * 2.0;
const LABEL_WIDTH: f64 = TRACK_LABEL_RECORD_X + TRACK_LABEL_BUTTON_SIZE + TRACK_LABEL_PADDING_X;
const TRACK_SELECTION_LABEL_ALPHA: f32 = 0.24;
const TRACK_SELECTION_ROW_ALPHA: f32 = 0.18;
const TRACK_SELECTION_EDGE_ALPHA: f32 = 0.44;
const TIMELINE_PADDING_LEFT: f64 = 2.0;
const TIMELINE_PADDING_RIGHT: f64 = 2.0;
const RULER_HEIGHT: f64 = 44.0;
const RULER_LABEL_ALPHA: f32 = 0.55;
const TRACK_HEIGHT: f64 = 36.0;
const PLAYHEAD_HANDLE_WIDTH: f64 = 16.0;
const PLAYHEAD_HANDLE_HEIGHT: f64 = 8.0;
const PLAYHEAD_HANDLE_TOP: f64 = 21.0;
const PLAYHEAD_HANDLE_TRIANGLE_HEIGHT: f64 = 5.0;
const PLAYHEAD_SCROLL_MARGIN_PX: f64 = 96.0;
const ITEM_PADDING_X: f64 = 2.0;
const ITEM_BORDER_STROKE_WIDTH: f32 = 2.0;
const SUBPIXEL_ITEM_WIDTH: f64 = 1.0;
const MIN_DETAILED_ITEM_WIDTH: f64 = 8.0;
const ITEM_RESIZE_HANDLE_WIDTH: f64 = 6.0;
const MIN_SECONDS_PER_PIXEL: f64 = 1.0 / 100_000.0;
const MAX_SECONDS_PER_PIXEL: f64 = 60.0;
const MAX_FRAME_PIXEL_WIDTH: f64 = 32.0;
const FRAME_TICK_MIN_WIDTH: f64 = 8.0;
const WAVEFORM_CHUNKS_PER_FRAME: u32 = 8;
const CLICK_DRAG_TOLERANCE: f64 = 2.0;
const SCROLL_PIXELS_PER_STEP: f64 = 120.0;
const WAVEFORM_POLL_INTERVAL: Duration = Duration::from_millis(33);
const WAVEFORM_RELOAD_DELAY: Duration = Duration::from_millis(75);
const BEAT_POLL_INTERVAL: Duration = Duration::from_millis(33);
const RECORDING_DURATION_HEADROOM_SECONDS: i64 = 10;
const VIDEO_RECORDING_POLL_INTERVAL: Duration = Duration::from_millis(33);
const PERFORMANCE_MARKER_HEIGHT: f64 = 3.0;
const PERFORMANCE_VISUAL_ALPHA: f32 = 0.42;
const SIDEBAR_WIDTH: i32 = 44;
const SIDEBAR_ICON_SIZE: i32 = 28;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolkitPointerButton {
    Primary,
    Middle,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum TimelineCursor {
    #[default]
    Default,
    ResizeStart,
    ResizeEnd,
    ResizeHorizontal,
    Crosshair,
}

pub struct RenderedVideoFrame {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct TrackAddMenuPresentation {
    pub kind: TrackKind,
    pub x: f32,
    pub y: f32,
}

pub struct ToolkitTimeline {
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    playback_performance: playback_performance::SharedCollector,
    selection_state: SharedSelectionState,
    tools: TimelineTools,
    context_menu: ContextMenu,
    context_track: Option<TrackKey>,
    context_folded_track: Option<crate::project::TrackAddress>,
    context_item: Option<crate::project::ItemAddress>,
    context_file_path: Option<PathBuf>,
    context_new_track_at_top: Option<bool>,
    track_add_request: Option<TrackAddMenuRequest>,
    track_add_presentation: Option<TrackAddMenuPresentation>,
    pending_track_import: Option<import::TrackImportInspection>,
    track_import_error: Option<String>,
    runtime: Rc<RefCell<TimelineRuntime>>,
    waveform: Option<setup::WaveformSubscription>,
    beats: Option<setup::BeatSubscription>,
    pointer_lock: Option<shrimply_wayland_pointer_lock::WaylandPointerLock>,
    pointer_lock_bounds: Option<Rect>,
    pointer_lock_origin: Option<Vec2>,
}

impl ToolkitTimeline {
    pub fn new(
        project: Rc<RefCell<Project>>,
        player_state: SharedPlayerState,
        playback_performance: playback_performance::SharedCollector,
        selection_state: SharedSelectionState,
        preferences: preferences_store::SharedPreferences,
        property_clipboard: shrimply_property_transfer::SharedClipboard,
    ) -> Self {
        let tools = TimelineTools::new(preferences.clone());
        let playhead_visibility_requested = Rc::new(Cell::new(false));
        let (timeline_zoom, timeline_center) = {
            let project = project.borrow();
            (
                project.timeline_zoom,
                project
                    .timeline_zoom
                    .filter(|zoom| *zoom > Time::ZERO)
                    .and(project.cursor_position),
            )
        };
        let runtime = Rc::new(RefCell::new(TimelineRuntime::new(
            WaveformMap::new(),
            preferences_store::snapshot(&preferences),
            playhead_visibility_requested,
            timeline_zoom,
            timeline_center,
            property_clipboard,
        )));
        let preference_runtime = runtime.clone();
        preferences_store::connect(&preferences, move |snapshot| {
            preference_runtime.borrow_mut().apply_preferences(&snapshot);
        });
        let (waveform, beats) = setup::toolkit_audio_loaders(&project.borrow());
        Self {
            project,
            player_state,
            playback_performance,
            selection_state,
            tools,
            context_menu: ContextMenu::default(),
            context_track: None,
            context_folded_track: None,
            context_item: None,
            context_file_path: None,
            context_new_track_at_top: None,
            track_add_request: None,
            track_add_presentation: None,
            pending_track_import: None,
            track_import_error: None,
            runtime,
            waveform: Some(waveform),
            beats: Some(beats),
            pointer_lock: None,
            pointer_lock_bounds: None,
            pointer_lock_origin: None,
        }
    }

    pub fn render(
        &mut self,
        width: u32,
        height: u32,
        pixels_per_point: f32,
        accent_color: Color,
    ) -> Result<(), String> {
        self.poll_track_import();
        let logical_width = f64::from(width) / f64::from(pixels_per_point);
        let logical_height = f64::from(height) / f64::from(pixels_per_point);
        self.pointer_lock_bounds = Some(Rect::from_min_max(
            vec2(timeline_x() as f32, 0.0),
            vec2(
                (timeline_x() + timeline_width(logical_width)) as f32,
                logical_height.max(0.0) as f32,
            ),
        ));
        self.poll_pointer_lock();
        let mut runtime = self.runtime.borrow_mut();
        setup::poll_toolkit_audio_loaders(&mut runtime, &mut self.waveform, &mut self.beats);
        runtime.track_controls_animating = false;
        let painter = runtime.renderer.begin_frame(
            glam::UVec2::new(width.max(1), height.max(1)),
            pixels_per_point,
            theme::current().view_bg,
        )?;
        timeline_gtk(
            &self.project,
            &self.player_state,
            &self.selection_state,
            &playback_performance::snapshot(&self.playback_performance),
            &mut runtime,
            &painter,
            logical_width,
            logical_height,
            accent_color,
        );
        runtime.finish_pointer_frame();
        runtime.renderer.end_frame()?;
        let pending_track_toggle = runtime.pending_track_toggle.take();
        let pending_sequence_toggle = runtime.pending_sequence_toggle.take();
        let pending_track_add_menu = runtime.pending_track_add_menu.take();
        let pending_pause_playback = std::mem::take(&mut runtime.pending_pause_playback);
        let view = runtime.view;
        drop(runtime);
        if pending_pause_playback {
            player_state::set_playing(&self.player_state, false);
        }
        if let Some(key) = pending_track_toggle {
            toggle_track_enabled_core(&self.project, &self.player_state, key);
        }
        if let Some(path) = pending_sequence_toggle {
            self.toggle_sequence(path);
        }
        if let Some(request) = pending_track_add_menu {
            let row = items::row_for_track(
                &self.project.borrow(),
                request.key.kind,
                request.key.track_index,
            )
            .expect("add menu track must exist");
            self.track_add_presentation = Some(TrackAddMenuPresentation {
                kind: request.key.kind,
                x: TRACK_LABEL_ADD_X as f32,
                y: track_label_button_y(row_screen_y(row, view)) as f32,
            });
            self.track_add_request = Some(request);
        }
        Ok(())
    }

    pub fn take_track_add_menu(&mut self) -> Option<TrackAddMenuPresentation> {
        self.track_add_presentation.take()
    }

    pub fn activate_track_add_action(&mut self, action: TrackAddAction) -> bool {
        let Some(request) = self.track_add_request.as_ref() else {
            return false;
        };
        let runtime = self.runtime.borrow();
        let default_text_font_family = runtime.default_text_font_family.clone();
        let settings = shrimply_timeline_core::TrackAddSettings {
            default_visual_duration: runtime.default_visual_duration,
            default_text_font_family: &default_text_font_family,
        };
        drop(runtime);
        shrimply_timeline_core::activate_track_add(
            &self.project,
            &self.player_state,
            &self.selection_state,
            request.key,
            action,
            settings,
        ) != shrimply_timeline_core::TrackAddOutcome::Unchanged
    }

    pub fn import_track_file(&mut self, path: PathBuf) -> Result<(), String> {
        let request = self
            .track_add_request
            .as_ref()
            .ok_or_else(|| "track add menu is no longer active".to_string())?;
        if request.import_targets.is_empty() {
            return Err("no import tracks were selected".to_string());
        }
        let kind = request.import_targets[0].kind;
        if !request
            .import_targets
            .iter()
            .all(|target| target.kind == kind)
        {
            return Err("selected tracks must have the same type".to_string());
        }
        let track_indices = request
            .import_targets
            .iter()
            .map(|target| target.track_index)
            .collect::<Vec<_>>();
        let start = player_state::snapshot(&self.player_state).position;
        let default_visual_duration = self.runtime.borrow().default_visual_duration;
        let started = import::start_track_import(
            &mut self.project.borrow_mut(),
            path,
            kind,
            track_indices,
            start,
            default_visual_duration,
        )?;
        match started {
            import::TrackImportStart::Inspect(inspection) => {
                self.pending_track_import = Some(inspection);
                Ok(())
            }
            import::TrackImportStart::Complete(result) => {
                import::finish_track_import(&self.player_state, &self.selection_state, Ok(result))
            }
        }
    }

    pub fn take_track_import_error(&mut self) -> Option<String> {
        self.track_import_error.take()
    }

    fn poll_track_import(&mut self) {
        let event = match self.pending_track_import.as_mut() {
            Some(pending) => pending.subscription.try_next(),
            None => return,
        };
        let result = match event {
            shrimply_resource_pipeline::TryNext::Event(
                shrimply_resource_pipeline::Event::Finished(info),
            ) => {
                let pending = self
                    .pending_track_import
                    .take()
                    .expect("finished track import must exist");
                import::finish_track_import_inspection(
                    &mut self.project.borrow_mut(),
                    pending.context,
                    &info,
                )
            }
            shrimply_resource_pipeline::TryNext::Event(
                shrimply_resource_pipeline::Event::Failed(error),
            ) => {
                self.pending_track_import = None;
                Err(error.to_string())
            }
            shrimply_resource_pipeline::TryNext::Event(
                shrimply_resource_pipeline::Event::Cancelled,
            )
            | shrimply_resource_pipeline::TryNext::Closed => {
                self.pending_track_import = None;
                return;
            }
            shrimply_resource_pipeline::TryNext::Event(
                shrimply_resource_pipeline::Event::Progress(_),
            )
            | shrimply_resource_pipeline::TryNext::Empty => return,
        };
        if let Err(error) =
            import::finish_track_import(&self.player_state, &self.selection_state, result)
        {
            self.track_import_error = Some(error);
        }
    }

    fn poll_pointer_lock(&mut self) {
        let Some(bounds) = self.pointer_lock_bounds else {
            return;
        };
        if let Some((delta_x, delta_y)) = self
            .pointer_lock
            .as_mut()
            .and_then(shrimply_wayland_pointer_lock::WaylandPointerLock::poll)
        {
            let delta = vec2(delta_x as f32, delta_y as f32);
            let mut runtime = self.runtime.borrow_mut();
            let display_position = runtime
                .software_cursor
                .as_ref()
                .map(|cursor| cursor.position)
                .or(runtime.pointer_pos)
                .unwrap_or(bounds.min);
            runtime.pointer_pos = Some(runtime.pointer_pos.unwrap_or(display_position) + delta);
            runtime
                .software_cursor
                .as_mut()
                .expect("toolkit pointer lock must own a software cursor")
                .position = bounds.wrap_point(display_position + delta);
        }
    }

    pub fn pointer_move(&self, x: f32, y: f32, ctrl: bool, shift: bool) {
        let mut runtime = self.runtime.borrow_mut();
        runtime.modifiers = TimelineModifiers { ctrl, shift };
        runtime.pointer_pos = Some(vec2(x, y));
    }

    pub fn pointer_cursor(&self) -> TimelineCursor {
        let runtime = self.runtime.borrow();
        let Some(position) = runtime.pointer_pos else {
            return TimelineCursor::Default;
        };
        interaction::timeline_cursor(
            &self.project.borrow(),
            &runtime,
            f64::from(position.x),
            f64::from(position.y),
        )
    }

    pub fn pointer_leave(&self) {
        let mut runtime = self.runtime.borrow_mut();
        runtime.pointer_pos = None;
        runtime.cut_preview = None;
    }

    pub fn pointer_press(
        &self,
        button: ToolkitPointerButton,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) {
        let mut runtime = self.runtime.borrow_mut();
        runtime.modifiers = TimelineModifiers { ctrl, shift };
        let position = vec2(x, y);
        runtime.pointer_pos = Some(position);
        runtime.pointer_press_origin = Some(position);
        runtime.pointer_release_pos = None;
        match button {
            ToolkitPointerButton::Primary => {
                runtime.primary_pressed = true;
                runtime.primary_down = true;
            }
            ToolkitPointerButton::Middle => {
                runtime.middle_pressed = true;
                runtime.middle_down = true;
            }
        }
    }

    pub fn pointer_release(
        &self,
        button: ToolkitPointerButton,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) {
        let mut runtime = self.runtime.borrow_mut();
        runtime.modifiers = TimelineModifiers { ctrl, shift };
        let position = vec2(x, y);
        runtime.pointer_pos = Some(position);
        runtime.pointer_release_pos = Some(position);
        match button {
            ToolkitPointerButton::Primary => {
                runtime.primary_released = true;
                runtime.primary_down = false;
            }
            ToolkitPointerButton::Middle => {
                runtime.middle_released = true;
                runtime.middle_down = false;
            }
        }
    }

    /// Starts relative pointer input for toolkit-backed timelines.
    ///
    /// # Safety
    ///
    /// The pointers must belong to the live Wayland connection hosting this timeline
    /// and remain valid until `end_pointer_lock` is called.
    pub unsafe fn begin_pointer_lock(
        &mut self,
        display: *mut c_void,
        surface: *mut c_void,
        seat: *mut c_void,
        software_cursor: shrimply_skia_adw_core::cursor::SoftwareCursor,
    ) -> bool {
        if self.pointer_lock.is_some() {
            return true;
        }
        let Some(lock) = (unsafe {
            shrimply_wayland_pointer_lock::WaylandPointerLock::new(display, surface, seat)
        }) else {
            return false;
        };
        let position = self.runtime.borrow().pointer_pos.unwrap_or(Vec2::ZERO);
        self.runtime.borrow_mut().software_cursor = Some(TimelineSoftwareCursor {
            position,
            cursor: software_cursor,
        });
        self.pointer_lock = Some(lock);
        self.pointer_lock_origin = Some(position);
        true
    }

    pub fn end_pointer_lock(&mut self, ctrl: bool, shift: bool) {
        self.poll_pointer_lock();
        let Some(mut lock) = self.pointer_lock.take() else {
            return;
        };
        let origin = self
            .pointer_lock_origin
            .take()
            .expect("toolkit pointer lock must have a local origin");
        let cursor = self
            .runtime
            .borrow()
            .software_cursor
            .as_ref()
            .expect("toolkit pointer lock must own a software cursor")
            .position;
        lock.restore_cursor_with_offset(
            f64::from(cursor.x - origin.x),
            f64::from(cursor.y - origin.y),
        );
        drop(lock);
        let mut runtime = self.runtime.borrow_mut();
        runtime.software_cursor = None;
        runtime.modifiers = TimelineModifiers { ctrl, shift };
        runtime.pointer_release_pos = runtime.pointer_pos;
        runtime.middle_released = true;
        runtime.middle_down = false;
    }

    pub fn scroll(&self, dx: f32, dy: f32, ctrl: bool, shift: bool) {
        let mut runtime = self.runtime.borrow_mut();
        let modifiers = TimelineModifiers { ctrl, shift };
        runtime.modifiers = modifiers;
        let pointer = runtime.pointer_pos;
        runtime.pending_scrolls.push(TimelineScrollEvent {
            delta: vec2(
                dx * SCROLL_PIXELS_PER_STEP as f32,
                dy * SCROLL_PIXELS_PER_STEP as f32,
            ),
            ctrl,
            pointer,
        });
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

    pub fn destroy(&self) {
        self.runtime.borrow_mut().renderer.destroy();
    }

    fn toggle_sequence(&self, path: Vec<uuid::Uuid>) {
        let mut project = self.project.borrow_mut();
        let collapsed = if let Some(index) = project
            .expanded_sequence_paths
            .iter()
            .position(|expanded| *expanded == path)
        {
            project.expanded_sequence_paths.remove(index);
            true
        } else {
            project.expanded_sequence_paths.push(path.clone());
            false
        };
        project::save_view_state(&project);
        drop(project);
        if collapsed {
            let mut selected = selection_state::selected_nested_items(&self.selection_state);
            selected.retain(|item| !item.sequence_path().starts_with(&path));
            let focused = selection_state::focused_nested_item(&self.selection_state)
                .filter(|item| selected.contains(item));
            selection_state::set_selected_nested_items(&self.selection_state, selected, focused);
        }
    }
}

pub fn new(
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    playback_performance: playback_performance::SharedCollector,
    selection_state: SharedSelectionState,
    preferences: preferences_store::SharedPreferences,
    audio_levels: SharedAudioLevels,
    property_clipboard: shrimply_property_transfer::SharedClipboard,
) -> gtk::Widget {
    let area = gtk::GLArea::builder()
        .auto_render(false)
        .has_depth_buffer(false)
        .has_stencil_buffer(false)
        .hexpand(true)
        .vexpand(false)
        .build();
    area.set_focusable(true);
    let toggle_state = player_state.clone();
    let speed_state = player_state.clone();
    playback_shortcuts::attach_space_play_toggle(
        &area,
        move || player_state::toggle_playing(&toggle_state),
        move || player_state::step_playback_speed_forward(&speed_state),
    );

    let playhead_visibility_requested = Rc::new(Cell::new(false));
    let (timeline_zoom, timeline_center) = {
        let project = project.borrow();
        (
            project.timeline_zoom,
            project
                .timeline_zoom
                .filter(|zoom| *zoom > Time::ZERO)
                .and(project.cursor_position),
        )
    };
    let runtime = Rc::new(RefCell::new(TimelineRuntime::new(
        WaveformMap::new(),
        preferences_store::snapshot(&preferences),
        playhead_visibility_requested.clone(),
        timeline_zoom,
        timeline_center,
        property_clipboard,
    )));
    let preference_runtime = runtime.clone();
    let preference_area = area.clone();
    preferences_store::connect(&preferences, move |snapshot| {
        preference_runtime.borrow_mut().apply_preferences(&snapshot);
        preference_area.queue_render();
    });
    start_waveform_loader(&area, project.clone(), runtime.clone());
    start_beat_loader(&area, project.clone(), runtime.clone());
    let performance_area = area.downgrade();
    let performance_updates = playback_performance::subscribe(&playback_performance);
    glib::spawn_future_local(async move {
        while performance_updates.recv().await.is_ok() {
            let Some(area) = performance_area.upgrade() else {
                break;
            };
            area.queue_render();
        }
    });
    interaction::add_input_controllers(
        &area,
        project.clone(),
        player_state.clone(),
        selection_state.clone(),
        runtime.clone(),
        preferences.clone(),
    );

    let redraw = area.clone();
    let redraw_project = project.clone();
    let redraw_runtime = runtime.clone();
    let waveform_reload_request = Rc::new(Cell::new(0_u64));
    let redraw_playhead_visibility_requested = playhead_visibility_requested.clone();
    player_state::connect_named(&player_state, "timeline redraw", move |event| {
        if matches!(event, player_state::PlayerEvent::State(_)) {
            redraw_playhead_visibility_requested.set(true);
        }
        if let player_state::PlayerEvent::Project(change) = event
            && change.audio_waveforms
        {
            let request = waveform_reload_request.get().wrapping_add(1);
            waveform_reload_request.set(request);
            let redraw = redraw.clone();
            let redraw_project = redraw_project.clone();
            let redraw_runtime = redraw_runtime.clone();
            let waveform_reload_request = waveform_reload_request.clone();
            glib::timeout_add_local_once(WAVEFORM_RELOAD_DELAY, move || {
                if waveform_reload_request.get() != request {
                    return;
                }
                start_waveform_loader(&redraw, redraw_project, redraw_runtime);
            });
        }
        if let player_state::PlayerEvent::Project(change) = event
            && (change.audio || change.audio_beats)
        {
            let redraw = redraw.clone();
            let redraw_project = redraw_project.clone();
            let redraw_runtime = redraw_runtime.clone();
            glib::idle_add_local_once(move || {
                start_beat_loader(&redraw, redraw_project, redraw_runtime);
            });
        }
        redraw.queue_render();
    });

    let recording_area = area.clone();
    let recording_project = project.clone();
    let recording_runtime = runtime.clone();
    let recording_player_state = player_state.clone();
    let recording_player_state_for_snapshot = player_state.clone();
    player_state::connect_named(&player_state, "timeline recording refresh", move |event| {
        if !matches!(event, player_state::PlayerEvent::State(_)) {
            return;
        }
        let recording_area = recording_area.clone();
        let recording_project = recording_project.clone();
        let recording_runtime = recording_runtime.clone();
        let recording_player_state = recording_player_state.clone();
        let recording_player_state_for_idle = recording_player_state_for_snapshot.clone();
        glib::idle_add_local_once(move || {
            let snapshot = player_state::snapshot(&recording_player_state_for_idle);
            if snapshot.playing {
                let mut stop_at_boundary = false;
                let mut runtime = recording_runtime.borrow_mut();
                if runtime.active_audio_recording.is_some() {
                    ensure_recording_duration(&recording_player_state_for_idle, snapshot.position);
                }
                if let Some(active) = runtime.active_video_recording.as_mut()
                    && active.ready
                    && !active.stopping
                {
                    ensure_recording_duration(&recording_player_state_for_idle, snapshot.position);
                    if active
                        .stop_at
                        .is_some_and(|stop_at| snapshot.position >= stop_at)
                    {
                        active.stopping = true;
                        active.recording.stop();
                        stop_at_boundary = true;
                    }
                }
                drop(runtime);
                if stop_at_boundary {
                    player_state::set_playing(&recording_player_state_for_idle, false);
                }
                return;
            }
            if let Some(active) = recording_runtime
                .borrow_mut()
                .active_video_recording
                .as_mut()
                && active.ready
                && !active.stopping
            {
                active.stopping = true;
                active.recording.stop();
            }
            let active = recording_runtime.borrow_mut().active_audio_recording.take();
            let Some(active) = active else {
                return;
            };
            if let Err(error) = finish_audio_recording(
                &recording_area,
                &recording_project,
                &recording_player_state,
                active,
            ) {
                interaction::show_error_dialog(&recording_area, "Could not record audio", &error);
            }
            recording_area.queue_render();
        });
    });

    let render_runtime = runtime.clone();
    let render_project = project.clone();
    let render_player_state = player_state.clone();
    let render_selection_state = selection_state.clone();
    let render_performance = playback_performance.clone();
    area.connect_render(move |area, _| {
        if let Some(error) = area.error() {
            tracing::error!("Timeline GLArea error: {error}");
            return glib::Propagation::Stop;
        }
        area.make_current();
        if let Some(error) = area.error() {
            tracing::error!("Timeline GLArea error after make_current: {error}");
            return glib::Propagation::Stop;
        }

        let width = area.width().max(1);
        let height = area.height().max(1);
        let pixels_per_point = area.scale_factor().max(1) as f32;
        let screen_size_px = glam::UVec2::new(
            (width as f32 * pixels_per_point).round().max(1.0) as u32,
            (height as f32 * pixels_per_point).round().max(1.0) as u32,
        );
        let _span = tracing::debug_span!(
            "timeline.render",
            surface.width = width,
            surface.height = height,
            pixels_per_point,
        )
        .entered();

        shrimply_support::crash::set_context(format!(
            "timeline render begin size={}x{} scale={}",
            width, height, pixels_per_point
        ));
        let mut runtime = render_runtime.borrow_mut();
        runtime.track_controls_animating = false;
        shrimply_support::crash::set_context("timeline render begin_frame");
        let painter = match runtime.renderer.begin_frame(
            screen_size_px,
            pixels_per_point,
            crate::theme::current().view_bg,
        ) {
            Ok(painter) => painter,
            Err(error) => {
                tracing::error!("Could not initialize skia timeline renderer: {error}");
                return glib::Propagation::Stop;
            }
        };
        let accent_color = adw::StyleManager::for_display(&area.display())
            .accent_color_rgba()
            .into();
        shrimply_support::crash::set_context(format!(
            "timeline render ui begin drag_mode={:?} playing={}",
            runtime.view.drag_mode,
            player_state::snapshot(&render_player_state).playing
        ));
        timeline_gtk(
            &render_project,
            &render_player_state,
            &render_selection_state,
            &playback_performance::snapshot(&render_performance),
            &mut runtime,
            &painter,
            width as f64,
            height as f64,
            accent_color,
        );
        shrimply_support::crash::set_context(format!(
            "timeline render ui end drag_mode={:?} pending_pause={}",
            runtime.view.drag_mode, runtime.pending_pause_playback
        ));
        runtime.finish_pointer_frame();
        shrimply_support::crash::set_context("timeline render end_frame");
        if let Err(error) = runtime.renderer.end_frame() {
            tracing::error!("Could not finalize skia timeline renderer: {error}");
            return glib::Propagation::Stop;
        }
        let pending_audio_record = runtime.pending_audio_record.take();
        let pause_after_audio_recording_stop = if let Some(key) = pending_audio_record {
            handle_audio_recording(
                area,
                &render_project,
                &render_player_state,
                &mut runtime,
                key,
            )
        } else {
            false
        };
        let pending_video_record = runtime.pending_video_record.take();
        let pause_for_video_recording = if let Some(key) = pending_video_record {
            handle_video_recording(
                area,
                &render_project,
                &render_player_state,
                &render_runtime,
                &mut runtime,
                key,
            )
        } else {
            false
        };
        let pending_track_toggle = runtime.pending_track_toggle.take();
        let pending_sequence_toggle = runtime.pending_sequence_toggle.take();
        let pending_track_add_menu = runtime.pending_track_add_menu.take();
        let pending_pause_playback = std::mem::take(&mut runtime.pending_pause_playback);
        let track_controls_animating = runtime.track_controls_animating;
        drop(runtime);

        if pause_after_audio_recording_stop || pause_for_video_recording || pending_pause_playback {
            shrimply_support::crash::set_context(format!(
                "timeline post-render pause playback audio_recording_stop={} video_recording={} item_edit={}",
                pause_after_audio_recording_stop, pause_for_video_recording, pending_pause_playback
            ));
            player_state::set_playing(&render_player_state, false);
        }
        if track_controls_animating {
            area.queue_render();
            interaction::start_timeline_animation_tick(area, render_runtime.clone());
        }
        if let Some(key) = pending_track_toggle {
            toggle_track_enabled(area, &render_project, &render_player_state, key);
        }
        if let Some(path) = pending_sequence_toggle {
            let mut project = render_project.borrow_mut();
            let collapsed = if let Some(index) = project
                .expanded_sequence_paths
                .iter()
                .position(|expanded| *expanded == path)
            {
                project.expanded_sequence_paths.remove(index);
                true
            } else {
                project.expanded_sequence_paths.push(path.clone());
                false
            };
            crate::project::save_view_state(&project);
            drop(project);
            if collapsed {
                let mut selected = selection_state::selected_nested_items(&render_selection_state);
                selected.retain(|item| !item.sequence_path().starts_with(&path));
                let focused = selection_state::focused_nested_item(&render_selection_state)
                    .filter(|item| selected.contains(item));
                selection_state::set_selected_nested_items(
                    &render_selection_state,
                    selected,
                    focused,
                );
            }
            area.queue_render();
        }
        if let Some(request) = pending_track_add_menu {
            show_track_add_menu(
                area,
                &render_project,
                &render_player_state,
                &render_selection_state,
                &render_runtime,
                request,
            );
        }

        glib::Propagation::Stop
    });

    let style = adw::StyleManager::for_display(&area.display());
    let theme_area = area.clone();
    style.connect_dark_notify(move |_| theme_area.queue_render());

    let destroy_runtime = runtime.clone();
    area.connect_unrealize(move |area| {
        area.make_current();
        destroy_runtime.borrow_mut().renderer.destroy();
    });

    let sidebar = timeline_sidebar(&area, &preferences);
    let timeline = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    timeline.append(&sidebar);
    timeline.append(&area);

    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_wide_handle(false);
    split.set_start_child(Some(&timeline));
    split.set_end_child(Some(&audio_meter_gtk::new(move || audio_levels.take_peaks())));
    split.set_resize_start_child(true);
    split.set_resize_end_child(false);
    split.set_shrink_start_child(false);
    split.set_shrink_end_child(false);
    split.upcast()
}
