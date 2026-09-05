use shrimply_gtk_components::tr;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use shrimply_gtk_components::ui;
use shrimply_inspector_core::tts::{TtsGeneration, TtsInputEdit};
use shrimply_math_core::fraction_as_f64;
use shrimply_project::project::Time;
use shrimply_state::preferences as preferences_store;
use shrimply_tts::{
    Fraction, InputDefinition, Speech, TableColumn, TtsModel, TtsSettings, TtsValue, is_visible,
};
use uuid::Uuid;

enum GenerationMessage {
    Progress(String),
    Done(Result<TtsGeneration, String>),
}

type VisibilityRefresh = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

#[derive(Clone)]
struct EditorCallbacks {
    on_changed: Rc<dyn Fn(TtsSettings)>,
    on_commit: Rc<dyn Fn()>,
    on_generated: Rc<dyn Fn(TtsGeneration, TtsModel)>,
}

struct CachedEditor {
    widget: gtk::Widget,
    settings: Rc<RefCell<TtsSettings>>,
    callbacks: Rc<RefCell<EditorCallbacks>>,
    active_generation: Rc<RefCell<Option<shrimply_server_client::CancellationToken>>>,
    server_url: String,
}

thread_local! {
    static EDITOR_CACHE: RefCell<HashMap<Uuid, CachedEditor>> = RefCell::new(HashMap::new());
    static MODEL_CACHE: RefCell<HashMap<String, Vec<TtsModel>>> = RefCell::new(HashMap::new());
}

