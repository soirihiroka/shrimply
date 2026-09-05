use shrimply_gtk_components::tr;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use shrimply_gtk_components::ui;
use shrimply_math_core::{
    fraction_as_f64, fraction_denominator, fraction_numerator, fraction_snapped,
};
use shrimply_state::preferences as preferences_store;
use shrimply_video_generation::{
    Fraction, GenerationResult, InputDefinition, MediaAsset, MediaKind, VideoGenerationModel,
    VideoGenerationSettings, VideoGenerationValue, generation_request, is_visible, sync_settings,
};
use uuid::Uuid;

enum GenerationMessage {
    Progress(String),
    Done(Result<(PathBuf, GenerationResult), String>),
}

type VisibilityRefresh = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

#[derive(Clone)]
struct EditorCallbacks {
    on_changed: Rc<dyn Fn(VideoGenerationSettings)>,
    on_commit: Rc<dyn Fn()>,
    on_generated: Rc<dyn Fn(PathBuf, GenerationResult)>,
}

struct CachedEditor {
    widget: gtk::Widget,
    settings: Rc<RefCell<VideoGenerationSettings>>,
    callbacks: Rc<RefCell<EditorCallbacks>>,
    active_generation: Rc<RefCell<Option<shrimply_server_client::CancellationToken>>>,
    server_url: String,
    has_output: bool,
}

thread_local! {
    static EDITOR_CACHE: RefCell<HashMap<Uuid, CachedEditor>> = RefCell::new(HashMap::new());
}

