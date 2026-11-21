use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::{gdk, glib};

use super::StringChoice;

type SearchItems = Rc<dyn Fn(&str) -> Vec<SearchMenuItem>>;
type Dismiss = Rc<dyn Fn()>;

pub struct SearchMenuItem {
    pub label: String,
    selected: bool,
    tooltip: Option<String>,
    activate: Rc<dyn Fn()>,
}

impl SearchMenuItem {
    pub fn new(label: impl Into<String>, activate: impl Fn() + 'static) -> Self {
        Self {
            label: label.into(),
            selected: false,
            tooltip: None,
            activate: Rc::new(activate),
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
}

pub fn searchable_menu(
    label: &str,
    placeholder: &str,
    items: impl Fn(&str) -> Vec<SearchMenuItem> + 'static,
) -> gtk::MenuButton {
    let items = Rc::new(items);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::MenuButton::builder()
        .child(&content)
        .halign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let popup_items = items.clone();
    let popover = searchable_popover(placeholder, 280, 240, 360, move |query| popup_items(query));
    popover.set_has_arrow(false);
    button.set_popover(Some(&popover));
    let placeholder = placeholder.to_owned();
    super::selector::on_touch_menu(&button, move |widget| {
        widget
            .downcast_ref::<gtk::MenuButton>()
            .expect("native menu button")
            .popdown();
        let items = items.clone();
        searchable_menu_at(widget, (0.0, 0.0), true, &placeholder, move |query| {
            items(query)
        });
    });
    button
}

pub fn searchable_popover(
    placeholder: &str,
    minimum_width: i32,
    minimum_height: i32,
    maximum_height: i32,
    items: impl Fn(&str) -> Vec<SearchMenuItem> + 'static,
) -> gtk::Popover {
    let popover = gtk::Popover::builder().autohide(true).build();
    popover.add_css_class("menu");
    let weak = popover.downgrade();
    let dismiss: Dismiss = Rc::new(move || {
        if let Some(popover) = weak.upgrade() {
            popover.popdown();
        }
    });
    popover.set_child(Some(&searchable_content(
        placeholder,
        minimum_width,
        minimum_height,
        maximum_height,
        items,
        dismiss,
    )));
    popover
}

/// The same searchable chooser content, with presentation selected by input source.
pub fn searchable_menu_at(
    parent: &impl IsA<gtk::Widget>,
    point: (f64, f64),
    touch: bool,
    placeholder: &str,
    items: impl Fn(&str) -> Vec<SearchMenuItem> + 'static,
) {
    if shrimply_components_core::layout::menu_uses_bottom_sheet(if touch {
        "touch"
    } else {
        "pointer"
    }) {
        let dialog = adw::Dialog::builder()
            .presentation_mode(adw::DialogPresentationMode::BottomSheet)
            .content_width(360)
            .build();
        let weak = dialog.downgrade();
        let dismiss: Dismiss = Rc::new(move || {
            if let Some(dialog) = weak.upgrade() {
                dialog.close();
            }
        });
        dialog.set_child(Some(&searchable_content(
            placeholder,
            280,
            180,
            240,
            items,
            dismiss,
        )));
        dialog.present(Some(parent));
    } else {
        let popover = searchable_popover(placeholder, 280, 180, 240, items);
        popover.set_parent(parent);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(
            point.0 as i32,
            point.1 as i32,
            1,
            1,
        )));
        super::unparent_on_close(&popover);
        popover.popup();
    }
}

fn searchable_content(
    placeholder: &str,
    minimum_width: i32,
    minimum_height: i32,
    maximum_height: i32,
    items: impl Fn(&str) -> Vec<SearchMenuItem> + 'static,
    dismiss: Dismiss,
) -> gtk::Box {
    let search = gtk::SearchEntry::builder()
        .placeholder_text(placeholder)
        .hexpand(true)
        .build();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_width(minimum_width)
        .min_content_height(minimum_height)
        .max_content_height(maximum_height)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    content.append(&search);
    content.append(&scroller);
    let items: SearchItems = Rc::new(items);
    let initial = Rc::new(RefCell::new(populate(&list, "", &items, &dismiss)));
    search.connect_search_changed({
        let initial = initial.clone();
        let items = items.clone();
        let list = list.clone();
        let dismiss = dismiss.clone();
        move |search| {
            *initial.borrow_mut() = populate(&list, search.text().as_str(), &items, &dismiss);
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let initial = initial.clone();
        let list = list.clone();
        let dismiss = dismiss.clone();
        move |_, key, _, _| match key {
            gdk::Key::Down => {
                if let Some(row) = list.first_child() {
                    row.grab_focus();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Up => {
                if let Some(row) = list.last_child() {
                    row.grab_focus();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Some(row) = initial.borrow().as_ref() {
                    row.emit_clicked();
                }
                glib::Propagation::Stop
            }
            gdk::Key::Escape => {
                dismiss();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    search.add_controller(keys);
    content.connect_map({
        let initial = initial.clone();
        let items = items.clone();
        let list = list.clone();
        let dismiss = dismiss.clone();
        move |_| {
            if search.text().is_empty() {
                *initial.borrow_mut() = populate(&list, "", &items, &dismiss);
            } else {
                search.set_text("");
            }
            search.grab_focus();
        }
    });
    content
}

pub fn modifier_menu(
    choices: Vec<StringChoice>,
    selected: impl Fn(String) + 'static,
) -> gtk::MenuButton {
    let choices = Rc::new(choices);
    let selected: Rc<dyn Fn(String)> = Rc::new(selected);
    searchable_menu("Add modifier", "Search modifiers", move |query| {
        choices
            .iter()
            .filter(|choice| {
                shrimply_components_core::selector::matches_query(&choice.label, query)
            })
            .map(|choice| {
                let value = choice.value.clone();
                let selected = selected.clone();
                SearchMenuItem::new(choice.label.clone(), move || selected(value.clone()))
            })
            .collect()
    })
}

fn populate(
    list: &gtk::Box,
    query: &str,
    items: &SearchItems,
    dismiss: &Dismiss,
) -> Option<gtk::Button> {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let mut first = None;
    let mut selected = None;
    for item in items(query) {
        let label = gtk::Label::builder()
            .label(&item.label)
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .hexpand(true)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        content.append(&label);
        if item.selected {
            content.append(&gtk::Image::from_icon_name("object-select-symbolic"));
        }
        let row = gtk::Button::builder()
            .child(&content)
            .halign(gtk::Align::Fill)
            .hexpand(true)
            .css_classes(["flat"])
            .build();
        first.get_or_insert_with(|| row.clone());
        if item.selected {
            selected = Some(row.clone());
        }
        if let Some(tooltip) = &item.tooltip {
            row.set_tooltip_text(Some(tooltip));
        }
        let activate = item.activate;
        let click_dismiss = dismiss.clone();
        row.connect_clicked(move |_| {
            activate();
            click_dismiss();
        });
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let row = row.clone();
            let dismiss = dismiss.clone();
            move |_, key, _, _| match key {
                gdk::Key::Down => {
                    if let Some(next) = row.next_sibling() {
                        next.grab_focus();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Up => {
                    if let Some(previous) = row.prev_sibling() {
                        previous.grab_focus();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Escape => {
                    dismiss();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        row.add_controller(keys);
        list.append(&row);
    }
    selected.or(first)
}
