use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use shrimply_components_gtk::{
    tr,
    ui::{
        ColorPicker, MultilineTextInput, Number2Picker, Number3Picker, StringChoice,
        labeled_string_selector,
    },
};
use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorController, InspectorTarget,
    rhai_editor::ExpressionValue,
};
use shrimply_project_document::project::Color;

use crate::{InspectorContext, player_state, section::InspectorSection};

const LAYERED_TEXT_MIN_CONTENT_HEIGHT: i32 = 86;

pub(super) fn boolean(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: InspectorControl,
) {
    let toggle = gtk::Switch::builder()
        .active(control.value.parse().expect("boolean presentation"))
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .build();
    let updating = Rc::new(Cell::new(false));
    let edited = control.clone();
    let controller = context.inspector_core.clone();
    let edit_target = target.clone();
    let changing = updating.clone();
    toggle.connect_active_notify(move |toggle| {
        if !changing.get() {
            report(
                controller
                    .set_basic_control_value(&edit_target, &edited, &toggle.is_active().to_string())
                    .and_then(|_| controller.commit_basic_control(&edit_target, &edited)),
            );
        }
    });
    if control.kind == ControlKind::LayeredBoolean {
        let display = toggle.downgrade();
        listen(
            context,
            target,
            &control,
            move |controller, target, control| {
                if let Some(display) = display.upgrade()
                    && let Ok(value) = controller.bool_expression_output(target, path(control))
                {
                    updating.set(true);
                    display.set_active(value.value);
                    updating.set(false);
                }
            },
        );
    }
    if control.kind == ControlKind::Boolean && !control.subtitle.is_empty() {
        let row = adw::ActionRow::builder()
            .title(tr!(&control.label).as_ref())
            .subtitle(tr!(&control.subtitle).as_ref())
            .sensitive(control.editable && control.sensitive)
            .build();
        row.add_suffix(&toggle);
        row.set_activatable_widget(Some(&toggle));
        if !control.tooltip.is_empty() {
            row.set_tooltip_text(Some(tr!(&control.tooltip).as_ref()));
        }
        section.add_wide_control(&row);
    } else {
        attach(section, context, target, control, toggle.upcast());
    }
}

pub(super) fn selector(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: InspectorControl,
) {
    assert_eq!(
        control.values.len(),
        control.labels.len(),
        "selector choices need labels"
    );
    let choices: Vec<_> = control
        .values
        .iter()
        .cloned()
        .zip(&control.labels)
        .map(|(value, label)| StringChoice {
            value,
            label: tr!(label).into_owned(),
        })
        .collect();
    let controller = context.inspector_core.clone();
    let edited = control.clone();
    let edit_target = target.clone();
    let layered = control.kind == ControlKind::LayeredSelector;
    let changed = move |value: String| {
        report(
            controller
                .set_basic_control_value(&edit_target, &edited, &value)
                .and_then(|_| controller.commit_basic_control(&edit_target, &edited)),
        );
    };
    if !control.icons.is_empty() {
        assert_eq!(
            control.icons.len(),
            choices.len(),
            "icon choices need icons"
        );
        let group = adw::ToggleGroup::builder()
            .halign(gtk::Align::End)
            .homogeneous(true)
            .build();
        for (choice, icon) in choices.iter().zip(&control.icons) {
            group.add(
                adw::Toggle::builder()
                    .name(&choice.value)
                    .icon_name(icon)
                    .tooltip(&choice.label)
                    .build(),
            );
        }
        group.set_active(
            choices
                .iter()
                .position(|choice| choice.value == control.value)
                .expect("selected icon choice exists")
                .try_into()
                .expect("choice index fits u32"),
        );
        let updating = Rc::new(Cell::new(false));
        let changing = updating.clone();
        let values = control.values.clone();
        group.connect_active_notify(move |group| {
            if !changing.get()
                && let Some(value) = values.get(group.active() as usize)
            {
                changed(value.clone());
            }
        });
        if layered {
            let display = group.downgrade();
            listen(
                context,
                target,
                &control,
                move |controller, target, control| {
                    if let Some(display) = display.upgrade()
                        && let Ok(value) = controller.step_expression_output(
                            target,
                            path(control),
                            control.timeline_id,
                        )
                        && let Some(index) = control
                            .values
                            .iter()
                            .position(|choice| *choice == value.value)
                    {
                        let index = index.try_into().expect("choice index fits u32");
                        if display.active() != index {
                            updating.set(true);
                            display.set_active(index);
                            updating.set(false);
                        }
                    }
                },
            );
        }
        attach(section, context, target, control, group.upcast());
        return;
    }
    let selector = labeled_string_selector(
        if layered { "" } else { &control.label },
        &control.value,
        choices.clone(),
        changed,
    );
    selector.set_sensitive(control.editable && control.sensitive);
    if layered {
        let display = selector.clone();
        let displayed = RefCell::new(control.value.clone());
        listen(
            context,
            target,
            &control,
            move |controller, target, control| {
                if let Ok(value) =
                    controller.step_expression_output(target, path(control), control.timeline_id)
                    && *displayed.borrow() != value.value
                {
                    display.set_choices(&value.value, choices.clone());
                    *displayed.borrow_mut() = value.value;
                }
            },
        );
        attach(section, context, target, control, selector.widget().clone());
    } else {
        section.add_wide_control(selector.widget());
    }
}