pub fn editor(
    id: Uuid,
    preferences: preferences_store::SharedPreferences,
    value: &TtsSettings,
    on_changed: impl Fn(TtsSettings) + 'static,
    on_commit: impl Fn() + 'static,
    on_generated: impl Fn(TtsGeneration, TtsModel) + 'static,
) -> gtk::Widget {
    let server_url = preferences_store::snapshot(&preferences).compute_server_url;
    let callbacks = EditorCallbacks {
        on_changed: Rc::new(on_changed),
        on_commit: Rc::new(on_commit),
        on_generated: Rc::new(on_generated),
    };
    if let Some(widget) = EDITOR_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|cached_id, cached| {
            *cached_id == id || cached.active_generation.borrow().is_some()
        });
        let replace = cache.get(&id).is_some_and(|cached| {
            cached.active_generation.borrow().is_none()
                && (cached.server_url != server_url || *cached.settings.borrow() != *value)
        });
        if replace {
            cache.remove(&id);
        }
        let cached = cache.get_mut(&id)?;
        *cached.callbacks.borrow_mut() = callbacks.clone();
        detach(&cached.widget);
        Some(cached.widget.clone())
    }) {
        return widget;
    }

    let settings = Rc::new(RefCell::new(value.clone()));
    let models = Rc::new(RefCell::new(Vec::<TtsModel>::new()));
    let callbacks = Rc::new(RefCell::new(callbacks));
    let on_changed: Rc<dyn Fn(TtsSettings)> = {
        let callbacks = callbacks.clone();
        Rc::new(move |value| (callbacks.borrow().on_changed)(value))
    };
    let on_commit: Rc<dyn Fn()> = {
        let callbacks = callbacks.clone();
        Rc::new(move || (callbacks.borrow().on_commit)())
    };
    let on_generated: Rc<dyn Fn(TtsGeneration, TtsModel)> = {
        let callbacks = callbacks.clone();
        Rc::new(move |generation, model| (callbacks.borrow().on_generated)(generation, model))
    };
    let configuration = configuration(
        preferences.clone(),
        settings.clone(),
        models.clone(),
        on_changed.clone(),
        on_commit.clone(),
    );

    let status = gtk::Label::builder()
        .label(tr!("Connecting to server…").as_ref())
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let spinner = gtk::Spinner::new();
    spinner.start();
    spinner.set_size_request(18, 18);
    let generate = gtk::Button::builder()
        .label(
            tr!(if value.model.is_some() {
                "Regenerate"
            } else {
                "Generate"
            })
            .as_ref(),
        )
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    let cancel = gtk::Button::with_label(tr!("Cancel").as_ref());
    cancel.set_visible(false);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&spinner);
    actions.append(&status);
    status.set_hexpand(true);
    actions.append(&cancel);
    actions.append(&generate);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(&configuration);
    content.append(&actions);

    let active_generation = Rc::new(RefCell::new(
        None::<shrimply_server_client::CancellationToken>,
    ));
    {
        let active_generation = active_generation.clone();
        let status = status.clone();
        let cancel = cancel.clone();
        cancel.clone().connect_clicked(move |_| {
            if let Some(cancellation) = active_generation.borrow().as_ref() {
                status.set_label(tr!("Cancelling…").as_ref());
                cancel.set_sensitive(false);
                cancellation.cancel();
            }
        });
    }

    {
        let generate = generate.clone();
        let spinner = spinner.clone();
        let status = status.clone();
        let models = models.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if !models.borrow().is_empty() {
                spinner.set_visible(false);
                status.set_label(tr!("Ready").as_ref());
                generate.set_sensitive(true);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    {
        let preferences = preferences.clone();
        let settings = settings.clone();
        let models = models.clone();
        let status = status.clone();
        let spinner = spinner.clone();
        let cancel = cancel.clone();
        let active_generation = active_generation.clone();
        generate.clone().connect_clicked(move |button| {
            let value = settings.borrow().clone();
            let Some(selected) = value.model.as_ref().and_then(|id| {
                models
                    .borrow()
                    .iter()
                    .find(|model| &model.id == id)
                    .cloned()
            }) else {
                status.set_label(tr!("Select a model").as_ref());
                return;
            };
            preferences_store::set_last_tts_model(&preferences, &selected.id);
            let server_url = preferences_store::snapshot(&preferences).compute_server_url;
            let cancellation = match shrimply_server_client::CancellationToken::new(&server_url) {
                Ok(cancellation) => cancellation,
                Err(error) => {
                    status.set_label(&error);
                    return;
                }
            };
            *active_generation.borrow_mut() = Some(cancellation.clone());
            button.set_sensitive(false);
            cancel.set_visible(true);
            cancel.set_sensitive(true);
            spinner.set_visible(true);
            status.set_tooltip_text(None);
            status.set_label(tr!("Sending request…").as_ref());

            let (sender, receiver) = mpsc::channel();
            let request_model = selected.clone();
            thread::spawn(move || {
                let result = shrimply_inspector_core::tts::generate(
                    &server_url,
                    &cancellation,
                    &request_model,
                    &value,
                    |message| {
                        let _ = sender.send(GenerationMessage::Progress(message.to_string()));
                        !cancellation.is_cancelled()
                    },
                );
                let _ = sender.send(GenerationMessage::Done(result));
            });

            let status = status.clone();
            let spinner = spinner.clone();
            let generate = button.clone();
            let cancel = cancel.clone();
            let active_generation = active_generation.clone();
            let on_generated = on_generated.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                loop {
                    match receiver.try_recv() {
                        Ok(GenerationMessage::Progress(message)) => status.set_label(&message),
                        Ok(GenerationMessage::Done(result)) => {
                            generate.set_sensitive(true);
                            cancel.set_visible(false);
                            cancel.set_sensitive(true);
                            spinner.set_visible(false);
                            let cancelled = active_generation
                                .borrow()
                                .as_ref()
                                .is_some_and(|cancellation| cancellation.is_cancelled());
                            active_generation.borrow_mut().take();
                            match result {
                                Ok(generation) => {
                                    generate.set_label(tr!("Regenerate").as_ref());
                                    status.set_label(tr!("Generated").as_ref());
                                    on_generated(generation, selected.clone());
                                }
                                Err(_) if cancelled => {
                                    status.set_label(tr!("Cancelled").as_ref())
                                }
                                Err(error) if error.starts_with("Compute server connection failed") => {
                                    tracing::error!(%error, "Text-to-speech compute connection failed");
                                    status.set_label(tr!("Compute server connection failed").as_ref());
                                    status.set_tooltip_text(Some(&error));
                                }
                                Err(error) => status.set_label(&error),
                            }
                            return glib::ControlFlow::Break;
                        }
                        Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(TryRecvError::Disconnected) => {
                            generate.set_sensitive(true);
                            cancel.set_visible(false);
                            cancel.set_sensitive(true);
                            spinner.set_visible(false);
                            active_generation.borrow_mut().take();
                            status.set_label(tr!("Generation worker stopped unexpectedly").as_ref());
                            return glib::ControlFlow::Break;
                        }
                    }
                }
            });
        });
    }

    let widget = content.upcast::<gtk::Widget>();
    EDITOR_CACHE.with(|cache| {
        cache.borrow_mut().insert(
            id,
            CachedEditor {
                widget: widget.clone(),
                settings,
                callbacks,
                active_generation,
                server_url,
            },
        );
    });
    widget
}

