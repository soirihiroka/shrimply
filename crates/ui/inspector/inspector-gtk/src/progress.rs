//! Native progress widgets render shared inspector job state and actions.
use gtk::{glib, prelude::*};
use shrimply_components_gtk::ui::{I18nWidgetExt, ProgressButton, ProgressButtonState};
use shrimply_inspector_core::{AnalysisControlPresentation, AnalysisTooltip, InspectorControl};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

fn update(button: &ProgressButton, status: &AnalysisControlPresentation, hovered: bool) {
    let status = status.button(hovered);
    button.widget().set_sensitive(status.sensitive);
    button.set_label(&status.label);
    match &status.tooltip {
        AnalysisTooltip::MessageKey(message) => button.widget().set_tooltip_i18n(message),
        AnalysisTooltip::RawError(error) => button
            .widget()
            .set_tooltip_text((!error.is_empty()).then_some(error.as_str())),
    }
    for (class, enabled) in [
        ("suggested-action", status.suggested),
        ("destructive-action", status.destructive),
    ] {
        if enabled {
            button.widget().add_css_class(class);
        } else {
            button.widget().remove_css_class(class);
        }
    }
    button.set_state(match status.progress {
        None => ProgressButtonState::Idle,
        Some(progress) if progress < 0.0 => ProgressButtonState::Indeterminate,
        Some(progress) => ProgressButtonState::Progress(progress),
    });
}

pub(super) fn widget(control: InspectorControl, context: &crate::InspectorContext) -> gtk::Widget {
    let button = ProgressButton::new(&control.value);
    button.widget().set_halign(gtk::Align::End);
    let hovered = Rc::new(Cell::new(false));
    let control = Rc::new(RefCell::new(control));
    let refresh: Rc<dyn Fn()> = Rc::new({
        let button = button.clone();
        let core = context.inspector_core.clone();
        let target = context.list_target.clone();
        let preferences = context.preferences.clone();
        let control = control.clone();
        let hovered = hovered.clone();
        move || {
            let mut control = control.borrow_mut();
            let preferences = shrimply_editor_state::preferences::snapshot(&preferences);
            core.populate_background_control(
                &target,
                &mut control,
                &preferences.compute_server_url,
            )
            .expect("progress control must match its selected item");
            update(
                &button,
                control
                    .analysis
                    .as_ref()
                    .expect("progress control has shared state"),
                hovered.get(),
            );
        }
    });
    refresh();
    let motion = gtk::EventControllerMotion::new();
    motion.connect_enter({
        let hovered = hovered.clone();
        let refresh = Rc::downgrade(&refresh);
        move |_, _, _| {
            hovered.set(true);
            if let Some(refresh) = refresh.upgrade() {
                refresh();
            }
        }
    });
    motion.connect_leave({
        let hovered = hovered.clone();
        let refresh = Rc::downgrade(&refresh);
        move |_| {
            hovered.set(false);
            if let Some(refresh) = refresh.upgrade() {
                refresh();
            }
        }
    });
    button.widget().add_controller(motion);
    button.widget().connect_clicked({
        let core = context.inspector_core.clone();
        let target = context.list_target.clone();
        let preferences = context.preferences.clone();
        let refresh = Rc::downgrade(&refresh);
        move |_| {
            let preferences = shrimply_editor_state::preferences::snapshot(&preferences);
            if let Err(error) =
                core.activate_control(&target, &control.borrow(), &preferences.compute_server_url)
            {
                tracing::warn!(%error, "Could not activate inspector job");
            }
            if let Some(refresh) = refresh.upgrade() {
                refresh();
            }
        }
    });
    let scope = Rc::downgrade(&context.listener_scope);
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if scope.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        refresh();
        glib::ControlFlow::Continue
    });
    button.widget().clone().upcast()
}
