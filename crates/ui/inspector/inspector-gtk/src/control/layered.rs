use std::{cell::RefCell, rc::Rc};

use gtk::{gio, prelude::*};
use shrimply_components_core::layered::LayeredPropertyController;
use shrimply_components_gtk::{
    tr,
    ui::{FrameGraph, InspectorLayeredProperty, NumberPicker},
};
use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorController, InspectorTarget,
    keyframe_graph::FrameGraphAction,
};

use crate::{InspectorContext, player_state, section::InspectorSection};

pub(super) fn fraction(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: InspectorControl,
) {
    let [numerator, denominator] = control.components.as_slice() else {
        panic!("fraction control needs exact components");
    };
    let value = format!("{numerator}/{denominator}")
        .parse()
        .expect("valid exact fraction presentation");
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let commit_controller = controller.clone();
    let commit_target = target.clone();
    let commit_control = control.clone();
    let mut picker = NumberPicker::fraction_builder(value)
        .drag_step(control.number.drag_step)
        .digits(
            control
                .number
                .digits
                .try_into()
                .expect("nonnegative digits"),
        )
        .width_chars(control.width_characters);
    if !control.number.unit.is_empty() {
        picker = picker.unit_name(control.number.unit);
    }
    let label = control.label.clone();
    let picker = picker
        .on_change_fraction(move |value| {
            report(controller.set_basic_control_fraction(&target, &control, value));
        })
        .on_commit(move |_| {
            report(commit_controller.commit_basic_control(&commit_target, &commit_control));
        })
        .build();
    section.add_control_row(&label, &picker);
}

pub(super) fn number(
    section: &InspectorSection,
    context: &InspectorContext,
    target: &InspectorTarget,
    control: InspectorControl,
) {
    let current = control.value.parse().expect("numeric layered presentation");
    let controller = context.inspector_core.clone();
    let edit_target = target.clone();
    let commit_controller = controller.clone();
    let commit_target = target.clone();
    let commit_control = control.clone();
    let update_control = control.clone();
    let mut builder = NumberPicker::builder(current)
        .minimum(control.number.minimum)
        .maximum(control.number.maximum)
        .drag_step(control.number.drag_step)
        .width_chars(control.width_characters)
        .digits(
            control
                .number
                .digits
                .try_into()
                .expect("nonnegative digits"),
        );
    if !control.number.unit.is_empty() {
        builder = builder.unit_name(control.number.unit);
    }
    if control.prefix_icon_rotates {
        builder = builder.rotating_prefix_icon_name_with_offset(
            control.prefix_icon.clone(),
            control.prefix_icon_rotation_offset_degrees,
        );
    }
    let picker = builder
        .on_change(move |value| {
            report(controller.set_basic_control_value(
                &edit_target,
                &update_control,
                &value.to_string(),
            ));
        })
        .on_commit(move |_| {
            report(commit_controller.commit_basic_control(&commit_target, &commit_control));
        })
        .build_with_handle();

    picker
        .widget
        .set_sensitive(control.editable && control.sensitive);
    let tooltip = if control.tooltip.is_empty() {
        &control.subtitle
    } else {
        &control.tooltip
    };
    if !tooltip.is_empty() {
        picker.widget.set_tooltip_text(Some(tr!(tooltip).as_ref()));
    }
    if control.kind == ControlKind::Number {
        section.add_control_row(&control.label, &picker.widget);
        return;
    }
    let display_controller = context.inspector_core.clone();
    let display_target = target.clone();
    let display_control = control.clone();
    let display = picker.handle.downgrade();
    let alive = Rc::downgrade(&context.listener_scope);
    player_state::connect_while_alive_named(
        &context.player_state,
        "native scalar display",
        move || alive.upgrade().is_some(),
        move |_| {
            if let Some(display) = display.upgrade()
                && let Ok(value) =
                    expression_output(&display_controller, &display_target, &display_control)
            {
                display.set_f64(display_control.display_number(value.value));
            }
        },
    );
    let expression = control
        .layered
        .expression
        .then(|| scalar_expression(context, target, &control));
    section.add_wide_control(&wrap(context, target, control, picker.widget, expression));
}

