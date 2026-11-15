use super::{PanelSide, preferences};
use adw::prelude::*;
use gtk::{gdk, glib};
use std::{cell::Cell, rc::Rc};

const DIVIDER_HIT_RADIUS: f64 = 12.0;

/// Persist user intent, not animation, allocation, overlay or fullscreen changes.
pub(super) fn connect(
    store: &preferences::SharedPreferences,
    button: &gtk::ToggleButton,
    paned: &gtk::Paned,
    side: PanelSide,
) {
    button.connect_clicked({
        let store = store.clone();
        let button = button.downgrade();
        move |_| {
            let store = store.clone();
            let button = button.clone();
            glib::idle_add_local_once(move || {
                if let Some(button) = button.upgrade() {
                    preferences::update_editor_layout(&store, |layout| match side {
                        PanelSide::Start => layout.inspector_visible = button.is_active(),
                        PanelSide::End => layout.timeline_visible = button.is_active(),
                    })
                    .expect("persist editor panel visibility");
                }
            });
        }
    });

    let tracking = Rc::new(Cell::new(false));
    let press = gtk::GestureClick::new();
    press.set_button(1);
    press.set_propagation_phase(gtk::PropagationPhase::Capture);
    press.connect_pressed({
        let paned = paned.downgrade();
        let tracking = tracking.clone();
        move |_, _, x, y| {
            if let Some(paned) = paned.upgrade() {
                let axis = match side {
                    PanelSide::Start => x,
                    PanelSide::End => y,
                };
                tracking.set((axis - f64::from(paned.position())).abs() <= DIVIDER_HIT_RADIUS);
            }
        }
    });
    paned.add_controller(press);
    let release = gtk::EventControllerLegacy::new();
    release.set_propagation_phase(gtk::PropagationPhase::Capture);
    release.connect_event({
        let paned = paned.downgrade();
        let store = store.clone();
        move |_, event| {
            if event.event_type() == gdk::EventType::TouchCancel {
                tracking.set(false);
            } else if matches!(
                event.event_type(),
                gdk::EventType::ButtonRelease | gdk::EventType::TouchEnd
            ) && tracking.replace(false)
            {
                let paned = paned.clone();
                let store = store.clone();
                glib::idle_add_local_once(move || {
                    if let Some(paned) = paned.upgrade() {
                        save_size(&store, &paned, side);
                    }
                });
            }
            glib::Propagation::Proceed
        }
    });
    paned.add_controller(release);
    paned.connect_accept_position({
        let store = store.clone();
        move |paned| {
            save_size(&store, paned, side);
            false
        }
    });

    if matches!(side, PanelSide::End) {
        let fraction = preferences::snapshot(store).editor_layout.timeline_fraction;
        let button = button.downgrade();
        paned.add_tick_callback(move |paned, _| {
            if paned.height() <= 0 {
                return glib::ControlFlow::Continue;
            }
            let visible = button.upgrade().is_some_and(|button| button.is_active());
            let content = paned.start_child().expect("preview pane exists").height()
                + paned.end_child().expect("timeline pane exists").height();
            if visible && content <= 0 {
                return glib::ControlFlow::Continue;
            }
            paned.set_position(if visible {
                shrimply_components_core::math::split_pane_extent(
                    paned.height() as f32,
                    (paned.height() - content).max(0) as f32,
                    1.0 - fraction,
                )
                .round() as i32
            } else {
                paned.height()
            });
            glib::ControlFlow::Break
        });
    }
}

fn save_size(store: &preferences::SharedPreferences, paned: &gtk::Paned, side: PanelSide) {
    if paned.height() <= 0 {
        return;
    }
    preferences::update_editor_layout(store, |layout| match side {
        PanelSide::Start => layout.inspector_width = paned.position() as f32,
        PanelSide::End => {
            if let Some(fraction) = shrimply_components_core::math::split_pane_fraction(
                paned.start_child().expect("preview pane exists").height() as f32,
                paned.end_child().expect("timeline pane exists").height() as f32,
            ) {
                layout.timeline_fraction = fraction;
            }
        }
    })
    .expect("persist editor divider position");
}
