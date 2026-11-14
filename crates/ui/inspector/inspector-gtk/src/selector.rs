use gtk::prelude::Cast;
use shrimply_components_gtk::tr;
use shrimply_inspector_core::selector::complete_icons;
use shrimply_property_model::timeline_value::TimelineStep;

pub(super) use crate::ui::{
    StringChoice, dropdown, enum_selector, labeled_string_selector, selector,
};

pub(super) fn step_editor<T: TimelineStep>(value: T, changed: impl Fn(T) + 'static) -> gtk::Widget {
    let variants = T::variants();
    if complete_icons(variants.iter().map(|variant| variant.icon)).is_some() {
        return step_button_editor(value, changed);
    }
    dropdown(
        value,
        variants
            .iter()
            .map(|variant| (variant.value, variant.label)),
        changed,
    )
    .upcast()
}

pub(super) fn step_button_editor<T: TimelineStep>(
    value: T,
    changed: impl Fn(T) + 'static,
) -> gtk::Widget {
    let variants = T::variants();
    let group = adw::ToggleGroup::builder()
        .halign(gtk::Align::End)
        .homogeneous(true)
        .build();
    for variant in variants {
        let mut toggle = adw::Toggle::builder().name(variant.key);
        toggle = if let Some(icon) = variant.icon {
            toggle.icon_name(icon).tooltip(tr!(variant.label).as_ref())
        } else {
            toggle.label(tr!(variant.label).as_ref())
        };
        group.add(toggle.build());
    }
    group.set_active(
        variants
            .iter()
            .position(|variant| variant.value == value)
            .expect("timeline step value must be a declared variant") as u32,
    );
    group.connect_active_notify(move |group| {
        if let Some(variant) = variants.get(group.active() as usize) {
            changed(variant.value);
        }
    });
    group.upcast()
}
