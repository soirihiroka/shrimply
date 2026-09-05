use shrimply_timeline_core::scene::{Event as TimelineEvent, PointerButton};
use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::rc::Rc;

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
use crate::audio::waveform;
use crate::player_state::SharedPlayerState;
use crate::preferences::store as preferences_store;
use crate::project::{
    AudioItem, Project, RepeatStrategy, Time, Transform, TransitionSide, VideoItem,
    VideoItemContent, VideoSampleMethod, default_playback_speed,
};
use crate::selection_state::SharedSelectionState;
use adw::prelude::*;
use gtk::glib;
use renderer::{Rect, Vec2, vec2};
use shrimply_core::timeline_value::{TimelineBool, TimelineValue};
use shrimply_cross_ui_theme as theme;
use shrimply_timeline::TrackKey;
pub use shrimply_timeline_core::{
    ContextItemKind, ContextMenu, ContextMenuAction, ContextMenuControl, ContextMenuEntry,
    ContextMenuItem, ContextMenuRequest, CursorTool, DragCollisionMode, FoldedItemMenuContext,
    ItemMenuContext, TimelineTools, ToolState, TrackAddAction, TrackAddMenuEntry, TrackMenuContext,
    VideoFrameSelection, track_add_menu,
};

pub use shrimply_timeline_core::beat_grid;
mod audio_meter_gtk;
mod clipboard;
mod context_menu;
mod native_menu;
pub use shrimply_gtk_components::cursor;
pub use shrimply_timeline_core::drawing;
mod caption_tts;
mod drag_and_drop;
mod external_content;
pub use shrimply_timeline_core::folded_sequence;
mod frame;
pub use shrimply_timeline_core::geometry;
pub mod import;
mod interaction;
pub use shrimply_timeline_core::items;
mod recording;
mod runtime;
pub use shrimply_gtk_components::canvas as renderer;
pub use shrimply_timeline_core::ruler;
mod setup;
mod silence;
use shrimply_timeline_core::snapping;
pub use shrimply_timeline_core::timeline_operation;
mod toolkit_context_menu;
mod track_controls;
pub use shrimply_timeline_core::view;

use drawing::row_screen_y;
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
    show_track_add_menu, timeline_sidebar, track_label_action_at, track_label_button_y,
};
use view::*;

use items::{ItemKey, TimelineClipboard, TrackKind};

pub use shrimply_timeline_core::metrics::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolkitPointerButton {
    Primary,
    Middle,
}