pub fn editor(
    id: Uuid,
    preferences: preferences_store::SharedPreferences,
    value: &VideoGenerationSettings,
    has_output: bool,
    on_changed: impl Fn(VideoGenerationSettings) + 'static,
    on_commit: impl Fn() + 'static,
    on_generated: impl Fn(PathBuf, GenerationResult) + 'static,
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
                && (cached.server_url != server_url
                    || cached.has_output != has_output
                    || *cached.settings.borrow() != *value)
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
    let models = Rc::new(RefCell::new(Vec::<VideoGenerationModel>::new()));
    let catalog_error = Rc::new(RefCell::new(None::<String>));
    let callbacks = Rc::new(RefCell::new(callbacks));
    let configuration = configuration(
        preferences.clone(),
        settings.clone(),
        models.clone(),
        catalog_error.clone(),
        {
            let callbacks = callbacks.clone();
            move |value| (callbacks.borrow().on_changed)(value)
        },
        {
            let callbacks = callbacks.clone();
            move || (callbacks.borrow().on_commit)()
        },
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
        .label(tr!(if has_output { "Regenerate" } else { "Generate" }).as_ref())
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
        let catalog_error = catalog_error.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            if !models.borrow().is_empty() {
                spinner.set_visible(false);
                status.set_label(tr!("Ready").as_ref());
                generate.set_sensitive(true);
                glib::ControlFlow::Break
            } else if let Some(error) = catalog_error.borrow_mut().take() {
                spinner.set_visible(false);
                status.set_label(&error);
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
        let callbacks = callbacks.clone();
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
            let directory =
                shrimply_project::project::project_directory().join("media/video-generation");
            let destination = directory.join(format!("{}.mp4", Uuid::new_v4()));
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                let result = generation_request(&selected, &value).and_then(|request| {
                    shrimply_video_generation::generate(
                        &server_url,
                        &cancellation,
                        &request,
                        &destination,
                        |message| {
                            let _ = sender.send(GenerationMessage::Progress(message.to_string()));
                            !cancellation.is_cancelled()
                        },
                    )
                    .map(|result| (destination, result))
                });
                let _ = sender.send(GenerationMessage::Done(result));
            });
            let status = status.clone();
            let spinner = spinner.clone();
            let generate = button.clone();
            let cancel = cancel.clone();
            let active_generation = active_generation.clone();
            let callbacks = callbacks.clone();
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
                                Ok((path, result)) => {
                                    generate.set_label(tr!("Regenerate").as_ref());
                                    status.set_label(tr!("Generated").as_ref());
                                    (callbacks.borrow().on_generated)(path, result);
                                }
                                Err(_) if cancelled => {
                                    status.set_label(tr!("Cancelled").as_ref())
                                }
                                Err(error) if error.starts_with("Compute server connection failed") => {
                                    tracing::error!(%error, "Video-generation compute connection failed");
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
                has_output,
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
        .expect("cached video-generation editor must be in an inspector box")
        .remove(widget);
}

pub fn configuration(
    preferences: preferences_store::SharedPreferences,
    settings: Rc<RefCell<VideoGenerationSettings>>,
    models: Rc<RefCell<Vec<VideoGenerationModel>>>,
    catalog_error: Rc<RefCell<Option<String>>>,
    on_changed: impl Fn(VideoGenerationSettings) + 'static,
    on_commit: impl Fn() + 'static,
) -> gtk::Widget {
    let on_changed: Rc<dyn Fn(VideoGenerationSettings)> = Rc::new(on_changed);
    let on_commit: Rc<dyn Fn()> = Rc::new(on_commit);
    let fields = Rc::new(RefCell::new(ui::KeyedBox::new(
        gtk::Orientation::Vertical,
        12,
    )));
    let definitions = Rc::new(RefCell::new(Vec::<InputDefinition>::new()));
    let update_visibility = Rc::new(RefCell::new(None::<Rc<dyn Fn()>>));
    let input = InputContext {
        settings: settings.clone(),
        on_changed,
        on_commit,
        update_visibility: update_visibility.clone(),
    };
    let field_state = FieldState {
        input: input.clone(),
        fields: fields.clone(),
        definitions: definitions.clone(),
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
        ui::labeled_string_selector("Model", &selected_model, Vec::new(), move |id| {
            let Some(model) = models.borrow().iter().find(|model| model.id == id).cloned() else {
                return;
            };
            sync_settings(&mut field_state.input.settings.borrow_mut(), &model);
            notify_changed(&field_state.input);
            (field_state.input.on_commit)();
            reconcile_fields(&field_state, &model);
        })
    };
    model_selector.set_sensitive(false);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(model_selector.widget());
    content.append(fields.borrow().widget());
    let state = ConfigurationState {
        models: models.clone(),
        catalog_error,
        model_selector,
        fields: field_state,
    };
    let server_url = preferences_store::snapshot(&preferences).compute_server_url;
    refresh_models(server_url, state, Rc::new(Cell::new(false)));
    content.upcast()
}

#[derive(Clone)]
struct ConfigurationState {
    models: Rc<RefCell<Vec<VideoGenerationModel>>>,
    catalog_error: Rc<RefCell<Option<String>>>,
    model_selector: ui::StringSelector,
    fields: FieldState,
}

#[derive(Clone)]
struct FieldState {
    input: InputContext,
    fields: Rc<RefCell<ui::KeyedBox<String, InputDefinition>>>,
    definitions: Rc<RefCell<Vec<InputDefinition>>>,
}

#[derive(Clone)]
struct InputContext {
    settings: Rc<RefCell<VideoGenerationSettings>>,
    on_changed: Rc<dyn Fn(VideoGenerationSettings)>,
    on_commit: Rc<dyn Fn()>,
    update_visibility: VisibilityRefresh,
}

fn apply_models(state: &ConfigurationState, available: Vec<VideoGenerationModel>) {
    if available.is_empty() || *state.models.borrow() == available {
        return;
    }
    let saved = state.fields.input.settings.borrow().model.clone();
    let selected = saved
        .as_ref()
        .and_then(|id| available.iter().position(|model| &model.id == id))
        .unwrap_or(0);
    let model = available[selected].clone();
    let choices = available
        .iter()
        .map(|model| ui::StringChoice {
            value: model.id.clone(),
            label: model.label.clone(),
        })
        .collect();
    *state.models.borrow_mut() = available;
    state.model_selector.set_choices(&model.id, choices);
    state.model_selector.set_sensitive(true);
    let original = state.fields.input.settings.borrow().clone();
    sync_settings(&mut state.fields.input.settings.borrow_mut(), &model);
    if *state.fields.input.settings.borrow() != original {
        notify_changed(&state.fields.input);
        (state.fields.input.on_commit)();
    }
    reconcile_fields(&state.fields, &model);
}

fn refresh_models(server_url: String, state: ConfigurationState, refreshing: Rc<Cell<bool>>) {
    if refreshing.replace(true) {
        return;
    }
    let (sender, receiver) = async_channel::bounded(1);
    let request_url = server_url;
    thread::spawn(move || {
        let _ = sender.send_blocking(available_models(&request_url));
    });
    glib::spawn_future_local(async move {
        refreshing.set(false);
        match receiver.recv().await {
            Ok(Ok(available)) if !available.is_empty() => apply_models(&state, available),
            Ok(Ok(_)) => {
                *state.catalog_error.borrow_mut() =
                    Some("Server has no available video-generation models".to_string());
            }
            Ok(Err(error)) => *state.catalog_error.borrow_mut() = Some(error),
            Err(error) => *state.catalog_error.borrow_mut() = Some(error.to_string()),
        }
    });
}

fn available_models(server_url: &str) -> Result<Vec<VideoGenerationModel>, String> {
    let advertised = shrimply_server_client::server_status(server_url)?
        .capabilities
        .into_iter()
        .filter_map(|capability| {
            capability
                .strip_prefix("video-generation:")
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    if advertised.is_empty() {
        return Err("Server does not advertise video generation".to_string());
    }
    shrimply_video_generation::models(server_url).map(|mut models| {
        models.retain(|model| advertised.contains(&model.id));
        models
    })
}

fn reconcile_fields(state: &FieldState, model: &VideoGenerationModel) {
    *state.definitions.borrow_mut() = model.inputs.clone();
    let input = state.input.clone();
    state.fields.borrow_mut().reconcile(
        model
            .inputs
            .iter()
            .cloned()
            .map(|definition| (definition.key().to_string(), definition)),
        move |_, definition| input_widget(definition, &input),
    );
    refresh_visibility(&state.input);
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
                    VideoGenerationValue::Select { value } => Some(value.clone()),
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
                set_input(&input, &key, VideoGenerationValue::Select { value });
                (input.on_commit)();
                refresh_visibility(&input);
            })
            .widget()
            .clone()
        }
        InputDefinition::Number { .. } => number_widget(definition, input.clone()),
        InputDefinition::Media {
            key,
            label,
            accepted,
            maximum_items,
            ordered,
            ..
        } => media_widget(
            key,
            label,
            accepted,
            *maximum_items,
            *ordered,
            input.clone(),
        ),
    }
}

