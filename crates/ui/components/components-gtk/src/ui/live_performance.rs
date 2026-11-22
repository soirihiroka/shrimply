use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use gtk::glib;

use super::InspectorCard;

pub fn live_performance() -> gtk::Widget {
    let rows_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let expanded = Rc::new(Cell::new(false));
    let rows = Rc::new(RefCell::new(Vec::<adw::ActionRow>::new()));
    let performance = Rc::new(RefCell::new(
        shrimply_components_core::performance::PerformanceRows::default(),
    ));
    let refresh_source = Rc::new(RefCell::new(None::<glib::SourceId>));

    let card =
        InspectorCard::with_expansion(crate::tr!("Live Performance").as_ref(), false, None, {
            let expanded = expanded.clone();
            let rows_box = rows_box.clone();
            let rows = rows.clone();
            let performance = performance.clone();
            let refresh_source = refresh_source.clone();
            move |is_expanded| {
                expanded.set(is_expanded);
                if let Some(source) = refresh_source.borrow_mut().take() {
                    source.remove();
                }
                if is_expanded {
                    refresh(&rows_box, &rows, &performance);
                    let weak_box = rows_box.downgrade();
                    let expanded = expanded.clone();
                    let rows = rows.clone();
                    let performance = performance.clone();
                    *refresh_source.borrow_mut() = Some(glib::timeout_add_local(
                        shrimply_components_core::performance::REFRESH_INTERVAL,
                        move || {
                            let Some(rows_box) = weak_box.upgrade() else {
                                return glib::ControlFlow::Break;
                            };
                            if !expanded.get() {
                                return glib::ControlFlow::Break;
                            }
                            refresh(&rows_box, &rows, &performance);
                            glib::ControlFlow::Continue
                        },
                    ));
                } else {
                    for row in rows.take() {
                        rows_box.remove(&row);
                    }
                    *performance.borrow_mut() = Default::default();
                }
            }
        });
    card.root().set_hexpand(true);
    card.append(&rows_box);

    let clear = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text(crate::tr!("Clear").as_ref())
        .css_classes(["flat"])
        .build();
    clear.connect_clicked({
        let expanded = expanded.clone();
        let rows_box = rows_box.clone();
        let rows = rows.clone();
        let performance = performance.clone();
        move |_| {
            shrimply_components_core::performance::clear();
            if expanded.get() {
                refresh(&rows_box, &rows, &performance);
            }
        }
    });
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(crate::tr!("Copy JSON").as_ref())
        .css_classes(["flat"])
        .build();
    copy.connect_clicked(|button| {
        button
            .display()
            .clipboard()
            .set_text(&shrimply_components_core::performance::report_json());
    });
    card.append_after_reset(&clear);
    card.append_after_reset(&copy);

    card.widget().clone()
}

fn refresh(
    rows_box: &gtk::ListBox,
    rows: &RefCell<Vec<adw::ActionRow>>,
    performance: &RefCell<shrimply_components_core::performance::PerformanceRows>,
) {
    let entries = {
        let mut performance = performance.borrow_mut();
        if !performance.refresh() {
            return;
        }
        performance.rows().to_vec()
    };
    for row in rows.take() {
        rows_box.remove(&row);
    }
    *rows.borrow_mut() = entries
        .into_iter()
        .map(|entry| {
            let row = adw::ActionRow::builder()
                .title(entry.title)
                .subtitle(entry.subtitle)
                .build();
            rows_box.append(&row);
            row
        })
        .collect();
}
