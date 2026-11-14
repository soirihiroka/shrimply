use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{glib, prelude::*};
use shrimply_components_gtk::{
    tr,
    ui::{I18nWidgetExt, NumberPicker, ProgressButton, ProgressButtonState},
};
use shrimply_inspector_core::{
    AnalysisControlPresentation, AnalysisTooltip, ControlKind, ControlRowRole,
    InspectorControlAction, InspectorTarget,
};
use shrimply_visual_modifiers::transparent_fill::TransparentFillModifier;
use uuid::Uuid;

use crate::InspectorContext;

pub fn add_rows(
    value: &TransparentFillModifier,
    out: &gtk::Box,
    modifier_id: Uuid,
    context: &InspectorContext,
) {
    let key = context
        .selected_item
        .clone()
        .expect("Transparent Fill inspector must have a selected item");
    let target = InspectorTarget::Item(key);
    let section = context
        .inspector_core
        .transparent_fill_presentation(&target, modifier_id)
        .expect("live Transparent Fill modifier must have a shared presentation");
    let mut controls = section.controls.into_iter();

    let tolerance = controls
        .next()
        .expect("Transparent Fill tolerance control is missing");
    for row in crate::control::section(
        &shrimply_inspector_core::InspectorSection {
            controls: vec![tolerance],
        },
        context,
    ) {
        out.append(&row);
    }

    let gap_control = controls
        .next()
        .expect("Transparent Fill maximum gap control is missing");
    assert_eq!(gap_control.kind, ControlKind::Number);
    assert_eq!(gap_control.target_id, Some(modifier_id));
    assert_eq!(gap_control.value, value.maximum_gap.to_string());
    assert!(gap_control.integer);
    assert_eq!(
        gap_control.commit_name,
        shrimply_inspector_core::visual_modifiers::TRANSPARENT_FILL_EDIT_COMMIT
    );
    assert_eq!(gap_control.number.digits, 0);
    assert_eq!(gap_control.action, None);
    let gap = NumberPicker::integer_builder(value.maximum_gap)
        .minimum(gap_control.number.minimum)
        .maximum(gap_control.number.maximum)
        .on_change_integer({
            let controller = context.inspector_core.clone();
            let target = target.clone();
            move |maximum_gap| {
                if let Err(error) =
                    controller.set_transparent_fill_maximum_gap(&target, modifier_id, maximum_gap)
                {
                    tracing::error!(%error, "Could not update GTK Transparent Fill maximum gap");
                }
            }
        })
        .build();
    gap.set_tooltip_i18n(&gap_control.tooltip);
    let row = shrimply_components_gtk::ui::control_row(&gap_control.label, &gap);
    row.set_sensitive(gap_control.sensitive);
    if gap_control.visible {
        out.append(&row);
    }

    for point in &value.points {
        let position_control = controls
            .next()
            .expect("Transparent Fill point position control is missing");
        assert_eq!(position_control.kind, ControlKind::LayeredVector2);
        assert_eq!(position_control.target_id, Some(modifier_id));
        assert_eq!(position_control.timeline_id, Some(point.position.id));
        assert_eq!(position_control.row_group, Some(point.id));
        assert_eq!(position_control.row_role, ControlRowRole::Primary);
        assert_eq!(position_control.number.minimum, 0.0);
        assert_eq!(position_control.number.maximum, 1.0);
        assert_eq!(position_control.number.unit, "x");

        let remove_control = controls
            .next()
            .expect("Transparent Fill point removal control is missing");
        assert_eq!(remove_control.kind, ControlKind::Action);
        assert_eq!(remove_control.target_id, Some(modifier_id));
        assert_eq!(remove_control.row_group, Some(point.id));
        assert_eq!(remove_control.row_role, ControlRowRole::TrailingAction);
        assert_eq!(remove_control.prefix_icon, "user-trash-symbolic");
        assert_eq!(
            remove_control.action,
            Some(InspectorControlAction::RemoveTransparentFillPoint {
                modifier_id,
                point_id: point.id,
            })
        );

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let position = super::vec_row(
            &position_control.label,
            &point.position,
            modifier_id,
            true,
            Some((0.0, 1.0)),
            context,
        );
        position.set_hexpand(true);
        position.set_sensitive(position_control.sensitive);
        row.append(&position);
        let remove = gtk::Button::builder()
            .icon_name(&remove_control.prefix_icon)
            .tooltip_text(tr!(&remove_control.tooltip).as_ref())
            .css_classes(["flat"])
            .sensitive(remove_control.sensitive)
            .build();
        remove.connect_clicked({
            let controller = context.inspector_core.clone();
            let target = target.clone();
            let point_id = point.id;
            move |_| {
                if let Err(error) =
                    controller.remove_transparent_fill_point(&target, modifier_id, point_id)
                {
                    tracing::error!(%error, "Could not remove GTK Transparent Fill point");
                }
            }
        });
        row.append(&remove);
        if position_control.visible || remove_control.visible {
            out.append(&row);
        }
    }

    let analysis_control = controls
        .next()
        .expect("Transparent Fill analysis control is missing");
    assert_eq!(analysis_control.kind, ControlKind::Analysis);
    assert_eq!(analysis_control.target_id, Some(modifier_id));
    assert_eq!(
        analysis_control.action,
        Some(InspectorControlAction::ToggleTransparentFillAnalysis { modifier_id })
    );
    assert!(controls.next().is_none());
    let analyze = ProgressButton::new(&analysis_control.value);
    analyze.widget().set_halign(gtk::Align::End);
    analyze.widget().connect_clicked({
        let controller = context.inspector_core.clone();
        let target = target.clone();
        move |_| {
            if let Err(error) = controller.toggle_transparent_fill_analysis(&target, modifier_id) {
                tracing::error!(%error, "Could not toggle GTK Transparent Fill analysis");
            }
        }
    });
    let hovered = Rc::new(Cell::new(false));
    let motion = gtk::EventControllerMotion::new();
    motion.connect_enter({
        let analyze = analyze.clone();
        let hovered = hovered.clone();
        let controller = context.inspector_core.clone();
        let target = target.clone();
        move |_, _, _| {
            hovered.set(true);
            if let Ok(status) = controller.transparent_fill_analysis_control(&target, modifier_id) {
                update_analysis_button(&analyze, &status, true);
            }
        }
    });
    motion.connect_leave({
        let analyze = analyze.clone();
        let hovered = hovered.clone();
        let controller = context.inspector_core.clone();
        let target = target.clone();
        move |_| {
            hovered.set(false);
            if let Ok(status) = controller.transparent_fill_analysis_control(&target, modifier_id) {
                update_analysis_button(&analyze, &status, false);
            }
        }
    });
    analyze.widget().add_controller(motion);
    let initial_status = context
        .inspector_core
        .transparent_fill_analysis_control(&target, modifier_id)
        .expect("live Transparent Fill analysis status must be available");
    update_analysis_button(&analyze, &initial_status, false);
    if context.inspector_core.observe_analysis_transition(
        &target,
        analysis_control
            .action
            .expect("Transparent Fill analysis action is missing"),
        &initial_status,
    ) {
        context.inspector_core.refresh_analysis_output();
    }
    if initial_status.active() {
        let analyze = analyze.clone();
        let controller = context.inspector_core.clone();
        let action = analysis_control
            .action
            .expect("Transparent Fill analysis action is missing");
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if analyze.widget().parent().is_none() {
                return glib::ControlFlow::Break;
            }
            let Ok(status) = controller.transparent_fill_analysis_control(&target, modifier_id)
            else {
                return glib::ControlFlow::Break;
            };
            let finished = controller.observe_analysis_transition(&target, action, &status);
            update_analysis_button(&analyze, &status, hovered.get());
            if finished {
                controller.refresh_analysis_output();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
    if analysis_control.visible {
        out.append(analyze.widget());
    }
}

fn update_analysis_button(
    button: &ProgressButton,
    status: &AnalysisControlPresentation,
    hovered: bool,
) {
    button.widget().remove_css_class("destructive-action");
    button.widget().remove_css_class("suggested-action");
    button.widget().set_sensitive(status.sensitive);
    match &status.tooltip {
        AnalysisTooltip::MessageKey(message) => button.widget().set_tooltip_i18n(message),
        AnalysisTooltip::RawError(error) => button.widget().set_tooltip_text(Some(error.as_str())),
    }
    button.set_label(if status.running && hovered {
        button.widget().add_css_class("destructive-action");
        "Cancel"
    } else {
        &status.label
    });
    if status.suggested {
        button.widget().add_css_class("suggested-action");
    }
    button.set_state(if !status.running && !status.cancelling {
        ProgressButtonState::Idle
    } else if status.progress < 0.0 {
        ProgressButtonState::Indeterminate
    } else {
        ProgressButtonState::Progress(status.progress)
    });
}
