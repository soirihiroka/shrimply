use super::*;
use shrimply_gtk_components::ui::I18nWidgetExt;

use shrimply_preview_interaction_core::fullscreen::{CONTROLS_HIDE_DELAY, ControlsMotion};
const RESTORE_ICON: &str = "arrows-pointing-inward-symbolic";
const CONTROLS_CLASS: &str = "preview-fullscreen-controls";

const CONTROLS_CSS: &str = "
box.preview-fullscreen-controls {
    border-radius: 22px;
    padding: 4px 8px;
}

box.preview-fullscreen-controls button {
    border-radius: 999px;
}

button.preview-control-chip {
    padding: 4px 4px;
    font-weight: 700;
}

";

struct PreviewFullscreenRestore {
    window: gtk::Window,
    toolbar_view: Option<(adw::ToolbarView, bool)>,
    hidden_widgets: Vec<(gtk::Widget, bool)>,
    side_controls_visible: bool,
    controls_visible: bool,
    controls_halign: gtk::Align,
    controls_valign: gtk::Align,
    controls_hexpand: bool,
    controls_vexpand: bool,
    controls_had_osd_class: bool,
    left_bar_visible: bool,
}

#[derive(Clone)]
pub(super) struct Widgets {
    pub(super) layout: gtk::Box,
    pub(super) video_overlay: gtk::Overlay,
    pub(super) controls: gtk::Box,
    pub(super) side_controls: gtk::Box,
    pub(super) left_bar: gtk::Box,
    pub(super) button: gtk::Button,
    pub(super) video_surface: PreviewController,
}

#[derive(Clone)]
struct PreviewFullscreenState {
    active: Rc<Cell<bool>>,
    restore: Rc<RefCell<Option<PreviewFullscreenRestore>>>,
    notify_id: Rc<RefCell<Option<glib::SignalHandlerId>>>,
    hide_source_id: Rc<RefCell<Option<glib::SourceId>>>,
    hide_generation: Rc<Cell<u64>>,
    motion: Rc<ControlsMotion>,
}

pub(super) fn attach(widgets: Widgets, player_state: SharedPlayerState) {
    let state = PreviewFullscreenState {
        active: Rc::new(Cell::new(false)),
        restore: Rc::new(RefCell::new(None::<PreviewFullscreenRestore>)),
        notify_id: Rc::new(RefCell::new(None::<glib::SignalHandlerId>)),
        hide_source_id: Rc::new(RefCell::new(None::<glib::SourceId>)),
        hide_generation: Rc::new(Cell::new(0u64)),
        motion: Rc::new(ControlsMotion::default()),
    };

    attach_fullscreen_controls_motion(&widgets, &state);
    attach_fullscreen_escape_key(&widgets, &state);
    attach_fullscreen_playback_hide(&widgets, &state, player_state);

    let click_widgets = widgets.clone();
    let click_state = state.clone();
    widgets.button.connect_clicked(move |_| {
        if click_state.active.get() {
            restore_preview_fullscreen(&click_widgets, &click_state, true);
        } else {
            enter_preview_fullscreen(&click_widgets, &click_state);
        }
    });
}

fn enter_preview_fullscreen(widgets: &Widgets, state: &PreviewFullscreenState) {
    let Some(window) = widgets
        .layout
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok())
    else {
        return;
    };

    let mut hidden_widgets = Vec::new();
    let top_paned = widgets
        .layout
        .ancestor(gtk::Paned::static_type())
        .and_then(|parent| parent.downcast::<gtk::Paned>().ok());
    if let Some(inspector) = top_paned.as_ref().and_then(|paned| paned.start_child()) {
        hidden_widgets.push((inspector.clone(), inspector.get_visible()));
        inspector.set_visible(false);
    }
    if let Some(timeline) = top_paned
        .as_ref()
        .and_then(|paned| paned.parent())
        .and_then(|parent| parent.downcast::<gtk::Paned>().ok())
        .and_then(|paned| paned.end_child())
    {
        hidden_widgets.push((timeline.clone(), timeline.get_visible()));
        timeline.set_visible(false);
    }

    let toolbar_view = toolbar_view_ancestor(widgets.layout.upcast_ref()).map(|toolbar_view| {
        let reveal_top_bars = toolbar_view.reveals_top_bars();
        toolbar_view.set_reveal_top_bars(false);
        (toolbar_view, reveal_top_bars)
    });

    let controls_visible = widgets.controls.get_visible();
    let side_controls_visible = widgets.side_controls.get_visible();
    let controls_halign = widgets.controls.halign();
    let controls_valign = widgets.controls.valign();
    let controls_hexpand = widgets.controls.hexpands();
    let controls_vexpand = widgets.controls.vexpands();
    let controls_had_osd_class = widgets.controls.has_css_class("osd");
    let left_bar_visible = widgets.left_bar.get_visible();
    widgets.layout.remove(&widgets.controls);
    widgets.controls.set_halign(gtk::Align::Fill);
    widgets.controls.set_valign(gtk::Align::End);
    widgets.controls.set_hexpand(true);
    widgets.controls.set_vexpand(false);
    widgets.side_controls.set_visible(false);
    widgets.left_bar.set_visible(false);
    widgets.controls.add_css_class("osd");
    widgets.controls.add_css_class(CONTROLS_CLASS);
    widgets.controls.set_visible(true);
    widgets.video_overlay.add_overlay(&widgets.controls);
    widgets
        .video_overlay
        .set_measure_overlay(&widgets.controls, false);

    *state.restore.borrow_mut() = Some(PreviewFullscreenRestore {
        window: window.clone(),
        toolbar_view,
        hidden_widgets,
        side_controls_visible,
        controls_visible,
        controls_halign,
        controls_valign,
        controls_hexpand,
        controls_vexpand,
        controls_had_osd_class,
        left_bar_visible,
    });
    state.active.set(true);
    state.motion.reset();
    widgets.video_surface.set_fullscreen(true);
    set_fullscreen_button_mode(&widgets.button, true);
    show_fullscreen_controls(widgets, state);

    let notify_widgets = widgets.clone();
    let notify_state = state.clone();
    *state.notify_id.borrow_mut() = Some(window.connect_fullscreened_notify(move |window| {
        if !window.is_fullscreen() && notify_state.active.get() {
            restore_preview_fullscreen(&notify_widgets, &notify_state, false);
        }
    }));

    window.set_fullscreened(true);
}

