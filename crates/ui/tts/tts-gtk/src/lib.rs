use gtk::{gio, glib, prelude::*};
use shrimply_components_gtk::{tr, ui};
use shrimply_editor_state::preferences as preferences_store;
use shrimply_inspector_core::{
    InspectorController, InspectorTarget,
    tts::{self, EditorPresentation, TtsEditorControl, TtsInputEdit},
};
use std::{cell::RefCell, rc::Rc};

type EditInput = dyn Fn(&str, TtsInputEdit) -> bool;

#[derive(Clone)]
struct Inputs {
    edit: Rc<EditInput>,
    commit: Rc<dyn Fn()>,
    refresh: Rc<dyn Fn()>,
}

pub fn editor(
    controller: InspectorController,
    target: InspectorTarget,
    preferences: preferences_store::SharedPreferences,
    presentation: EditorPresentation,
    refresh: Rc<dyn Fn()>,
) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let actions = presentation.actions();
    let status = gtk::Label::builder()
        .label(tr!(actions.status).as_ref())
        .wrap(true)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["dim-label"])
        .build();
    let spinner = gtk::Spinner::new();
    spinner.start();
    spinner.set_visible(actions.busy);
    let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    status_row.append(&spinner);
    status_row.append(&status);
    let model_controller = controller.clone();
    let model_target = target.clone();
    let model_preferences = preferences.clone();
    let model_refresh = refresh.clone();
    let server_url = presentation.server_url.clone();
    let selector = ui::labeled_string_selector(
        tts::MODEL_LABEL,
        presentation
            .model
            .as_ref()
            .map_or("", |model| model.id.as_str()),
        presentation
            .models
            .iter()
            .map(|choice| ui::StringChoice {
                value: choice.value.clone(),
                label: choice.label.clone(),
            })
            .collect(),
        move |id| {
            if report(model_controller.select_tts_model(&model_target, &server_url, &id)) {
                preferences_store::set_last_tts_model(&model_preferences, &id);
                model_refresh();
            }
        },
    );
    selector.set_sensitive(actions.model_sensitive);
    content.append(selector.widget());
    if presentation.can_retry {
        let retry = gtk::Button::with_label(tr!(tts::RETRY_LABEL).as_ref());
        let retry_controller = controller.clone();
        let retry_refresh = refresh.clone();
        let server_url = presentation.server_url.clone();
        retry.connect_clicked(move |_| {
            retry_controller.retry_tts_models(&server_url);
            retry_refresh();
        });
        content.append(&retry);
    }
    if let Some(model) = &presentation.model {
        let editing_controller = controller.clone();
        let editing_target = target.clone();
        let editing_model = model.clone();
        let committed = controller.clone();
        let input = Inputs {
            edit: Rc::new(move |key, edit| {
                match editing_controller.edit_tts_input(&editing_target, &editing_model, key, edit)
                {
                    Ok(_) => true,
                    Err(error) => {
                        report::<()>(Err(error));
                        false
                    }
                }
            }),
            commit: Rc::new(move || committed.finish_live_edit()),
            refresh: refresh.clone(),
        };
        let fields = fields(&presentation.controls, input);
        fields.set_sensitive(actions.inputs_sensitive);
        content.append(&fields);
        let generate = gtk::Button::with_label(tr!(actions.generate_label).as_ref());
        generate.set_sensitive(actions.generate_sensitive);
        generate.add_css_class("suggested-action");
        let model = model.clone();
        let server_url = presentation.server_url.clone();
        generate.connect_clicked(move |button| {
            controller.finish_live_edit();
            match controller.toggle_tts_generation(&target, &server_url, &model) {
                Ok(()) => {
                    preferences_store::set_last_tts_model(&preferences, &model.id);
                    refresh();
                }
                Err(error) => {
                    shrimply_components_gtk::toast::show_confirmation_text_for_widget(
                        button, &error,
                    );
                }
            }
        });
        status_row.append(&generate);
    }
    content.append(&status_row);
    content.upcast()
}

