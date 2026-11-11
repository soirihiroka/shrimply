use super::store;
use adw::prelude::*;
use gtk::{gio, glib};
use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::I18nAlertDialogExt;
use shrimply_components_gtk::ui::I18nWidgetExt;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::thread;
use store::ServerStatus;

const DEVICE_LABEL_MAX_WIDTH_CHARS: i32 = 60;
const DEVICE_LIST_SPACING: i32 = 12;

#[derive(Clone)]
struct Rows {
    spinner: adw::Spinner,
    selected_url: Rc<RefCell<String>>,
    version: adw::ActionRow,
    protocol: adw::ActionRow,
    torch: adw::ActionRow,
    cuda: adw::ActionRow,
    device: adw::ComboRow,
    jobs: adw::ActionRow,
    reservations: adw::ActionRow,
    workers: adw::ActionRow,
    device_ids: Rc<RefCell<Vec<String>>>,
    current_device: Rc<Cell<Option<u32>>>,
    updating_device: Rc<Cell<bool>>,
    features: adw::ActionRow,
}

#[derive(Clone)]
struct ServerSelection {
    preferences: store::SharedPreferences,
    rows: Rows,
    revision: Rc<Cell<u64>>,
}

#[derive(Clone)]
struct ServerList {
    group: glib::WeakRef<adw::PreferencesGroup>,
    add: glib::WeakRef<adw::ButtonRow>,
    urls: Rc<RefCell<Vec<String>>>,
    rows: Rc<RefCell<Vec<adw::ActionRow>>>,
    generation: Rc<Cell<u64>>,
    selection: ServerSelection,
}

#[derive(Clone)]
struct ServerRow {
    row: adw::ActionRow,
    radio: gtk::CheckButton,
    spinner: gtk::Spinner,
    warning: gtk::Image,
}

