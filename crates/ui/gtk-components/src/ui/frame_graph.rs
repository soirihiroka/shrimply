use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use shrimply_interpolation::Interpolation;
use shrimply_keyframe_graph_core::{
    FrameGraphAction, FrameGraphComponentAction, FrameGraphComponents, FrameGraphKey,
    FrameGraphModifiers, FrameGraphPointerButton, FrameGraphPointerPosition, FrameGraphScrollInput,
    FrameGraphState, FrameGraphStatus,
};
use shrimply_skia_adw_core::canvas::UVec2;
use shrimply_skia_gl::TimelineRenderer;

use super::modifier_menu::{SearchMenuItem, searchable_popover};

type ActionHandler = Rc<dyn Fn(FrameGraphComponentAction)>;
type StatusHandler = Rc<dyn Fn(FrameGraphStatus)>;
type StatusHandlers = Rc<RefCell<Vec<StatusHandler>>>;
pub type SharedFrameGraphState = Rc<RefCell<FrameGraphComponents>>;

#[derive(Clone)]
pub struct FrameGraph {
    widget: gtk::Box,
    area: gtk::GLArea,
    state: SharedFrameGraphState,
    on_action: ActionHandler,
    sync: Rc<dyn Fn()>,
    status_handlers: StatusHandlers,
}

impl FrameGraph {
    pub fn new(state: FrameGraphState) -> Self {
        Self::with_actions(state, |_| {})
    }

    pub fn with_actions(
        state: FrameGraphState,
        on_action: impl Fn(FrameGraphAction) + 'static,
    ) -> Self {
        Self::with_component_actions(vec![state], 0, move |action| on_action(action.action))
    }

    pub fn with_component_actions(
        states: Vec<FrameGraphState>,
        active_component: usize,
        on_action: impl Fn(FrameGraphComponentAction) + 'static,
    ) -> Self {
        Self::with_components(
            FrameGraphComponents::new(states, active_component),
            on_action,
        )
    }

    pub fn with_components(
        states: FrameGraphComponents,
        on_action: impl Fn(FrameGraphComponentAction) + 'static,
    ) -> Self {
        Self::with_shared_components(Rc::new(RefCell::new(states)), on_action)
    }