fn set_input(input: &InputContext, key: &str, value: VideoGenerationValue) {
    input
        .settings
        .borrow_mut()
        .inputs
        .insert(key.to_string(), value);
    notify_changed(input);
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
            VideoGenerationValue::Text { value } => Some(value.clone()),
            _ => None,
        })
        .unwrap_or_default();
    if !multiline {
        let key = key.to_string();
        let changed = input.clone();
        let entry = ui::SingleLineTextInput::builder(value)
            .max_length(max_length)
            .on_change(move |value| {
                set_input(&changed, &key, VideoGenerationValue::Text { value });
            })
            .on_commit(move |_| (input.on_commit)())
            .build();
        return ui::control_row(label, &entry);
    }
    let key = key.to_string();
    let changed = input.clone();
    let editor = ui::MultilineTextInput::builder(value)
        .max_length(max_length)
        .on_change(move |value| {
            set_input(&changed, &key, VideoGenerationValue::Text { value });
            true
        })
        .on_commit(move || (input.on_commit)())
        .build();
    ui::control_row(label, editor.widget())
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
    let value = fraction_as_f64(
        input
            .settings
            .borrow()
            .inputs
            .get(key)
            .and_then(|value| match value {
                VideoGenerationValue::Number { value } => Some(*value),
                _ => None,
            })
            .unwrap_or(*default),
    );
    let key = key.clone();
    let minimum = *minimum;
    let step = *step;
    let control = ui::NumberPicker::builder(value)
        .accepted_range(fraction_as_f64(minimum), fraction_as_f64(*maximum))
        .drag_step(fraction_as_f64(step))
        .digits(decimal_places(step) as usize)
        .on_change({
            let input = input.clone();
            move |value| {
                set_input(
                    &input,
                    &key,
                    VideoGenerationValue::Number {
                        value: fraction_snapped(value, minimum, step),
                    },
                );
            }
        })
        .on_commit(move |_| (input.on_commit)())
        .build();
    ui::control_row(label, &control)
}

fn decimal_places(step: Fraction) -> u32 {
    let mut scaled = fraction_numerator(step).unsigned_abs();
    let denominator = fraction_denominator(step).unsigned_abs();
    for digits in 0..=6 {
        if scaled.is_multiple_of(denominator) {
            return digits;
        }
        scaled = scaled.saturating_mul(10);
    }
    6
}

