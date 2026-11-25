pub use shrimply_visual_cuda::compositor::RgbaVideoFrame as RenderedVideoFrame;
use std::cell::RefCell;
use std::rc::Rc;

pub use shrimply_audio_engine as audio;
pub use shrimply_components_gtk::{desktop_open, playback_shortcuts, skia_font, skia_system_font};
pub use shrimply_editor_state::player_state;
pub use shrimply_math_color::Color;
pub use shrimply_math_core::Fraction;
pub use shrimply_project_document::{caption, project, time_format, timeline_search};
pub use shrimply_timeline_edit::selection_state;
pub use shrimply_transcription as transcription;

pub mod preferences {
    pub use shrimply_editor_state::preferences as store;
}

pub mod export {
    pub use shrimply_components_gtk::export_feedback::show_export_finished_for_widget;
    pub use shrimply_export_core::audio;
    pub use shrimply_export_gtk::present_export_job;
}

use shrimply_playback_performance as playback_performance;
use shrimply_video_recording as video_recording;

use crate::audio::SharedAudioLevels;
use crate::player_state::SharedPlayerState;
use crate::preferences::store as preferences_store;
use crate::project::Project;
use crate::selection_state::SharedSelectionState;
use adw::prelude::*;
use gtk::glib;
use renderer::vec2;
use shrimply_cross_ui_theme as theme;
use shrimply_timeline_edit::TrackKey;
pub use shrimply_timeline_skia::{
    ContextItemKind, ContextMenu, ContextMenuAction, ContextMenuControl, ContextMenuEntry,
    ContextMenuItem, ContextMenuRequest, CursorTool, DragCollisionMode, FoldedItemMenuContext,
    ItemMenuContext, TimelineTools, ToolState, TrackAddAction, TrackAddMenuEntry, TrackMenuContext,
    VideoFrameSelection, track_add_menu,
};

pub use shrimply_timeline_skia::beat_grid;
mod audio_meter_gtk;
mod clipboard;
mod context_menu;
mod native_menu;
pub use shrimply_components_gtk::cursor;
pub use shrimply_timeline_skia::drawing;
mod caption_tts;
mod drag_and_drop;
mod external_content;
pub use shrimply_timeline_skia::folded_sequence;
mod frame;
pub use shrimply_timeline_skia::geometry;
pub mod import;
mod interaction;
pub use shrimply_timeline_skia::items;
mod recording;
mod runtime;
pub use shrimply_components_gtk::canvas as renderer;
pub use shrimply_timeline_skia::ruler;
mod setup;
mod silence;
pub use shrimply_timeline_skia::timeline_operation;
mod track_controls;
pub use shrimply_timeline_skia::view;

use frame::timeline_gtk;
use recording::handle_video_recording;
use renderer::{TimelinePainter, TimelineRenderer};
use runtime::*;
use track_controls::{show_track_add_menu, timeline_sidebar};
use view::*;

use items::TrackKind;

