use shrimply_components_gtk::tr;
use std::cell::Cell;

use gtk::prelude::*;

use crate::ui::control_row;

pub(super) struct InspectorSection {
    pub(super) grid: gtk::Grid,
    pub(super) row: Cell<i32>,
}

impl InspectorSection {
    pub(super) fn new(title: &str, reset: Option<Box<dyn Fn() + 'static>>) -> Self {
        let grid = gtk::Grid::builder()
            .column_spacing(16)
            .row_spacing(8)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.set_hexpand(true);
        let title = gtk::Label::builder()
            .label(tr!(title).as_ref())
            .css_classes(["title-2"])
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        header.append(&title);

        if let Some(reset) = reset {
            let button = gtk::Button::builder()
                .icon_name("edit-undo-symbolic")
                .tooltip_text(tr!("Reset").as_ref())
                .halign(gtk::Align::End)
                .build();
            button.add_css_class("flat");
            button.connect_clicked(move |_| reset());
            header.append(&button);
        }

        grid.attach(&header, 0, 0, 2, 1);
        Self {
            grid,
            row: Cell::new(1),
        }
    }

    pub(super) fn controls() -> Self {
        Self {
            grid: gtk::Grid::builder()
                .column_spacing(6)
                .row_spacing(8)
                .hexpand(true)
                .build(),
            row: Cell::new(0),
        }
    }

    pub(super) fn add_control_row(&self, label: &str, child: &impl IsA<gtk::Widget>) {
        self.add_wide_control(&control_row(label, child));
    }

    pub(super) fn add_wide_control(&self, child: &impl IsA<gtk::Widget>) {
        let row = self.row.get();
        self.row.set(row + 1);
        self.grid.attach(child, 0, row, 2, 1);
    }

    pub(super) fn into_widget(self) -> gtk::Widget {
        self.grid.upcast()
    }
}