fn detach(widget: &gtk::Widget) {
    let Some(parent) = widget.parent() else {
        return;
    };
    parent
        .downcast::<gtk::Box>()
        .expect("cached TTS editor must be parented by an inspector control box")
        .remove(widget);
}

pub fn configuration(
    preferences: preferences_store::SharedPreferences,
    settings: Rc<RefCell<TtsSettings>>,
    models: Rc<RefCell<Vec<TtsModel>>>,
    on_changed: Rc<dyn Fn(TtsSettings)>,
    on_commit: Rc<dyn Fn()>,
) -> gtk::Widget {
    configuration_for(
        preferences,
        settings,
        models,
        on_changed,
        on_commit,
        ConfigurationKind::Regular,
    )
}

pub fn caption_configuration(
    preferences: preferences_store::SharedPreferences,
    settings: Rc<RefCell<TtsSettings>>,
    models: Rc<RefCell<Vec<TtsModel>>>,
    on_changed: Rc<dyn Fn(TtsSettings)>,
    on_commit: Rc<dyn Fn()>,
) -> gtk::Widget {
    configuration_for(
        preferences,
        settings,
        models,
        on_changed,
        on_commit,
        ConfigurationKind::Caption,
    )
}

fn configuration_for(
    preferences: preferences_store::SharedPreferences,
    settings: Rc<RefCell<TtsSettings>>,
    models: Rc<RefCell<Vec<TtsModel>>>,
    on_changed: Rc<dyn Fn(TtsSettings)>,
    on_commit: Rc<dyn Fn()>,
    kind: ConfigurationKind,
) -> gtk::Widget {
    let fields = Rc::new(RefCell::new(ui::KeyedBox::new(
        gtk::Orientation::Vertical,
        12,
    )));
    let definitions = Rc::new(RefCell::new(Vec::new()));
    let update_visibility = Rc::new(RefCell::new(None::<Rc<dyn Fn()>>));
    let input = InputContext {
        settings: settings.clone(),
        models: models.clone(),
        on_changed: on_changed.clone(),
        on_commit: on_commit.clone(),
        update_visibility: update_visibility.clone(),
    };
    let field_state = FieldState {
        input: input.clone(),
        fields: fields.clone(),
        definitions: definitions.clone(),
        kind,
    };
    *update_visibility.borrow_mut() = Some({
        let settings = settings.clone();
        let fields = Rc::downgrade(&fields);
        Rc::new(move || {
            let Some(fields) = fields.upgrade() else {
                return;
            };
            let fields = fields.borrow();
            for definition in definitions.borrow().iter() {
                if let Some(widget) = fields.child(&definition.key().to_string()) {
                    widget.set_visible(is_visible(definition, &settings.borrow().inputs));
                }
            }
        })
    });

    let selected_model = settings.borrow().model.clone().unwrap_or_default();
    let model_selector = {
        let field_state = field_state.clone();
        let models = models.clone();
        let preferences = preferences.clone();
        ui::labeled_string_selector("Model", &selected_model, Vec::new(), move |id| {
            let Some(model) = models.borrow().iter().find(|model| model.id == id).cloned() else {
                return;
            };
            let synchronized = shrimply_inspector_core::tts::synchronized_settings(
                &field_state.input.settings.borrow(),
                &model,
            );
            *field_state.input.settings.borrow_mut() = synchronized;
            (field_state.input.on_changed)(field_state.input.settings.borrow().clone());
            (field_state.input.on_commit)();
            preferences_store::set_last_tts_model(&preferences, &model.id);
            reconcile_fields(&field_state, &model);
        })
    };
    model_selector.set_sensitive(false);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(model_selector.widget());
    content.append(fields.borrow().widget());

    let configuration = ConfigurationState {
        preferences: preferences.clone(),
        models: models.clone(),
        model_selector: model_selector.clone(),
        fields: field_state,
        kind,
    };

    {
        let server_url = preferences_store::snapshot(&preferences).compute_server_url;
        if let Some(cached) = MODEL_CACHE.with(|cache| cache.borrow().get(&server_url).cloned()) {
            apply_models(&configuration, cached);
        }
        let refreshing = Rc::new(Cell::new(false));
        refresh_models(server_url, configuration, refreshing);
    }

    content.upcast()
}