pub use shrimply_timeline_skia::metrics::*;

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
        playback_performance.clone(),
    )));
    let preference_area = area.downgrade();
    preferences_store::connect(&preferences, move |_| {
        if let Some(area) = preference_area.upgrade() {
            area.queue_render();
        }
    });
    setup::watch_updates(&area, &runtime);
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
    let sidebar = timeline_sidebar(&area, &preferences);
    let sidebar_width = sidebar.measure(gtk::Orientation::Horizontal, -1).1;
    let tools_clip = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(false)
        .min_content_width(0)
        .width_request(sidebar_width)
        .child(&sidebar)
        .build();
    let audio_meter = audio_meter_gtk::new(move || audio_levels.take_peaks());
    let meter_width = shrimply_components_skia::audio_meter::DEFAULT_WIDTH;
    let meter_clip = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(false)
        .min_content_width(0)
        .width_request(meter_width)
        .child(&audio_meter)
        .build();
    let render_tools_clip = tools_clip.downgrade();
    let render_meter_clip = meter_clip.downgrade();
    let render_runtime = runtime.clone();
    let render_project = project.clone();
    let render_player_state = player_state.clone();
    let render_selection_state = selection_state.clone();
    let render_preferences = preferences.clone();
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

        shrimply_process_reporting::crash::set_context(format!(
            "timeline render begin size={}x{} scale={}",
            width, height, pixels_per_point
        ));
        let mut runtime = render_runtime.borrow_mut();
        shrimply_process_reporting::crash::set_context("timeline render begin_frame");
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
        shrimply_process_reporting::crash::set_context(format!(
            "timeline render ui begin drag_mode={:?} playing={}",
            runtime.scene.view().drag_mode,
            player_state::snapshot(&render_player_state).playing
        ));
        timeline_gtk(
            &mut runtime,
            &painter,
            width as f64,
            height as f64,
            accent_color,
        );
        shrimply_process_reporting::crash::set_context(format!(
            "timeline render ui end drag_mode={:?}",
            runtime.scene.view().drag_mode
        ));
        shrimply_process_reporting::crash::set_context("timeline render end_frame");
        if let Err(error) = runtime.renderer.end_frame() {
            tracing::error!("Could not finalize skia timeline renderer: {error}");
            return glib::Propagation::Stop;
        }
        if let Err(error) = recording::apply_video_recording_commands(&mut runtime) {
            interaction::show_error_dialog(area, "Could not record screen or application", &error);
        }
        let requests = runtime.scene.take_requests();
        let pending_audio_record = requests.audio_record;
        if let Some(request) = pending_audio_record
            && let Err(error) = runtime
                .scene
                .toggle_audio_recording_at(request.key, request.position)
        {
            interaction::show_error_dialog(area, "Could not record audio", &error);
        }
        let pending_video_record = requests.video_record;
        let pause_for_video_recording = if let Some(key) = pending_video_record {
            handle_video_recording(area, &render_runtime, &mut runtime, key)
        } else {
            false
        };
        let pending_track_add_menu = requests.track_add;
        let pending_pause_playback = requests.pause_playback;
        let pending_context_menu = requests.context_menu;
        let track_controls_animating = runtime.scene.animating();
        let tools_width =
            (f64::from(sidebar_width) * runtime.scene.tools_progress()).round() as i32;
        let monitor_width =
            (f64::from(meter_width) * runtime.scene.audio_monitor_progress()).round() as i32;
        drop(runtime);
        if let Some(clip) = render_tools_clip.upgrade() {
            clip.set_visible(tools_width > 0);
            clip.set_width_request(tools_width);
        }
        if let Some(clip) = render_meter_clip.upgrade() {
            clip.set_visible(monitor_width > 0);
            clip.set_width_request(monitor_width);
        }

        if let Some(request) = pending_context_menu {
            interaction::show_timeline_item_context_menu(
                area,
                &render_project,
                &render_player_state,
                &render_selection_state,
                &render_runtime,
                &render_preferences,
                request,
            );
        }

        if pause_for_video_recording || pending_pause_playback {
            shrimply_process_reporting::crash::set_context(format!(
                "timeline post-render pause playback video_recording={} item_edit={}",
                pause_for_video_recording, pending_pause_playback
            ));
            player_state::set_playing(&render_player_state, false);
        }
        if track_controls_animating {
            area.queue_render();
            interaction::start_timeline_animation_tick(area, render_runtime.clone());
        }
        if let Some(request) = pending_track_add_menu {
            show_track_add_menu(area, &render_runtime, request);
        }

        glib::Propagation::Stop
    });

    let style = adw::StyleManager::for_display(&area.display());
    let theme_area = area.clone();
    style.connect_dark_notify(move |_| theme_area.queue_render());

    let destroy_runtime = runtime.clone();
    area.connect_unrealize(move |area| {
        let mut runtime = destroy_runtime.borrow_mut();
        runtime.scene.suspend();
        if let Err(error) = recording::apply_video_recording_commands(&mut runtime) {
            tracing::error!(%error, "Could not stop screen recording while unrealizing timeline");
        }
        area.make_current();
        runtime.renderer.destroy();
    });

    let timeline = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    timeline.append(&tools_clip);
    timeline.append(&area);

    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_wide_handle(false);
    split.set_start_child(Some(&timeline));
    split.set_end_child(Some(&meter_clip));
    split.set_resize_start_child(true);
    split.set_resize_end_child(false);
    split.set_shrink_start_child(false);
    split.set_shrink_end_child(true);
    split.upcast()
}
