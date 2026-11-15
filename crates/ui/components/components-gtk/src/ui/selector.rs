use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::rc::Rc;

use adw::prelude::*;
pub use shrimply_components_core::selector::StringChoice;
use shrimply_components_core::selector::{
    identity_choices, ranked_matching_indices, searchable, selected_index,
};
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
    let dropdown = choice_dropdown(&labels, selected);
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
    let dropdown = choice_dropdown(&labels, selected);
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

fn choice_dropdown(labels: &[&str], selected: usize) -> gtk::DropDown {
    let dropdown = gtk::DropDown::builder()
        .model(&gtk::StringList::new(labels))
        .selected(selected as u32)
        .expression(gtk::StringObject::this_expression("string"))
        .halign(gtk::Align::Fill)
        .hexpand(true)
        .css_classes(["inspector-selector"])
        .build();
    adaptive_choice(&dropdown);
    dropdown
}

/// Keep native choice models and selection callbacks when touch opens a sheet.
pub fn adaptive_choice(widget: &impl IsA<gtk::Widget>) {
    let widget = widget.as_ref();
    assert!(
        widget.is::<gtk::DropDown>() || widget.is::<adw::ComboRow>(),
        "adaptive choice requires a native selector"
    );
    let update_search = |widget: &gtk::Widget| {
        let model = widget.property::<Option<gtk::gio::ListModel>>("model");
        widget.set_property(
            "enable-search",
            searchable(model.map_or(0, |model| model.n_items() as usize)),
        );
    };
    update_search(widget);
    widget.connect_notify_local(Some("model"), move |widget, _| update_search(widget));
    on_touch_menu(widget, choice_sheet);
}

pub(super) fn on_touch_menu(
    widget: &impl IsA<gtk::Widget>,
    activate: impl Fn(&gtk::Widget) + 'static,
) {
    let widget = widget.as_ref();
    let touch = gtk::GestureClick::builder()
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let target = widget.downgrade();
    touch.connect_released(move |gesture, _, _, _| {
        let Some(device) = gesture.current_event_device() else {
            return;
        };
        let source = match device.source() {
            gtk::gdk::InputSource::Touchscreen => "touch",
            gtk::gdk::InputSource::Pen => "pen",
            _ => "pointer",
        };
        if crate::layout::menu_uses_bottom_sheet(source) {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(widget) = target.upgrade() {
                activate(&widget);
            }
        }
    });
    widget.add_controller(touch);
}

fn choice_sheet(widget: &gtk::Widget) {
    let model = widget
        .property::<Option<gtk::gio::ListModel>>("model")
        .expect("selector has a choices model");
    let labels = (0..model.n_items())
        .map(|index| {
            model
                .item(index)
                .and_downcast::<gtk::StringObject>()
                .expect("selector choices are strings")
                .string()
                .to_string()
        })
        .collect::<Vec<_>>();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .activate_on_single_click(true)
        .css_classes(["boxed-list"])
        .build();
    let search = searchable(model.n_items() as usize)
        .then(|| gtk::SearchEntry::builder().hexpand(true).build());
    let dialog = super::menu::menu_sheet(&list, search.as_ref().map(|entry| entry.upcast_ref()));
    let visible = Rc::new(RefCell::new(Vec::<usize>::new()));
    let target = widget.downgrade();
    let weak_dialog = dialog.downgrade();
    list.connect_row_activated({
        let visible = visible.clone();
        let model = model.clone();
        move |_, row| {
            let Some(dialog) = weak_dialog.upgrade() else {
                return;
            };
            dialog.close();
            let Some(widget) = target.upgrade() else {
                return;
            };
            if widget
                .property::<Option<gtk::gio::ListModel>>("model")
                .as_ref()
                == Some(&model)
                && let Some(index) = visible.borrow().get(row.index() as usize)
            {
                widget.set_property("selected", *index as u32);
            }
        }
    });
    let populate = {
        let list = list.clone();
        let selected = widget.property::<u32>("selected") as usize;
        move |query: &str| {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let indices = ranked_matching_indices(&labels, &[], query);
            for &index in &indices {
                let row = adw::ActionRow::builder()
                    .title(&labels[index])
                    .title_lines(0)
                    .use_markup(false)
                    .activatable(true)
                    .build();
                if index == selected {
                    row.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
                }
                list.append(&row);
            }
            *visible.borrow_mut() = indices;
        }
    };
    populate("");
    if let Some(search) = search {
        search.connect_search_changed(move |search| populate(search.text().as_str()));
    }
    dialog.present(Some(widget));
}
