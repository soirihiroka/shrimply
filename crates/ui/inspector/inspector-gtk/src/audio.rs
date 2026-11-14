use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use shrimply_components_gtk::{
    tr,
    ui::{SearchMenuItem, search_rank, searchable_menu},
};
use shrimply_inspector_core::{ControlKind, InspectorControl};
use shrimply_project_document::project::{AudioItem, AudioSource};

use super::{Inspectable, InspectorContext, list, section::InspectorSection};

impl Inspectable for AudioItem {
    fn title(&self) -> &'static str {
        shrimply_inspector_core::audio_title(self)
    }
    fn add_rows(&self, _section: &InspectorSection, _context: &InspectorContext) {}
    fn inspect(&self, context: &InspectorContext) -> Vec<gtk::Widget> {
        let preferences = shrimply_editor_state::preferences::snapshot(&context.preferences);
        let document = shrimply_inspector_document::document(
            &context.inspector_core,
            super::info::format_date,
            &preferences.compute_server_url,
            &preferences.last_tts_model,
        );
        list::render_document(document, context, controls)
    }
}

fn controls(
    model: &shrimply_inspector_core::InspectorSection,
    context: &InspectorContext,
) -> Vec<gtk::Widget> {
    if let Some(control) = model
        .controls
        .iter()
        .find(|control| control.kind == ControlKind::AudioCache)
    {
        let project = context.project.borrow();
        let item = project
            .audio_item(context.selected_item.as_ref().expect("audio cache item"))
            .expect("audio cache item exists");
        let modifier = item
            .modifiers
            .iter()
            .find(|modifier| Some(modifier.id) == control.target_id)
            .expect("audio cache modifier exists");
        let shrimply_audio_modifiers::AudioModifierEffect::Cache(cache) = &modifier.effect else {
            panic!("audio cache control requires cache effect");
        };
        return super::audio_cache::rows(cache, modifier.id, context);
    }
    let section = InspectorSection::controls();
    for control in model.controls.iter().filter(|control| control.visible) {
        match control.kind {
            ControlKind::TtsEditor => {
                let project = context.project.borrow();
                let item = project
                    .audio_item(context.selected_item.as_ref().expect("TTS item"))
                    .expect("TTS item exists");
                let AudioSource::Tts(settings) = &item.source else {
                    panic!("TTS control requires TTS source");
                };
                section.add_wide_control(&tts_editor(item.id, settings, context));
            }
            ControlKind::BeatDetection => {
                section.add_wide_control(&beat_detection(control, context));
            }
            ControlKind::AudioModifierMenu => {
                section.add_wide_control(&modifier_menu(control, context))
            }
            _ => super::control::add_control(
                &section,
                context,
                &context.list_target,
                control.clone(),
            ),
        }
    }
    vec![section.into_widget()]
}

fn beat_detection(control: &InspectorControl, context: &InspectorContext) -> gtk::Widget {
    let id = control.target_id.expect("beat detection item");
    let enabled = control.value.parse().expect("beat detection is boolean");
    let row = adw::ActionRow::builder()
        .title(tr!(&control.label).as_ref())
        .subtitle(tr!(&control.subtitle).as_ref())
        .build();
    let spinner = adw::Spinner::new();
    spinner.set_size_request(18, 18);
    spinner.set_visible(enabled && shrimply_audio_engine::beat::is_loading(id));
    row.add_suffix(&spinner);
    let toggle = gtk::Switch::builder()
        .active(enabled)
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&toggle);
    row.set_activatable_widget(Some(&toggle));
    if enabled {
        let spinner = spinner.downgrade();
        let mut polls_without_loading = 0_u8;
        let mut observed_loading = shrimply_audio_engine::beat::is_loading(id);
        glib::timeout_add_local(Duration::from_millis(33), move || {
            let Some(spinner) = spinner.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let loading = shrimply_audio_engine::beat::is_loading(id);
            spinner.set_visible(loading);
            observed_loading |= loading;
            if observed_loading && !loading {
                return glib::ControlFlow::Break;
            }
            polls_without_loading = polls_without_loading.saturating_add((!loading) as u8);
            if polls_without_loading >= 60 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
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
    let menu_controller = controller.clone();
    let menu_target = target.clone();
    buttons.append(&searchable_menu(
        tr!("Add modifier").as_ref(),
        tr!("Search modifiers").as_ref(),
        move |query| {
            let mut choices = control
                .values
                .iter()
                .zip(&control.labels)
                .zip(&control.search_terms)
                .filter_map(|((value, label), search)| {
                    Some((search_rank(label, [search.as_str()], query)?, value, label))
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
                        if let Err(error) = controller.add_control_modifier(
                            &target,
                            ControlKind::AudioModifierMenu,
                            &value,
                        ) {
                            tracing::warn!(%error, "Could not add audio modifier");
                        }
                    })
                })
                .collect()
        },
    ));
    if controller.can_paste_audio_modifiers(&target, &context.property_clipboard) {
        let paste = gtk::Button::builder()
            .icon_name("edit-paste-symbolic")
            .tooltip_text(tr!("Paste Modifier").as_ref())
            .build();
        let clipboard = context.property_clipboard.clone();
        paste.connect_clicked(move |button| {
            match controller.paste_audio_modifiers(&target, &clipboard) {
                Ok(count) if count > 0 => {
                    let message = if count == 1 {
                        tr!("1 effect pasted").into_owned()
                    } else {
                        shrimply_components_gtk::i18n::text_args(
                            "%{count} effects pasted",
                            &[("count", count.to_string())],
                        )
                    };
                    shrimply_components_gtk::toast::show_confirmation_text_for_widget(
                        button, &message,
                    );
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "Could not paste audio modifiers"),
            }
        });
        buttons.append(&paste);
    }
    buttons.upcast()
}

fn tts_editor(
    id: uuid::Uuid,
    settings: &shrimply_tts::TtsSettings,
    context: &InspectorContext,
) -> gtk::Widget {
    let target = context.list_target.clone();
    let changed_controller = context.inspector_core.clone();
    let changed_target = target.clone();
    let commit_controller = context.inspector_core.clone();
    let generated_controller = context.inspector_core.clone();
    shrimply_tts_gtk::editor(
        id,
        context.preferences.clone(),
        settings,
        move |settings| {
            if let Err(error) = changed_controller.set_tts_settings(&changed_target, settings, true)
            {
                tracing::error!(%error, "Could not edit GTK TTS settings");
            }
        },
        move || commit_controller.finish_live_edit(),
        move |generation, model| {
            if let Err(error) =
                generated_controller.apply_tts_generation(&target, id, &model, generation)
            {
                tracing::error!(%error, "Could not apply GTK TTS generation");
            }
        },
    )
}
