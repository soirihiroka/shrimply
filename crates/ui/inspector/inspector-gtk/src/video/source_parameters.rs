//! Shared reflected parameter widgets. Type checking and storage stay in inspector-core.
use super::super::{
    InspectorContext,
    item::{DefaultInspectorItem, InspectorListItem},
    section::InspectorSection,
};
use gtk::prelude::*;
use shrimply_components_gtk::ui::{ColorPicker, NumberPicker, control_row, switch_row};
use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorController, InspectorTarget, VideoCard,
    manim_parameters::ManimParametersReset,
};
use shrimply_project_document::project::Color;

pub(super) fn manim_item(card: VideoCard, reset: ManimParametersReset) -> InspectorListItem {
    item(card, move |context, _| {
        if let Some(target) = target(context) {
            report(
                &context.category_bar,
                context
                    .inspector_core
                    .reset_manim_parameters(&target, &reset),
            );
        }
    })
}
pub(super) fn sanim_item(card: VideoCard) -> InspectorListItem {
    item(card, |context, card| {
        if let (Some(target), Some(reset)) = (target(context), &card.reset) {
            report(
                &context.category_bar,
                context.inspector_core.reset_video(&target, reset),
            );
        }
    })
}
fn target(context: &InspectorContext) -> Option<InspectorTarget> {
    context.selected_item.clone().map(InspectorTarget::Item)
}
fn item(
    card: VideoCard,
    reset: impl Fn(&InspectorContext, &VideoCard) + 'static,
) -> InspectorListItem {
    let default = card.clone();
    DefaultInspectorItem::new_with_default(
        card.key,
        card.title,
        card,
        controls,
        move |_| default.clone(),
        move |context, card| reset(context, &card),
    )
    .boxed()
}
fn controls(card: &VideoCard, context: &InspectorContext) -> Vec<gtk::Widget> {
    let section = InspectorSection::controls();
    let Some(target) = target(context) else {
        return vec![section.into_widget()];
    };
    for control in &card.section.controls {
        match control.kind {
            ControlKind::Number => add_number(&section, context, &target, control),
            ControlKind::Fraction => add_fraction(&section, context, &target, control),
            ControlKind::Color => add_color(&section, context, &target, control),
            ControlKind::Selector => add_selector(&section, context, &target, control),
            ControlKind::Boolean => add_boolean(&section, context, &target, control),
            ControlKind::Text => add_text(&section, context, &target, control),
            ControlKind::ReadOnly => {
                let label = gtk::Label::builder()
                    .label(&control.value)
                    .wrap(true)
                    .xalign(0.)
                    .selectable(true)
                    .build();
                section.add_wide_control(&label);
            }
            _ => panic!("unsupported reflected parameter control kind"),
        }
    }
    vec![section.into_widget()]
}
fn add_number(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: &InspectorControl,
) {
    let value = control
        .value
        .parse::<f64>()
        .expect("reflected number must be numeric");
    let picker = NumberPicker::builder(value)
        .minimum(control.number.minimum)
        .maximum(control.number.maximum)
        .drag_step(control.number.drag_step)
        .unit_name(&control.suffix)
        .digits(
            usize::try_from(control.number.digits).expect("numeric digits must be nonnegative"),
        );
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let edited = control.clone();
    let feedback = context.category_bar.clone();
    let picker = if control.integer {
        picker.on_commit_integer(move |value: i64| {
            set_text(&feedback, &controller, &target, &edited, &value.to_string())
        })
    } else {
        picker.on_commit(move |value| {
            set_text(&feedback, &controller, &target, &edited, &value.to_string())
        })
    };
    let picker = picker.build();
    picker.set_tooltip_text(Some(&control.tooltip));
    section.add_wide_control(&control_row(&control.label, &picker));
}
fn add_fraction(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: &InspectorControl,
) {
    let [numerator, denominator] = control.components.as_slice() else {
        panic!("reflected fraction needs numerator and denominator")
    };
    let value = shrimply_math_core::fraction_new(
        numerator.parse().expect("fraction numerator"),
        denominator.parse().expect("fraction denominator"),
    );
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let edited = control.clone();
    let feedback = context.category_bar.clone();
    let picker = NumberPicker::fraction_builder(value)
        .minimum(control.number.minimum)
        .maximum(control.number.maximum)
        .drag_step(control.number.drag_step)
        .unit_name(&control.suffix)
        .digits(usize::try_from(control.number.digits).expect("numeric digits must be nonnegative"))
        .on_commit_fraction(move |value| {
            report(
                &feedback,
                controller.set_basic_control_fraction(&target, &edited, value),
            );
        })
        .build();
    picker.set_tooltip_text(Some(&control.tooltip));
    section.add_wide_control(&control_row(&control.label, &picker));
}
fn add_color(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: &InspectorControl,
) {
    let [red, green, blue, alpha] = control.components.as_slice() else {
        panic!("reflected color needs four channels")
    };
    let channel = |value: &String| value.parse::<u8>().expect("reflected color channel");
    let color = Color::new(channel(red), channel(green), channel(blue), channel(alpha));
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let edited = control.clone();
    let feedback = context.category_bar.clone();
    let picker = ColorPicker::builder(color)
        .title(&control.label)
        .with_alpha(control.with_alpha)
        .hexpand(true)
        .on_change(move |value| {
            report(
                &feedback,
                controller.set_basic_control_components(
                    &target,
                    &edited,
                    &[
                        f64::from(value.r),
                        f64::from(value.g),
                        f64::from(value.b),
                        f64::from(value.a),
                    ],
                ),
            );
        })
        .build();
    picker.set_tooltip_text(Some(&control.tooltip));
    section.add_wide_control(&control_row(&control.label, &picker));
}
fn add_selector(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: &InspectorControl,
) {
    assert_eq!(
        control.values.len(),
        control.labels.len(),
        "reflected selector values need labels"
    );
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let edited = control.clone();
    let feedback = context.category_bar.clone();
    let selector = shrimply_components_gtk::ui::string_selector(
        &control.label,
        &control.value,
        control.values.clone(),
        move |value| set_text(&feedback, &controller, &target, &edited, &value),
    );
    selector.widget().set_tooltip_text(Some(&control.tooltip));
    section.add_wide_control(selector.widget());
}
fn add_boolean(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: &InspectorControl,
) {
    let value = control.value.parse::<bool>().expect("reflected boolean");
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let edited = control.clone();
    let feedback = context.category_bar.clone();
    let row = switch_row(&control.label, None, value, move |value| {
        set_text(&feedback, &controller, &target, &edited, &value.to_string())
    });
    row.set_tooltip_text(Some(&control.tooltip));
    section.add_wide_control(&row);
}
fn add_text(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: &InspectorControl,
) {
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let edited = control.clone();
    let feedback = context.category_bar.clone();
    let input = shrimply_components_gtk::ui::SingleLineTextInput::builder(&control.value)
        .on_commit(move |value| set_text(&feedback, &controller, &target, &edited, &value))
        .build();
    input.set_tooltip_text(Some(&control.tooltip));
    section.add_wide_control(&control_row(&control.label, &input));
}
fn set_text(
    feedback: &gtk::Box,
    controller: &InspectorController,
    target: &InspectorTarget,
    control: &InspectorControl,
    text: &str,
) {
    report(
        feedback,
        controller.set_video_field(
            target,
            &control.path,
            text,
            &control.commit_name,
            control.commit_immediately,
        ),
    );
}
fn report(feedback: &gtk::Box, result: Result<(), String>) {
    if let Err(error) = result {
        tracing::error!(%error,"Could not update reflected parameter");
        shrimply_components_gtk::toast::show_confirmation_text_for_widget(feedback, &error);
    }
}
