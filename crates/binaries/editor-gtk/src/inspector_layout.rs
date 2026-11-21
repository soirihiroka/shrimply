use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
use shrimply_components_core::layout::inspector_min_width;

/// Keep the preview in its Paned; libadwaita owns the inspector overlay.
pub(super) fn connect(
    paned: &gtk::Paned,
    inspector: &gtk::Widget,
    toggle: &gtk::ToggleButton,
    overlay_mode: Rc<Cell<bool>>,
    panel: (Rc<Cell<i32>>, adw::TimedAnimation),
) -> adw::BreakpointBin {
    let (expanded_width, animation) = panel;
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
    let condition = |width| {
        adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            f64::from(inspector_min_width(width as f32)).next_down(),
            adw::LengthUnit::Px,
        )
    };
    let breakpoint = adw::Breakpoint::new(condition(expanded_width.get()));
    let updating = Rc::new(Cell::new(false));
    let position_mode = overlay_mode.clone();
    let position_width = expanded_width.clone();
    let position_toggle = toggle.downgrade();
    let position_animation = animation.clone();
    let position_updating = updating.clone();
    let position_breakpoint = breakpoint.clone();
    paned.connect_position_notify(move |paned| {
        if !position_updating.get()
            && !position_mode.get()
            && position_animation.state() != adw::AnimationState::Playing
            && position_toggle
                .upgrade()
                .is_some_and(|toggle| toggle.is_active())
            && paned.position() > 0
        {
            position_width.set(paned.position());
            position_breakpoint.set_condition(Some(&condition(paned.position())));
        }
    });
    let apply_split = split.downgrade();
    let apply_paned = paned.downgrade();
    let apply_inspector = inspector.downgrade();
    let apply_mode = overlay_mode.clone();
    let apply_width = expanded_width.clone();
    let apply_updating = updating.clone();
    let apply_animation = animation.clone();
    breakpoint.connect_apply(move |_| {
        let (Some(split), Some(paned), Some(inspector)) = (
            apply_split.upgrade(),
            apply_paned.upgrade(),
            apply_inspector.upgrade(),
        ) else {
            return;
        };
        apply_updating.set(true);
        apply_mode.set(true);
        apply_animation.pause();
        paned.set_start_child(None::<&gtk::Widget>);
        inspector.set_visible(true);
        split.set_max_sidebar_width(f64::from(apply_width.get()));
        split.set_sidebar(Some(&inspector));
        split.set_collapsed(true);
        apply_updating.set(false);
    });
    let restore_split = split.downgrade();
    let restore_paned = paned.downgrade();
    let restore_inspector = inspector.downgrade();
    let restore_toggle = toggle.downgrade();
    breakpoint.connect_unapply(move |_| {
        let (Some(split), Some(paned), Some(inspector), Some(toggle)) = (
            restore_split.upgrade(),
            restore_paned.upgrade(),
            restore_inspector.upgrade(),
            restore_toggle.upgrade(),
        ) else {
            return;
        };
        updating.set(true);
        animation.pause();
        split.set_sidebar(None::<&gtk::Widget>);
        split.set_max_sidebar_width(0.0);
        split.set_collapsed(false);
        overlay_mode.set(false);
        inspector.set_visible(toggle.is_active());
        paned.set_start_child(Some(&inspector));
        paned.set_position(if toggle.is_active() {
            expanded_width.get()
        } else {
            0
        });
        paned.set_shrink_start_child(false);
        updating.set(false);
    });
    bin.add_breakpoint(breakpoint);
    bin
}