pub(super) fn color(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: InspectorControl,
) {
    let values: Vec<u8> = control
        .components
        .iter()
        .map(|value| value.parse().expect("color component"))
        .collect();
    let [red, green, blue, alpha] = values.as_slice() else {
        panic!("color presentation needs four components")
    };
    let controller = context.inspector_core.clone();
    let edited = control.clone();
    let edit_target = target.clone();
    let picker = ColorPicker::builder(Color::from_rgba(*red, *green, *blue, *alpha))
        .title(
            tr!(if control.tooltip.is_empty() {
                &control.label
            } else {
                &control.tooltip
            })
            .as_ref(),
        )
        .with_alpha(control.with_alpha)
        .hexpand(true)
        .on_change(move |color| {
            report(
                controller
                    .set_basic_control_components(
                        &edit_target,
                        &edited,
                        &color.to_array().map(f64::from),
                    )
                    .and_then(|_| controller.commit_basic_control(&edit_target, &edited)),
            );
        })
        .build_with_handle();
    if control.kind == ControlKind::LayeredColor {
        let display = picker.handle.downgrade();
        listen(
            context,
            target,
            &control,
            move |controller, target, control| {
                if let Ok(value) = controller.color_expression_output(
                    target,
                    path(control),
                    control.timeline_id.expect("color timeline"),
                ) {
                    display.set_color(value.value);
                }
            },
        );
    }
    attach(section, context, target, control, picker.widget);
}

pub(super) fn text(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: InspectorControl,
) {
    let controller = context.inspector_core.clone();
    let edited = control.clone();
    let edit_target = target.clone();
    let commit_controller = controller.clone();
    let commit_control = control.clone();
    let commit_target = target.clone();
    let mut editor = MultilineTextInput::builder(&control.value);
    if control.kind == ControlKind::LayeredText {
        editor = editor.min_content_height(LAYERED_TEXT_MIN_CONTENT_HEIGHT);
    }
    let editor = editor
        .on_change(move |value| {
            report(controller.set_basic_control_value(&edit_target, &edited, &value))
        })
        .on_commit(move || {
            report(commit_controller.commit_basic_control(&commit_target, &commit_control));
        })
        .build();
    if control.kind == ControlKind::LayeredText {
        let display = editor.set_text_handler();
        listen(
            context,
            target,
            &control,
            move |controller, target, control| {
                if let Ok(value) = controller.text_expression_output(
                    target,
                    path(control),
                    control.timeline_id.expect("text timeline"),
                ) {
                    display(&value.value);
                }
            },
        );
    }
    attach(section, context, target, control, editor.widget().clone());
}