#[derive(Clone)]
struct ConfigurationState {
    preferences: preferences_store::SharedPreferences,
    models: Rc<RefCell<Vec<TtsModel>>>,
    model_selector: ui::StringSelector,
    fields: FieldState,
    kind: ConfigurationKind,
}

#[derive(Clone)]
struct FieldState {
    input: InputContext,
    fields: Rc<RefCell<ui::KeyedBox<String, InputDefinition>>>,
    definitions: Rc<RefCell<Vec<InputDefinition>>>,
    kind: ConfigurationKind,
}

#[derive(Clone, Copy)]
enum ConfigurationKind {
    Regular,
    Caption,
}

#[derive(Clone)]
struct InputContext {
    settings: Rc<RefCell<TtsSettings>>,
    models: Rc<RefCell<Vec<TtsModel>>>,
    on_changed: Rc<dyn Fn(TtsSettings)>,
    on_commit: Rc<dyn Fn()>,
    update_visibility: VisibilityRefresh,
}

fn apply_models(configuration: &ConfigurationState, mut available: Vec<TtsModel>) {
    if matches!(configuration.kind, ConfigurationKind::Caption) {
        available.retain(supports_duration);
    }
    if available.is_empty() || *configuration.models.borrow() == available {
        return;
    }
    let previous = configuration.models.borrow().clone();
    let remembered = preferences_store::snapshot(&configuration.preferences).last_tts_model;
    let model = shrimply_inspector_core::tts::selected_model(
        &available,
        &configuration.fields.input.settings.borrow(),
        &remembered,
    )
    .expect("nonempty TTS model list must have a selection")
    .clone();
    let choices_changed = previous
        .iter()
        .map(|model| (&model.id, &model.label))
        .ne(available.iter().map(|model| (&model.id, &model.label)));
    *configuration.models.borrow_mut() = available;

    if choices_changed {
        let choices = configuration
            .models
            .borrow()
            .iter()
            .map(|model| ui::StringChoice {
                value: model.id.clone(),
                label: model.label.clone(),
            })
            .collect::<Vec<_>>();
        configuration.model_selector.set_choices(&model.id, choices);
    }
    configuration.model_selector.set_sensitive(true);

    let original = configuration.fields.input.settings.borrow().clone();
    let synchronized = shrimply_inspector_core::tts::synchronized_settings(
        &configuration.fields.input.settings.borrow(),
        &model,
    );
    *configuration.fields.input.settings.borrow_mut() = synchronized;
    if *configuration.fields.input.settings.borrow() != original {
        (configuration.fields.input.on_changed)(
            configuration.fields.input.settings.borrow().clone(),
        );
        (configuration.fields.input.on_commit)();
    }
    preferences_store::set_last_tts_model(&configuration.preferences, &model.id);
    reconcile_fields(&configuration.fields, &model);
}

fn refresh_models(
    server_url: String,
    configuration: ConfigurationState,
    refreshing: Rc<Cell<bool>>,
) {
    if refreshing.replace(true) {
        return;
    }
    let (sender, receiver) = async_channel::bounded(1);
    let request_url = server_url.clone();
    thread::spawn(move || {
        let _ = sender.send_blocking(available_models(&request_url));
    });
    glib::spawn_future_local(async move {
        let result = receiver.recv().await;
        refreshing.set(false);
        let Ok(Ok(available)) = result else {
            return;
        };
        MODEL_CACHE.with(|cache| {
            cache.borrow_mut().insert(server_url, available.clone());
        });
        apply_models(&configuration, available);
    });
}

fn available_models(server_url: &str) -> Result<Vec<TtsModel>, String> {
    shrimply_inspector_core::tts::available_models(server_url)
}

fn reconcile_fields(state: &FieldState, model: &TtsModel) {
    let definitions = model
        .inputs
        .iter()
        .filter(|definition| {
            matches!(state.kind, ConfigurationKind::Regular) || is_caption_input(definition)
        })
        .cloned()
        .collect::<Vec<_>>();
    *state.definitions.borrow_mut() = definitions.clone();
    let input = state.input.clone();
    state.fields.borrow_mut().reconcile(
        definitions
            .into_iter()
            .map(|definition| (definition.key().to_string(), definition)),
        move |_, definition| input_widget(definition, &input),
    );
    if let Some(refresh) = state.input.update_visibility.borrow().as_ref() {
        refresh();
    }
}