    pub fn with_shared_components(
        state: SharedFrameGraphState,
        on_action: impl Fn(FrameGraphComponentAction) + 'static,
    ) -> Self {
        let graph_height = state.borrow().preferred_height();
        let on_action: ActionHandler = Rc::new(on_action);
        let status_handlers = Rc::new(RefCell::new(Vec::<StatusHandler>::new()));
        let area = gtk::GLArea::builder()
            .auto_render(false)
            .has_depth_buffer(false)
            .has_stencil_buffer(false)
            .height_request(graph_height)
            .hexpand(true)
            .focusable(true)
            .build();
        let previous = flat_button("go-previous-symbolic", "Previous keyframe");
        let toggle = flat_button("list-add-symbolic", "Add keyframe at playhead");
        let next = flat_button("go-next-symbolic", "Next keyframe");
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        controls.append(&spacer);
        controls.append(&previous);
        controls.append(&toggle);
        controls.append(&next);
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.set_hexpand(true);
        widget.append(&controls);
        widget.append(&area);

        let sync = {
            let state = state.clone();
            let previous = previous.clone();
            let toggle = toggle.clone();
            let next = next.clone();
            let status_handlers = status_handlers.clone();
            Rc::new(move || {
                let status = state.borrow().status();
                previous.set_sensitive(status.can_previous);
                next.set_sensitive(status.can_next);
                toggle.set_icon_name(if status.key_at_playhead {
                    "list-remove-symbolic"
                } else {
                    "list-add-symbolic"
                });
                toggle.set_tooltip_text(Some(if status.key_at_playhead {
                    "Delete keyframe at playhead"
                } else {
                    "Add keyframe at playhead"
                }));
                for handler in status_handlers.borrow().iter() {
                    handler(status);
                }
            }) as Rc<dyn Fn()>
        };
        sync();
        area.connect_map({
            let sync = sync.clone();
            move |area| {
                sync();
                area.queue_render();
            }
        });

        connect_button(&previous, &area, &state, &on_action, &sync, |state| {
            state.previous_key()
        });
        connect_button(&toggle, &area, &state, &on_action, &sync, |state| {
            state.toggle_key()
        });
        connect_button(&next, &area, &state, &on_action, &sync, |state| {
            state.next_key()
        });

        let animation_active = Rc::new(Cell::new(false));
        let renderer = Rc::new(RefCell::new(TimelineRenderer::new()));
        area.connect_render({
            let renderer = renderer.clone();
            let state = state.clone();
            let animation_active = animation_active.clone();
            move |area, _| {
                area.make_current();
                if let Some(error) = area.error() {
                    panic!("could not make the frame graph current: {error}");
                }
                let width = area.width().max(1);
                let height = area.height().max(1);
                let scale = area.scale_factor().max(1) as f32;
                let mut renderer = renderer.borrow_mut();
                let painter = renderer
                    .begin_frame(
                        UVec2::new(
                            (width as f32 * scale).round() as u32,
                            (height as f32 * scale).round() as u32,
                        ),
                        scale,
                        shrimply_cross_ui_theme::current().view_bg,
                    )
                    .unwrap_or_else(|error| panic!("could not draw the frame graph: {error}"));
                state
                    .borrow_mut()
                    .draw(&painter, f64::from(width), f64::from(height));
                renderer
                    .end_frame()
                    .unwrap_or_else(|error| panic!("could not finish the frame graph: {error}"));
                start_animation_if_needed(area, &state, &animation_active);
                glib::Propagation::Stop
            }
        });
        area.connect_unrealize(move |area| {
            area.make_current();
            renderer.borrow_mut().destroy();
        });

        let pointer = Rc::new(Cell::new(None::<(f64, f64)>));
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion({
            let pointer = pointer.clone();
            let state = state.clone();
            let area = area.clone();
            move |_, x, y| {
                pointer.set(Some((x, y)));
                state.borrow_mut().pointer_moved(x, y);
                area.queue_render();
            }
        });
        motion.connect_leave({
            let state = state.clone();
            let area = area.clone();
            let pointer = pointer.clone();
            move |_| {
                pointer.set(None);
                state.borrow_mut().pointer_left();
                area.queue_render();
            }
        });
        area.add_controller(motion);

        add_drag(
            &area,
            &state,
            &on_action,
            &sync,
            gdk::BUTTON_PRIMARY,
            FrameGraphPointerButton::Primary,
        );
        add_drag(
            &area,
            &state,
            &on_action,
            &sync,
            gdk::BUTTON_MIDDLE,
            FrameGraphPointerButton::Middle,
        );

        let secondary = gtk::GestureClick::new();
        secondary.set_button(gdk::BUTTON_SECONDARY);
        secondary.connect_released({
            let area = area.clone();
            let state = state.clone();
            let on_action = on_action.clone();
            move |gesture, _, x, y| {
                area.grab_focus();
                let actions = state.borrow_mut().active_actions(|state| {
                    state.begin_pointer(
                        FrameGraphPointerButton::Secondary,
                        x,
                        y,
                        f64::from(area.width().max(1)),
                        f64::from(area.height().max(1)),
                        modifiers(gesture.current_event_state()),
                    )
                });
                for component_action in actions {
                    let FrameGraphComponentAction { component, action } = component_action;
                    if let FrameGraphAction::InterpolationRequested {
                        owner_id,
                        interpolation,
                        x,
                        y,
                    } = action
                    {
                        show_interpolation_popover(
                            &area,
                            &state,
                            &on_action,
                            InterpolationPopoverRequest {
                                component,
                                owner_id,
                                selected: interpolation,
                                x,
                                y,
                            },
                        );
                    } else {
                        on_action(FrameGraphComponentAction { component, action });
                    }
                }
            }
        });
        area.add_controller(secondary);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.connect_scroll({
            let area = area.clone();
            let state = state.clone();
            let pointer = pointer.clone();
            let animation_active = animation_active.clone();
            move |controller, dx, dy| {
                let (x, y) = controller
                    .current_event()
                    .and_then(|event| event.position())
                    .or(pointer.get())
                    .unwrap_or_else(|| {
                        (
                            f64::from(area.width().max(1)) / 2.0,
                            f64::from(area.height().max(1)) / 2.0,
                        )
                    });
                let handled = state.borrow_mut().scroll(
                    dx,
                    dy,
                    FrameGraphPointerPosition {
                        x,
                        y,
                        width: f64::from(area.width().max(1)),
                        height: f64::from(area.height().max(1)),
                    },
                    controller
                        .current_event_state()
                        .contains(gdk::ModifierType::CONTROL_MASK),
                    if controller.unit() == gdk::ScrollUnit::Wheel {
                        FrameGraphScrollInput::Wheel
                    } else {
                        FrameGraphScrollInput::Surface
                    },
                );
                area.queue_render();
                start_animation_if_needed(&area, &state, &animation_active);
                if handled {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        area.add_controller(scroll);

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let area = area.clone();
            let state = state.clone();
            let on_action = on_action.clone();
            let sync = sync.clone();
            move |controller, key, _, _| {
                let mods = controller.current_event_state();
                let graph_key = match key {
                    gdk::Key::space => FrameGraphKey::TogglePlayback,
                    gdk::Key::Left => FrameGraphKey::PreviousFrame,
                    gdk::Key::Right => FrameGraphKey::NextFrame,
                    gdk::Key::Home => FrameGraphKey::Start,
                    gdk::Key::End => FrameGraphKey::End,
                    gdk::Key::Delete | gdk::Key::BackSpace | gdk::Key::KP_Delete => {
                        FrameGraphKey::Delete
                    }
                    gdk::Key::c if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                        FrameGraphKey::Copy
                    }
                    gdk::Key::v if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                        FrameGraphKey::Paste
                    }
                    gdk::Key::plus | gdk::Key::equal => FrameGraphKey::ZoomIn,
                    gdk::Key::minus => FrameGraphKey::ZoomOut,
                    _ => return glib::Propagation::Proceed,
                };
                let actions = state
                    .borrow_mut()
                    .active_actions(|state| state.key(graph_key));
                dispatch(&on_action, actions);
                sync();
                area.queue_render();
                glib::Propagation::Stop
            }
        });
        area.add_controller(keys);

        let style = adw::StyleManager::for_display(&area.display());
        style.connect_dark_notify({
            let area = area.clone();
            move |style| {
                shrimply_cross_ui_theme::set_dark(style.is_dark());
                area.queue_render();
            }
        });

        Self {
            widget,
            area,
            state,
            on_action,
            sync,
            status_handlers,
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.widget
    }

    pub fn graph_area(&self) -> &gtk::GLArea {
        &self.area
    }

    pub fn state(&self) -> SharedFrameGraphState {
        self.state.clone()
    }

    pub fn edit_value(&self, value: f64) {
        let actions = self
            .state
            .borrow_mut()
            .active_actions(|state| state.set_value(value));
        dispatch(&self.on_action, actions);
        (self.sync)();
        self.area.queue_render();
    }

    pub fn edit_component_value(&self, component: usize, value: f64) {
        self.edit_component_values(component, &[(component, value)]);
    }

    pub fn edit_component_values(&self, active_component: usize, values: &[(usize, f64)]) {
        let actions = self
            .state
            .borrow_mut()
            .set_component_values(active_component, values);
        dispatch(&self.on_action, actions);
        (self.sync)();
        self.area.queue_render();
    }

    pub fn activate_component(&self, component: usize) {
        self.state.borrow_mut().activate(component);
        (self.sync)();
        self.area.queue_render();
    }

    pub fn set_playhead(&self, playhead: shrimply_math_core::Time) {
        self.state.borrow_mut().set_playhead(playhead);
        if self.area.is_mapped() {
            (self.sync)();
            self.area.queue_render();
        }
    }

    pub fn replace_state(&self, state: FrameGraphState) {
        self.replace_component_states(vec![state], 0);
    }

    pub fn replace_component_states(&self, states: Vec<FrameGraphState>, active_component: usize) {
        self.replace_components(FrameGraphComponents::new(states, active_component));
    }

    pub fn replace_components(&self, states: FrameGraphComponents) {
        *self.state.borrow_mut() = states;
        self.refresh();
    }

    pub fn refresh(&self) {
        self.area
            .set_height_request(self.state.borrow().preferred_height());
        if self.area.is_mapped() {
            (self.sync)();
            self.area.queue_render();
        }
    }

    pub fn connect_status(&self, handler: impl Fn(FrameGraphStatus) + 'static) {
        let handler = Rc::new(handler) as StatusHandler;
        self.status_handlers.borrow_mut().push(handler);
    }
}

fn flat_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .css_classes(["flat"])
        .build()
}

