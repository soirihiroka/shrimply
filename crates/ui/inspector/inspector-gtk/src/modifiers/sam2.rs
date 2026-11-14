use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{glib, prelude::*};
use shrimply_components_gtk::{
    tr,
    ui::{
        I18nWidgetExt, ProgressButton, ProgressButtonState, control_row, enum_dropdown, switch_row,
    },
};
use shrimply_inspector_core::{
    AnalysisControlPresentation, AnalysisTooltip, ControlKind, ControlRowRole,
    InspectorControlAction, InspectorTarget, visual_modifiers::sam2_analysis_control,
};
use shrimply_visual_modifiers::sam2::{Sam2Model, Sam2Modifier, Sam2PointLabel};
use uuid::Uuid;

use crate::InspectorContext;

pub fn add_rows(value: &Sam2Modifier, out: &gtk::Box, id: Uuid, context: &InspectorContext) {
    let key = context
        .selected_item
        .clone()
        .expect("SAM2 inspector must have a selected item");
    let analysis_target = Rc::new(shrimply_visual_core::sam2::analysis::AnalysisTarget {
        address: key.clone(),
        modifier_id: id,
    });
    let target = InspectorTarget::Item(key);
    let section = context
        .inspector_core
        .sam2_presentation(&target, id)
        .expect("live SAM2 modifier must have a shared presentation");
    let mut controls = section.controls.into_iter();

    let model_control = controls.next().expect("SAM2 model control is missing");
    assert_eq!(model_control.kind, ControlKind::Selector);
    assert_eq!(
        model_control.action,
        Some(InspectorControlAction::SetSam2Model { modifier_id: id })
    );
    assert!(model_control.path.ends_with("/effect/effect/config/model"));
    assert_eq!(model_control.target_id, Some(id));
    assert_eq!(
        model_control.value,
        match value.model {
            Sam2Model::Tiny => "tiny",
            Sam2Model::Small => "small",
            Sam2Model::BasePlus => "base_plus",
            Sam2Model::Large => "large",
        }
    );
    assert_eq!(
        model_control.values,
        ["tiny", "small", "base_plus", "large"]
    );
    assert_eq!(model_control.labels, ["Tiny", "Small", "Base+", "Large"]);
    let model = enum_dropdown(value.model, {
        let controller = context.inspector_core.clone();
        let target = target.clone();
        move |model| {
            if let Err(error) = controller.set_sam2_model(&target, id, model) {
                tracing::error!(%error, "Could not update GTK SAM2 model");
            }
        }
    });
    out.append(&control_row(&model_control.label, &model));

    let threshold = controls.next().expect("SAM2 threshold control is missing");
    for row in crate::control::section(
        &shrimply_inspector_core::InspectorSection {
            controls: vec![threshold],
        },
        context,
    ) {
        out.append(&row);
    }
    let softness = controls.next().expect("SAM2 softness control is missing");
    for row in crate::control::section(
        &shrimply_inspector_core::InspectorSection {
            controls: vec![softness],
        },
        context,
    ) {
        out.append(&row);
    }

    for point in &value.points {
        let position_control = controls
            .next()
            .expect("SAM2 point position control is missing");
        assert_eq!(position_control.kind, ControlKind::LayeredVector2);
        assert_eq!(position_control.target_id, Some(id));
        assert_eq!(position_control.timeline_id, Some(point.position.id));
        assert_eq!(position_control.row_group, Some(point.id));
        assert_eq!(position_control.row_role, ControlRowRole::Primary);
        assert_eq!(
            position_control.action,
            Some(InspectorControlAction::SetSam2PointPosition {
                modifier_id: id,
                point_id: point.id,
            })
        );
        assert_eq!(position_control.number.minimum, 0.0);
        assert_eq!(position_control.number.maximum, 1.0);
        assert_eq!(position_control.number.unit, "x");
        assert_eq!(position_control.commit_name, "visual-modifier-vector");

        let point_type_control = controls.next().expect("SAM2 point type control is missing");
        assert_eq!(point_type_control.kind, ControlKind::Selector);
        assert_eq!(point_type_control.target_id, Some(id));
        assert_eq!(point_type_control.row_group, Some(point.id));
        assert_eq!(point_type_control.row_role, ControlRowRole::Auxiliary);
        assert_eq!(
            point_type_control.value,
            match point.label {
                Sam2PointLabel::Foreground => "foreground",
                Sam2PointLabel::Background => "background",
            }
        );
        assert_eq!(point_type_control.values, ["foreground", "background"]);
        assert_eq!(point_type_control.labels, ["Foreground", "Background"]);
        assert_eq!(
            point_type_control.action,
            Some(InspectorControlAction::SetSam2PointLabel {
                modifier_id: id,
                point_id: point.id,
            })
        );

        let remove_control = controls
            .next()
            .expect("SAM2 point removal control is missing");
        assert_eq!(remove_control.kind, ControlKind::Action);
        assert_eq!(remove_control.target_id, Some(id));
        assert_eq!(remove_control.row_group, Some(point.id));
        assert_eq!(remove_control.row_role, ControlRowRole::TrailingAction);
        assert_eq!(remove_control.prefix_icon, "user-trash-symbolic");
        assert_eq!(
            remove_control.action,
            Some(InspectorControlAction::RemoveSam2Point {
                modifier_id: id,
                point_id: point.id,
            })
        );

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let position = super::vec_row(
            &position_control.label,
            &point.position,
            id,
            true,
            Some((0.0, 1.0)),
            context,
        );
        position.set_hexpand(true);
        row.append(&position);
        let point_type = enum_dropdown(point.label, {
            let controller = context.inspector_core.clone();
            let target = target.clone();
            let point_id = point.id;
            move |label| {
                if let Err(error) = controller.set_sam2_point_label(&target, id, point_id, label) {
                    tracing::error!(%error, "Could not update GTK SAM2 point type");
                }
            }
        });
        point_type.set_tooltip_i18n(&point_type_control.label);
        row.append(&point_type);
        let remove = gtk::Button::builder()
            .icon_name(&remove_control.prefix_icon)
            .tooltip_text(tr!(&remove_control.tooltip).as_ref())
            .css_classes(["flat"])
            .build();
        remove.connect_clicked({
            let controller = context.inspector_core.clone();
            let target = target.clone();
            let point_id = point.id;
            move |_| {
                if let Err(error) = controller.remove_sam2_point(&target, id, point_id) {
                    tracing::error!(%error, "Could not remove GTK SAM2 point");
                }
            }
        });
        row.append(&remove);
        out.append(&row);
    }

    if let Some(box_prompt) = value.box_prompt {
        let remove_control = controls
            .next()
            .expect("SAM2 box removal control is missing");
        assert_eq!(remove_control.kind, ControlKind::Action);
        assert_eq!(remove_control.target_id, Some(id));
        assert_eq!(remove_control.prefix_icon, "user-trash-symbolic");
        assert_eq!(
            remove_control.action,
            Some(InspectorControlAction::RemoveSam2Box {
                modifier_id: id,
                box_id: box_prompt.id,
            })
        );
        let remove = gtk::Button::builder()
            .icon_name(&remove_control.prefix_icon)
            .tooltip_text(tr!(&remove_control.tooltip).as_ref())
            .css_classes(["flat"])
            .build();
        remove.connect_clicked({
            let controller = context.inspector_core.clone();
            let target = target.clone();
            move |_| {
                if let Err(error) = controller.remove_sam2_box(&target, id, box_prompt.id) {
                    tracing::error!(%error, "Could not remove GTK SAM2 box");
                }
            }
        });
        out.append(&control_row(&remove_control.label, &remove));
    }

    let analysis_control = controls.next().expect("SAM2 analysis control is missing");
    assert_eq!(analysis_control.kind, ControlKind::Analysis);
    assert_eq!(analysis_control.target_id, Some(id));
    let (generation, prompt_signature, can_analyze) = match analysis_control.action {
        Some(InspectorControlAction::ToggleSam2Analysis {
            modifier_id,
            generation,
            prompt_signature,
            can_analyze,
        }) => {
            assert_eq!(modifier_id, id);
            (generation, prompt_signature, can_analyze)
        }
        _ => panic!("SAM2 analysis control must have its shared action"),
    };
    let analyze = ProgressButton::new("Analyze");
    analyze.widget().set_halign(gtk::Align::End);
    analyze.widget().connect_clicked({
        let controller = context.inspector_core.clone();
        let target = target.clone();
        let preferences = context.preferences.clone();
        move |_| {
            let server_url =
                shrimply_editor_state::preferences::snapshot(&preferences).compute_server_url;
            if let Err(error) = controller.toggle_sam2_analysis(&target, id, server_url) {
                tracing::error!(%error, "Could not toggle GTK SAM2 analysis");
            }
        }
    });
    let hovered = Rc::new(Cell::new(false));
    let motion = gtk::EventControllerMotion::new();
    motion.connect_enter({
        let analyze = analyze.clone();
        let hovered = hovered.clone();
        let analysis_target = analysis_target.clone();
        move |_, _, _| {
            hovered.set(true);
            update_analysis_status(
                &analyze,
                &sam2_analysis_control(
                    analysis_target.as_ref(),
                    generation,
                    prompt_signature,
                    can_analyze,
                ),
                true,
            );
        }
    });
    motion.connect_leave({
        let analyze = analyze.clone();
        let hovered = hovered.clone();
        let analysis_target = analysis_target.clone();
        move |_| {
            hovered.set(false);
            update_analysis_status(
                &analyze,
                &sam2_analysis_control(
                    analysis_target.as_ref(),
                    generation,
                    prompt_signature,
                    can_analyze,
                ),
                false,
            );
        }
    });
    analyze.widget().add_controller(motion);
    update_analysis_status(
        &analyze,
        &sam2_analysis_control(
            analysis_target.as_ref(),
            generation,
            prompt_signature,
            can_analyze,
        ),
        false,
    );
    if generation > 0 {
        let analyze = analyze.clone();
        let analysis_target = analysis_target.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if analyze.widget().parent().is_none() {
                return glib::ControlFlow::Break;
            }
            let analysis = sam2_analysis_control(
                analysis_target.as_ref(),
                generation,
                prompt_signature,
                can_analyze,
            );
            let finished = !analysis.running && !analysis.cancelling;
            update_analysis_status(&analyze, &analysis, hovered.get());
            if finished {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
    out.append(analyze.widget());

    let invert_control = controls.next().expect("SAM2 invert control is missing");
    assert_eq!(invert_control.kind, ControlKind::Boolean);
    assert_eq!(invert_control.target_id, Some(id));
    assert_eq!(invert_control.value, value.invert.to_string());
    assert_eq!(
        invert_control.commit_name,
        shrimply_inspector_core::visual_modifiers::SAM2_EDIT_COMMIT
    );
    out.append(&switch_row(&invert_control.label, None, value.invert, {
        let controller = context.inspector_core.clone();
        move |invert| {
            if let Err(error) = controller.set_sam2_inverted(&target, id, invert) {
                tracing::error!(%error, "Could not update GTK SAM2 inversion");
            }
        }
    }));
    assert!(
        controls.next().is_none(),
        "SAM2 presentation has extra controls"
    );
}

fn update_analysis_status(
    button: &ProgressButton,
    status: &AnalysisControlPresentation,
    hovered: bool,
) {
    button.widget().remove_css_class("destructive-action");
    button.widget().remove_css_class("suggested-action");
    match &status.tooltip {
        AnalysisTooltip::MessageKey(message) => button.widget().set_tooltip_i18n(message),
        AnalysisTooltip::RawError(error) => button.widget().set_tooltip_text(Some(error.as_str())),
    }
    button.widget().set_sensitive(status.sensitive);
    button.set_label(if hovered && status.running {
        "Cancel"
    } else {
        &status.label
    });
    if hovered && status.running {
        button.widget().add_css_class("destructive-action");
    } else if status.suggested {
        button.widget().add_css_class("suggested-action");
    }
    button.set_state(if status.running || status.cancelling {
        if status.progress < 0.0 {
            ProgressButtonState::Indeterminate
        } else {
            ProgressButtonState::Progress(status.progress)
        }
    } else {
        ProgressButtonState::Idle
    });
}