fn supports_duration(model: &TtsModel) -> bool {
    model
        .inputs
        .iter()
        .any(|input| input.purpose() == Some(shrimply_tts::InputPurpose::Duration))
}

fn is_caption_input(input: &InputDefinition) -> bool {
    match input {
        InputDefinition::Text { purpose, .. } | InputDefinition::Number { purpose, .. } => {
            !matches!(
                purpose,
                Some(
                    shrimply_tts::InputPurpose::Text
                        | shrimply_tts::InputPurpose::Duration
                        | shrimply_tts::InputPurpose::SpeedFactor
                )
            )
        }
        InputDefinition::Select { options, .. } => !options.iter().any(|option| {
            matches!(
                option.purpose,
                Some(
                    shrimply_tts::InputPurpose::Duration | shrimply_tts::InputPurpose::SpeedFactor
                )
            )
        }),
        _ => true,
    }
}

fn input_widget(definition: &InputDefinition, input: &InputContext) -> gtk::Widget {
    match definition {
        InputDefinition::Text {
            key,
            label,
            multiline,
            max_length,
            ..
        } => text_widget(key, label, *multiline, *max_length, input.clone()),
        InputDefinition::Select {
            key,
            label,
            options,
            ..
        } => {
            let selected = input
                .settings
                .borrow()
                .inputs
                .get(key)
                .and_then(|value| match value {
                    TtsValue::Select { value } => Some(value.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| options[0].value.clone());
            let key = key.clone();
            let input = input.clone();
            let choices = options
                .iter()
                .map(|option| ui::StringChoice {
                    value: option.value.clone(),
                    label: option.label.clone(),
                })
                .collect();
            ui::labeled_string_selector(label, &selected, choices, move |value| {
                if edit_input(&input, &key, TtsInputEdit::Select(value)) {
                    (input.on_commit)();
                    refresh_visibility(&input);
                }
            })
            .widget()
            .clone()
        }
        InputDefinition::Audio { key, label, .. } => audio_widget(key, label, input.clone()),
        InputDefinition::Toggle {
            key,
            label,
            default,
            ..
        } => {
            let active = input
                .settings
                .borrow()
                .inputs
                .get(key)
                .and_then(|value| match value {
                    TtsValue::Toggle { value } => Some(*value),
                    _ => None,
                })
                .unwrap_or(*default);
            let key = key.clone();
            let input = input.clone();
            ui::switch_row(label, None, active, move |active| {
                if edit_input(&input, &key, TtsInputEdit::Toggle(active)) {
                    (input.on_commit)();
                    refresh_visibility(&input);
                }
            })
        }
        InputDefinition::Number { .. } => number_widget(definition, input.clone()),
        InputDefinition::Table {
            key,
            label,
            columns,
            ..
        } => table_widget(key, label, columns, input.clone()),
    }
}

fn edit_input(input: &InputContext, key: &str, edit: TtsInputEdit) -> bool {
    let model = {
        let settings = input.settings.borrow();
        let models = input.models.borrow();
        settings
            .model
            .as_ref()
            .and_then(|id| models.iter().find(|model| &model.id == id))
            .cloned()
    };
    let Some(model) = model else {
        return false;
    };
    let result = shrimply_inspector_core::tts::edit_input(
        &mut input.settings.borrow_mut(),
        &model,
        key,
        edit,
    );
    match result {
        Ok(_) => {
            notify_changed(input);
            true
        }
        Err(error) => {
            tracing::error!(%error, %key, "Could not edit TTS input");
            false
        }
    }
}

fn notify_changed(input: &InputContext) {
    (input.on_changed)(input.settings.borrow().clone());
}

fn refresh_visibility(input: &InputContext) {
    if let Some(refresh) = input.update_visibility.borrow().as_ref() {
        refresh();
    }
}

fn text_widget(
    key: &str,
    label: &str,
    multiline: bool,
    max_length: usize,
    input: InputContext,
) -> gtk::Widget {
    let value = input
        .settings
        .borrow()
        .inputs
        .get(key)
        .and_then(|value| match value {
            TtsValue::Text { value } => Some(value.clone()),
            _ => None,
        })
        .unwrap_or_default();
    if !multiline {
        let key = key.to_string();
        let changed = input.clone();
        let entry = ui::SingleLineTextInput::builder(value)
            .max_length(max_length)
            .on_change(move |value| {
                edit_input(&changed, &key, TtsInputEdit::Text(value));
            })
            .on_commit(move |_| (input.on_commit)())
            .build();
        return ui::control_row(label, &entry);
    }

    let key = key.to_string();
    let changed = input.clone();
    let editor = ui::MultilineTextInput::builder(value)
        .max_length(max_length)
        .on_change(move |value| edit_input(&changed, &key, TtsInputEdit::Text(value)))
        .on_commit(move || (input.on_commit)())
        .build();
    let row = ui::control_row(label, editor.widget());
    row.first_child()
        .expect("text control row has a label")
        .set_valign(gtk::Align::Start);
    row
}

fn audio_widget(key: &str, label: &str, input: InputContext) -> gtk::Widget {
    let selected = input
        .settings
        .borrow()
        .inputs
        .get(key)
        .and_then(|value| match value {
            TtsValue::Audio { value } => Some(value.path().to_path_buf()),
            _ => None,
        });
    let subtitle = selected
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "Choose an audio file".to_string());
    let path_label = gtk::Label::builder()
        .label(&subtitle)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .hexpand(true)
        .xalign(0.0)
        .build();
    let chooser_label = label.to_owned();
    let chooser = gtk::FileDialog::builder()
        .title(tr!(label).as_ref())
        .build();
    let choose = gtk::Button::with_label(tr!("Choose…").as_ref());
    let key = key.to_string();
    let clear = gtk::Button::builder()
        .label(tr!("Clear").as_ref())
        .sensitive(selected.is_some())
        .css_classes(["flat"])
        .build();
    let show = gtk::Button::builder()
        .label(tr!("Show in Folder").as_ref())
        .sensitive(selected.is_some())
        .css_classes(["flat"])
        .build();
    let menu_content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();
    menu_content.append(&clear);
    menu_content.append(&show);
    let popover = gtk::Popover::builder()
        .child(&menu_content)
        .has_arrow(false)
        .build();
    popover.add_css_class("menu");
    let menu = gtk::MenuButton::builder()
        .icon_name("pan-down-symbolic")
        .tooltip_text(tr!("Reference audio actions").as_ref())
        .popover(&popover)
        .build();
    let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    split.add_css_class("linked");
    split.append(&choose);
    split.append(&menu);

    {
        let input = input.clone();
        let key = key.clone();
        let path_label = path_label.clone();
        let show = show.clone();
        clear.connect_clicked(move |button| {
            if !edit_input(&input, &key, TtsInputEdit::Audio(None)) {
                return;
            }
            path_label.set_label(tr!("Choose an audio file").as_ref());
            button.set_sensitive(false);
            show.set_sensitive(false);
            (input.on_commit)();
            if let Some(popover) = button
                .ancestor(gtk::Popover::static_type())
                .and_downcast::<gtk::Popover>()
            {
                popover.popdown();
            }
        });
    }
    {
        let input = input.clone();
        let key = key.clone();
        show.connect_clicked(move |button| {
            let path = input
                .settings
                .borrow()
                .inputs
                .get(&key)
                .and_then(|value| match value {
                    TtsValue::Audio { value } => Some(value.path().to_path_buf()),
                    _ => None,
                });
            let Some(path) = path else {
                return;
            };
            if let Err(error) = shrimply_gtk_components::desktop_open::show_path_in_folder(
                button.upcast_ref(),
                path,
            ) {
                let dialog = gtk::AlertDialog::builder()
                    .message("Could not show reference audio")
                    .detail(error)
                    .buttons(["Close"])
                    .build();
                let parent = button.root().and_downcast::<gtk::Window>();
                dialog.choose(parent.as_ref(), None::<&gio::Cancellable>, |_| {});
            }
        });
    }

    let selected_path = path_label.clone();
    let clear_for_choose = clear.clone();
    let show_for_choose = show.clone();
    choose.connect_clicked(move |button| {
        let input = input.clone();
        let key = key.clone();
        let path_label = selected_path.clone();
        let clear = clear_for_choose.clone();
        let show = show_for_choose.clone();
        shrimply_gtk_components::file_picker::open(
            &chooser_label,
            &chooser,
            button
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok())
                .as_ref(),
            move |result| {
                let Some(path) = result.ok().and_then(|file| file.path()) else {
                    return;
                };
                path_label.set_label(&path.display().to_string());
                if !edit_input(&input, &key, TtsInputEdit::Audio(Some(path))) {
                    return;
                }
                clear.set_sensitive(true);
                show.set_sensitive(true);
                (input.on_commit)();
            },
        );
    });
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    controls.append(&path_label);
    controls.append(&split);
    ui::control_row(label, &controls)
}

