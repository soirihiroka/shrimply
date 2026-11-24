use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
use gtk::glib;
use shrimply_components_core::layout::INSPECTOR_OVERLAY_WIDTH;
use shrimply_preferences_core as preferences;

/// Keep the preview in its Paned; libadwaita owns the inspector overlay.
/// Overlay switching depends solely on window width.
pub(super) fn connect(
    paned: &gtk::Paned,
    inspector: &gtk::Widget,
    toggle: &gtk::ToggleButton,
    overlay_mode: Rc<Cell<bool>>,
    stop_panel: Rc<dyn Fn()>,
    preferences: &preferences::SharedPreferences,
) -> adw::BreakpointBin {
    let split = adw::OverlaySplitView::builder()
        .content(paned)
        .pin_sidebar(true)
        .min_sidebar_width(0.0)
        .max_sidebar_width(0.0)
        .sidebar_width_unit(adw::LengthUnit::Px)
        .overflow(gtk::Overflow::Hidden)
        .show_sidebar(false)
        .build();
    // An empty sidebar has zero slide distance. Only animate the native overlay
    // once it owns the inspector and has allocated it.
    toggle.connect_toggled({
        let split = split.downgrade();
        move |toggle| {
            if let Some(split) = split.upgrade()
                && split.is_collapsed()
                && split.distance() > 0.0
            {
                split.set_show_sidebar(toggle.is_active());
            }
        }
    });
    split.connect_show_sidebar_notify({
        let toggle = toggle.downgrade();
        move |split| {
            if split.is_collapsed()
                && let Some(toggle) = toggle.upgrade()
            {
                toggle.set_active(split.shows_sidebar());
            }
        }
    });
    let bin = adw::BreakpointBin::new();
    bin.set_child(Some(&split));
    let preview = paned.end_child().expect("persistent preview pane");
    bin.set_size_request(
        preview.measure(gtk::Orientation::Horizontal, -1).0,
        paned.measure(gtk::Orientation::Vertical, -1).0,
    );
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        f64::from(INSPECTOR_OVERLAY_WIDTH).next_down(),
        adw::LengthUnit::Px,
    ));
    let saved_width = {
        let preferences = preferences.clone();
        move || {
            preferences::snapshot(&preferences)
                .editor_layout
                .inspector_width
                .round() as i32
        }
    };
    breakpoint.connect_apply({
        let split = split.downgrade();
        let paned = paned.downgrade();
        let inspector = inspector.downgrade();
        let toggle = toggle.downgrade();
        let overlay_mode = overlay_mode.clone();
        let stop_panel = stop_panel.clone();
        let saved_width = saved_width.clone();
        move |_| {
            let (Some(split), Some(paned), Some(inspector)) =
                (split.upgrade(), paned.upgrade(), inspector.upgrade())
            else {
                return;
            };
            overlay_mode.set(true);
            stop_panel();
            paned.set_start_child(None::<&gtk::Widget>);
            inspector.set_visible(true);
            split.set_max_sidebar_width(f64::from(saved_width()));
            split.set_sidebar(Some(&inspector));
            split.set_collapsed(true);
            let toggle = toggle.clone();
            split.add_tick_callback(move |split, _| {
                if !split.is_collapsed() {
                    return glib::ControlFlow::Break;
                }
                if split.distance() <= 0.0 {
                    return glib::ControlFlow::Continue;
                }
                if let Some(toggle) = toggle.upgrade() {
                    split.set_show_sidebar(toggle.is_active());
                }
                glib::ControlFlow::Break
            });
        }
    });
    breakpoint.connect_unapply({
        let split = split.downgrade();
        let paned = paned.downgrade();
        let inspector = inspector.downgrade();
        let toggle = toggle.downgrade();
        move |_| {
            let (Some(split), Some(paned), Some(inspector), Some(toggle)) = (
                split.upgrade(),
                paned.upgrade(),
                inspector.upgrade(),
                toggle.upgrade(),
            ) else {
                return;
            };
            stop_panel();
            split.set_collapsed(false);
            split.set_show_sidebar(false);
            split.set_sidebar(None::<&gtk::Widget>);
            split.set_max_sidebar_width(0.0);
            overlay_mode.set(false);
            inspector.set_visible(toggle.is_active());
            paned.set_start_child(Some(&inspector));
            paned.set_position(if toggle.is_active() { saved_width() } else { 0 });
        }
    });
    bin.add_breakpoint(breakpoint);
    bin
}
