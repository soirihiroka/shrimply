use adw::prelude::*;

pub fn control_row(label: &str, child: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let label = crate::i18n::text(label);
    let row = adw::WrapBox::builder()
        .orientation(gtk::Orientation::Horizontal)
        .child_spacing(shrimply_components_core::layout::CONTROL_ROW_GAP)
        .line_spacing(shrimply_components_core::layout::CONTROL_ROW_GAP)
        .wrap_policy(adw::WrapPolicy::Minimum)
        .hexpand(true)
        .build();
    row.append(
        &gtk::Label::builder()
            .label(label.as_ref())
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Center)
            .width_chars(
                i32::try_from(shrimply_components_core::layout::CONTROL_ROW_LABEL_COLUMNS)
                    .expect("control row label width fits i32"),
            )
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build(),
    );
    child.set_hexpand(true);
    row.append(child);
    row.upcast()
}