pub fn page(
    preferences: store::SharedPreferences,
    blender_group: &adw::PreferencesGroup,
) -> adw::PreferencesPage {
    let preference_snapshot = store::snapshot(&preferences);
    let spinner = adw::Spinner::new();
    spinner.set_size_request(18, 18);
    spinner.set_visible(false);

    let device_factory = gtk::SignalListItemFactory::new();
    let device = adw::ComboRow::builder()
        .title(tr!("Device").as_ref())
        .model(&gtk::StringList::new(&["—"]))
        .factory(&device_factory)
        .use_subtitle(true)
        .build();
    device.set_sensitive(false);

    let factory_handlers = Rc::new(RefCell::new(HashMap::new()));
    device_factory.connect_setup(|_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("device selector factory received a non-list item");
        let label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .max_width_chars(DEVICE_LABEL_MAX_WIDTH_CHARS)
            .build();
        let selected = gtk::Image::from_icon_name("object-select-symbolic");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, DEVICE_LIST_SPACING);
        row.append(&label);
        row.append(&selected);
        item.set_child(Some(&row));
    });
    let factory_device = device.downgrade();
    let bind_handlers = factory_handlers.clone();
    device_factory.connect_bind(move |_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("device selector factory received a non-list item");
        let row = item
            .child()
            .and_downcast::<gtk::Box>()
            .expect("device selector list item has no row");
        let label = row
            .first_child()
            .and_downcast::<gtk::Label>()
            .expect("device selector list item has no label");
        let selected = row
            .last_child()
            .and_downcast::<gtk::Image>()
            .expect("device selector list item has no selection icon");
        let value = item
            .item()
            .and_downcast::<gtk::StringObject>()
            .expect("device selector received a non-string item");
        label.set_label(&value.string());
        label.set_tooltip_text(Some(&value.string()));

        let device = factory_device
            .upgrade()
            .expect("device selector was destroyed before its factory");
        update_device_selection(&device, item, &selected);
        update_device_selection_visibility(&device, &row, &selected);

        let weak_item = item.downgrade();
        let weak_selected = selected.downgrade();
        let selected_handler = device.connect_selected_item_notify(move |device| {
            if let (Some(item), Some(selected)) = (weak_item.upgrade(), weak_selected.upgrade()) {
                update_device_selection(device, &item, &selected);
            }
        });
        let weak_device = device.downgrade();
        let weak_selected = selected.downgrade();
        let root_handler = row.connect_root_notify(move |row| {
            if let (Some(device), Some(selected)) = (weak_device.upgrade(), weak_selected.upgrade())
            {
                update_device_selection_visibility(&device, row, &selected);
            }
        });
        bind_handlers
            .borrow_mut()
            .insert(item.as_ptr() as usize, (selected_handler, root_handler));
    });
    let factory_device = device.downgrade();
    device_factory.connect_unbind(move |_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("device selector factory received a non-list item");
        let Some((selected_handler, root_handler)) = factory_handlers
            .borrow_mut()
            .remove(&(item.as_ptr() as usize))
        else {
            return;
        };
        if let Some(device) = factory_device.upgrade() {
            device.disconnect(selected_handler);
        }
        item.child()
            .and_downcast::<gtk::Box>()
            .expect("device selector list item has no row")
            .disconnect(root_handler);
    });

    let version = row("Version");
    version.add_suffix(&spinner);
    let rows = Rows {
        spinner,
        selected_url: Rc::new(RefCell::new(preference_snapshot.compute_server_url.clone())),
        version,
        protocol: row("Protocol"),
        torch: row("Torch"),
        cuda: row("CUDA"),
        device,
        jobs: row("Jobs"),
        reservations: row("Reserved memory"),
        workers: row("Workers"),
        device_ids: Rc::new(RefCell::new(Vec::new())),
        current_device: Rc::new(Cell::new(None)),
        updating_device: Rc::new(Cell::new(false)),
        features: row("Available"),
    };

    let endpoint_group = adw::PreferencesGroup::builder()
        .title(tr!("Inference Servers").as_ref())
        .build();
    let add_server = adw::ButtonRow::builder()
        .title(tr!("Add Server").as_ref())
        .start_icon_name("list-add-symbolic")
        .build();
    endpoint_group.add(&add_server);

    let server_group = adw::PreferencesGroup::new();
    server_group.set_title(tr!("Selected Server").as_ref());
    server_group.add(&rows.version);
    server_group.add(&rows.protocol);

    let compute_group = adw::PreferencesGroup::new();
    compute_group.set_title(tr!("Compute").as_ref());
    compute_group.add(&rows.torch);
    compute_group.add(&rows.cuda);
    compute_group.add(&rows.device);
    compute_group.add(&rows.jobs);
    compute_group.add(&rows.reservations);
    compute_group.add(&rows.workers);

    let features_group = adw::PreferencesGroup::new();
    features_group.set_title(tr!("Features").as_ref());
    features_group.add(&rows.features);

    let page = adw::PreferencesPage::builder()
        .title(tr!("External").as_ref())
        .icon_name("application-x-executable-symbolic")
        .name("external")
        .build();
    let cli_group = adw::PreferencesGroup::new();
    cli_group.add(&crate::cli::row());
    page.add(&cli_group);
    page.add(blender_group);
    page.add(&endpoint_group);
    page.add(&server_group);
    page.add(&compute_group);
    page.add(&features_group);

    let revision = Rc::new(Cell::new(0_u64));
    let server_list = ServerList {
        group: endpoint_group.downgrade(),
        add: add_server.downgrade(),
        urls: Rc::new(RefCell::new(
            preference_snapshot.compute_server_urls.clone(),
        )),
        rows: Rc::new(RefCell::new(Vec::new())),
        generation: Rc::new(Cell::new(0)),
        selection: ServerSelection {
            preferences: preferences.clone(),
            rows: rows.clone(),
            revision: revision.clone(),
        },
    };
    let add_list = server_list.clone();
    add_server.connect_activated(move |button| {
        show_server_editor(button, "Add Server", "Add Server", "", {
            let servers = add_list.clone();
            move |url| add_server_url(&servers, url)
        });
    });
    rebuild_server_rows(&server_list);
    let initial_url = rows.selected_url.borrow().clone();
    select_server(&server_list.selection, &initial_url);

    let selected_rows = rows.clone();
    let selected_revision = revision.clone();
    rows.device.connect_selected_notify(move |device| {
        if selected_rows.updating_device.get() {
            return;
        }
        let Some(device_id) = selected_rows
            .device_ids
            .borrow()
            .get(device.selected() as usize)
            .cloned()
        else {
            return;
        };
        if selected_rows.current_device.get() == Some(device.selected()) {
            return;
        }
        selected_rows.device.remove_css_class("error");
        selected_rows.device.set_tooltip_text(None);
        selected_rows.device.set_sensitive(false);
        selected_rows.spinner.set_visible(true);
        let url = selected_rows.selected_url.borrow().clone();
        let current_revision = selected_revision.get();
        let (sender, receiver) = async_channel::bounded(1);
        thread::spawn(move || {
            let _ = sender.send_blocking(store::select_compute_device(&url, &device_id));
        });
        let rows = selected_rows.clone();
        let revision = selected_revision.clone();
        glib::spawn_future_local(async move {
            let result = receiver.recv().await;
            if revision.get() != current_revision {
                return;
            }
            match result {
                Ok(Ok(status)) => show_status(&rows, &status),
                Ok(Err(error)) => show_device_error(&rows, &error),
                Err(_) => show_device_error(&rows, "Device selection stopped unexpectedly"),
            }
            rows.spinner.set_visible(false);
        });
    });

    page
}