pub(super) fn vector(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: InspectorControl,
) {
    let values: Vec<f64> = control
        .components
        .iter()
        .map(|value| value.parse().expect("vector component"))
        .collect();
    let current = Rc::new(RefCell::new(values.clone()));
    let digits = control
        .number
        .digits
        .try_into()
        .expect("nonnegative digits");
    let (widget, handles) = match values.as_slice() {
        [x, y] => {
            let mut picker = Number2Picker::builder(*x, *y)
                .minimum(control.number.minimum)
                .maximum(control.number.maximum)
                .drag_step(control.number.drag_step)
                .digits(digits)
                .width_chars(control.width_characters)
                .unit_name(control.number.unit);
            if let [x, y] = control.prefixes.as_slice() {
                picker = picker.first_prefix(x).second_prefix(y);
            }
            if control.lock {
                picker = picker.enable_lock();
            }
            let controller = context.inspector_core.clone();
            let edited = control.clone();
            let edit_target = target.clone();
            picker = picker.on_change(move |values, _| {
                report(controller.set_basic_control_components(&edit_target, &edited, &values));
            });
            let controller = context.inspector_core.clone();
            let edited = control.clone();
            let edit_target = target.clone();
            picker = picker.on_first_commit(move |_| {
                report(controller.commit_basic_control(&edit_target, &edited));
            });
            let controller = context.inspector_core.clone();
            let edited = control.clone();
            let edit_target = target.clone();
            let parts = picker
                .on_second_commit(move |_| {
                    report(controller.commit_basic_control(&edit_target, &edited));
                })
                .build_with_handles();
            (
                parts.widget,
                vec![parts.first.downgrade(), parts.second.downgrade()],
            )
        }
        [x, y, z] => {
            let mut picker = Number3Picker::builder(*x, *y, *z)
                .minimum(control.number.minimum)
                .drag_step(control.number.drag_step)
                .digits(digits)
                .width_chars(control.width_characters)
                .unit_name(control.number.unit);
            if let [x, y, z] = control.prefixes.as_slice() {
                picker = picker.prefixes([x, y, z]);
            }
            if control.lock {
                picker = picker.enable_lock();
            }
            for axis in 0..values.len() {
                let controller = context.inspector_core.clone();
                let edited = control.clone();
                let edit_target = target.clone();
                let current = current.clone();
                picker = picker.on_change(axis, move |value| {
                    current.borrow_mut()[axis] = value;
                    let values = current.borrow().clone();
                    report(controller.set_basic_control_components(&edit_target, &edited, &values));
                });
                let controller = context.inspector_core.clone();
                let edited = control.clone();
                let edit_target = target.clone();
                picker = picker.on_commit(axis, move |_| {
                    report(controller.commit_basic_control(&edit_target, &edited));
                });
            }
            let parts = picker.build_with_handles();
            (
                parts.widget,
                parts
                    .handles
                    .iter()
                    .map(|handle| handle.downgrade())
                    .collect(),
            )
        }
        _ => panic!("vector presentation needs two or three components"),
    };
    if matches!(
        control.kind,
        ControlKind::LayeredVector2 | ControlKind::LayeredVector3
    ) {
        listen(
            context,
            target,
            &control,
            move |controller, target, control| {
                let values = match control.kind {
                    ControlKind::LayeredVector2 => controller
                        .vector2_expression_output(target, path(control), control.timeline_id)
                        .map(|output| output.value.to_array().to_vec()),
                    _ => controller
                        .vector3_expression_output(
                            target,
                            path(control),
                            control.timeline_id.expect("vector timeline"),
                        )
                        .map(|output| output.value.to_array().to_vec()),
                };
                if let Ok(values) = values {
                    *current.borrow_mut() = values
                        .iter()
                        .map(|value| f64::from(*value) / control.store_multiplier)
                        .collect();
                    for (handle, value) in handles.iter().zip(values) {
                        if let Some(handle) = handle.upgrade() {
                            handle.set_f64(f64::from(value) / control.store_multiplier);
                        }
                    }
                }
            },
        );
    }
    attach(section, context, target, control, widget);
}

fn attach(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: InspectorControl,
    editor: gtk::Widget,
) {
    editor.set_sensitive(control.editable && control.sensitive);
    let tooltip = if control.tooltip.is_empty() {
        &control.subtitle
    } else {
        &control.tooltip
    };
    if !tooltip.is_empty() {
        editor.set_tooltip_text(Some(tr!(tooltip).as_ref()));
    }
    if matches!(
        control.kind,
        ControlKind::LayeredBoolean
            | ControlKind::LayeredSelector
            | ControlKind::LayeredColor
            | ControlKind::LayeredText
            | ControlKind::LayeredVector2
            | ControlKind::LayeredVector3
    ) {
        let expression = control
            .layered
            .expression
            .then(|| expression(context, target, &control));
        section.add_wide_control(&super::layered::wrap(
            context, target, control, editor, expression,
        ));
    } else {
        section.add_control_row(&control.label, &editor);
    }
}

