use super::control::Context;
use objc2::rc::{Retained, Weak};
use objc2_app_kit::NSView;
use objc2_foundation::MainThreadMarker;
use shrimply_components_appkit::{
    ActionButton, FrameGraph, MultilineTextInput, column_append_intrinsic, column_stack,
    control_row_with_suffix, row_stack,
};
use shrimply_inspector_core::{ControlKind, InspectorControl, keyframe_graph::FrameGraphAction};
use std::{cell::RefCell, rc::Rc};

pub(super) fn view(
    control: &InspectorControl,
    editor: &NSView,
    context: &Context,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let root = column_stack(6.0, mtm);
    let suffix = row_stack(4.0, mtm);
    let edited = control.clone();
    let editing = context.clone();
    let keyframes = ActionButton::symbol_toggle(
        "stopwatch",
        "Toggle keyframes",
        control.layered.keyframes,
        move |requested| {
            let result =
                editing
                    .controller
                    .set_control_keyframes(&editing.target, &edited, requested);
            let accepted = if result.is_ok() {
                requested
            } else {
                !requested
            };
            editing.refresh(result);
            accepted
        },
        mtm,
    );
    keyframes
        .view()
        .setEnabled(control.editable && control.sensitive);
    suffix.addArrangedSubview(keyframes.view());
    let edited = control.clone();
    let editing = context.clone();
    let expression = ActionButton::symbol_toggle(
        "chevron.left.forwardslash.chevron.right",
        "Toggle expression",
        control.layered.expression,
        move |requested| {
            let result =
                editing
                    .controller
                    .set_control_expression(&editing.target, &edited, requested);
            let accepted = if result.is_ok() {
                requested
            } else {
                !requested
            };
            editing.refresh(result);
            accepted
        },
        mtm,
    );
    expression
        .view()
        .setEnabled(control.editable && control.sensitive);
    suffix.addArrangedSubview(expression.view());
    if control.kind == ControlKind::LayeredText {
        column_append_intrinsic(
            &root,
            &control_row_with_suffix(&control.label, &NSView::new(mtm), Some(&suffix), mtm),
        );
        column_append_intrinsic(&root, editor);
    } else {
        column_append_intrinsic(
            &root,
            &control_row_with_suffix(&control.label, editor, Some(&suffix), mtm),
        );
    }
    if control.layered.keyframes {
        match shrimply_inspector_core::document::graph::frame_graph(control) {
            Ok(state) => {
                let editing = context.clone();
                let edited = control.clone();
                let canvas = Rc::new(RefCell::new(None::<Weak<NSView>>));
                let action_canvas = canvas.clone();
                let graph = FrameGraph::with_actions(
                    state,
                    move |action| {
                        if let FrameGraphAction::TextInterpolationRequested { owner_id, x, y } =
                            action
                        {
                            let Some(canvas) = action_canvas.borrow().as_ref().and_then(Weak::load)
                            else {
                                return;
                            };
                            let menu = match editing.controller.control_text_interpolation_menu(
                                &editing.target,
                                &edited,
                                owner_id,
                            ) {
                                Ok(menu) => menu,
                                Err(error) => {
                                    editing.finish(Err(error));
                                    return;
                                }
                            };
                            let choices =
                                menu.values
                                    .into_iter()
                                    .zip(menu.labels)
                                    .map(|(value, label)| {
                                        shrimply_components_appkit::StringChoice { value, label }
                                    })
                                    .collect();
                            let selected_context = editing.clone();
                            let selected_control = edited.clone();
                            shrimply_components_appkit::show_searchable_popover_at(
                                &canvas,
                                objc2_foundation::NSPoint::new(x, y),
                                "Search text interpolations",
                                choices,
                                menu.value,
                                move |value| {
                                    let selected = value
                                        .parse()
                                        .expect("text interpolation choice is an index");
                                    selected_context.refresh(
                                        selected_context.controller.set_control_text_interpolation(
                                            &selected_context.target,
                                            &selected_control,
                                            owner_id,
                                            selected,
                                        ),
                                    );
                                },
                                mtm,
                            );
                            return;
                        }
                        let live = matches!(
                            action,
                            FrameGraphAction::KeysMoved(_) | FrameGraphAction::PlayheadChanged(_)
                        );
                        let result = editing.controller.apply_control_graph_action(
                            &editing.target,
                            &edited,
                            action,
                        );
                        if live && result.is_ok() {
                            editing.finish(result);
                        } else {
                            editing.refresh(result);
                        }
                    },
                    mtm,
                );
                canvas.replace(Some(Weak::new(graph.canvas_view())));
                column_append_intrinsic(&root, graph.view());
            }
            Err(error) => {
                let error = shrimply_components_appkit::ReadOnlyField::new(&error, false, mtm);
                column_append_intrinsic(&root, error.view());
            }
        }
    }
    if control.layered.expression {
        let editing = context.clone();
        let edited = control.clone();
        let committing = context.clone();
        let committed = control.clone();
        let input = MultilineTextInput::code(
            &control.layered.expression_source,
            120.0,
            move |source| {
                let result = editing.controller.set_control_expression_source(
                    &editing.target,
                    &edited,
                    &source,
                );
                let success = result.is_ok();
                editing.finish(result);
                success
            },
            move || committing.commit(&committed),
            mtm,
        );
        column_append_intrinsic(&root, input.view());
    }
    root.into_super()
}
