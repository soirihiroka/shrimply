use crate::{InspectorContext, player_state};
use gtk::prelude::*;
use std::rc::Rc;

pub(super) fn section(
    context: &InspectorContext,
    listener_name: &'static str,
    editor: impl FnOnce(Rc<dyn Fn()>) -> gtk::Widget,
    evaluate: impl Fn() -> Option<shrimply_inspector_core::InspectorExpressionOutput<String>> + 'static,
) -> gtk::Widget {
    let output = gtk::Label::builder()
        .hexpand(true)
        .xalign(1.0)
        .selectable(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let output_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let output_title = gtk::Label::new(Some(shrimply_components_gtk::tr!("Output").as_ref()));
    output_title.add_css_class("dim-label");
    output_row.append(&output_title);
    output_row.append(&output);

    let refresh: Rc<dyn Fn()> = Rc::new({
        let output = output.clone();
        move || {
            if !output.is_mapped() {
                return;
            }
            let Some(result) = evaluate() else {
                return;
            };
            output.set_label(&result.value);
            output.set_tooltip_text(result.error.as_deref());
            if result.error.is_some() {
                output.add_css_class("error");
            } else {
                output.remove_css_class("error");
            }
        }
    });

    let section = gtk::Box::new(gtk::Orientation::Vertical, 6);
    section.append(&editor(refresh.clone()));
    section.append(&output_row);
    section.connect_map({
        let refresh = refresh.clone();
        move |_| refresh()
    });

    let alive = Rc::downgrade(&context.listener_scope);
    let output = output.downgrade();
    player_state::connect_while_alive_named(
        &context.player_state,
        listener_name,
        move || alive.upgrade().is_some(),
        move |_| {
            if output.upgrade().is_some_and(|output| output.is_mapped()) {
                refresh();
            }
        },
    );
    section.upcast()
}