fn rebuild_server_rows(servers: &ServerList) {
    let (Some(group), Some(add)) = (servers.group.upgrade(), servers.add.upgrade()) else {
        return;
    };
    servers
        .generation
        .set(servers.generation.get().wrapping_add(1));
    group.remove(&add);
    for row in servers.rows.borrow_mut().drain(..) {
        group.remove(&row);
    }

    let radio_group = gtk::CheckButton::new();
    let urls = servers.urls.borrow().clone();
    let can_delete = urls.len() > 1;
    for url in urls {
        let widgets = create_server_row(&url);
        widgets.radio.set_group(Some(&radio_group));
        widgets
            .radio
            .set_active(servers.selection.rows.selected_url.borrow().as_str() == url);

        let selected_radio = widgets.radio.clone();
        let activation_group = radio_group.clone();
        widgets.row.connect_activated(move |_| {
            let _ = &activation_group;
            selected_radio.set_active(true);
        });
        let selection = servers.selection.clone();
        let selected_url = url.clone();
        let toggle_group = radio_group.clone();
        widgets.radio.connect_toggled(move |radio| {
            let _ = &toggle_group;
            if radio.is_active() {
                select_server(&selection, &selected_url);
            }
        });

        let edit_parent = widgets.row.clone();
        let edit_servers = servers.clone();
        let old_url = url.clone();
        let delete_servers = servers.clone();
        let deleted_url = url.clone();
        let menu = create_server_menu_button(
            can_delete,
            move || {
                let servers = edit_servers.clone();
                let old_url = old_url.clone();
                let edited_url = old_url.clone();
                show_server_editor(&edit_parent, "Edit Server", "Save", &old_url, move |url| {
                    edit_server_url(&servers, &edited_url, url);
                });
            },
            move || delete_server_url(&delete_servers, &deleted_url),
        );
        widgets.row.add_suffix(&menu);
        group.add(&widgets.row);
        servers.rows.borrow_mut().push(widgets.row.clone());
        check_server_summary(servers, url, widgets);
    }
    group.add(&add);
}

fn create_server_row(url: &str) -> ServerRow {
    let row = adw::ActionRow::builder()
        .title(url)
        .subtitle(tr!("Checking…").as_ref())
        .title_lines(1)
        .subtitle_lines(1)
        .build();
    let radio = gtk::CheckButton::new();
    radio.set_valign(gtk::Align::Center);
    radio.set_tooltip_i18n("Use this server");
    row.add_prefix(&radio);
    row.set_activatable_widget(Some(&radio));

    let spinner = gtk::Spinner::new();
    spinner.set_size_request(16, 16);
    spinner.set_tooltip_i18n("Checking server URL");
    spinner.set_valign(gtk::Align::Center);
    spinner.start();
    row.add_suffix(&spinner);

    let warning = gtk::Image::from_icon_name("dialog-warning-symbolic");
    warning.add_css_class("warning");
    warning.set_tooltip_i18n("Could not reach a Shrimply server at this URL");
    warning.set_valign(gtk::Align::Center);
    warning.set_visible(false);
    row.add_suffix(&warning);
    ServerRow {
        row,
        radio,
        spinner,
        warning,
    }
}

