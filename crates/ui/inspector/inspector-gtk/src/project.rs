use super::{InspectorContext, section::InspectorSection};

pub(super) fn settings(
    section: &InspectorSection,
    context: &InspectorContext,
    control: shrimply_inspector_core::InspectorControl,
) {
    use adw::prelude::*;
    use shrimply_components_gtk::{
        tr,
        ui::{I18nAlertDialogExt, Number2Picker, StringChoice, labeled_string_selector},
    };
    use shrimply_inspector_core::project as shared;
    let model = control
        .project_settings
        .expect("shared project settings model");
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::End)
        .visible(model.dirty)
        .build();
    let discard = gtk::Button::with_label(tr!(shared::SETTINGS_DISCARD).as_ref());
    let apply = gtk::Button::with_label(tr!(shared::SETTINGS_APPLY).as_ref());
    apply.add_css_class("suggested-action");
    actions.append(&discard);
    actions.append(&apply);
    let controller = context.inspector_core.clone();
    let rate_actions = actions.clone();
    let fps = labeled_string_selector(
        &model.frame_rate.label,
        &model.frame_rate.value,
        model
            .frame_rate
            .values
            .iter()
            .zip(&model.frame_rate.labels)
            .map(|(value, label)| StringChoice {
                value: value.clone(),
                label: label.clone(),
            })
            .collect(),
        move |value| {
            controller
                .stage_project_frame_rate(value.parse().expect("shared frame rate choice"))
                .expect("valid frame rate");
            rate_actions.set_visible(controller.project_settings_presentation().dirty);
        },
    );
    section.add_wide_control(fps.widget());
    let resolution = &model.resolution;
    let [width, height] = resolution.components.as_slice() else {
        panic!("resolution has two dimensions")
    };
    let [width_prefix, height_prefix] = resolution.prefixes.as_slice() else {
        panic!("resolution has two prefixes")
    };
    let controller = context.inspector_core.clone();
    let dimension_actions = actions.clone();
    let mut dimensions = Number2Picker::builder(
        width.parse().expect("project width"),
        height.parse().expect("project height"),
    )
    .minimum(resolution.number.minimum)
    .maximum(resolution.number.maximum)
    .drag_step(resolution.number.drag_step)
    .digits(
        resolution
            .number
            .digits
            .try_into()
            .expect("nonnegative digits"),
    )
    .width_chars(resolution.width_characters)
    .first_prefix(width_prefix)
    .second_prefix(height_prefix);
    if resolution.lock {
        dimensions = dimensions.enable_lock();
    }
    let dimensions = dimensions
        .on_change(move |values, _| {
            controller
                .stage_project_dimensions(values[0].round() as u32, values[1].round() as u32)
                .expect("valid project dimensions");
            dimension_actions.set_visible(controller.project_settings_presentation().dirty);
        })
        .build_with_handles();
    section.add_control_row(&resolution.label, &dimensions.widget);
    section.add_wide_control(&actions);
    let controller = context.inspector_core.clone();
    let refresh = context.refresh.clone();
    discard.connect_clicked(move |_| {
        controller.discard_project_settings();
        refresh();
    });
    let controller = context.inspector_core.clone();
    apply.connect_clicked(move |button| {
        let dialog = adw::AlertDialog::new(
            Some(tr!(shared::SETTINGS_TITLE).as_ref()),
            Some(tr!(shared::SETTINGS_WARNING).as_ref()),
        );
        dialog.add_responses_i18n(&[
            ("cancel", shared::SETTINGS_CANCEL),
            ("apply", shared::SETTINGS_APPLY),
        ]);
        dialog.set_close_response("cancel");
        dialog.set_default_response(Some("cancel"));
        dialog.set_response_appearance("apply", adw::ResponseAppearance::Destructive);
        let controller = controller.clone();
        dialog.choose(
            Some(button.upcast_ref::<gtk::Widget>()),
            None::<&gtk::gio::Cancellable>,
            move |response| {
                if response.as_str() == "apply" {
                    controller
                        .apply_staged_project_settings()
                        .expect("validated settings apply");
                }
            },
        );
    });
}