fn restore_preview_fullscreen(
    widgets: &Widgets,
    state: &PreviewFullscreenState,
    leave_window_fullscreen: bool,
) {
    let Some(restore) = state.restore.borrow_mut().take() else {
        state.active.set(false);
        widgets.video_surface.set_fullscreen(false);
        set_fullscreen_button_mode(&widgets.button, false);
        return;
    };

    if let Some(notify_id) = state.notify_id.borrow_mut().take() {
        restore.window.disconnect(notify_id);
    }

    for (widget, was_visible) in restore.hidden_widgets {
        widget.set_visible(was_visible);
    }
    if let Some((toolbar_view, revealed_top_bars)) = restore.toolbar_view {
        toolbar_view.set_reveal_top_bars(revealed_top_bars);
    }
    widgets.video_overlay.remove_overlay(&widgets.controls);
    widgets.layout.append(&widgets.controls);
    widgets.controls.set_halign(restore.controls_halign);
    widgets.controls.set_valign(restore.controls_valign);
    widgets.controls.set_hexpand(restore.controls_hexpand);
    widgets.controls.set_vexpand(restore.controls_vexpand);
    widgets
        .side_controls
        .set_visible(restore.side_controls_visible);
    widgets.left_bar.set_visible(restore.left_bar_visible);
    if !restore.controls_had_osd_class {
        widgets.controls.remove_css_class("osd");
    }
    widgets.controls.remove_css_class(CONTROLS_CLASS);
    widgets.controls.set_visible(restore.controls_visible);
    widgets.video_surface.set_caption_bottom_inset(0.0);
    widgets.video_surface.set_fullscreen(false);
    if leave_window_fullscreen {
        restore.window.set_fullscreened(false);
    }

    state.active.set(false);
    state.motion.reset();
    cancel_fullscreen_controls_hide(state);
    set_fullscreen_button_mode(&widgets.button, false);
}

fn attach_fullscreen_escape_key(widgets: &Widgets, state: &PreviewFullscreenState) {
    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);

    let target = widgets.layout.clone();
    let widgets = widgets.clone();
    let state = state.clone();
    key.connect_key_pressed(move |_, key, _, _| {
        if key != gdk::Key::Escape || !state.active.get() {
            return glib::Propagation::Proceed;
        }

        restore_preview_fullscreen(&widgets, &state, true);
        glib::Propagation::Stop
    });
    target.add_controller(key);
}

fn attach_fullscreen_playback_hide(
    widgets: &Widgets,
    state: &PreviewFullscreenState,
    player_state: SharedPlayerState,
) {
    let last_playing = Rc::new(Cell::new(player_state::snapshot(&player_state).playing));
    let hide_widgets = widgets.clone();
    let hide_state = state.clone();
    let hide_last_playing = last_playing.clone();
    let hide_player_state = player_state.clone();
    player_state::connect_named(
        &player_state,
        "preview fullscreen playback hide",
        move |_| {
            let playing = player_state::snapshot(&hide_player_state).playing;
            let started_playing = playing && !hide_last_playing.get();
            hide_last_playing.set(playing);
            if started_playing && hide_state.active.get() {
                hide_fullscreen_controls(&hide_widgets, &hide_state, true);
            }
        },
    );
}

