use adw::prelude::AdwDialogExt;
use gtk::prelude::*;
use shrimply_components_gtk::{tr, ui::I18nFileFilterExt};
use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorControlAction, InspectorSection, InspectorTarget,
    VisualModifierBodyPresentation, visual_modifier_presentations,
};
use shrimply_visual_modifiers::scene_3d::Object3dModifier;
use uuid::Uuid;

use crate::InspectorContext;

pub fn add_rows(_value: &Object3dModifier, out: &gtk::Box, id: Uuid, context: &InspectorContext) {
    let key = context
        .selected_item
        .clone()
        .expect("3D object inspector must have a selected item");
    let target = InspectorTarget::Item(key.clone());
    let snapshot = context.inspector_core.snapshot();
    assert_eq!(
        snapshot.target, target,
        "3D object inspector target changed"
    );
    let project = context.project.borrow();
    let item = project
        .video_item(&key)
        .expect("3D object inspector item must still be available");
    let section = visual_modifier_presentations(&project, &key, item, snapshot.runtime)
        .into_iter()
        .find(|modifier| modifier.id == id)
        .and_then(|modifier| match modifier.body {
            Some(VisualModifierBodyPresentation::Object3d(section)) => Some(section),
            _ => None,
        })
        .expect("3D object modifier must still be available");
    drop(project);

    add_shared_rows(out, id, &target, section, context);
}

fn add_shared_rows(
    out: &gtk::Box,
    id: Uuid,
    target: &InspectorTarget,
    section: InspectorSection,
    context: &InspectorContext,
) {
    let mut controls = section.controls.into_iter();
    let model = controls.next().expect("3D object model control is missing");
    let select = controls
        .next()
        .expect("3D object select-model action is missing");
    let clear = controls
        .next()
        .expect("3D object clear-model action is missing");
    out.append(&file_row(model, select, clear, id, target, context));

    let native = crate::section::InspectorSection::controls();
    for control in controls.filter(|control| control.visible) {
        crate::control::add_control(&native, context, target, control);
    }
    out.append(&native.into_widget());
}

fn file_row(
    model: InspectorControl,
    select: InspectorControl,
    clear: InspectorControl,
    id: Uuid,
    target: &InspectorTarget,
    context: &InspectorContext,
) -> gtk::Widget {
    assert_eq!(model.kind, ControlKind::ReadOnly);
    assert_eq!(select.kind, ControlKind::Action);
    assert_eq!(clear.kind, ControlKind::Action);
    assert_eq!(model.target_id, Some(id));
    assert_eq!(select.target_id, Some(id));
    assert_eq!(clear.target_id, Some(id));
    assert_eq!(
        select.action,
        Some(InspectorControlAction::SelectObject3dModel { modifier_id: id }),
    );
    assert_eq!(
        clear.action,
        Some(InspectorControlAction::ClearObject3dModel { modifier_id: id }),
    );

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let filename = gtk::Label::builder()
        .label(&model.value)
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .css_classes(["dim-label"])
        .build();
    filename.set_tooltip_text((!model.tooltip.is_empty()).then_some(model.tooltip.as_str()));
    row.append(&filename);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.set_halign(gtk::Align::End);
    actions.add_css_class("linked");
    let choose_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    choose_content.append(&gtk::Image::from_icon_name(&select.prefix_icon));
    choose_content.append(&gtk::Label::new(Some(&select.value)));
    let choose = gtk::Button::builder()
        .child(&choose_content)
        .sensitive(select.sensitive)
        .build();
    choose.set_tooltip_text((!select.tooltip.is_empty()).then_some(select.tooltip.as_str()));
    let clear_button = gtk::Button::builder()
        .icon_name(&clear.prefix_icon)
        .tooltip_text(tr!(&clear.tooltip).as_ref())
        .sensitive(clear.sensitive)
        .build();
    actions.append(&choose);
    actions.append(&clear_button);
    row.append(&actions);

    let choose_target = target.clone();
    let controller = context.inspector_core.clone();
    choose.connect_clicked(move |_| {
        let label = "Select 3D model";
        let filter = gtk::FileFilter::new();
        filter.set_name_i18n("3D models");
        filter.add_pattern("*.obj");
        filter.add_pattern("*.glb");
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(tr!(label).as_ref())
            .filters(&filters)
            .build();
        let target = choose_target.clone();
        let controller = controller.clone();
        shrimply_components_gtk::file_picker::open(
            label,
            &dialog,
            None::<&gtk::Window>,
            move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                if let Err(error) = controller.set_object_3d_model(&target, id, &path) {
                    adw::AlertDialog::new(Some("Could not use 3D model"), Some(&error))
                        .present(None::<&gtk::Widget>);
                }
            },
        );
    });
    let controller = context.inspector_core.clone();
    let clear_target = target.clone();
    let action = clear.action.expect("3D object clear action is missing");
    clear_button.connect_clicked(move |_| {
        if let Err(error) = controller.trigger_video_control_action(&clear_target, action) {
            tracing::error!(%error, "Could not clear GTK 3D object model");
        }
    });
    crate::ui::control_row(&model.label, &row)
}