fn number_widget(definition: &InputDefinition, input: InputContext) -> gtk::Widget {
    let InputDefinition::Number {
        key,
        label,
        default,
        minimum,
        maximum,
        step,
        ..
    } = definition
    else {
        unreachable!("number_widget requires a number definition");
    };
    let value = input
        .settings
        .borrow()
        .inputs
        .get(key)
        .and_then(|value| match value {
            TtsValue::Number { value } => Some(*value),
            _ => None,
        })
        .unwrap_or(*default);
    let key = key.clone();
    let control = ui::NumberPicker::fraction_builder(value)
        .accepted_range(fraction_as_f64(*minimum), fraction_as_f64(*maximum))
        .drag_step(fraction_as_f64(*step))
        .digits(shrimply_inspector_core::tts::decimal_places(*step))
        .on_change_fraction({
            let input = input.clone();
            move |value| {
                edit_input(&input, &key, TtsInputEdit::Number(value));
            }
        })
        .on_commit(move |_| (input.on_commit)())
        .build();
    ui::control_row(label, &control)
}

fn table_widget(
    key: &str,
    label: &str,
    columns: &[TableColumn],
    input: InputContext,
) -> gtk::Widget {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .valign(gtk::Align::Center)
        .tooltip_text(tr!("Add row").as_ref())
        .build();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header.set_halign(gtk::Align::End);
    header.append(&add);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
    container.append(&ui::control_row(label, &header));
    container.append(&rows);
    let state = TableEditor {
        key: key.to_string(),
        columns: columns.to_vec(),
        rows,
        input,
    };
    rebuild_table(&state);
    {
        let state = state.clone();
        add.connect_clicked(move |_| {
            if !edit_input(&state.input, &state.key, TtsInputEdit::AddTableRow) {
                return;
            }
            (state.input.on_commit)();
            rebuild_table(&state);
        });
    }
    container.upcast()
}

