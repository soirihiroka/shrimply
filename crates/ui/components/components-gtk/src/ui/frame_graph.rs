use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use shrimply_components_skia::canvas::UVec2;
pub use shrimply_framegraph_skia::SharedFrameGraphState;
use shrimply_framegraph_skia::{
    FrameGraphAction, FrameGraphCommand, FrameGraphComponentAction, FrameGraphComponents,
    FrameGraphInputResult, FrameGraphKey, FrameGraphModifiers, FrameGraphPointerButton,
    FrameGraphPointerPosition, FrameGraphScrollInput, FrameGraphState, FrameGraphStatus,
};
use shrimply_math_interpolation::Interpolation;
use shrimply_surface_gl_skia::TimelineRenderer;

use super::modifier_menu::{SearchMenuItem, searchable_popover};

type ActionHandler = Rc<dyn Fn(FrameGraphComponentAction)>;
type StatusHandler = Rc<dyn Fn(FrameGraphStatus)>;
type StatusHandlers = Rc<RefCell<Vec<StatusHandler>>>;

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
        Self::with_shared_components(SharedFrameGraphState::new(states), on_action)
    }

    pub fn with_shared_components(
        state: SharedFrameGraphState,
        on_action: impl Fn(FrameGraphComponentAction) + 'static,
    ) -> Self {
        let graph_height = state.preferred_height();
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
                let status = state.status();
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

        connect_button(
            &previous,
            &area,
            &state,
            &on_action,
            &sync,
            FrameGraphCommand::PreviousKey,
        );
        connect_button(
            &toggle,
            &area,
            &state,
            &on_action,
            &sync,
            FrameGraphCommand::ToggleKey,
        );
        connect_button(
            &next,
            &area,
            &state,
            &on_action,
            &sync,
            FrameGraphCommand::NextKey,
        );

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
                state.draw(&painter, f64::from(width), f64::from(height));
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
                if state.pointer_moved(x, y).redraw {
                    area.queue_render();
                }
            }
        });
        motion.connect_leave({
            let state = state.clone();
            let area = area.clone();
            let pointer = pointer.clone();
            move |_| {
                pointer.set(None);
                if state.pointer_left().redraw {
                    area.queue_render();
                }
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
                let result = state.begin_pointer(
                    FrameGraphPointerButton::Secondary,
                    FrameGraphPointerPosition {
                        x,
                        y,
                        width: f64::from(area.width().max(1)),
                        height: f64::from(area.height().max(1)),
                    },
                    modifiers(gesture.current_event_state()),
                );
                if result.focus {
                    area.grab_focus();
                }
                for component_action in result.actions {
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
                if result.redraw {
                    area.queue_render();
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
                let wheel = controller.unit() == gdk::ScrollUnit::Wheel;
                if !wheel && dy.abs() >= dx.abs() {
                    return glib::Propagation::Proceed;
                }
                let (x, y) = pointer.get().unwrap_or_else(|| {
                    (
                        f64::from(area.width().max(1)) / 2.0,
                        f64::from(area.height().max(1)) / 2.0,
                    )
                });
                let result = state.scroll(
                    dx,
                    if wheel { dy } else { 0.0 },
                    FrameGraphPointerPosition {
                        x,
                        y,
                        width: f64::from(area.width().max(1)),
                        height: f64::from(area.height().max(1)),
                    },
                    wheel
                        && controller
                            .current_event_state()
                            .contains(gdk::ModifierType::CONTROL_MASK),
                    if wheel {
                        FrameGraphScrollInput::Wheel
                    } else {
                        FrameGraphScrollInput::Surface
                    },
                );
                let handled = result.handled;
                if result.redraw {
                    area.queue_render();
                }
                start_animation_if_needed(&area, &state, &animation_active);
                if handled {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        area.add_controller(scroll);

        crate::pinch_zoom::connect_pinch_zoom(
            &area,
            {
                let pointer = pointer.clone();
                move || pointer.get()
            },
            {
                let area = area.clone();
                let state = state.clone();
                move |(x, y), magnification| {
                    state.magnify(
                        magnification,
                        FrameGraphPointerPosition {
                            x,
                            y,
                            width: f64::from(area.width().max(1)),
                            height: f64::from(area.height().max(1)),
                        },
                    );
                }
            },
        );

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
                finish_input(&area, &on_action, &sync, state.key(graph_key));
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
        let actions = self.state.edit_value(value);
        dispatch(&self.on_action, actions);
        (self.sync)();
        self.area.queue_render();
    }

    pub fn edit_component_value(&self, component: usize, value: f64) {
        self.edit_component_values(component, &[(component, value)]);
    }

    pub fn edit_component_values(&self, active_component: usize, values: &[(usize, f64)]) {
        let actions = self.state.edit_component_values(active_component, values);
        dispatch(&self.on_action, actions);
        (self.sync)();
        self.area.queue_render();
    }

    pub fn activate_component(&self, component: usize) {
        self.state.activate_component(component);
        (self.sync)();
        self.area.queue_render();
    }

    pub fn set_playhead(&self, playhead: shrimply_math_core::Time) {
        self.state.set_playhead(playhead);
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
        self.state.replace_components(states);
        self.refresh();
    }

    pub fn refresh(&self) {
        self.area.set_height_request(self.state.preferred_height());
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

fn finish_input(
    area: &gtk::GLArea,
    handler: &ActionHandler,
    sync: &Rc<dyn Fn()>,
    result: FrameGraphInputResult,
) {
    if result.focus {
        area.grab_focus();
    }
    dispatch(handler, result.actions);
    sync();
    if result.redraw {
        area.queue_render();
    }
}

fn connect_button(
    button: &gtk::Button,
    area: &gtk::GLArea,
    state: &SharedFrameGraphState,
    handler: &ActionHandler,
    sync: &Rc<dyn Fn()>,
    command: FrameGraphCommand,
) {
    let area = area.clone();
    let state = state.clone();
    let handler = handler.clone();
    let sync = sync.clone();
    button.connect_clicked(move |_| {
        finish_input(&area, &handler, &sync, state.command(command));
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
            start.set((x, y));
            let result = state.begin_pointer(
                button,
                FrameGraphPointerPosition {
                    x,
                    y,
                    width: f64::from(area.width().max(1)),
                    height: f64::from(area.height().max(1)),
                },
                modifiers(gesture.current_event_state()),
            );
            finish_input(&area, &handler, &sync, result);
        }
    });
    drag.connect_drag_update({
        let area = area.clone();
        let state = state.clone();
        let handler = handler.clone();
        let sync = sync.clone();
        move |_, dx, dy| {
            let (start_x, start_y) = start.get();
            let result = state.update_pointer(FrameGraphPointerPosition {
                x: start_x + dx,
                y: start_y + dy,
                width: f64::from(area.width().max(1)),
                height: f64::from(area.height().max(1)),
            });
            finish_input(&area, &handler, &sync, result);
        }
    });
    drag.connect_drag_end({
        let area = area.clone();
        let state = state.clone();
        let handler = handler.clone();
        let sync = sync.clone();
        move |_, _, _| {
            finish_input(&area, &handler, &sync, state.end_pointer());
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
    if active.get() || !state.is_animating() {
        return;
    }
    active.set(true);
    let state = state.clone();
    let active = active.clone();
    area.add_tick_callback(move |area, _| {
        area.queue_render();
        if state.is_animating() {
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
                        shrimply_components_core::selector::matches_query(
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
                                state.set_interpolation(owner_id, interpolation);
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
