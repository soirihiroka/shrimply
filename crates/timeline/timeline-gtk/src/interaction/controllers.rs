use super::*;
use shrimply_timeline_core::scene::{Event, PointerButton};

use shrimply_gtk_components::{cursor::software_cursor_from_name, ui::PointerLock};

struct CursorGrab {
    pointer_lock: PointerLock,
    cursor_name: String,
}

pub(crate) fn add_input_controllers(
    area: &gtk::GLArea,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    runtime: Rc<RefCell<TimelineRuntime>>,
    preferences: preferences_store::SharedPreferences,
) {
    let cursor_grab = Rc::new(RefCell::new(None));
    let motion = gtk::EventControllerMotion::new();
    let motion_area = area.clone();
    let motion_project = project.clone();
    let motion_runtime = runtime.clone();
    let motion_cursor_grab = cursor_grab.clone();
    motion.connect_motion(move |controller, x, y| {
        push_modifiers(&motion_runtime, controller.current_event_state());
        let pos = vec2(x as f32, y as f32);
        let mut runtime = motion_runtime.borrow_mut();
        let cursor_grabbed = motion_cursor_grab.borrow().is_some();
        if !cursor_grabbed {
            let modifiers = runtime.scene.pointer_state().modifiers;
            runtime.scene.event(Event::Motion {
                point: pos,
                modifiers,
            });
        }
        if !cursor_grabbed {
            let cursor = timeline_cursor(&motion_project.borrow(), &runtime, x, y);
            motion_area.set_cursor_from_name(gtk_cursor_name(cursor));
        }
        let begin_cursor_grab = !cursor_grabbed && runtime.scene.pointer_state().capture_requested;
        drop(runtime);

        if begin_cursor_grab {
            let cursor_name = motion_area
                .cursor()
                .and_then(|cursor| cursor.name())
                .map_or_else(|| String::from("default"), |name| name.to_string());
            let relative_area = motion_area.clone();
            let relative_runtime = motion_runtime.clone();
            if let Some(pointer_lock) =
                PointerLock::new_2d(&motion_area, move |delta_x, delta_y| {
                    let delta = vec2(delta_x as f32, delta_y as f32);
                    let mut runtime = relative_runtime.borrow_mut();
                    runtime.scene.event(Event::RelativeMotion { delta });
                    drop(runtime);
                    relative_area.queue_render();
                })
            {
                motion_runtime.borrow_mut().scene.begin_relative_pointer(
                    pos,
                    software_cursor_from_name("grabbing", &motion_area.display()),
                );
                motion_area.set_cursor_from_name(Some("none"));
                *motion_cursor_grab.borrow_mut() = Some(CursorGrab {
                    pointer_lock,
                    cursor_name,
                });
            }
        }
        motion_area.queue_render();
        start_timeline_animation_tick(&motion_area, motion_runtime.clone());
    });
    let leave_area = area.clone();
    let leave_runtime = runtime.clone();
    motion.connect_leave(move |_| {
        let mut runtime = leave_runtime.borrow_mut();
        if runtime.scene.pointer_state().software_position.is_none() {
            runtime.scene.event(Event::Leave);
            leave_area.set_cursor_from_name(None);
        }
        drop(runtime);
        leave_area.queue_render();
        start_timeline_animation_tick(&leave_area, leave_runtime.clone());
    });
    area.add_controller(motion);

    let click = gtk::GestureClick::new();
    click.set_button(1);
    let press_area = area.clone();
    let press_runtime = runtime.clone();
    click.connect_pressed(move |controller, n_press, x, y| {
        press_area.grab_focus();
        let modifiers = modifiers_from_state(controller.current_event_state());
        let mut runtime = press_runtime.borrow_mut();
        runtime.scene.event(Event::Press {
            point: vec2(x as f32, y as f32),
            button: PointerButton::Primary,
            double: n_press == 2,
            modifiers,
        });
        drop(runtime);
        press_area.queue_render();
        start_timeline_animation_tick(&press_area, press_runtime.clone());
    });
    let release_area = area.clone();
    let release_runtime = runtime.clone();
    let release_cursor_grab = cursor_grab.clone();
    click.connect_released(move |controller, _, x, y| {
        let modifiers = modifiers_from_state(controller.current_event_state());
        {
            let mut runtime = release_runtime.borrow_mut();
            let point = if release_cursor_grab.borrow().is_some() {
                runtime
                    .scene
                    .pointer_state()
                    .position
                    .unwrap_or(vec2(x as f32, y as f32))
            } else {
                vec2(x as f32, y as f32)
            };
            runtime.scene.event(Event::Release {
                point,
                button: PointerButton::Primary,
                modifiers,
            });
        }
        finish_cursor_grab(&release_area, &release_runtime, &release_cursor_grab);
        release_area.queue_render();
        start_timeline_animation_tick(&release_area, release_runtime.clone());
    });
    area.add_controller(click);

    let middle_click = gtk::GestureClick::new();
    middle_click.set_button(2);
    let middle_press_area = area.clone();
    let middle_press_runtime = runtime.clone();
    middle_click.connect_pressed(move |controller, _, x, y| {
        middle_press_area.grab_focus();
        let modifiers = modifiers_from_state(controller.current_event_state());
        let mut runtime = middle_press_runtime.borrow_mut();
        runtime.scene.event(Event::Press {
            point: vec2(x as f32, y as f32),
            button: PointerButton::Middle,
            double: false,
            modifiers,
        });
        middle_press_area.queue_render();
    });
    let middle_release_area = area.clone();
    let middle_release_runtime = runtime.clone();
    let middle_release_cursor_grab = cursor_grab.clone();
    middle_click.connect_released(move |controller, _, x, y| {
        let modifiers = modifiers_from_state(controller.current_event_state());
        let pos = vec2(x as f32, y as f32);
        {
            let mut runtime = middle_release_runtime.borrow_mut();
            let point = if middle_release_cursor_grab.borrow().is_some() {
                runtime.scene.pointer_state().position.unwrap_or(pos)
            } else {
                pos
            };
            runtime.scene.event(Event::Release {
                point,
                button: PointerButton::Middle,
                modifiers,
            });
        }
        finish_cursor_grab(
            &middle_release_area,
            &middle_release_runtime,
            &middle_release_cursor_grab,
        );
        middle_release_area.queue_render();
    });
    area.add_controller(middle_click);

    let secondary_click = gtk::GestureClick::new();
    secondary_click.set_button(3);
    let secondary_area = area.clone();
    let secondary_project = project.clone();
    let secondary_player_state = player_state.clone();
    let secondary_selection_state = selection_state.clone();
    let secondary_runtime = runtime.clone();
    let secondary_preferences = preferences.clone();
    secondary_click.connect_pressed(move |controller, _, x, y| {
        secondary_area.grab_focus();
        push_modifiers(&secondary_runtime, controller.current_event_state());
        show_timeline_item_context_menu(
            &secondary_area,
            &secondary_project,
            &secondary_player_state,
            &secondary_selection_state,
            &secondary_runtime,
            &secondary_preferences,
            x,
            y,
        );
    });
    area.add_controller(secondary_click);

    keyboard::add_controller(
        area,
        project.clone(),
        player_state.clone(),
        selection_state.clone(),
        runtime.clone(),
    );

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    let scroll_area = area.clone();
    let scroll_runtime = runtime.clone();
    scroll.connect_scroll(move |controller, dx, dy| {
        let modifiers = modifiers_from_state(controller.current_event_state());
        let mut runtime = scroll_runtime.borrow_mut();
        runtime
            .scene
            .event(shrimply_timeline_core::scene::Event::Modifiers(modifiers));
        let pointer = runtime.scene.pointer_state().position;
        runtime.scene.event(Event::Scroll(TimelineScrollEvent {
            delta: vec2(
                (dx * SCROLL_PIXELS_PER_STEP) as f32,
                (dy * SCROLL_PIXELS_PER_STEP) as f32,
            ),
            ctrl: modifiers.ctrl,
            pointer,
        }));
        drop(runtime);
        scroll_area.queue_render();
        start_timeline_animation_tick(&scroll_area, scroll_runtime.clone());
        glib::Propagation::Stop
    });
    area.add_controller(scroll);

    crate::drag_and_drop::setup(area, project, player_state, selection_state, runtime);
}

