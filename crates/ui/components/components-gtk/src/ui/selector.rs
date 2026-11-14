use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::rc::Rc;

use gtk::prelude::*;
pub use shrimply_components_core::selector::StringChoice;
use shrimply_components_core::selector::{identity_choices, searchable, selected_index};
use strum::IntoEnumIterator;

use super::control_row;

#[derive(Clone)]
pub struct StringSelector {
    widget: gtk::Widget,
    dropdown: gtk::DropDown,
    choices: Rc<RefCell<Vec<StringChoice>>>,
    updating: Rc<Cell<bool>>,
}

impl StringSelector {
    pub fn widget(&self) -> &gtk::Widget {
        &self.widget
    }

    pub fn set_options(&self, value: &str, options: Vec<String>) {
        self.set_choices(value, identity_choices(options));
    }

    pub fn set_choices(&self, value: &str, choices: Vec<StringChoice>) {
        let enable_search = searchable(choices.len());
        let selected = selected_index(value, &choices);
        let labels = choices
            .iter()
            .map(|choice| choice.label.as_str())
            .collect::<Vec<_>>();
        let model = gtk::StringList::new(&labels);
        self.updating.set(true);
        *self.choices.borrow_mut() = choices;
        self.dropdown.set_model(Some(&model));
        self.dropdown.set_selected(selected as u32);
        self.dropdown.set_enable_search(enable_search);
        self.updating.set(false);
    }

    pub fn set_sensitive(&self, sensitive: bool) {
        self.dropdown.set_sensitive(sensitive);
    }
}

pub fn string_selector(
    label: &str,
    value: &str,
    options: Vec<String>,
    changed: impl Fn(String) + 'static,
) -> StringSelector {
    labeled_string_selector(label, value, identity_choices(options), changed)
}

pub fn labeled_string_selector(
    label: &str,
    value: &str,
    choices: Vec<StringChoice>,
    changed: impl Fn(String) + 'static,
) -> StringSelector {
    let labels = choices
        .iter()
        .map(|choice| choice.label.as_str())
        .collect::<Vec<_>>();
    let selected = selected_index(value, &choices);
    let expression = gtk::StringObject::this_expression("string");
    let dropdown = gtk::DropDown::builder()
        .model(&gtk::StringList::new(&labels))
        .selected(selected as u32)
        .enable_search(searchable(choices.len()))
        .expression(&expression)
        .halign(gtk::Align::Fill)
        .hexpand(true)
        .css_classes(["inspector-selector"])
        .build();
    let choices = Rc::new(RefCell::new(choices));
    let updating = Rc::new(Cell::new(false));
    let callback_choices = choices.clone();
    let callback_updating = updating.clone();
    dropdown.connect_selected_notify(move |dropdown| {
        if callback_updating.get() {
            return;
        }
        let choice = callback_choices
            .borrow()
            .get(dropdown.selected() as usize)
            .cloned();
        if let Some(choice) = choice {
            changed(choice.value);
        }
    });

    StringSelector {
        widget: if label.is_empty() {
            dropdown.clone().upcast()
        } else {
            control_row(label, &dropdown)
        },
        dropdown,
        choices,
        updating,
    }
}

pub fn dropdown<T, L>(
    value: T,
    options: impl IntoIterator<Item = (T, L)>,
    changed: impl Fn(T) + 'static,
) -> gtk::DropDown
where
    T: Copy + Eq + 'static,
    L: AsRef<str> + 'static,
{
    let options = options.into_iter().collect::<Vec<_>>();
    let labels = options
        .iter()
        .map(|(_, label)| crate::i18n::text(label.as_ref()))
        .collect::<Vec<_>>();
    let labels = labels
        .iter()
        .map(|label| label.as_ref())
        .collect::<Vec<_>>();
    let selected = options
        .iter()
        .position(|(option, _)| *option == value)
        .expect("dropdown value must be one of its options");
    let expression = gtk::StringObject::this_expression("string");
    let dropdown = gtk::DropDown::builder()
        .model(&gtk::StringList::new(&labels))
        .selected(selected as u32)
        .enable_search(searchable(options.len()))
        .expression(&expression)
        .halign(gtk::Align::Fill)
        .hexpand(true)
        .css_classes(["inspector-selector"])
        .build();
    dropdown.connect_selected_notify(move |dropdown| {
        let Some((value, _)) = options.get(dropdown.selected() as usize) else {
            return;
        };
        changed(*value);
    });
    dropdown
}

pub fn enum_dropdown<T>(value: T, changed: impl Fn(T) + 'static) -> gtk::DropDown
where
    T: Copy + Display + Eq + IntoEnumIterator + 'static,
{
    dropdown(
        value,
        T::iter().map(|option| (option, option.to_string())),
        changed,
    )
}

pub fn selector<T, L>(
    label: &str,
    value: T,
    options: impl IntoIterator<Item = (T, L)>,
    changed: impl Fn(T) + 'static,
) -> gtk::Widget
where
    T: Copy + Eq + 'static,
    L: AsRef<str> + 'static,
{
    let dropdown = dropdown(value, options, changed);
    control_row(label, &dropdown)
}

pub fn enum_selector<T>(label: &str, value: T, changed: impl Fn(T) + 'static) -> gtk::Widget
where
    T: Copy + Display + Eq + IntoEnumIterator + 'static,
{
    let dropdown = enum_dropdown(value, changed);
    control_row(label, &dropdown)
}
