use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
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
        .build();
    toggle
        .bind_property("active", &split, "show-sidebar")
        .bidirectional()
        .sync_create()
        .build();
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
            split.set_sidebar(None::<&gtk::Widget>);
            split.set_max_sidebar_width(0.0);
            split.set_collapsed(false);
            overlay_mode.set(false);
            inspector.set_visible(toggle.is_active());
            paned.set_start_child(Some(&inspector));
            paned.set_position(if toggle.is_active() { saved_width() } else { 0 });
        }
    });
    bin.add_breakpoint(breakpoint);
    bin
}
