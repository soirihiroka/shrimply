use super::{InspectorContext, section::InspectorSection};
use adw::prelude::*;
use shrimply_inspector_core::{ControlKind, InspectorControl, InspectorTarget};
pub(crate) mod layered;
mod value;

pub(super) fn section(
    model: &shrimply_inspector_core::InspectorSection,
    context: &InspectorContext,
) -> Vec<gtk::Widget> {
    let section = InspectorSection::controls();
    for control in model.controls.iter().filter(|control| control.visible) {
        add_control(&section, context, &context.list_target, control.clone());
    }
    vec![section.into_widget()]
}

pub(super) fn add_control(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: InspectorControl,
) {
    let first_row = section.row.get();
    let focus = control.preview_focus.clone();
    match control.kind {
        ControlKind::Selector
        | ControlKind::OptionalSelector
        | ControlKind::OptionalNumberSelector
        | ControlKind::VoiceModel
        | ControlKind::AudioCachePreset
        | ControlKind::VisualCacheQuality
        | ControlKind::LayeredSelector => value::selector(section, context, target, control),
        ControlKind::Number | ControlKind::LayeredNumber => {
            layered::number(section, context, target, control)
        }
        ControlKind::Fraction => layered::fraction(section, context, target, control),
        ControlKind::Boolean | ControlKind::LayeredBoolean => {
            value::boolean(section, context, target, control)
        }
        ControlKind::Color | ControlKind::LayeredColor => {
            value::color(section, context, target, control)
        }
        ControlKind::Text | ControlKind::MultilineText | ControlKind::LayeredText => {
            value::text(section, context, target, control)
        }
        ControlKind::Vector2
        | ControlKind::Vector3
        | ControlKind::LayeredVector2
        | ControlKind::LayeredVector3 => value::vector(section, context, target, control),
        ControlKind::ReadOnly => {
            let row = gtk::Label::new(Some(&control.value));
            row.set_wrap(true);
            row.set_selectable(true);
            section.add_control_row(&control.label, &row);
        }
        ControlKind::FileLocation => {
            section.add_wide_control(&super::info::file_row(control.value.into(), &control.label))
        }
        ControlKind::InfoHeading => {
            section.add_wide_control(
                &gtk::Label::builder()
                    .label(&control.label)
                    .halign(gtk::Align::Start)
                    .css_classes(["heading"])
                    .build(),
            );
        }
        ControlKind::InfoLoading => {
            let row = adw::ActionRow::builder()
                .title(&control.label)
                .subtitle(&control.value)
                .build();
            row.add_suffix(&adw::Spinner::new());
            section.add_wide_control(&row);
        }
        ControlKind::InfoArtwork => {
            if let Some(bytes) = control.image_bytes
                && let Some(group) = super::info::artwork(bytes, &control.label)
            {
                section.add_wide_control(&group);
            }
        }
        _ => panic!("unsupported generic GTK control kind: {:?}", control.kind),
    }
    if let Some(focus) = focus {
        let controller = context.inspector_core.clone();
        let target = target.clone();
        let state = context.preview_focus.clone();
        let activate: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(move || {
            if let Some(focused) = controller.resolve_preview_focus(&target, &focus) {
                crate::preview_focus::set(&state, focused);
            }
        });
        for row in first_row..section.row.get() {
            let widget = section
                .grid
                .child_at(0, row)
                .expect("inserted inspector row");
            let click = gtk::GestureClick::new();
            click.set_button(0);
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let pressed = activate.clone();
            click.connect_pressed(move |_, _, _, _| pressed());
            widget.add_controller(click);
            let focus = gtk::EventControllerFocus::new();
            let entered = activate.clone();
            focus.connect_enter(move |_| entered());
            widget.add_controller(focus);
        }
    }
}