pub fn caption_configuration(
    preferences: preferences_store::SharedPreferences,
    settings: Rc<RefCell<shrimply_tts::TtsSettings>>,
    models: Rc<RefCell<Vec<shrimply_tts::TtsModel>>>,
    on_changed: Rc<dyn Fn(shrimply_tts::TtsSettings)>,
    on_commit: Rc<dyn Fn()>,
) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let runtime = Rc::new(RefCell::new(tts::Runtime::default()));
    let refresh = Rc::new(RefCell::new(None::<Rc<dyn Fn()>>));
    let weak_refresh = Rc::downgrade(&refresh);
    let rerender: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(refresh) = weak_refresh.upgrade()
            && let Some(refresh) = refresh.borrow().as_ref()
        {
            refresh();
        }
    });
    *refresh.borrow_mut() = Some(Rc::new({
        let content = content.downgrade();
        let runtime = runtime.clone();
        let rerender = rerender.clone();
        move || {
            let Some(content) = content.upgrade() else {
                return;
            };
            let snapshot = preferences_store::snapshot(&preferences);
            let original = settings.borrow().clone();
            let presentation = runtime.borrow_mut().configuration(
                &mut settings.borrow_mut(),
                &snapshot.compute_server_url,
                &snapshot.last_tts_model,
                tts::ConfigurationKind::Caption,
            );
            *models.borrow_mut() = presentation.model.iter().cloned().collect();
            if let Some(model) = &presentation.model {
                preferences_store::set_last_tts_model(&preferences, &model.id);
            }
            if *settings.borrow() != original {
                on_changed(settings.borrow().clone());
                on_commit();
            }
            while let Some(child) = content.first_child() {
                content.remove(&child);
            }
            let selector_runtime = runtime.clone();
            let selector_settings = settings.clone();
            let selector_preferences = preferences.clone();
            let selector_changed = on_changed.clone();
            let selector_commit = on_commit.clone();
            let selector_refresh = rerender.clone();
            let server_url = presentation.server_url.clone();
            let selector = ui::labeled_string_selector(
                tts::MODEL_LABEL,
                presentation
                    .model
                    .as_ref()
                    .map_or("", |model| model.id.as_str()),
                presentation
                    .models
                    .iter()
                    .map(|choice| ui::StringChoice {
                        value: choice.value.clone(),
                        label: choice.label.clone(),
                    })
                    .collect(),
                move |id| {
                    if report(selector_runtime.borrow_mut().select_configuration_model(
                        &mut selector_settings.borrow_mut(),
                        &server_url,
                        &id,
                        tts::ConfigurationKind::Caption,
                    )) {
                        selector_changed(selector_settings.borrow().clone());
                        selector_commit();
                        preferences_store::set_last_tts_model(&selector_preferences, &id);
                        selector_refresh();
                    }
                },
            );
            selector.set_sensitive(presentation.actions().model_sensitive);
            content.append(selector.widget());
            if let Some(model) = &presentation.model {
                let model = model.clone();
                let edited = settings.clone();
                let changed = on_changed.clone();
                let input = Inputs {
                    edit: Rc::new(move |key, edit| {
                        let result = tts::edit_input(&mut edited.borrow_mut(), &model, key, edit);
                        if report(result) {
                            changed(edited.borrow().clone());
                            true
                        } else {
                            false
                        }
                    }),
                    commit: on_commit.clone(),
                    refresh: rerender.clone(),
                };
                content.append(&fields(&presentation.controls, input));
            }
            if presentation.can_retry {
                let retry = gtk::Button::with_label(tr!(tts::RETRY_LABEL).as_ref());
                let retry_runtime = runtime.clone();
                let retry_refresh = rerender.clone();
                let server_url = presentation.server_url.clone();
                retry.connect_clicked(move |_| {
                    retry_runtime.borrow_mut().retry_models(&server_url);
                    retry_refresh();
                });
                content.append(&retry);
            }
            if !presentation.catalog_status.is_empty() {
                content.append(
                    &gtk::Label::builder()
                        .label(tr!(&presentation.catalog_status).as_ref())
                        .wrap(true)
                        .xalign(0.0)
                        .css_classes(["dim-label"])
                        .build(),
                );
            }
        }
    }));
    refresh
        .borrow()
        .as_ref()
        .expect("caption refresh installed")();
    let content_weak = content.downgrade();
    glib::timeout_add_local(shrimply_inspector_core::tts::POLL_INTERVAL, move || {
        if content_weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        if runtime.borrow_mut().poll_catalog() {
            refresh
                .borrow()
                .as_ref()
                .expect("caption refresh installed")();
        }
        glib::ControlFlow::Continue
    });
    content.upcast()
}

