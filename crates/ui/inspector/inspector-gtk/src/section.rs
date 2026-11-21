use std::cell::Cell;

use gtk::prelude::*;

use crate::ui::control_row;

pub(super) struct InspectorSection {
    pub(super) grid: gtk::Grid,
    pub(super) row: Cell<i32>,
}

impl InspectorSection {
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
