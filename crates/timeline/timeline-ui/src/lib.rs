use hashbrown::HashMap;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub use shrimply_audio as audio;
pub use shrimply_math_color::Color;
pub use shrimply_math_core::Fraction;
pub use shrimply_project::{caption, project, time_format, timeline_search};
pub use shrimply_state::player_state;
pub use shrimply_timeline::selection_state;
pub use shrimply_transcription as transcription;
pub use shrimply_ui_foundation::{desktop_open, playback_shortcuts, skia_font, skia_system_font};

pub mod preferences {
    pub use shrimply_state::preferences as store;
}

pub mod export {
    pub use shrimply_export::audio;
    pub use shrimply_ui_foundation::export_feedback::show_export_finished_for_widget;
}

use shrimply_math_media as math;
use shrimply_playback_performance as playback_performance;
#[cfg(feature = "screen-recording")]
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
use shrimply_timeline::{TrackGap, TrackKey};

mod audio_meter;
mod beat_grid;
mod clipboard;
mod context_menu;
mod drawing;
pub use shrimply_ui_foundation::cursor;
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
pub use shrimply_ui_foundation::canvas as renderer;
pub(crate) mod ruler;
mod setup;
mod silence;
mod snapping;
mod timeline_operation;
mod track_controls;
mod view;

use drawing::{
    TimelineInput, active_virtual_tracks, draw_timeline, item_rect, row_screen_y, row_y,
};
use frame::timeline_ui;
use geometry::*;
use recording::{
    create_audio_generator_item_at_playhead, create_generated_item_at_playhead,
    create_tts_item_at_playhead, create_video_generation_item_at_playhead,
    ensure_recording_duration, finish_audio_recording, handle_audio_recording,
    handle_video_recording, live_recording_draw,
};
use renderer::{TimelinePainter, TimelineRenderer};
use runtime::*;
use setup::*;
use track_controls::{
    for_each_visible_track_row, show_track_add_menu, timeline_sidebar, toggle_track_enabled,
    track_button_at, track_enabled, track_label_action_at, track_label_button_y, visible_row_range,
};
use view::*;

use items::{
    DragCollisionMode, DragIndicator, DragPreviewStatus, DraggedGroup, ItemKey, ResizeDrag,
    TimelineClipboard, TrackKind, fitted_transition_durations, is_item_dragged, is_item_selected,
    resize_item_times, row_for_track, target_item_times, target_track_index, transition_durations,
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
const SIDEBAR_WIDTH: i32 = 44;
const SIDEBAR_ICON_SIZE: i32 = 28;

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
        let mut runtime = preference_runtime.borrow_mut();
        runtime.default_visual_duration = snapshot.default_visual_duration;
        runtime.default_text_font_family = snapshot.default_text_font_family;
        runtime.beat_grid_enabled =
            timeline_beat_grid_from_preference(&snapshot.timeline_beat_grid);
        runtime.snap_radius_px = f64::from(snapshot.timeline_snap_radius_px);
        drop(runtime);
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
                #[cfg(feature = "screen-recording")]
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
            #[cfg(feature = "screen-recording")]
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
            Color::VIEW_BG_DARK,
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
        timeline_ui(
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
        #[cfg(feature = "screen-recording")]
        let pending_video_record = runtime.pending_video_record.take();
        #[cfg(feature = "screen-recording")]
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
        #[cfg(not(feature = "screen-recording"))]
        let pause_for_video_recording = {
            let _ = runtime.pending_video_record.take();
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

    let destroy_runtime = runtime.clone();
    area.connect_unrealize(move |area| {
        area.make_current();
        destroy_runtime.borrow_mut().renderer.destroy();
    });

    let sidebar = timeline_sidebar(&area, &runtime, &preferences);
    let timeline = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    timeline.append(&sidebar);
    timeline.append(&area);

    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_wide_handle(false);
    split.set_start_child(Some(&timeline));
    split.set_end_child(Some(&audio_meter::new(audio_levels)));
    split.set_resize_start_child(true);
    split.set_resize_end_child(false);
    split.set_shrink_start_child(false);
    split.set_shrink_end_child(false);
    split.upcast()
}
