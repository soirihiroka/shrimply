use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use gtk::glib;

thread_local! {
    static CSS_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

pub fn live_performance() -> gtk::Widget {
    let status = adw::ExpanderRow::builder()
        .title(crate::tr!("Live Performance").as_ref())
        .hexpand(true)
        .build();
    let clear = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text(crate::tr!("Clear").as_ref())
        .valign(gtk::Align::Center)
        .css_classes(["flat", "live-performance-action"])
        .build();
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(crate::tr!("Copy JSON").as_ref())
        .valign(gtk::Align::Center)
        .css_classes(["flat", "live-performance-action"])
        .build();
    install_css(&status.display());
    status.add_suffix(&clear);
    status.add_suffix(&copy);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("card");
    content.append(&status);
    let rows = Rc::new(RefCell::new(Vec::<adw::ActionRow>::new()));
    let performance = Rc::new(RefCell::new(
        shrimply_components_core::performance::PerformanceRows::default(),
    ));

    clear.connect_clicked({
        let status = status.clone();
        let rows = rows.clone();
        let performance = performance.clone();
        move |_| {
            shrimply_components_core::performance::clear();
            if status.is_expanded() {
                refresh(&status, &rows, &performance);
            }
        }
    });
    copy.connect_clicked(|button| {
        button
            .display()
            .clipboard()
            .set_text(&shrimply_components_core::performance::report_json());
    });
    let refresh_source = Rc::new(RefCell::new(None::<glib::SourceId>));
    status.connect_expanded_notify({
        let rows = rows.clone();
        let performance = performance.clone();
        let refresh_source = refresh_source.clone();
        move |status| {
            if let Some(source) = refresh_source.borrow_mut().take() {
                source.remove();
            }
            if status.is_expanded() {
                refresh(status, &rows, &performance);
                let status = status.downgrade();
                let rows = rows.clone();
                let performance = performance.clone();
                *refresh_source.borrow_mut() = Some(glib::timeout_add_local(
                    shrimply_components_core::performance::REFRESH_INTERVAL,
                    move || {
                        let Some(status) = status.upgrade() else {
                            return glib::ControlFlow::Break;
                        };
                        if !status.is_expanded() {
                            return glib::ControlFlow::Break;
                        }
                        refresh(&status, &rows, &performance);
                        glib::ControlFlow::Continue
                    },
                ));
            } else {
                for row in rows.take() {
                    status.remove(&row);
                }
                *performance.borrow_mut() = Default::default();
            }
        }
    });
    content.upcast()
}

fn install_css(display: &gtk::gdk::Display) {
    CSS_INSTALLED.with(|installed| {
        if installed.replace(true) {
            return;
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(
            ".live-performance-action, \
             .live-performance-action:hover, \
             .live-performance-action:active { \
                 background: transparent; \
                 box-shadow: none; \
             }",
        );
        gtk::style_context_add_provider_for_display(
            display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}

fn refresh(
    status: &adw::ExpanderRow,
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
        status.remove(&row);
    }
    let next_rows = entries
        .into_iter()
        .map(|entry| {
            let row = adw::ActionRow::builder()
                .title(entry.title)
                .subtitle(entry.subtitle)
                .build();
            status.add_row(&row);
            row
        })
        .collect();
    *rows.borrow_mut() = next_rows;
}
