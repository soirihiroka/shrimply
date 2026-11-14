use gtk::prelude::*;
use shrimply_components_gtk::tr;
use shrimply_inspector_core::{ControlKind, InspectorControlAction, InspectorTarget};
use shrimply_property_model::timeline_value::TimelineStep;
use shrimply_visual_modifiers::mask::{MaskMode, MaskModifier};
use uuid::Uuid;

use crate::InspectorContext;
use shrimply_components_gtk::ui::{control_row, switch_row};

pub fn add_rows(value: &MaskModifier, out: &gtk::Box, id: Uuid, context: &InspectorContext) {
    let Some(key) = context.selected_item.clone() else {
        return;
    };
    let section = context
        .inspector_core
        .mask_presentation(&InspectorTarget::Item(key), id)
        .expect("live Mask modifier must have a shared presentation");
    let [source_control, mode_control, invert_control] = section
        .controls
        .try_into()
        .expect("Mask presentation must contain exactly three controls");

    assert_eq!(source_control.kind, ControlKind::Action);
    assert_eq!(source_control.target_id, Some(id));
    assert_eq!(source_control.drag_payload, id.to_string());
    assert_eq!(source_control.action.is_some(), value.item_id.is_some());
    let source = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let pick = gtk::Button::builder()
        .label(&source_control.value)
        .tooltip_text(tr!(&source_control.tooltip).as_ref())
        .hexpand(true)
        .build();
    let drag_payload = source_control.drag_payload;
    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::COPY)
        .build();
    drag.connect_prepare(move |_, _, _| {
        Some(gtk::gdk::ContentProvider::for_value(
            &gtk::glib::Bytes::from_owned(drag_payload.clone().into_bytes()).to_value(),
        ))
    });
    pick.add_controller(drag);
    source.append(&pick);
    let clear = gtk::Button::builder()
        .icon_name(&source_control.action_icon)
        .tooltip_text(tr!(&source_control.action_tooltip).as_ref())
        .sensitive(source_control.action.is_some())
        .build();
    if let Some(InspectorControlAction::ClearMaskSource { modifier_id }) = source_control.action {
        assert_eq!(modifier_id, id);
    }
    clear.connect_clicked({
        let controller = context.inspector_core.clone();
        let key = context.selected_item.clone();
        move |_| {
            let Some(key) = key.clone() else {
                return;
            };
            if let Err(error) = controller.clear_mask_source(&InspectorTarget::Item(key), id) {
                tracing::error!(%error, "Could not clear GTK mask source");
            }
        }
    });
    source.append(&clear);
    out.append(&control_row(&source_control.label, &source));

    for row in crate::control::section(
        &shrimply_inspector_core::InspectorSection {
            controls: vec![mode_control.clone()],
        },
        context,
    ) {
        out.append(&row);
    }
    assert_eq!(
        mode_control.values,
        MaskMode::variants()
            .iter()
            .map(|variant| variant.key.to_string())
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        mode_control.labels,
        MaskMode::variants()
            .iter()
            .map(|variant| variant.label.to_string())
            .collect::<Vec<_>>(),
    );
    assert!(mode_control.icons.is_empty());

    assert_eq!(invert_control.kind, ControlKind::Boolean);
    assert!(
        invert_control
            .path
            .ends_with("/effect/effect/config/invert")
    );
    assert_eq!(invert_control.target_id, Some(id));
    assert_eq!(invert_control.value, value.invert.to_string());
    assert_eq!(invert_control.commit_name, "edit-mask");
    out.append(&switch_row(&invert_control.label, None, value.invert, {
        let controller = context.inspector_core.clone();
        let key = context.selected_item.clone();
        move |invert| {
            let Some(key) = key.clone() else {
                return;
            };
            if let Err(error) =
                controller.set_mask_inverted(&InspectorTarget::Item(key), id, invert)
            {
                tracing::error!(%error, "Could not update GTK mask inversion");
            }
        }
    }));
}
