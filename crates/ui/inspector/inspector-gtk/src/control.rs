use super::{InspectorContext, section::InspectorSection};
use adw::prelude::*;
use shrimply_components_gtk::tr;
use shrimply_inspector_core::{ControlKind, ControlRowRole, InspectorControl, InspectorTarget};
mod expression;
pub(crate) mod layered;
mod value;

pub(super) fn section(
    model: &shrimply_inspector_core::InspectorSection,
    context: &InspectorContext,
) -> Vec<gtk::Widget> {
    let section = InspectorSection::controls();
    let mut information: Option<adw::PreferencesGroup> = None;
    let mut controls = model
        .controls
        .iter()
        .filter(|control| control.visible)
        .peekable();
    while let Some(control) = controls.next() {
        if let Some(group) = control.row_group {
            information = None;
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let mut grouped = vec![control];
            while controls
                .peek()
                .is_some_and(|control| control.row_group == Some(group))
            {
                grouped.push(controls.next().expect("matching grouped control"));
            }
            for control in grouped {
                let native = InspectorSection::controls();
                let mut control = control.clone();
                let primary = control.row_role == ControlRowRole::Primary;
                if control.row_role == ControlRowRole::Auxiliary {
                    control.tooltip = control.label.clone();
                    control.label.clear();
                }
                add_control(&native, context, &context.list_target, control);
                let native = native.into_widget();
                native.set_hexpand(primary);
                row.append(&native);
            }
            section.add_wide_control(&row);
            continue;
        }
        match control.kind {
            ControlKind::InfoHeading => {
                let group = adw::PreferencesGroup::builder()
                    .title(tr!(&control.label).as_ref())
                    .build();
                section.add_wide_control(&group);
                information = Some(group);
            }
            ControlKind::ReadOnly if control.preview_focus.is_none() => {
                let group = information.get_or_insert_with(|| {
                    let group = adw::PreferencesGroup::new();
                    section.add_wide_control(&group);
                    group
                });
                group.add(&super::info::property_row(&control.label, &control.value));
            }
            _ => {
                information = None;
                add_control(&section, context, &context.list_target, control.clone());
            }
        }
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
        ControlKind::Action => value::action(section, context, target, control),
        ControlKind::AudioCache | ControlKind::VisualCache | ControlKind::Analysis => {
            section.add_wide_control(&super::progress::widget(control, context));
        }
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
            let group = adw::PreferencesGroup::new();
            group.add(&super::info::property_row(&control.label, &control.value));
            section.add_wide_control(&group);
        }
        ControlKind::TtsEditor
        | ControlKind::BeatDetection
        | ControlKind::AudioModifierMenu
        | ControlKind::VisualModifierMenu => {
            section.add_wide_control(&super::audio::control(&control, context));
        }
        ControlKind::FontFamilies => value::fonts(section, context, target, control),
        ControlKind::LayeredDrawing => value::drawing(section, context, target, control),
        ControlKind::Performance => section.add_wide_control(&super::benchmarking::widget()),
        ControlKind::ProjectSettings => super::project::settings(section, context, control),
        ControlKind::InfoHeading => unreachable!("information heading handled by section"),
        ControlKind::FileLocation => {
            section.add_wide_control(&super::info::file_row(control.value.into(), &control.label))
        }
        ControlKind::InfoLoading => {
            let group = adw::PreferencesGroup::new();
            let row = adw::ActionRow::builder()
                .title(tr!(&control.label).as_ref())
                .subtitle(tr!(&control.value).as_ref())
                .build();
            row.add_suffix(&adw::Spinner::new());
            group.add(&row);
            section.add_wide_control(&group);
        }
        ControlKind::InfoArtwork => {
            if let Some(bytes) = control.image_bytes
                && let Some(group) = super::info::artwork(bytes, &control.label)
            {
                section.add_wide_control(&group);
            }
        }
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
