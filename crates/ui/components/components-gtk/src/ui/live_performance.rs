use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::glib;

use super::InspectorCard;

#[derive(Default)]
struct LivePerformanceState {
    performance: shrimply_components_core::performance::PerformanceRows,
    timer: Option<glib::SourceId>,
}

pub fn live_performance() -> gtk::Widget {
    let rows_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let state = Rc::new(RefCell::new(LivePerformanceState::default()));

    let card =
        InspectorCard::with_expansion(crate::tr!("Live Performance").as_ref(), false, None, {
            let rows_box = rows_box.clone();
            let state = state.clone();
            move |expanded| {
                let mut current = state.borrow_mut();
                if let Some(source) = current.timer.take() {
                    source.remove();
                }
                if expanded {
                    refresh(&rows_box, &mut current.performance);
                    let weak_box = rows_box.downgrade();
                    let state = state.clone();
                    current.timer = Some(glib::timeout_add_local(
                        shrimply_components_core::performance::REFRESH_INTERVAL,
                        move || {
                            let Some(rows_box) = weak_box.upgrade() else {
                                return glib::ControlFlow::Break;
                            };
                            refresh(&rows_box, &mut state.borrow_mut().performance);
                            glib::ControlFlow::Continue
                        },
                    ));
                } else {
                    rows_box.remove_all();
                    current.performance = Default::default();
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
    clear.connect_clicked(move |_| {
        shrimply_components_core::performance::clear();
        let mut current = state.borrow_mut();
        if current.timer.is_some() {
            refresh(&rows_box, &mut current.performance);
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
    performance: &mut shrimply_components_core::performance::PerformanceRows,
) {
    if !performance.refresh() {
        return;
    }
    rows_box.remove_all();
    for entry in performance.rows() {
        rows_box.append(
            &adw::ActionRow::builder()
                .title(entry.title.as_str())
                .subtitle(entry.subtitle.as_str())
                .build(),
        );
    }
}