fn create_server_menu_button(
    can_delete: bool,
    on_edit: impl Fn() + 'static,
    on_delete: impl Fn() + 'static,
) -> gtk::MenuButton {
    let popover = gtk::Popover::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let edit = gtk::Button::with_label(tr!("Edit Server").as_ref());
    edit.set_has_frame(false);
    edit.add_css_class("flat");
    let edit_popover = popover.clone();
    edit.connect_clicked(move |_| {
        edit_popover.popdown();
        on_edit();
    });
    content.append(&edit);

    let delete = gtk::Button::with_label(tr!("Delete Server").as_ref());
    delete.set_has_frame(false);
    delete.add_css_class("flat");
    delete.add_css_class("destructive-action");
    delete.set_sensitive(can_delete);
    let delete_popover = popover.clone();
    delete.connect_clicked(move |_| {
        delete_popover.popdown();
        on_delete();
    });
    content.append(&delete);
    popover.set_child(Some(&content));

    let menu = gtk::MenuButton::new();
    menu.set_icon_name("view-more-symbolic");
    menu.set_has_frame(false);
    menu.add_css_class("flat");
    menu.set_tooltip_i18n("Server menu");
    menu.set_valign(gtk::Align::Center);
    menu.set_popover(Some(&popover));
    menu
}