pub(crate) fn wrap(
    context: &InspectorContext,
    target: &InspectorTarget,
    control: InspectorControl,
    editor: gtk::Widget,
    expression: Option<gtk::Widget>,
) -> gtk::Widget {
    let state = LayeredPropertyController::default();
    state.set_keyframes(control.layered.keyframes);
    state.set_expression(control.layered.expression);
    let mut property =
        InspectorLayeredProperty::builder(tr!(&control.label).as_ref(), &editor, state);
    if matches!(
        control.kind,
        ControlKind::LayeredText | ControlKind::LayeredDrawing
    ) {
        property = property.wide();
    }
    if control.layered.keyframes {
        property = property.keyframe_section(&graph(context, target, &control));
    }
    if let Some(expression) = expression {
        property = property.expression_section(&expression);
    }
    let keyframe_controller = context.inspector_core.clone();
    let keyframe_target = target.clone();
    let keyframe_control = control.clone();
    let expression_controller = context.inspector_core.clone();
    let expression_target = target.clone();
    property
        .on_keyframes_changed(move |enabled| {
            report(keyframe_controller.set_control_keyframes(
                &keyframe_target,
                &keyframe_control,
                enabled,
            ));
        })
        .on_expression_changed(move |enabled| {
            report(expression_controller.set_control_expression(
                &expression_target,
                &control,
                enabled,
            ));
        })
        .build()
        .widget()
        .clone()
}

fn graph(
    context: &InspectorContext,
    target: &InspectorTarget,
    control: &InspectorControl,
) -> gtk::Widget {
    let initial = shrimply_inspector_core::document::graph::frame_graph(control)
        .expect("keyframed control has graph metadata");
    let scope = format!(
        "inspector:{target:?}:{}:{:?}:{:?}",
        control.path, control.target_id, control.timeline_id
    );
    let initial_graph = initial.graph().clone();
    let state = context.keyframe_graph_state(scope, initial);
    let source = control
        .scalar_graph
        .as_ref()
        .expect("keyframed control has a graph");
    state.replace_active_graph(initial_graph);
    state.set_view(
        source.range,
        source.frame_step,
        source.playhead,
        crate::keyframe_model::graph_snapping(&context.preferences),
        true,
        control.kind == shrimply_inspector_core::ControlKind::LayeredText,
    );
    let controller = context.inspector_core.clone();
    let action_target = target.clone();
    let action_control = control.clone();
    let area = Rc::new(RefCell::new(None::<gtk::glib::WeakRef<gtk::GLArea>>));
    let action_area = area.clone();
    let graph = FrameGraph::with_shared_components(state.clone(), move |action| {
        let action = action.action;
        let Some(area) = action_area
            .borrow()
            .as_ref()
            .and_then(gtk::glib::WeakRef::upgrade)
        else {
            return;
        };
        if let FrameGraphAction::TextInterpolationRequested { owner_id, x, y } = action {
            let Ok(menu) = controller.control_text_interpolation_menu(
                &action_target,
                &action_control,
                owner_id,
            ) else {
                return;
            };
            let controller = controller.clone();
            let target = action_target.clone();
            let control = action_control.clone();
            let popover = shrimply_components_gtk::ui::searchable_popover(
                tr!("Search interpolations").as_ref(),
                280,
                180,
                240,
                move |query| {
                    menu.values
                        .iter()
                        .zip(&menu.labels)
                        .filter(|(_, label)| {
                            shrimply_components_gtk::ui::matches_query(label, query)
                        })
                        .map(|(value, label)| {
                            let selected = value == &menu.value;
                            let index =
                                value.parse().expect("text interpolation option is numeric");
                            let controller = controller.clone();
                            let target = target.clone();
                            let control = control.clone();
                            shrimply_components_gtk::ui::SearchMenuItem::new(
                                tr!(label).as_ref(),
                                move || {
                                    report(controller.set_control_text_interpolation(
                                        &target, &control, owner_id, index,
                                    ));
                                },
                            )
                            .selected(selected)
                        })
                        .collect()
                },
            );
            popover.set_parent(&area);
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.connect_closed(|popover| popover.unparent());
            popover.popup();
        } else if matches!(action, FrameGraphAction::PasteRequested(_)) {
            let controller = controller.clone();
            let target = action_target.clone();
            let control = action_control.clone();
            let FrameGraphAction::PasteRequested(time) = action else {
                unreachable!();
            };
            let toast_area = area.clone();
            area.display()
                .clipboard()
                .read_text_async(None::<&gio::Cancellable>, move |result| {
                    if result.ok().flatten().as_deref()
                        == Some(crate::keyframe_model::KEYFRAME_CLIPBOARD_MARKER)
                        && let Ok(count) =
                            controller.paste_control_keyframes(&target, &control, time)
                    {
                        confirmation(&toast_area, count, true);
                    }
                });
        } else if let FrameGraphAction::CopyRequested(times) = action {
            if let Ok(count) =
                controller.copy_control_keyframes(&action_target, &action_control, &times)
            {
                if count > 0 {
                    area.display()
                        .clipboard()
                        .set_text(crate::keyframe_model::KEYFRAME_CLIPBOARD_MARKER);
                }
                confirmation(&area, count, false);
            }
        } else {
            report(controller.apply_control_graph_action(&action_target, &action_control, action));
        }
    });
    area.replace(Some(graph.graph_area().downgrade()));
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let control = control.clone();
    let refreshed = graph.clone();
    let preferences = context.preferences.clone();
    let project = context.project.clone();
    let selected = context.selected_item.clone();
    let player = context.player_state.clone();
    let alive = Rc::downgrade(&context.listener_scope);
    player_state::connect_while_alive_named(
        &context.player_state,
        "native inspector graph",
        move || alive.upgrade().is_some(),
        move |event| {
            if !refreshed.graph_area().is_mapped() {
                return;
            }
            if matches!(event, player_state::PlayerEvent::State(_)) {
                let position = player_state::snapshot(&player).position;
                let local = selected
                    .as_ref()
                    .and_then(|key| project.borrow().keyframe_time(key, position))
                    .unwrap_or(position);
                refreshed.set_playhead(local);
            } else if let Ok(Some(source)) = controller.control_graph(&target, &control) {
                let mut control = control.clone();
                control.scalar_graph = Some(source.clone());
                let next = shrimply_inspector_core::document::graph::frame_graph(&control)
                    .expect("current keyframe graph");
                state.replace_active_graph(next.graph().clone());
                state.set_view(
                    source.range,
                    source.frame_step,
                    source.playhead,
                    crate::keyframe_model::graph_snapping(&preferences),
                    true,
                    control.kind == shrimply_inspector_core::ControlKind::LayeredText,
                );
                refreshed.refresh();
            }
        },
    );
    graph.widget().clone().upcast()
}

