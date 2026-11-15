use adw::prelude::*;
use gtk::gio;
use std::rc::Rc;

/// GtkModelButton pops down its menu before dispatching its native action.
pub fn unparent_on_close(popover: &impl IsA<gtk::Popover>) {
    let popover = popover.as_ref();
    popover.connect_closed(|popover| {
        let popover = popover.downgrade();
        gtk::glib::idle_add_local_once(move || {
            if let Some(popover) = popover.upgrade()
                && !popover.is_visible()
                && popover.parent().is_some()
            {
                popover.unparent();
            }
        });
    });
}

/// The shared command returns the actual state; native widgets never toggle it themselves.
pub fn menu_action(
    name: &str,
    checked: Option<bool>,
    activate: impl Fn() -> Option<bool> + 'static,
) -> gio::SimpleAction {
    let action = match checked {
        Some(checked) => gio::SimpleAction::new_stateful(name, None, &checked.to_variant()),
        None => gio::SimpleAction::new(name, None),
    };
    let dispatch = Rc::new(move |action: &gio::SimpleAction| {
        if let Some(checked) = activate() {
            action.set_state(&checked.to_variant());
        }
    });
    let activate = dispatch.clone();
    action.connect_activate(move |action, _| activate(action));
    if checked.is_some() {
        action.connect_change_state(move |action, requested| {
            if let Some(requested) = requested.and_then(|state| state.get::<bool>())
                && Some(requested) != action.state().and_then(|state| state.get::<bool>())
            {
                dispatch(action);
            }
        });
    }
    action
}

pub fn menu_sheet(content: &impl IsA<gtk::Widget>, header: Option<&gtk::Widget>) -> adw::Dialog {
    const CONTENT_WIDTH: i32 = 360;
    const MAX_CONTENT_HEIGHT: i32 = 560;
    const PADDING: i32 = 12;
    let layout = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(PADDING)
        .margin_bottom(PADDING)
        .margin_start(PADDING)
        .margin_end(PADDING)
        .build();
    if let Some(header) = header {
        layout.append(header);
    }
    layout.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(MAX_CONTENT_HEIGHT)
            .child(content)
            .build(),
    );
    adw::Dialog::builder()
        .presentation_mode(adw::DialogPresentationMode::BottomSheet)
        .content_width(CONTENT_WIDTH)
        .child(&layout)
        .build()
}

/// Render the same native action menu in a sheet when it is opened by touch.
pub fn adaptive_menu(
    button: &gtk::MenuButton,
    namespace: &str,
    actions: &impl IsA<gio::ActionGroup>,
) {
    let namespace = namespace.to_owned();
    let actions = actions.as_ref().downgrade();
    super::selector::on_touch_menu(button, move |widget| {
        let button = widget
            .downcast_ref::<gtk::MenuButton>()
            .expect("native menu button");
        let Some(actions) = actions.upgrade() else {
            return;
        };
        let model = button
            .menu_model()
            .or_else(|| {
                button
                    .popover()
                    .and_downcast::<gtk::PopoverMenu>()
                    .and_then(|popover| popover.menu_model())
            })
            .expect("adaptive menu requires a native menu model");
        button.popdown();
        let dialog = action_menu_sheet(&model, &namespace, &actions, &[]);
        dialog.present(Some(button));
    });
}

/// Preserve native action state, targets and custom children in touch menus.
pub fn action_menu_sheet(
    model: &impl IsA<gio::MenuModel>,
    namespace: &str,
    actions: &impl IsA<gio::ActionGroup>,
    custom_children: &[(String, gtk::Widget)],
) -> adw::Dialog {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let dialog = menu_sheet(&content, None);
    dialog.insert_action_group(namespace, Some(actions));
    append_menu(
        &content,
        model.as_ref(),
        &dialog,
        (namespace, actions.as_ref()),
        custom_children,
    );
    dialog
}

fn append_menu(
    content: &gtk::Box,
    model: &gio::MenuModel,
    dialog: &adw::Dialog,
    action_group: (&str, &gio::ActionGroup),
    custom_children: &[(String, gtk::Widget)],
) {
    for index in 0..model.n_items() {
        let label = model
            .item_attribute_value(index, "label", None)
            .map(|value| value.get::<String>().expect("menu label is a string"));
        if let Some(section) = model.item_link(index, "section") {
            if index > 0 {
                content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            }
            if let Some(label) = label {
                content.append(&gtk::Label::new(Some(&label)));
            }
            append_menu(content, &section, dialog, action_group, custom_children);
        } else if let Some(submenu) = model.item_link(index, "submenu") {
            let children = gtk::Box::new(gtk::Orientation::Vertical, 0);
            append_menu(&children, &submenu, dialog, action_group, custom_children);
            content.append(
                &gtk::Expander::builder()
                    .label(label.as_deref().unwrap_or_default())
                    .use_underline(true)
                    .child(&children)
                    .build(),
            );
        } else if let Some(custom) = model
            .item_attribute_value(index, "custom", None)
            .and_then(|value| value.get::<String>())
        {
            let (_, child) = custom_children
                .iter()
                .find(|(name, _)| name == &custom)
                .expect("native menu custom child must be provided");
            content.append(child);
        } else {
            let action = model
                .item_attribute_value(index, "action", None)
                .and_then(|value| value.get::<String>())
                .expect("menu item has a native action");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            if let Some(icon) = model
                .item_attribute_value(index, "icon", None)
                .and_then(|value| gio::Icon::deserialize(&value))
            {
                row.append(&gtk::Image::from_gicon(&icon));
            }
            row.append(
                &gtk::Label::builder()
                    .label(label.as_deref().unwrap_or_default())
                    .use_underline(true)
                    .xalign(0.0)
                    .hexpand(true)
                    .build(),
            );
            let stateful = action
                .split_once('.')
                .filter(|(namespace, _)| *namespace == action_group.0)
                .and_then(|(_, name)| action_group.1.action_state_type(name))
                .is_some();
            let dialog = dialog.downgrade();
            let button: gtk::Actionable = if stateful {
                let button = gtk::CheckButton::builder().child(&row).build();
                // State synchronization must not dismiss the sheet.
                button.connect_activate(move |_| {
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.close();
                    }
                });
                button.upcast()
            } else {
                let button = gtk::Button::builder().has_frame(false).child(&row).build();
                button.connect_clicked(move |_| {
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.close();
                    }
                });
                button.upcast()
            };
            button.set_action_name(Some(&action));
            button.set_action_target_value(
                model.item_attribute_value(index, "target", None).as_ref(),
            );
            content.append(&button);
        }
    }
}