#[derive(Clone)]
struct TableEditor {
    key: String,
    columns: Vec<TableColumn>,
    rows: gtk::Box,
    input: InputContext,
}

fn rebuild_table(state: &TableEditor) {
    while let Some(child) = state.rows.first_child() {
        state.rows.remove(&child);
    }
    let count = state
        .input
        .settings
        .borrow()
        .inputs
        .get(&state.key)
        .and_then(|value| match value {
            TtsValue::Table { rows } => Some(rows.len()),
            _ => None,
        })
        .unwrap_or_default();
    for index in 0..count {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for (column_index, column) in state.columns.iter().enumerate() {
            let value = state
                .input
                .settings
                .borrow()
                .inputs
                .get(&state.key)
                .and_then(|value| match value {
                    TtsValue::Table { rows } => rows.get(index),
                    _ => None,
                })
                .and_then(|row| row.get(&column.key))
                .cloned()
                .unwrap_or_default();
            let changed = state.clone();
            let input = state.input.clone();
            let entry = ui::SingleLineTextInput::builder(value)
                .placeholder(&column.label)
                .max_length(column.max_length)
                .on_change(move |value| {
                    edit_input(
                        &changed.input,
                        &changed.key,
                        TtsInputEdit::TableCell {
                            row: index,
                            column: column_index,
                            value,
                        },
                    );
                })
                .on_commit(move |_| (input.on_commit)())
                .build();
            row.append(&entry);
        }
        let remove = gtk::Button::builder()
            .icon_name("list-remove-symbolic")
            .tooltip_text(tr!("Remove row").as_ref())
            .build();
        {
            let state = state.clone();
            remove.connect_clicked(move |_| {
                if !edit_input(
                    &state.input,
                    &state.key,
                    TtsInputEdit::RemoveTableRow(index),
                ) {
                    return;
                }
                (state.input.on_commit)();
                rebuild_table(&state);
            });
        }
        row.append(&remove);
        state.rows.append(&row);
    }
}

pub fn save_speech(speech: Speech) -> Result<(PathBuf, Time, Fraction), String> {
    shrimply_inspector_core::tts::save_speech(speech).map(|generation| {
        (
            generation.path,
            generation.duration,
            generation.speed_factor,
        )
    })
}
