use std::rc::Rc;

use gtk::prelude::*;
use shrimply_inspector_core::document::InspectorToggleAction;
use shrimply_project_document::project::VisualAlphaMaskTarget;

use crate::{InspectorContext, item::HeaderButtonToggle};

pub(crate) fn button_toggle(
    mask_target: VisualAlphaMaskTarget,
    context: &InspectorContext,
) -> HeaderButtonToggle {
    let target = context.list_target.clone();
    let controller = context.inspector_core.clone();
    let active = controller
        .alpha_mask_presentation(&target, mask_target)
        .is_ok_and(|presentation| presentation.active);
    let action = InspectorToggleAction::AlphaMask(mask_target);
    let preview_focus = context.preview_focus.clone();
    HeaderButtonToggle {
        icon: "select-symbolic",
        active,
        tooltip: "Mask",
        activate: Rc::new(move |active| {
            if let Err(error) = controller.apply_basic_action(&target, &action.action(active)) {
                tracing::error!(%error, "Could not toggle GTK alpha mask");
                return;
            }
            if let Some(focus) = controller.toggle_preview_focus(&target, &action, active) {
                crate::preview_focus::set(&preview_focus, focus);
            }
        }),
    }
}

pub(crate) fn widget(
    mask_target: VisualAlphaMaskTarget,
    context: &InspectorContext,
) -> gtk::Widget {
    let out = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let Ok(presentation) = context
        .inspector_core
        .alpha_mask_presentation(&context.list_target, mask_target)
    else {
        return out.upcast();
    };
    if presentation.active {
        out.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        for widget in crate::control::section(&presentation.section, context) {
            out.append(&widget);
        }
    }
    out.upcast()
}