pub use shrimply_timeline_core::view::TimelineCursor;

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
    pointer_lock: Option<shrimply_wayland_pointer_lock::WaylandPointerLock>,
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
        let runtime = Rc::new(RefCell::new(TimelineRuntime::new(
            project.clone(),
            player_state.clone(),
            selection_state.clone(),
            preferences,
            property_clipboard,
        )));
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
            pointer_lock: None,
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
        self.poll_pointer_lock();
        let mut runtime = self.runtime.borrow_mut();
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
        runtime.renderer.end_frame()?;
        let requests = runtime.scene.take_requests();
        let pending_track_add_menu = requests.track_add;
        let pending_pause_playback = requests.pause_playback;
        let view = runtime.scene.view();
        drop(runtime);
        if pending_pause_playback {
            player_state::set_playing(&self.player_state, false);
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
        let default_text_font_family = runtime.scene.default_text_font_family.clone();
        let settings = shrimply_timeline_core::TrackAddSettings {
            default_visual_duration: runtime.scene.default_visual_duration,
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
        let default_visual_duration = self.runtime.borrow().scene.default_visual_duration;
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
        if let Some((delta_x, delta_y)) = self
            .pointer_lock
            .as_mut()
            .and_then(shrimply_wayland_pointer_lock::WaylandPointerLock::poll)
        {
            let delta = vec2(delta_x as f32, delta_y as f32);
            let mut runtime = self.runtime.borrow_mut();
            runtime.scene.relative_motion(delta);
        }
    }

    pub fn pointer_move(&self, x: f32, y: f32, ctrl: bool, shift: bool) {
        let mut runtime = self.runtime.borrow_mut();
        runtime.scene.event(TimelineEvent::Motion {
            point: vec2(x, y),
            modifiers: TimelineModifiers { ctrl, shift },
        });
    }

    pub fn pointer_cursor(&self) -> TimelineCursor {
        let runtime = self.runtime.borrow();
        let Some(position) = runtime.scene.pointer_state().position else {
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
        runtime.scene.event(TimelineEvent::Leave);
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
        runtime.scene.event(TimelineEvent::Press {
            point: vec2(x, y),
            double: false,
            modifiers: TimelineModifiers { ctrl, shift },
            button: match button {
                ToolkitPointerButton::Primary => PointerButton::Primary,
                ToolkitPointerButton::Middle => PointerButton::Middle,
            },
        });
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
        runtime.scene.event(TimelineEvent::Release {
            point: vec2(x, y),
            modifiers: TimelineModifiers { ctrl, shift },
            button: match button {
                ToolkitPointerButton::Primary => PointerButton::Primary,
                ToolkitPointerButton::Middle => PointerButton::Middle,
            },
        });
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
        let position = self
            .runtime
            .borrow()
            .scene
            .pointer_state()
            .position
            .unwrap_or(Vec2::ZERO);
        self.runtime
            .borrow_mut()
            .scene
            .begin_relative_pointer(position, software_cursor);
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
            .borrow_mut()
            .scene
            .end_relative_pointer()
            .expect("toolkit pointer lock must own a software cursor");
        lock.restore_cursor_with_offset(
            f64::from(cursor.x - origin.x),
            f64::from(cursor.y - origin.y),
        );
        drop(lock);
        let mut runtime = self.runtime.borrow_mut();
        if let Some(point) = runtime.scene.pointer_state().position {
            runtime.scene.event(TimelineEvent::Release {
                point,
                button: PointerButton::Middle,
                modifiers: TimelineModifiers { ctrl, shift },
            });
        }
    }

    pub fn scroll(&self, dx: f32, dy: f32, ctrl: bool, shift: bool) {
        let mut runtime = self.runtime.borrow_mut();
        let modifiers = TimelineModifiers { ctrl, shift };
        runtime
            .scene
            .event(shrimply_timeline_core::scene::Event::Modifiers(modifiers));
        let pointer = runtime.scene.pointer_state().position;
        runtime
            .scene
            .event(TimelineEvent::Scroll(TimelineScrollEvent {
                delta: vec2(
                    dx * SCROLL_PIXELS_PER_STEP as f32,
                    dy * SCROLL_PIXELS_PER_STEP as f32,
                ),
                ctrl,
                pointer,
            }));
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
        let mut runtime = self.runtime.borrow_mut();
        runtime.scene.suspend();
        runtime.renderer.destroy();
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

    let runtime = Rc::new(RefCell::new(TimelineRuntime::new(
        project.clone(),
        player_state.clone(),
        selection_state.clone(),
        preferences.clone(),
        property_clipboard,
    )));
    let preference_area = area.downgrade();
    preferences_store::connect(&preferences, move |_| {
        if let Some(area) = preference_area.upgrade() {
            area.queue_render();
        }
    });
    setup::watch_updates(&area, &runtime);
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

    let redraw = area.downgrade();
    let redraw_alive = redraw.clone();
    player_state::connect_while_alive_named(
        &player_state,
        "timeline redraw",
        move || redraw_alive.upgrade().is_some(),
        move |_| {
            if let Some(area) = redraw.upgrade() {
                area.queue_render();
            }
        },
    );
    let recording_area = area.downgrade();
    let recording_project = project.clone();
    let recording_runtime = Rc::downgrade(&runtime);
    let recording_player_state = player_state.clone();
    let recording_player_state_for_snapshot = player_state.clone();
    let recording_alive = recording_runtime.clone();
    player_state::connect_while_alive_named(
        &player_state,
        "timeline recording refresh",
        move || recording_alive.strong_count() > 0,
        move |event| {
            let (Some(recording_area), Some(recording_runtime)) =
                (recording_area.upgrade(), recording_runtime.upgrade())
            else {
                return;
            };
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
                        ensure_recording_duration(
                            &recording_player_state_for_idle,
                            snapshot.position,
                        );
                    }
                    if let Some(active) = runtime.active_video_recording.as_mut()
                        && active.ready
                        && !active.stopping
                    {
                        ensure_recording_duration(
                            &recording_player_state_for_idle,
                            snapshot.position,
                        );
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
                    interaction::show_error_dialog(
                        &recording_area,
                        "Could not record audio",
                        &error,
                    );
                }
                recording_area.queue_render();
            });
        },
    );

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
            runtime.scene.view().drag_mode,
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
            "timeline render ui end drag_mode={:?}",
            runtime.scene.view().drag_mode
        ));
        shrimply_support::crash::set_context("timeline render end_frame");
        if let Err(error) = runtime.renderer.end_frame() {
            tracing::error!("Could not finalize skia timeline renderer: {error}");
            return glib::Propagation::Stop;
        }
        let requests = runtime.scene.take_requests();
        let pending_audio_record = requests.audio_record;
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
        let pending_video_record = requests.video_record;
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
        let pending_track_add_menu = requests.track_add;
        let pending_pause_playback = requests.pause_playback;
        let track_controls_animating = runtime.scene.animating();
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
        destroy_runtime.borrow_mut().scene.suspend();
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
    split.set_end_child(Some(&audio_meter_gtk::new(move || {
        audio_levels.take_peaks()
    })));
    split.set_resize_start_child(true);
    split.set_resize_end_child(false);
    split.set_shrink_start_child(false);
    split.set_shrink_end_child(false);
    split.upcast()
}