fn expression_output(
    controller: &InspectorController,
    target: &InspectorTarget,
    control: &InspectorControl,
) -> Result<shrimply_inspector_core::InspectorExpressionOutput<f64>, String> {
    let path = control.timeline_path.as_deref().unwrap_or(&control.path);
    if control.scalar_storage == shrimply_inspector_core::ScalarStorage::UnsignedInteger {
        let output = controller.background_integer_expression_output(
            target,
            path,
            control.timeline_id.expect("integer timeline"),
        )?;
        return Ok(shrimply_inspector_core::InspectorExpressionOutput {
            value: f64::from(output.value),
            error: output.error,
        });
    }
    let output = if control.audio_modifier {
        controller.audio_modifier_expression_output(
            target,
            control.target_id.expect("audio modifier owner"),
            control.timeline_id.expect("audio modifier timeline"),
        )?
    } else {
        controller.scalar_expression_output(target, path, control.timeline_id)?
    };
    Ok(shrimply_inspector_core::InspectorExpressionOutput {
        value: f64::from(output.value),
        error: output.error,
    })
}

fn scalar_expression(
    context: &InspectorContext,
    target: &InspectorTarget,
    control: &InspectorControl,
) -> gtk::Widget {
    let controller = context.inspector_core.clone();
    let target = target.clone();
    let source_control = control.clone();
    let output_control = control.clone();
    let output_controller = controller.clone();
    let output_target = target.clone();
    crate::timeline_value::expression_section(
        context,
        "native scalar expression",
        move |refresh| {
            crate::rhai_editor::editor(
                Some(source_control.layered.expression_source.clone()),
                crate::rhai_editor::ExpressionValue::Scalar,
                move |source| {
                    report(controller.set_control_expression_source(
                        &target,
                        &source_control,
                        &source,
                    ));
                    refresh();
                },
            )
        },
        move |_, _, _, _| {
            let output =
                expression_output(&output_controller, &output_target, &output_control).ok()?;
            Some(crate::timeline_value::ExpressionOutput {
                value: format!(
                    "{:.*}{}",
                    usize::try_from(output_control.number.digits).expect("nonnegative digits"),
                    output_control.display_number(output.value),
                    output_control.number.unit
                ),
                error: output.error,
            })
        },
    )
}

fn report<T>(result: Result<T, String>) -> bool {
    match result {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, "Could not edit inspector control");
            false
        }
    }
}

fn confirmation(area: &gtk::GLArea, count: usize, pasted: bool) {
    if let Some(message) = shrimply_inspector_core::document::graph::confirmation(count, pasted) {
        shrimply_components_gtk::toast::show_confirmation_text_for_widget(area, &message);
    }
}
