use crate::ui::{SingleLineTextInput, control_row, tag_pill};
use adw::prelude::*;
use gtk::glib;
use shrimply_components_core::presentation::tag_editor as presentation;
use shrimply_project_types::tags::{TagEdit, edit_tags, normalize_tags};
use std::{cell::RefCell, rc::Rc};

pub fn tag_editor(tags: Vec<String>, on_change: impl Fn(&[String]) + 'static) -> gtk::Box {
    let on_change = Rc::new(on_change);
    let tags = Rc::new(RefCell::new(normalize_tags(&tags)));
    let labels = presentation(&[]);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let pills = adw::WrapBox::builder()
        .child_spacing(6)
        .line_spacing(6)
        .build();
    populate(&pills, &tags, &on_change);
    let entry = SingleLineTextInput::builder("")
        .placeholder(labels["placeholder"].as_str().expect("tag placeholder"))
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(
        labels["add_label"].as_str().expect("add tag label"),
    )]);
    entry.connect_activate(glib::clone!(
        #[weak]
        pills,
        move |entry| {
            let next = edit_tags(&tags.borrow(), TagEdit::Add(entry.text().as_str()));
            if next != *tags.borrow() {
                *tags.borrow_mut() = next;
                on_change(&tags.borrow());
                populate(&pills, &tags, &on_change);
            }
            entry.set_text("");
        }
    ));
    content.append(&control_row(
        labels["label"].as_str().expect("tag label"),
        &entry,
    ));
    content.append(&pills);
    content
}

fn populate(
    pills: &adw::WrapBox,
    tags: &Rc<RefCell<Vec<String>>>,
    on_change: &Rc<impl Fn(&[String]) + 'static>,
) {
    while let Some(child) = pills.first_child() {
        pills.remove(&child);
    }
    pills.set_visible(!tags.borrow().is_empty());
    let model = presentation(&tags.borrow());
    for item in model["tags"].as_array().expect("tag entries") {
        let tag = item["value"].as_str().expect("tag value").to_owned();
        let pill = tag_pill(&tag, true);
        let remove_label = item["remove_label"].as_str().expect("remove tag label");
        pill.update_property(&[gtk::accessible::Property::Label(remove_label)]);
        pill.set_tooltip_text(Some(remove_label));
        pill.connect_clicked(glib::clone!(
            #[weak]
            pills,
            #[strong]
            tags,
            #[strong]
            on_change,
            #[strong]
            tag,
            move |_| {
                let next = edit_tags(&tags.borrow(), TagEdit::Remove(&tag));
                *tags.borrow_mut() = next;
                on_change(&tags.borrow());
                populate(&pills, &tags, &on_change);
                pills
                    .parent()
                    .expect("tag pills remain in their control")
                    .child_focus(gtk::DirectionType::TabForward);
            }
        ));
        pills.append(&pill);
    }
}