fn attach_fullscreen_controls_motion(widgets: &Widgets, state: &PreviewFullscreenState) {
    let overlay_motion = gtk::EventControllerMotion::new();
    overlay_motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let motion_widgets = widgets.clone();
    let motion_state = state.clone();
    overlay_motion.connect_motion(move |_, x, y| {
        if motion_state.active.get() && motion_state.motion.pointer_motion(vec2(x as f32, y as f32))
        {
            show_fullscreen_controls(&motion_widgets, &motion_state);
        }
    });
    let leave_widgets = widgets.clone();
    let leave_state = state.clone();
    overlay_motion.connect_leave(move |_| {
        if leave_state.active.get() {
            hide_fullscreen_controls(&leave_widgets, &leave_state, false);
        }
    });
    widgets.video_overlay.add_controller(overlay_motion);

    let controls_motion = gtk::EventControllerMotion::new();
    let enter_widgets = widgets.clone();
    let enter_state = state.clone();
    controls_motion.connect_enter(move |_, x, y| {
        if enter_state.active.get() {
            enter_state.motion.controls_enter(vec2(x as f32, y as f32));
            enter_widgets.controls.set_visible(true);
            update_fullscreen_caption_inset(&enter_widgets);
            schedule_fullscreen_controls_hide(&enter_widgets, &enter_state);
        }
    });
    let motion_widgets = widgets.clone();
    let motion_state = state.clone();
    controls_motion.connect_motion(move |_, x, y| {
        if motion_state.active.get()
            && motion_state
                .motion
                .controls_motion(vec2(x as f32, y as f32))
        {
            show_fullscreen_controls(&motion_widgets, &motion_state);
        }
    });
    let leave_widgets = widgets.clone();
    let leave_state = state.clone();
    controls_motion.connect_leave(move |_| {
        schedule_fullscreen_controls_hide(&leave_widgets, &leave_state);
    });
    widgets.controls.add_controller(controls_motion);
}

fn show_fullscreen_controls(widgets: &Widgets, state: &PreviewFullscreenState) {
    state.motion.shown();
    widgets.controls.set_visible(true);
    update_fullscreen_caption_inset(widgets);
    schedule_fullscreen_controls_hide(widgets, state);

    let widgets = widgets.clone();
    glib::idle_add_local_once(move || {
        if widgets.controls.get_visible() {
            update_fullscreen_caption_inset(&widgets);
        }
    });
}

fn hide_fullscreen_controls(
    widgets: &Widgets,
    state: &PreviewFullscreenState,
    require_pointer_move: bool,
) {
    widgets.controls.set_visible(false);
    widgets.video_surface.set_caption_bottom_inset(0.0);
    state.motion.hidden(require_pointer_move);
    cancel_fullscreen_controls_hide(state);
}

fn update_fullscreen_caption_inset(widgets: &Widgets) {
    widgets
        .video_surface
        .set_caption_bottom_inset(fullscreen_controls_caption_inset(&widgets.controls));
}

fn fullscreen_controls_caption_inset(controls: &gtk::Box) -> f32 {
    (controls.height() + controls.margin_top() + controls.margin_bottom()).max(0) as f32
}

fn schedule_fullscreen_controls_hide(widgets: &Widgets, state: &PreviewFullscreenState) {
    let generation = state.hide_generation.get().wrapping_add(1);
    state.hide_generation.set(generation);
    cancel_fullscreen_controls_hide(state);

    let widgets = widgets.clone();
    let timeout_state = state.clone();
    let source_id = glib::timeout_add_local_once(CONTROLS_HIDE_DELAY, move || {
        timeout_state.hide_source_id.borrow_mut().take();
        let active = timeout_state.active.get();
        let current_generation = timeout_state.hide_generation.get();
        if active && current_generation == generation {
            hide_fullscreen_controls(&widgets, &timeout_state, true);
        }
    });
    *state.hide_source_id.borrow_mut() = Some(source_id);
}

fn cancel_fullscreen_controls_hide(state: &PreviewFullscreenState) {
    if let Some(source_id) = state.hide_source_id.borrow_mut().take() {
        source_id.remove();
    }
}

pub(super) fn install_css() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CONTROLS_CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn toolbar_view_ancestor(widget: &gtk::Widget) -> Option<adw::ToolbarView> {
    widget
        .ancestor(adw::ToolbarView::static_type())?
        .downcast()
        .ok()
}

fn set_fullscreen_button_mode(button: &gtk::Button, fullscreen: bool) {
    if fullscreen {
        button.set_icon_name(RESTORE_ICON);
        button.set_tooltip_i18n("Restore preview");
    } else {
        button.set_icon_name(PREVIEW_FULLSCREEN_ICON);
        button.set_tooltip_i18n("Fullscreen preview");
    }
}