fn fields(controls: &[TtsEditorControl], input: Inputs) -> gtk::Widget {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 12);
    for field in controls {
        let row = match field {
            TtsEditorControl::Text {
                key,
                label,
                value,
                multiline,
                max_length,
            } => text(key, label, value, *multiline, *max_length, input.clone()),
            TtsEditorControl::Select {
                key,
                label,
                value,
                choices,
            } => {
                let key = key.clone();
                let edited = input.clone();
                ui::labeled_string_selector(
                    label,
                    value,
                    choices
                        .iter()
                        .map(|choice| ui::StringChoice {
                            value: choice.value.clone(),
                            label: choice.label.clone(),
                        })
                        .collect(),
                    move |value| {
                        if (edited.edit)(&key, TtsInputEdit::Select(value)) {
                            (edited.commit)();
                            (edited.refresh)();
                        }
                    },
                )
                .widget()
                .clone()
            }
            TtsEditorControl::Toggle { key, label, value } => {
                let key = key.clone();
                let edited = input.clone();
                ui::switch_row(label, None, *value, move |value| {
                    if (edited.edit)(&key, TtsInputEdit::Toggle(value)) {
                        (edited.commit)();
                        (edited.refresh)();
                    }
                })
            }
            TtsEditorControl::Number {
                key,
                label,
                value,
                minimum,
                maximum,
                step,
                digits,
            } => {
                let key = key.clone();
                let edited = input.clone();
                let committed = input.clone();
                let picker = ui::NumberPicker::fraction_builder(*value)
                    .accepted_range(
                        shrimply_math_core::fraction_as_f64(*minimum),
                        shrimply_math_core::fraction_as_f64(*maximum),
                    )
                    .drag_step(shrimply_math_core::fraction_as_f64(*step))
                    .digits(*digits)
                    .on_change_fraction(move |value| {
                        (edited.edit)(&key, TtsInputEdit::Number(value));
                    })
                    .on_commit(move |_| (committed.commit)())
                    .build();
                ui::control_row(label, &picker)
            }
            TtsEditorControl::Audio { key, label, path } => {
                audio(key, label, path.as_deref(), input.clone())
            }
            TtsEditorControl::Table {
                key,
                label,
                columns,
                rows,
            } => {
                let table = gtk::Box::new(gtk::Orientation::Vertical, 6);
                let add = gtk::Button::builder()
                    .icon_name("list-add-symbolic")
                    .tooltip_text(tr!(tts::ADD_ROW_LABEL).as_ref())
                    .build();
                let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                header.set_halign(gtk::Align::End);
                header.append(&add);
                table.append(&ui::control_row(label, &header));
                let added = input.clone();
                let add_key = key.clone();
                add.connect_clicked(move |_| {
                    if (added.edit)(&add_key, TtsInputEdit::AddTableRow) {
                        (added.commit)();
                        (added.refresh)();
                    }
                });
                for (index, values) in rows.iter().enumerate() {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    for (column_index, column) in columns.iter().enumerate() {
                        let edited = input.clone();
                        let committed = input.clone();
                        let key = key.clone();
                        let entry = ui::SingleLineTextInput::builder(
                            values.get(&column.key).cloned().unwrap_or_default(),
                        )
                        .placeholder(&column.label)
                        .max_length(column.max_length)
                        .on_change(move |value| {
                            (edited.edit)(
                                &key,
                                TtsInputEdit::TableCell {
                                    row: index,
                                    column: column_index,
                                    value,
                                },
                            );
                        })
                        .on_commit(move |_| (committed.commit)())
                        .build();
                        row.append(&entry);
                    }
                    let remove = gtk::Button::builder()
                        .icon_name("list-remove-symbolic")
                        .tooltip_text(tr!(tts::REMOVE_ROW_LABEL).as_ref())
                        .build();
                    let removed = input.clone();
                    let key = key.clone();
                    remove.connect_clicked(move |_| {
                        if (removed.edit)(&key, TtsInputEdit::RemoveTableRow(index)) {
                            (removed.commit)();
                            (removed.refresh)();
                        }
                    });
                    row.append(&remove);
                    table.append(&row);
                }
                table.upcast()
            }
        };
        container.append(&row);
    }
    container.upcast()
}

fn text(
    key: &str,
    label: &str,
    value: &str,
    multiline: bool,
    max_length: usize,
    input: Inputs,
) -> gtk::Widget {
    let key = key.to_owned();
    let edited = input.clone();
    if multiline {
        let editor = ui::MultilineTextInput::builder(value)
            .max_length(max_length)
            .on_change(move |value| (edited.edit)(&key, TtsInputEdit::Text(value)))
            .on_commit(move || (input.commit)())
            .build();
        let row = ui::control_row(label, editor.widget());
        row.first_child()
            .expect("text row label")
            .set_valign(gtk::Align::Start);
        row
    } else {
        let editor = ui::SingleLineTextInput::builder(value)
            .max_length(max_length)
            .on_change(move |value| {
                (edited.edit)(&key, TtsInputEdit::Text(value));
            })
            .on_commit(move |_| (input.commit)())
            .build();
        ui::control_row(label, &editor)
    }
}

