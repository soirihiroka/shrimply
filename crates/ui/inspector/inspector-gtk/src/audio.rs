use std::{rc::Rc, time::Duration};

use adw::prelude::*;
use gtk::glib;
use shrimply_components_gtk::{
    tr,
    ui::{SearchMenuItem, search_rank, searchable_menu},
};
use shrimply_inspector_core::{ControlKind, InspectorControl};

use super::InspectorContext;

pub(super) fn control(control: &InspectorControl, context: &InspectorContext) -> gtk::Widget {
    match control.kind {
        ControlKind::TtsEditor => {
            if let Some(presentation) = &control.tts {
                shrimply_tts_gtk::editor(
                    context.inspector_core.clone(),
                    context.list_target.clone(),
                    context.preferences.clone(),
                    (**presentation).clone(),
                    context.refresh.clone(),
                )
            } else {
                gtk::Label::builder()
                    .label(tr!(&control.subtitle).as_ref())
                    .wrap(true)
                    .build()
                    .upcast()
            }
        }
        ControlKind::BeatDetection => beat_detection(control, context),
        ControlKind::AudioModifierMenu | ControlKind::VisualModifierMenu => {
            modifier_menu(control, context)
        }
        _ => panic!("unsupported audio widget kind: {:?}", control.kind),
    }
}

fn beat_detection(control: &InspectorControl, context: &InspectorContext) -> gtk::Widget {
    let enabled = control.value.parse().expect("beat detection is boolean");
    let row = adw::ActionRow::builder()
        .title(tr!(&control.label).as_ref())
        .subtitle(tr!(&control.subtitle).as_ref())
        .build();
    let spinner = adw::Spinner::new();
    spinner.set_size_request(18, 18);
    spinner.set_visible(
        context
            .inspector_core
            .audio_beat_detection_loading(&context.list_target),
    );
    row.add_suffix(&spinner);
    let toggle = gtk::Switch::builder()
        .active(enabled)
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&toggle);
    row.set_activatable_widget(Some(&toggle));
    if enabled {
        let spinner = spinner.downgrade();
        let scope = Rc::downgrade(&context.listener_scope);
        let controller = context.inspector_core.clone();
        let target = context.list_target.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if scope.upgrade().is_none() {
                return glib::ControlFlow::Break;
            }
            let Some(spinner) = spinner.upgrade() else {
                return glib::ControlFlow::Break;
            };
            spinner.set_visible(controller.audio_beat_detection_loading(&target));
            glib::ControlFlow::Continue
        });
    }
    let controller = context.inspector_core.clone();
    let target = context.list_target.clone();
    let control = control.clone();
    toggle.connect_active_notify(move |toggle| {
        if let Err(error) =
            controller.set_basic_control_value(&target, &control, &toggle.is_active().to_string())
        {
            tracing::warn!(%error, "Could not change beat detection");
        }
    });
    row.upcast()
}

fn modifier_menu(control: &InspectorControl, context: &InspectorContext) -> gtk::Widget {
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.set_halign(gtk::Align::Center);
    let controller = context.inspector_core.clone();
    let target = context.list_target.clone();
    let control = control.clone();
    let kind = control.kind;
    let menu_controller = controller.clone();
    let menu_target = target.clone();
    let menu = control.clone();
    buttons.append(&searchable_menu(
        tr!(&control.label).as_ref(),
        tr!(&control.tooltip).as_ref(),
        move |query| {
            let mut choices = menu
                .values
                .iter()
                .zip(&menu.labels)
                .zip(&menu.search_terms)
                .filter_map(|((value, label), search)| {
                    (value != "__paste__").then_some((
                        search_rank(label, [search.as_str()], query)?,
                        value,
                        label,
                    ))
                })
                .collect::<Vec<_>>();
            choices.sort_by_key(|(rank, _, _)| *rank);
            choices
                .into_iter()
                .map(|(_, value, label)| {
                    let controller = menu_controller.clone();
                    let target = menu_target.clone();
                    let value = value.clone();
                    SearchMenuItem::new(tr!(label).as_ref(), move || {
                        if let Err(error) = controller.add_control_modifier(&target, kind, &value) {
                            tracing::warn!(%error, "Could not add audio modifier");
                        }
                    })
                })
                .collect()
        },
    ));
    if let Some(index) = control.values.iter().position(|value| value == "__paste__") {
        let paste = gtk::Button::builder()
            .icon_name("edit-paste-symbolic")
            .tooltip_text(tr!(&control.labels[index]).as_ref())
            .build();
        paste.connect_clicked(move |button| {
            if let Err(error) = controller.add_control_modifier(&target, kind, "__paste__") {
                shrimply_components_gtk::toast::show_confirmation_text_for_widget(button, &error);
            }
        });
        buttons.append(&paste);
    }
    buttons.upcast()
}
