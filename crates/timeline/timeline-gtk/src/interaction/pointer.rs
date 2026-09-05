use super::*;
mod actions;
mod dragging;
mod scrolling;
mod selection;

use actions::*;
use scrolling::*;
pub(crate) use selection::select_item_in_context;
pub(crate) use selection::set_timeline_selection;
use selection::{activate_track_button, timeline_cut};
pub(super) use selection::{select_track, set_selection};

pub(super) fn push_modifiers(runtime: &Rc<RefCell<TimelineRuntime>>, state: gdk::ModifierType) {
    runtime.borrow_mut().modifiers = modifiers_from_state(state);
}

pub(super) fn modifiers_from_state(state: gdk::ModifierType) -> TimelineModifiers {
    let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
    TimelineModifiers {
        ctrl,
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
    }
}

pub(crate) fn content_y(view: TimelineViewState, y: f64) -> f64 {
    y.max(RULER_HEIGHT) + view.scroll_y
}

#[derive(Clone, Copy)]
struct TimelinePointerInput {
    pressed: bool,
    down: bool,
    released: bool,
    middle_pressed: bool,
    middle_down: bool,
    middle_released: bool,
    press_origin: Option<Vec2>,
    release_pos: Option<Vec2>,
    interact_pos: Option<Vec2>,
    hover_pos: Option<Vec2>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_timeline_input(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &mut TimelineRuntime,
    width: f64,
    timeline_width: f64,
    height: f64,
    track_content_height: f64,
    duration_seconds: f64,
    frame_step_seconds: f64,
) {
    for scroll in std::mem::take(&mut runtime.pending_scrolls) {
        if !scroll.ctrl {
            let scrollbar = horizontal_scrollbar(
                runtime.view,
                timeline_width,
                height,
                duration_seconds,
                shrimply_skia_adw_core::slider::idle_state(),
            );
            let delta = if scroll.delta.x.abs() > f32::EPSILON {
                scroll.delta.x as f64
            } else {
                scroll.delta.y as f64
            };
            let mut scroll_seconds = runtime.view.scroll_seconds;
            let event = runtime.horizontal_scrollbar.scroll_pages_at(
                scrollbar,
                scroll.pointer,
                shrimply_timeline_core::math::scrollbar_wheel_pages(delta),
                |value| scroll_seconds = value,
            );
            if event.handled {
                runtime.view.scroll_seconds = scroll_seconds;
                runtime.vertical_scrollbar.cancel_scroll();
                runtime.overscroll = None;
                continue;
            }

            if let Some(scrollbar) = vertical_scrollbar(
                runtime.view,
                width,
                height,
                track_content_height,
                shrimply_skia_adw_core::slider::idle_state(),
            ) {
                let delta = if scroll.delta.y.abs() > f32::EPSILON {
                    scroll.delta.y as f64
                } else {
                    scroll.delta.x as f64
                };
                let mut scroll_y = runtime.view.scroll_y;
                let event = runtime.vertical_scrollbar.scroll_units_at(
                    scrollbar,
                    scroll.pointer,
                    delta,
                    |value| scroll_y = value,
                );
                if event.handled {
                    runtime.view.scroll_y = scroll_y;
                    runtime.horizontal_scrollbar.cancel_scroll();
                    runtime.overscroll = None;
                    continue;
                }
            }
        }
        let previous_zoom = runtime.view.seconds_per_pixel;
        runtime.overscroll = handle_scroll(
            &mut runtime.view,
            scroll.delta,
            scroll.ctrl,
            scroll.pointer,
            timeline_width,
            height,
            track_content_height,
            duration_seconds,
            frame_step_seconds,
        )
        .map(|(edge, distance)| TimelineOverscroll {
            edge,
            started_at: Instant::now(),
            distance,
        });
        if runtime.view.seconds_per_pixel != previous_zoom {
            let zoom = Time::from_seconds_f64(runtime.view.seconds_per_pixel);
            let mut project = project.borrow_mut();
            if project.timeline_zoom != Some(zoom) {
                project.timeline_zoom = Some(zoom);
                crate::project::save_view_state(&project);
            }
        }
    }

    let pointer = TimelinePointerInput {
        pressed: runtime.primary_pressed,
        down: runtime.primary_down,
        released: runtime.primary_released,
        middle_pressed: runtime.middle_pressed,
        middle_down: runtime.middle_down,
        middle_released: runtime.middle_released,
        press_origin: runtime.pointer_press_origin,
        release_pos: runtime.pointer_release_pos,
        interact_pos: runtime.pointer_pos,
        hover_pos: runtime.pointer_pos,
    };

    let hover_pos = pointer.interact_pos.or(pointer.hover_pos);
    let hovered_button = hover_pos.and_then(|pos| {
        let project = project.borrow();
        track_button_at(&project, runtime.view, pos.x as f64, pos.y as f64)
    });
    update_track_button_hover(runtime, hovered_button);
    if let Some(pos) = hover_pos {
        let project = project.borrow();
        update_cut_preview(
            runtime,
            &project,
            selection_state,
            pos.x as f64,
            pos.y as f64,
        );
    } else {
        runtime.cut_preview = None;
    }

    let hovered = hover_pos.is_some_and(|pos| {
        pos.x >= 0.0 && pos.y >= 0.0 && pos.x <= width as f32 && pos.y <= height as f32
    });
    if !hovered
        && matches!(runtime.view.drag_mode, DragMode::None)
        && runtime.pressed_track_button.is_none()
    {
        return;
    }

    if pointer.middle_pressed
        && let Some(pos) = pointer.press_origin.or(pointer.interact_pos)
    {
        runtime.view.begin_pan(pos.as_dvec2());
    }

    if pointer.middle_down
        && let Some(pos) = pointer.interact_pos.or(pointer.hover_pos)
        && matches!(runtime.view.drag_mode, DragMode::MiddlePan)
    {
        let min_seconds_per_pixel = min_seconds_per_pixel(frame_step_seconds);
        runtime.view.pan_to(pos.as_dvec2());
        runtime.view.clamp(
            duration_seconds,
            timeline_width,
            min_seconds_per_pixel,
            track_content_height,
            height,
        );
    }

    if pointer.pressed
        && let Some(pos) = pointer.press_origin.or(pointer.interact_pos)
    {
        let project = project.borrow();
        begin_pointer_action(
            pos,
            &project,
            player_state,
            selection_state,
            runtime,
            width,
            timeline_width,
            height,
            track_content_height,
            duration_seconds,
        );
    }

    if pointer.down
        && let Some(pos) = pointer.interact_pos.or(pointer.hover_pos)
    {
        let project = project.borrow();
        update_pointer_action(
            pos,
            &project,
            player_state,
            selection_state,
            runtime,
            width,
            timeline_width,
            height,
            track_content_height,
            duration_seconds,
        );
    }

    if pointer.released {
        let fallback = vec2(
            runtime.view.drag_start_x as f32,
            runtime.view.drag_start_y as f32,
        );
        let pos = pointer
            .release_pos
            .or(pointer.interact_pos)
            .or(pointer.hover_pos)
            .unwrap_or(fallback);
        end_pointer_action(
            pos,
            project,
            player_state,
            selection_state,
            runtime,
            width,
            timeline_width,
            height,
            track_content_height,
        );
    }

    if pointer.middle_released && matches!(runtime.view.drag_mode, DragMode::MiddlePan) {
        runtime.view.drag_mode = DragMode::None;
        runtime.view.drag_moved = false;
    }
}

fn update_track_button_hover(runtime: &mut TimelineRuntime, hovered: Option<TrackButtonId>) {
    if runtime.hovered_track_button == hovered {
        return;
    }
    if let Some(id) = runtime.hovered_track_button
        && let Some(button) = runtime.track_buttons.get_mut(&id)
    {
        runtime.track_controls_animating |= button
            .event(shrimply_skia_adw_core::button::Event::PointerLeft)
            .animating;
    }
    if let Some(id) = hovered {
        runtime.track_controls_animating |= runtime
            .track_buttons
            .entry(id)
            .or_default()
            .event(shrimply_skia_adw_core::button::Event::PointerEntered)
            .animating;
    }
    runtime.hovered_track_button = hovered;
}

fn update_cut_preview(
    runtime: &mut TimelineRuntime,
    project: &Project,
    selection_state: &SharedSelectionState,
    x: f64,
    y: f64,
) {
    if !runtime.cut_enabled || !matches!(runtime.view.drag_mode, DragMode::None | DragMode::Cut) {
        runtime.cut_preview = None;
        return;
    }

    runtime.cut_preview = shrimply_timeline_core::cutting::preview(
        project,
        &selection_state::selected_item_addresses(selection_state, project),
        runtime.cut_preview.as_ref(),
        shrimply_timeline_core::cutting::PreviewRequest {
            view: runtime.view,
            position: glam::DVec2::new(x, y),
            active: runtime.view.drag_mode == DragMode::Cut,
            snaps: &runtime.snap_repository,
        },
    );
}

pub(super) fn insert_caption_on_double_click(
    project: &Rc<RefCell<Project>>,
    x: f64,
    y: f64,
    view: TimelineViewState,
    snap_repository: &SnapRepo,
    default_duration: Time,
) -> Option<(ItemKey, Time)> {
    let mut project_state = project.borrow_mut();
    let item_key = shrimply_timeline_core::caption_creation::insert(
        &mut project_state,
        glam::DVec2::new(x, y),
        view,
        snap_repository,
        default_duration,
    )?;
    let duration = project_state.duration();
    crate::project::commit_edit(&project_state, "create-caption-on-double-click");
    drop(project_state);
    Some((item_key, duration))
}
