use super::*;
use shrimply_components_skia::input::{TouchAction, TouchPhase};

pub(super) fn attach(preview: &PreviewController, guides: Rc<RefCell<guides::GuideInput>>) {
    let area = preview.widget();
    let project = preview.project.clone();
    let player = preview.player_state.clone();
    let selection = preview.selection_state.clone();
    let focus = preview.preview_focus.clone();
    let state = preview.surface.state.clone();
    let controller = preview.controller.clone();
    shrimply_components_gtk::touch::connect(area, {
        let area = area.downgrade();
        let project = project.clone();
        let player = player.clone();
        let state = state.clone();
        let controller = controller.clone();
        let guides = guides.clone();
        move |change| {
            let Some(area) = area.upgrade() else {
                return;
            };
            let surface = ProviderDispatch {
                area: &area,
                project: &project,
                player_state: &player,
                selection_state: &selection,
                preview_focus: &focus,
                state: &state,
                controller: &controller,
            };
            if change.phase == TouchPhase::Begin {
                area.grab_focus();
                state.borrow().sanim_inspector.dismiss();
            }
            let actions = state.borrow_mut().touch.update(&[change]);
            // Only this shared semantic pair is a completed tap. A second
            // contact never emits it, so caption cuts cannot precede a pinch.
            let tap = match actions.as_slice() {
                [TouchAction::Press(_), TouchAction::Release(point)] => Some(*point),
                _ => None,
            };
            for action in actions {
                if dispatch(surface, &guides, action, tap) {
                    break;
                }
            }
            area.queue_render();
        }
    });
    area.add_tick_callback({
        let state = state.clone();
        move |area, _| {
            let point = state.borrow_mut().touch.poll_long_press();
            if let Some(point) = point {
                area.activate_action(
                    "preview-context.open",
                    Some(&(f64::from(point.x), f64::from(point.y), true).to_variant()),
                )
                .expect("preview context menu is installed");
            }
            glib::ControlFlow::Continue
        }
    });
    let cancel = Rc::new(move |area: &gtk::GLArea| {
        state.borrow_mut().touch.cancel();
        gtk_guides::cancel(&guides, area, &project, &controller);
        cancel_provider(area, &project, &player, &state, &controller, true);
        area.set_cursor_from_name(None);
    });
    area.connect_unmap({
        let cancel = cancel.clone();
        move |area| cancel(area)
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |focus| {
        if let Some(area) = focus.widget().and_downcast::<gtk::GLArea>() {
            cancel(&area);
        }
    });
    area.add_controller(focus);
}

fn dispatch(
    surface: ProviderDispatch<'_>,
    guides: &Rc<RefCell<guides::GuideInput>>,
    action: TouchAction,
    tap: Option<GlamVec2>,
) -> bool {
    let pointer = |position| PointerInput {
        sample: PointerSample {
            position,
            ..Default::default()
        },
        tool: PointerTool::Touch,
        ..Default::default()
    };
    match action {
        TouchAction::Press(point) => {
            if gtk_guides::press(guides, surface.area, surface.project, surface.state, point) {
                surface.controller.borrow_mut().core.sequence = PointerSequence::Guide;
            } else if tap.is_some_and(|point| {
                split_caption_at_pointer(
                    surface.area,
                    surface.project,
                    surface.player_state,
                    surface.selection_state,
                    surface.state,
                    point,
                )
            }) {
                return true;
            } else {
                dispatch_pointer(surface, PointerEvent::Begin(pointer(point)));
            }
        }
        TouchAction::Move(point) => {
            if guides.borrow().active() {
                gtk_guides::move_to(guides, surface.area, surface.project, surface.state, point);
            } else {
                dispatch_pointer(
                    surface,
                    PointerEvent::Samples {
                        input: pointer(point),
                        samples: &[],
                    },
                );
            }
        }
        TouchAction::Release(point) => {
            if guides.borrow().active() {
                gtk_guides::finish(
                    guides,
                    surface.area,
                    surface.project,
                    surface.state,
                    surface.controller,
                    point,
                );
            } else {
                dispatch_pointer(surface, PointerEvent::End(pointer(point)));
            }
        }
        TouchAction::Cancel(_) | TouchAction::History { .. } => {
            gtk_guides::cancel(guides, surface.area, surface.project, surface.controller);
            cancel_provider(
                surface.area,
                surface.project,
                surface.player_state,
                surface.state,
                surface.controller,
                true,
            );
            if let TouchAction::History { redo } = action {
                player_state::change_history(
                    surface.project,
                    surface.player_state,
                    if redo {
                        crate::project::redo
                    } else {
                        crate::project::undo
                    },
                );
            }
        }
        TouchAction::Transform {
            previous_center,
            center,
            scale,
        } => {
            let changed = {
                let project = surface.project.borrow();
                let mut state = surface.state.borrow_mut();
                let fit = geometry::fit_viewport(surface.area, &project, &state);
                let bounds = geometry::surface_bounds(surface.area, &state);
                state.zoom_pan_enabled
                    && state
                        .navigation
                        .transform(fit, bounds, previous_center, center, scale)
            };
            if changed {
                surface.state.borrow().sanim_inspector.dismiss();
                let mut controller = surface.controller.borrow_mut();
                controller.core.context_invalidated = true;
                controller.core.frame_pending = true;
            }
        }
    }
    false
}