fn audio(key: &str, label: &str, path: Option<&std::path::Path>, input: Inputs) -> gtk::Widget {
    let selected = Rc::new(RefCell::new(path.map(std::path::Path::to_path_buf)));
    let path_label = gtk::Label::builder()
        .label(
            path.map(|path| path.display().to_string())
                .unwrap_or_else(|| tr!(tts::AUDIO_EMPTY_LABEL).into_owned()),
        )
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .hexpand(true)
        .xalign(0.0)
        .build();
    let choose = gtk::Button::with_label(tr!(tts::CHOOSE_AUDIO_LABEL).as_ref());
    let model = gio::Menu::new();
    model.append(
        Some(tr!(tts::CLEAR_AUDIO_LABEL).as_ref()),
        Some("audio.clear"),
    );
    model.append(
        Some(tr!(tts::REVEAL_AUDIO_LABEL).as_ref()),
        Some("audio.show"),
    );
    let actions = gio::SimpleActionGroup::new();
    let menu = gtk::MenuButton::builder()
        .icon_name("pan-down-symbolic")
        .tooltip_text(tr!(tts::AUDIO_ACTIONS_LABEL).as_ref())
        .menu_model(&model)
        .build();
    menu.insert_action_group("audio", Some(&actions));
    ui::adaptive_menu(&menu, "audio", &actions);
    let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    split.add_css_class("linked");
    split.append(&choose);
    split.append(&menu);
    let shown = selected.clone();
    let show_parent = menu.downgrade();
    let show = ui::menu_action("show", None, move || {
        if let Some(parent) = show_parent.upgrade()
            && let Some(path) = shown.borrow().clone()
            && let Err(error) = shrimply_components_gtk::desktop_open::show_path_in_folder(
                parent.upcast_ref(),
                path,
            )
        {
            shrimply_components_gtk::toast::show_confirmation_text_for_widget(&parent, &error);
        }
        None
    });
    show.set_enabled(path.is_some());
    actions.add_action(&show);
    let cleared = input.clone();
    let clear_key = key.to_owned();
    let clear_show = show.clone();
    let clear_path = selected.clone();
    let clear_label = path_label.clone();
    let cleared_actions = actions.downgrade();
    let clear = ui::menu_action("clear", None, move || {
        if (cleared.edit)(&clear_key, TtsInputEdit::Audio(None)) {
            clear_path.borrow_mut().take();
            clear_label.set_label(tr!(tts::AUDIO_EMPTY_LABEL).as_ref());
            clear_show.set_enabled(false);
            if let Some(actions) = cleared_actions.upgrade()
                && let Some(clear) = actions
                    .lookup_action("clear")
                    .and_downcast::<gio::SimpleAction>()
            {
                clear.set_enabled(false);
            }
            (cleared.commit)();
            (cleared.refresh)();
        }
        None
    });
    clear.set_enabled(path.is_some());
    actions.add_action(&clear);
    let key = key.to_owned();
    let chooser_label = tts::AUDIO_PICKER_TITLE.to_owned();
    let chosen_path = selected.clone();
    let chosen_label = path_label.clone();
    choose.connect_clicked(move |button| {
        let dialog = gtk::FileDialog::builder()
            .title(tr!(&chooser_label).as_ref())
            .build();
        let input = input.clone();
        let key = key.clone();
        let selected = chosen_path.clone();
        let path_label = chosen_label.clone();
        let clear = clear.clone();
        let show = show.clone();
        shrimply_components_gtk::file_picker::open(
            &chooser_label,
            &dialog,
            button.root().and_downcast::<gtk::Window>().as_ref(),
            move |result| {
                if let Some(path) = result.ok().and_then(|file| file.path())
                    && (input.edit)(&key, TtsInputEdit::Audio(Some(path.clone())))
                {
                    path_label.set_label(&path.display().to_string());
                    *selected.borrow_mut() = Some(path);
                    clear.set_enabled(true);
                    show.set_enabled(true);
                    (input.commit)();
                    (input.refresh)();
                }
            },
        );
    });
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    controls.append(&path_label);
    controls.append(&split);
    ui::control_row(label, &controls)
}

fn report<T>(result: Result<T, String>) -> bool {
    match result {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, "Could not edit text-to-speech controls");
            false
        }
    }
}