fn show_server_editor(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    confirm_label: &str,
    initial_url: &str,
    on_confirm: impl Fn(String) + 'static,
) {
    let url = adw::EntryRow::builder()
        .title(tr!("Server URL").as_ref())
        .text(initial_url)
        .input_purpose(gtk::InputPurpose::Url)
        .activates_default(true)
        .build();
    let warning = gtk::Image::from_icon_name("dialog-warning-symbolic");
    warning.add_css_class("warning");
    warning.set_visible(false);
    url.add_suffix(&warning);
    let spinner = gtk::Spinner::new();
    spinner.set_size_request(16, 16);
    spinner.set_tooltip_i18n("Checking server URL");
    spinner.set_visible(false);
    url.add_suffix(&spinner);
    let status = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .margin_start(14)
        .margin_end(14)
        .margin_bottom(8)
        .build();
    status.add_css_class("dim-label");
    let group = adw::PreferencesGroup::new();
    group.add(&url);
    group.add(&status);

    let dialog = adw::AlertDialog::builder()
        .heading(title)
        .extra_child(&group)
        .build();
    dialog.add_responses_i18n(&[("cancel", "Cancel"), ("save", confirm_label)]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("save"));
    dialog.set_response_enabled("save", false);

    let revision = Rc::new(Cell::new(0_u64));
    let selected_url = Rc::new(RefCell::new(None));
    let validate = {
        let dialog = dialog.clone();
        let url = url.clone();
        let warning = warning.clone();
        let spinner = spinner.clone();
        let status = status.clone();
        let revision = revision.clone();
        let selected_url = selected_url.clone();
        Rc::new(move || {
            let current_revision = revision.get().wrapping_add(1);
            revision.set(current_revision);
            spinner.stop();
            spinner.set_visible(false);
            warning.set_visible(false);
            url.remove_css_class("warning");
            status.remove_css_class("warning");
            status.add_css_class("dim-label");
            selected_url.replace(None);
            dialog.set_response_enabled("save", false);

            let value = url.text();
            if value.trim().is_empty() {
                status.set_text(tr!("").as_ref());
                return;
            }
            let normalized = match store::normalize_server_url(&value) {
                Ok(url) => url,
                Err(error) => {
                    status.set_text(error);
                    status.remove_css_class("dim-label");
                    status.add_css_class("warning");
                    warning.set_visible(true);
                    warning.set_tooltip_text(Some(error));
                    url.add_css_class("warning");
                    return;
                }
            };
            selected_url.replace(Some(normalized.clone()));
            dialog.set_response_enabled("save", true);
            status.set_text(tr!("Checking…").as_ref());
            spinner.set_visible(true);
            spinner.start();
            check_editor_server(
                normalized,
                url.clone(),
                status.clone(),
                warning.clone(),
                spinner.clone(),
                revision.clone(),
                current_revision,
            );
        })
    };
    let changed_validate = validate.clone();
    url.connect_changed(move |_| changed_validate());
    validate();

    dialog.choose(
        Some(parent.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |answer| {
            if answer == "save"
                && let Some(url) = selected_url.borrow().clone()
            {
                on_confirm(url);
            }
        },
    );
}

fn check_editor_server(
    server_url: String,
    url: adw::EntryRow,
    status: gtk::Label,
    warning: gtk::Image,
    spinner: gtk::Spinner,
    revision: Rc<Cell<u64>>,
    current_revision: u64,
) {
    let (sender, receiver) = async_channel::bounded(1);
    thread::spawn(move || {
        let _ = sender.send_blocking(store::compute_server_status(&server_url));
    });
    glib::spawn_future_local(async move {
        let result = receiver.recv().await;
        if revision.get() != current_revision {
            return;
        }
        spinner.stop();
        spinner.set_visible(false);
        match result {
            Ok(Ok(server)) => status.set_text(&server_summary(&server)),
            Ok(Err(error)) => {
                status.set_text(tr!("Could not reach a Shrimply server at this URL").as_ref());
                status.remove_css_class("dim-label");
                status.add_css_class("warning");
                warning.set_visible(true);
                warning.set_tooltip_text(Some(&error));
                url.add_css_class("warning");
            }
            Err(_) => {
                let error = "Server check stopped unexpectedly";
                status.set_text(error);
                status.remove_css_class("dim-label");
                status.add_css_class("warning");
                warning.set_visible(true);
                warning.set_tooltip_text(Some(error));
                url.add_css_class("warning");
            }
        }
    });
}

fn add_server_url(servers: &ServerList, url: String) {
    store::add_compute_server(&servers.selection.preferences, &url)
        .expect("GTK only submits normalized compute server URLs");
    let snapshot = store::snapshot(&servers.selection.preferences);
    servers.urls.replace(snapshot.compute_server_urls);
    select_server(&servers.selection, &snapshot.compute_server_url);
    rebuild_server_rows(servers);
}

fn edit_server_url(servers: &ServerList, old_url: &str, new_url: String) {
    store::edit_compute_server(&servers.selection.preferences, old_url, &new_url)
        .expect("GTK only edits an existing server to a normalized URL");
    let snapshot = store::snapshot(&servers.selection.preferences);
    servers.urls.replace(snapshot.compute_server_urls);
    select_server(&servers.selection, &snapshot.compute_server_url);
    rebuild_server_rows(servers);
}

fn delete_server_url(servers: &ServerList, deleted_url: &str) {
    store::remove_compute_server(&servers.selection.preferences, deleted_url);
    let snapshot = store::snapshot(&servers.selection.preferences);
    servers.urls.replace(snapshot.compute_server_urls);
    select_server(&servers.selection, &snapshot.compute_server_url);
    rebuild_server_rows(servers);
}

fn select_server(selection: &ServerSelection, url: &str) {
    assert!(
        store::select_compute_server(&selection.preferences, url),
        "GTK selected a compute server outside the shared preference list"
    );
    selection.rows.selected_url.replace(url.to_string());
    let current_revision = selection.revision.get().wrapping_add(1);
    selection.revision.set(current_revision);
    clear(&selection.rows);
    if url.is_empty() {
        selection.rows.spinner.set_visible(false);
        return;
    }
    check(
        url.to_string(),
        selection.rows.clone(),
        selection.revision.clone(),
        current_revision,
    );
}

fn check_server_summary(servers: &ServerList, url: String, widgets: ServerRow) {
    let generation = servers.generation.get();
    let current_generation = servers.generation.clone();
    let (sender, receiver) = async_channel::bounded(1);
    thread::spawn(move || {
        let _ = sender.send_blocking(store::compute_server_status(&url));
    });
    glib::spawn_future_local(async move {
        let result = receiver.recv().await;
        if current_generation.get() != generation {
            return;
        }
        widgets.spinner.stop();
        widgets.spinner.set_visible(false);
        match result {
            Ok(Ok(status)) => {
                widgets.warning.set_visible(false);
                widgets.row.set_subtitle(&server_summary(&status));
                widgets.row.set_tooltip_text(None);
            }
            Ok(Err(error)) => {
                widgets
                    .row
                    .set_subtitle(tr!("Could not reach a Shrimply server at this URL").as_ref());
                widgets.warning.set_visible(true);
                widgets.warning.set_tooltip_text(Some(&error));
            }
            Err(_) => {
                let error = "Connection check stopped unexpectedly";
                widgets.row.set_subtitle(error);
                widgets.warning.set_visible(true);
                widgets.warning.set_tooltip_text(Some(error));
            }
        }
    });
}

fn update_device_selection(device: &adw::ComboRow, item: &gtk::ListItem, selected: &gtk::Image) {
    selected.set_opacity(if device.selected_item() == item.item() {
        1.0
    } else {
        0.0
    });
}

fn update_device_selection_visibility(
    device: &adw::ComboRow,
    row: &gtk::Box,
    selected: &gtk::Image,
) {
    let in_device_popover = row
        .ancestor(gtk::Popover::static_type())
        .and_then(|popover| popover.ancestor(adw::ComboRow::static_type()))
        .and_downcast::<adw::ComboRow>()
        .is_some_and(|combo| combo == *device);
    selected.set_visible(in_device_popover);
}

fn row(title: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(tr!(title).as_ref())
        .subtitle(tr!("—").as_ref())
        .build()
}

fn server_summary(status: &ServerStatus) -> String {
    store::present_compute_server(status).summary
}

fn check(url: String, rows: Rows, revision: Rc<Cell<u64>>, current_revision: u64) {
    rows.spinner.set_visible(true);
    let (sender, receiver) = async_channel::bounded(1);
    thread::spawn(move || {
        let _ = sender.send_blocking(store::compute_server_status(&url));
    });
    glib::spawn_future_local(async move {
        let result = receiver.recv().await;
        if revision.get() != current_revision {
            return;
        }
        match result {
            Ok(Ok(status)) => show_status(&rows, &status),
            Ok(Err(error)) => show_error(&rows, &error),
            Err(_) => show_error(&rows, "Connection check stopped unexpectedly"),
        }
        rows.spinner.set_visible(false);
    });
}

fn show_status(rows: &Rows, status: &ServerStatus) {
    let presentation = store::present_compute_server(status);
    rows.version.remove_css_class("error");
    rows.version
        .set_tooltip_text(presentation.version_detail.as_deref());
    rows.version.set_subtitle(&presentation.version);
    rows.protocol.set_subtitle(&presentation.protocol);
    rows.torch.set_subtitle(&presentation.torch);
    rows.cuda.set_subtitle(&presentation.cuda);
    let selected_device = presentation.selected_device.map(|position| position as u32);
    rows.updating_device.set(true);
    rows.device_ids.replace(
        presentation
            .devices
            .iter()
            .map(|device| device.id.clone())
            .collect(),
    );
    let device_label_refs = presentation
        .devices
        .iter()
        .map(|device| device.label.as_str())
        .collect::<Vec<_>>();
    rows.device
        .set_model(Some(&gtk::StringList::new(&device_label_refs)));
    rows.device.set_selected(selected_device.unwrap_or(0));
    rows.device.set_sensitive(selected_device.is_some());
    rows.device.remove_css_class("error");
    rows.device.set_tooltip_i18n_opt(
        (selected_device.is_none() && !presentation.devices.is_empty())
            .then_some("Server does not support device selection"),
    );
    rows.current_device.set(selected_device);
    rows.updating_device.set(false);
    rows.jobs.set_subtitle(&presentation.jobs);
    rows.reservations.set_subtitle(&presentation.reservations);
    rows.workers.set_subtitle(&presentation.workers);
    rows.workers.set_tooltip_text(
        (presentation.workers != "None").then_some(presentation.workers.as_str()),
    );
    rows.features.set_subtitle(&presentation.features);
}

fn show_device_error(rows: &Rows, error: &str) {
    rows.device.add_css_class("error");
    rows.device.set_tooltip_text(Some(error));
    rows.updating_device.set(true);
    if let Some(current) = rows.current_device.get() {
        rows.device.set_selected(current);
    }
    rows.updating_device.set(false);
    rows.device
        .set_sensitive(rows.current_device.get().is_some());
}

fn show_error(rows: &Rows, error: &str) {
    clear(rows);
    rows.version.add_css_class("error");
    rows.version.set_subtitle(tr!("Unavailable").as_ref());
    rows.version.set_tooltip_text(Some(error));
}

fn clear(rows: &Rows) {
    rows.version.remove_css_class("error");
    rows.version.set_tooltip_text(None);
    rows.updating_device.set(true);
    rows.device_ids.borrow_mut().clear();
    rows.current_device.set(None);
    rows.device.set_model(Some(&gtk::StringList::new(&["—"])));
    rows.device.set_selected(0);
    rows.device.set_sensitive(false);
    rows.device.remove_css_class("error");
    rows.device.set_tooltip_text(None);
    rows.updating_device.set(false);
    for row in [
        &rows.version,
        &rows.protocol,
        &rows.torch,
        &rows.cuda,
        &rows.jobs,
        &rows.reservations,
        &rows.workers,
        &rows.features,
    ] {
        row.set_subtitle(tr!("—").as_ref());
    }
}