fn dispatch(handler: &ActionHandler, actions: Vec<FrameGraphComponentAction>) {
    for action in actions {
        handler(action);
    }
}

fn connect_button(
    button: &gtk::Button,
    area: &gtk::GLArea,
    state: &SharedFrameGraphState,
    handler: &ActionHandler,
    sync: &Rc<dyn Fn()>,
    action: impl Fn(&mut FrameGraphState) -> Vec<FrameGraphAction> + 'static,
) {
    let area = area.clone();
    let state = state.clone();
    let handler = handler.clone();
    let sync = sync.clone();
    button.connect_clicked(move |_| {
        let actions = state.borrow_mut().active_actions(|state| action(state));
        dispatch(&handler, actions);
        sync();
        area.queue_render();
        area.grab_focus();
    });
}

fn add_drag(
    area: &gtk::GLArea,
    state: &SharedFrameGraphState,
    handler: &ActionHandler,
    sync: &Rc<dyn Fn()>,
    native_button: u32,
    button: FrameGraphPointerButton,
) {
    let start = Rc::new(Cell::new((0.0, 0.0)));
    let drag = gtk::GestureDrag::new();
    drag.set_button(native_button);
    drag.connect_drag_begin({
        let area = area.clone();
        let state = state.clone();
        let handler = handler.clone();
        let sync = sync.clone();
        let start = start.clone();
        move |gesture, x, y| {
            area.grab_focus();
            start.set((x, y));
            let actions = state.borrow_mut().active_actions(|state| {
                state.begin_pointer(
                    button,
                    x,
                    y,
                    f64::from(area.width().max(1)),
                    f64::from(area.height().max(1)),
                    modifiers(gesture.current_event_state()),
                )
            });
            dispatch(&handler, actions);
            sync();
            area.queue_render();
        }
    });
    drag.connect_drag_update({
        let area = area.clone();
        let state = state.clone();
        let handler = handler.clone();
        let sync = sync.clone();
        move |_, dx, dy| {
            let (start_x, start_y) = start.get();
            let actions = state.borrow_mut().active_actions(|state| {
                state.update_pointer(
                    start_x + dx,
                    start_y + dy,
                    f64::from(area.width().max(1)),
                    f64::from(area.height().max(1)),
                )
            });
            dispatch(&handler, actions);
            sync();
            area.queue_render();
        }
    });
    drag.connect_drag_end({
        let area = area.clone();
        let state = state.clone();
        let handler = handler.clone();
        move |_, _, _| {
            let actions = state
                .borrow_mut()
                .active_actions(FrameGraphState::end_pointer);
            dispatch(&handler, actions);
            area.queue_render();
        }
    });
    area.add_controller(drag);
}

