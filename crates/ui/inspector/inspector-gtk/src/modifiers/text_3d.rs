use gtk::prelude::*;
use shrimply_inspector_core::{ControlKind, InspectorControl, InspectorTarget};
use shrimply_visual_modifiers::scene_3d::Text3dModifier;
use uuid::Uuid;

use crate::{InspectorContext, font_selector::font_selector_list};

pub fn add_rows(value: &Text3dModifier, out: &gtk::Box, id: Uuid, context: &InspectorContext) {
    let target = context.list_target.clone();
    let section = context
        .inspector_core
        .text_3d_presentation(&target, id)
        .expect("3D text modifier must still be available");
    let native = crate::section::InspectorSection::controls();
    for control in section
        .controls
        .into_iter()
        .filter(|control| control.visible)
    {
        if control.kind == ControlKind::FontFamilies {
            native.add_wide_control(&fonts_row(value, id, &target, &control, context));
        } else {
            crate::control::add_control(&native, context, &target, control);
        }
    }
    out.append(&native.into_widget());
}

fn fonts_row(
    value: &Text3dModifier,
    id: Uuid,
    target: &InspectorTarget,
    control: &InspectorControl,
    context: &InspectorContext,
) -> gtk::Widget {
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let path = control.path.clone();
    let commit = control.commit_name.clone();
    crate::ui::control_row(
        &control.label,
        &font_selector_list(&value.font_families, move |families| {
            let serialized =
                serde_json::to_string(&families).expect("font families must serialize");
            if let Err(error) = controller
                .ensure_visual_modifier(&target, &path, id)
                .and_then(|()| {
                    controller.set_video_field(&target, &path, &serialized, &commit, true)
                })
            {
                tracing::error!(%error, "Could not update GTK 3D text fonts");
            }
        }),
    )
}