fn expression(
    context: &InspectorContext,
    target: &InspectorTarget,
    control: &InspectorControl,
) -> gtk::Widget {
    let kind = match control.kind {
        ControlKind::LayeredBoolean => ExpressionValue::Bool,
        ControlKind::LayeredSelector => ExpressionValue::Step,
        ControlKind::LayeredColor => ExpressionValue::Color,
        ControlKind::LayeredText => ExpressionValue::Text,
        ControlKind::LayeredVector2 => ExpressionValue::Vec2,
        ControlKind::LayeredVector3 => ExpressionValue::Vec3,
        _ => unreachable!("layered value expression"),
    };
    let controller = context.inspector_core.clone();
    let source = control.clone();
    let source_target = target.clone();
    let output_controller = controller.clone();
    let output = control.clone();
    let output_target = target.clone();
    crate::timeline_value::expression_section(
        context,
        "native value expression",
        move |refresh| {
            crate::rhai_editor::editor(
                Some(source.layered.expression_source.clone()),
                kind,
                move |value| {
                    report(controller.set_control_expression_source(
                        &source_target,
                        &source,
                        &value,
                    ));
                    refresh();
                },
            )
        },
        move |_, _, _, _| {
            let path = path(&output);
            let (value, error) = match output.kind {
                ControlKind::LayeredBoolean => {
                    let value = output_controller
                        .bool_expression_output(&output_target, path)
                        .ok()?;
                    (value.value.to_string(), value.error)
                }
                ControlKind::LayeredSelector => {
                    let value = output_controller
                        .step_expression_output(&output_target, path, output.timeline_id)
                        .ok()?;
                    (value.value, value.error)
                }
                ControlKind::LayeredText => {
                    let value = output_controller
                        .text_expression_output(&output_target, path, output.timeline_id?)
                        .ok()?;
                    (value.value, value.error)
                }
                ControlKind::LayeredColor => {
                    let value = output_controller
                        .color_expression_output(&output_target, path, output.timeline_id?)
                        .ok()?;
                    (
                        shrimply_components_core::color::color_hex(value.value, true),
                        value.error,
                    )
                }
                ControlKind::LayeredVector2 => {
                    let value = output_controller
                        .vector2_expression_output(&output_target, path, output.timeline_id)
                        .ok()?;
                    let [x, y] = output.prefixes.as_slice() else {
                        panic!("vector expression needs two prefixes")
                    };
                    (
                        shrimply_inspector_core::timeline_value::vector::vec2::format_value(
                            value.value,
                            x,
                            y,
                            output.number.digits.try_into().expect("nonnegative digits"),
                            output.number.unit,
                        ),
                        value.error,
                    )
                }
                ControlKind::LayeredVector3 => {
                    let value = output_controller
                        .vector3_expression_output(&output_target, path, output.timeline_id?)
                        .ok()?;
                    let [x, y, z] = output.prefixes.as_slice() else {
                        panic!("vector expression needs three prefixes")
                    };
                    (
                        shrimply_inspector_core::timeline_value::vector::vec3::format_value(
                            value.value,
                            [x, y, z],
                            output.number.digits.try_into().expect("nonnegative digits"),
                            output.number.unit,
                        ),
                        value.error,
                    )
                }
                _ => unreachable!("layered value expression"),
            };
            Some(crate::timeline_value::ExpressionOutput { value, error })
        },
    )
}

fn listen(
    context: &InspectorContext,
    target: &InspectorTarget,
    control: &InspectorControl,
    update: impl Fn(&InspectorController, &InspectorTarget, &InspectorControl) + 'static,
) {
    let alive = Rc::downgrade(&context.listener_scope);
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let control = control.clone();
    player_state::connect_while_alive_named(
        &context.player_state,
        "native value display",
        move || alive.upgrade().is_some(),
        move |_| update(&controller, &target, &control),
    );
}

fn path(control: &InspectorControl) -> &str {
    control.timeline_path.as_deref().unwrap_or(&control.path)
}

fn report<T>(result: Result<T, String>) -> bool {
    match result {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error,"Could not edit inspector value");
            false
        }
    }
}