fn modifiers(state: gdk::ModifierType) -> FrameGraphModifiers {
    FrameGraphModifiers {
        control: state.contains(gdk::ModifierType::CONTROL_MASK),
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
    }
}

fn start_animation_if_needed(
    area: &gtk::GLArea,
    state: &SharedFrameGraphState,
    active: &Rc<Cell<bool>>,
) {
    if active.get() || !state.borrow().is_animating() {
        return;
    }
    active.set(true);
    let state = state.clone();
    let active = active.clone();
    area.add_tick_callback(move |area, _| {
        area.queue_render();
        if state.borrow().is_animating() {
            glib::ControlFlow::Continue
        } else {
            active.set(false);
            glib::ControlFlow::Break
        }
    });
}

struct InterpolationPopoverRequest {
    component: usize,
    owner_id: uuid::Uuid,
    selected: Interpolation,
    x: f64,
    y: f64,
}

fn show_interpolation_popover(
    area: &gtk::GLArea,
    state: &SharedFrameGraphState,
    handler: &ActionHandler,
    request: InterpolationPopoverRequest,
) {
    let InterpolationPopoverRequest {
        component,
        owner_id,
        selected,
        x,
        y,
    } = request;
    let interpolations = Interpolation::KEYFRAME;
    let popover = searchable_popover(
        crate::i18n::text("Search interpolations").as_ref(),
        280,
        180,
        240,
        {
            let area = area.clone();
            let state = state.clone();
            let handler = handler.clone();
            move |query| {
                interpolations
                    .into_iter()
                    .filter(|interpolation| {
                        shrimply_component_core::selector::matches_query(
                            interpolation.label(),
                            query,
                        )
                    })
                    .map(|interpolation| {
                        let area = area.clone();
                        let state = state.clone();
                        let handler = handler.clone();
                        SearchMenuItem::new(
                            crate::i18n::text(interpolation.label()).as_ref(),
                            move || {
                                state
                                    .borrow_mut()
                                    .set_interpolation(owner_id, interpolation);
                                handler(FrameGraphComponentAction {
                                    component,
                                    action: FrameGraphAction::InterpolationRequested {
                                        owner_id,
                                        interpolation,
                                        x,
                                        y,
                                    },
                                });
                                area.queue_render();
                            },
                        )
                        .selected(interpolation == selected)
                    })
                    .collect()
            }
        },
    );
    popover.set_parent(area);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|popover| popover.unparent());
    popover.popup();
}
