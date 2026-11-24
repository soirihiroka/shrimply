use gtk::prelude::*;

use crate::ui::control_row;

pub(super) struct InspectorSection {
    pub(super) rows: gtk::Box,
}

impl InspectorSection {
    pub(super) fn controls() -> Self {
        Self {
            rows: gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(8)
                .hexpand(true)
                .build(),
        }
    }

    pub(super) fn add_control_row(&self, label: &str, child: &impl IsA<gtk::Widget>) {
        self.add_wide_control(&control_row(label, child));
    }

    pub(super) fn add_wide_control(&self, child: &impl IsA<gtk::Widget>) {
        self.rows.append(child);
    }

    pub(super) fn into_widget(self) -> gtk::Widget {
        self.rows.upcast()
    }
}