fn media_widget(
    key: &str,
    label: &str,
    accepted: &[MediaKind],
    maximum_items: usize,
    ordered: bool,
    input: InputContext,
) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let add = gtk::Button::builder()
        .label(
            tr!(if maximum_items == 1 {
                "Choose…"
            } else {
                "Add…"
            })
            .as_ref(),
        )
        .halign(gtk::Align::End)
        .build();
    content.append(&rows);
    content.append(&add);
    let key = key.to_string();
    let accepted = accepted.to_vec();
    let rebuild = Rc::new(RefCell::<Option<Rc<dyn Fn()>>>::default());
    *rebuild.borrow_mut() = Some({
        let rows = rows.clone();
        let add = add.clone();
        let input = input.clone();
        let key = key.clone();
        let rebuild = rebuild.clone();
        Rc::new(move || {
            while let Some(child) = rows.first_child() {
                rows.remove(&child);
            }
            let items = input
                .settings
                .borrow()
                .inputs
                .get(&key)
                .and_then(|value| match value {
                    VideoGenerationValue::Media { items } => Some(items.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            add.set_sensitive(items.len() < maximum_items);
            for (index, item) in items.iter().enumerate() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                let path = gtk::Label::builder()
                    .label(item.value.path().to_string_lossy())
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .xalign(0.0)
                    .hexpand(true)
                    .build();
                row.append(&path);
                if ordered {
                    for (icon, offset, sensitive) in [
                        ("go-up-symbolic", -1_isize, index > 0),
                        ("go-down-symbolic", 1_isize, index + 1 < items.len()),
                    ] {
                        let button = gtk::Button::builder()
                            .icon_name(icon)
                            .sensitive(sensitive)
                            .css_classes(["flat"])
                            .build();
                        let input = input.clone();
                        let key = key.clone();
                        let rebuild = rebuild.clone();
                        button.connect_clicked(move |_| {
                            let mut settings = input.settings.borrow_mut();
                            let Some(VideoGenerationValue::Media { items }) =
                                settings.inputs.get_mut(&key)
                            else {
                                return;
                            };
                            let target = index.saturating_add_signed(offset);
                            if target < items.len() {
                                items.swap(index, target);
                            }
                            drop(settings);
                            notify_changed(&input);
                            (input.on_commit)();
                            if let Some(rebuild) = rebuild.borrow().as_ref() {
                                rebuild();
                            }
                        });
                        row.append(&button);
                    }
                }
                let remove = gtk::Button::builder()
                    .icon_name("list-remove-symbolic")
                    .css_classes(["flat"])
                    .build();
                let input = input.clone();
                let key = key.clone();
                let rebuild = rebuild.clone();
                remove.connect_clicked(move |_| {
                    let mut settings = input.settings.borrow_mut();
                    let Some(VideoGenerationValue::Media { items }) = settings.inputs.get_mut(&key)
                    else {
                        return;
                    };
                    if index < items.len() {
                        items.remove(index);
                    }
                    drop(settings);
                    notify_changed(&input);
                    (input.on_commit)();
                    if let Some(rebuild) = rebuild.borrow().as_ref() {
                        rebuild();
                    }
                });
                row.append(&remove);
                rows.append(&row);
            }
        })
    });
    if let Some(rebuild) = rebuild.borrow().as_ref() {
        rebuild();
    }
    {
        let chooser = gtk::FileDialog::builder()
            .title(tr!(label).as_ref())
            .build();
        let label = label.to_string();
        let input = input.clone();
        let key = key.clone();
        let rebuild = rebuild.clone();
        add.connect_clicked(move |button| {
            let input = input.clone();
            let key = key.clone();
            let accepted = accepted.clone();
            let rebuild = rebuild.clone();
            shrimply_gtk_components::file_picker::open(
                &label,
                &chooser,
                button
                    .root()
                    .and_then(|root| root.downcast::<gtk::Window>().ok())
                    .as_ref(),
                move |result| {
                    let Some(path) = result.ok().and_then(|file| file.path()) else {
                        return;
                    };
                    let Some(kind) = media_kind(&path).filter(|kind| accepted.contains(kind))
                    else {
                        return;
                    };
                    let mut settings = input.settings.borrow_mut();
                    let Some(VideoGenerationValue::Media { items }) = settings.inputs.get_mut(&key)
                    else {
                        return;
                    };
                    let item = MediaAsset {
                        kind,
                        value: path.into(),
                    };
                    if maximum_items == 1 {
                        items.clear();
                    }
                    if items.len() < maximum_items {
                        items.push(item);
                    }
                    drop(settings);
                    notify_changed(&input);
                    (input.on_commit)();
                    if let Some(rebuild) = rebuild.borrow().as_ref() {
                        rebuild();
                    }
                },
            );
        });
    }
    ui::control_row(label, &content)
}

fn media_kind(path: &Path) -> Option<MediaKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" => Some(MediaKind::Image),
        "mp4" | "mov" | "mkv" | "webm" => Some(MediaKind::Video),
        "wav" | "mp3" | "flac" | "ogg" | "opus" | "m4a" | "aac" => Some(MediaKind::Audio),
        _ => None,
    }
}