fn finish_cursor_grab(
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    cursor_grab: &Rc<RefCell<Option<CursorGrab>>>,
) {
    let Some(CursorGrab {
        pointer_lock,
        cursor_name,
    }) = cursor_grab.borrow_mut().take()
    else {
        return;
    };
    let position = runtime
        .borrow_mut()
        .scene
        .end_relative_pointer()
        .and_then(|position| pointer_surface_position(area, position));
    if let Some(position) = position {
        pointer_lock.restore_cursor_at(f64::from(position.x()), f64::from(position.y()));
    }
    drop(pointer_lock);
    area.set_cursor_from_name(Some(&cursor_name));
}

fn pointer_surface_position(area: &gtk::GLArea, pointer: Vec2) -> Option<gtk::graphene::Point> {
    let native = area.native()?;
    let (surface_x, surface_y) = native.surface_transform();
    let native_widget = native.dynamic_cast::<gtk::Widget>().ok()?;
    let position = area.compute_point(
        &native_widget,
        &gtk::graphene::Point::new(pointer.x, pointer.y),
    )?;
    Some(gtk::graphene::Point::new(
        position.x() + surface_x as f32,
        position.y() + surface_y as f32,
    ))
}

pub(crate) fn start_timeline_animation_tick(
    area: &gtk::GLArea,
    runtime: Rc<RefCell<TimelineRuntime>>,
) {
    {
        let mut runtime = runtime.borrow_mut();
        if runtime.animation_tick_active {
            return;
        }
        runtime.animation_tick_active = true;
    }

    area.add_tick_callback(move |area, _| {
        let should_continue = {
            let mut runtime = runtime.borrow_mut();
            let should_continue = runtime.scene.animating();
            if !should_continue {
                runtime.animation_tick_active = false;
            }
            should_continue
        };

        if should_continue {
            area.queue_render();
            glib::ControlFlow::Continue
        } else {
            area.queue_render();
            glib::ControlFlow::Break
        }
    });
}

fn gtk_cursor_name(cursor: TimelineCursor) -> Option<&'static str> {
    match cursor {
        TimelineCursor::Default => None,
        TimelineCursor::ResizeStart => Some("w-resize"),
        TimelineCursor::ResizeEnd => Some("e-resize"),
        TimelineCursor::ResizeHorizontal => Some("ew-resize"),
        TimelineCursor::Crosshair => Some("crosshair"),
    }
}
