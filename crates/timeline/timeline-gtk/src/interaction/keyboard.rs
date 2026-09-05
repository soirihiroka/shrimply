use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::items::{ripple_trim_item_addresses, split_item_addresses};
use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::project::{ItemKind, Project, Time};
use crate::selection_state::SharedSelectionState;
use crate::{
    TimelineRuntime, frame_step_seconds, min_seconds_per_pixel, selected_timeline_items,
    timeline_width,
};

use super::{
    SequenceTimeline, TimelineOperationContext, copy_selected_timeline_items,
    cut_selected_timeline_items, delete_selected_addressed_items, delete_selected_gap,
    delete_selected_tracks, group_selected_timeline_items, replace_selected_item_properties,
    ungroup_selected_timeline_items,
};

pub(super) fn add_controller(
    area: &gtk::GLArea,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    runtime: Rc<RefCell<TimelineRuntime>>,
) {
    let key = gtk::EventControllerKey::new();
    let key_area = area.clone();
    key.connect_key_pressed(move |_, key, _, state| {
        let is_g = key
            .to_unicode()
            .is_some_and(|key| key.eq_ignore_ascii_case(&'g'))
            || matches!(key, gdk::Key::g);
        if state.contains(gdk::ModifierType::CONTROL_MASK) {
            match key.to_unicode().map(|key| key.to_ascii_lowercase()) {
                Some('c') => {
                    copy_selected_timeline_items(
                        &key_area,
                        &project,
                        &player_state,
                        &selection_state,
                        &runtime,
                    );
                    return glib::Propagation::Stop;
                }
                Some('x') => {
                    cut_selected_timeline_items(
                        &key_area,
                        &project,
                        &player_state,
                        &selection_state,
                        &runtime,
                    );
                    return glib::Propagation::Stop;
                }
                Some('v') => {
                    if state.contains(gdk::ModifierType::SHIFT_MASK) {
                        replace_selected_item_properties(
                            &key_area,
                            &project,
                            &player_state,
                            &selection_state,
                            &runtime,
                        );
                    } else {
                        crate::clipboard::paste(
                            &key_area,
                            &project,
                            &player_state,
                            &selection_state,
                            &runtime,
                        );
                    }
                    return glib::Propagation::Stop;
                }
                _ if is_g => {
                    if state.contains(gdk::ModifierType::SHIFT_MASK) {
                        ungroup_selected_timeline_items(&key_area, &project, &selection_state);
                    } else {
                        group_selected_timeline_items(&key_area, &project, &selection_state);
                    }
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }

        if !state.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        ) {
            match key.to_unicode().map(|key| key.to_ascii_lowercase()) {
                Some('s') => {
                    split_timeline_at_playhead(
                        &key_area,
                        &project,
                        &player_state,
                        &selection_state,
                        state.contains(gdk::ModifierType::SHIFT_MASK),
                    );
                    return glib::Propagation::Stop;
                }
                Some('q') => {
                    ripple_trim_selected_timeline_items(
                        &key_area,
                        &project,
                        &player_state,
                        &selection_state,
                    );
                    return glib::Propagation::Stop;
                }
                Some('z') => {
                    toggle_timeline_zoom(&key_area, &project, &player_state, &runtime);
                    return glib::Propagation::Stop;
                }
                Some('d') => {
                    delete_selection(
                        &key_area,
                        &project,
                        &player_state,
                        &selection_state,
                        state.contains(gdk::ModifierType::SHIFT_MASK),
                    );
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }

        match key {
            gdk::Key::Delete if state.contains(gdk::ModifierType::SHIFT_MASK) => {
                delete_selection(&key_area, &project, &player_state, &selection_state, true);
                glib::Propagation::Stop
            }
            gdk::Key::BackSpace | gdk::Key::Delete => {
                delete_selection(&key_area, &project, &player_state, &selection_state, false);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    area.add_controller(key);
}

fn delete_selection(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    ripple: bool,
) {
    let has_addressed_items = {
        let project = project.borrow();
        !crate::selection_state::selected_item_addresses(selection_state, &project).is_empty()
    };
    if crate::selection_state::selected_gap(selection_state).is_some() {
        delete_selected_gap(area, project, player_state, selection_state);
    } else if has_addressed_items {
        delete_selected_addressed_items(area, project, player_state, selection_state, ripple);
    } else if selected_timeline_items(selection_state).is_empty() && {
        let project = project.borrow();
        !crate::selection_state::selected_track_addresses(selection_state, &project).is_empty()
    } {
        delete_selected_tracks(area, project, player_state, selection_state);
    }
}

fn split_timeline_at_playhead(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    select_left: bool,
) {
    let cut = player_state::snapshot(player_state).position;
    let scope = crate::selection_state::active_scope(selection_state);
    let (selected, change) = {
        let mut project_state = project.borrow_mut();
        let context = SequenceTimeline::new(scope);
        let addresses = context
            .items(&project_state)
            .into_iter()
            .filter(|address| {
                context
                    .timeline_item_times(&project_state, address)
                    .is_some_and(|(start, end)| start < cut && cut < end)
            })
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            return;
        }
        let (left, right) = split_item_addresses(&context, &mut project_state, &addresses, cut);
        let selected = if select_left { left } else { right };
        if selected.is_empty() {
            return;
        }
        let mut change = ProjectChange {
            duration: Some(project_state.duration()),
            ..ProjectChange::default()
        };
        for address in &selected {
            match address.kind() {
                ItemKind::Caption => change.captions = true,
                ItemKind::Video => change.video = true,
                ItemKind::Audio => {
                    change.audio = true;
                    change.audio_waveforms = true;
                }
            }
        }
        project_state.normalize_clip_transitions();
        crate::project::commit_edit(&project_state, "split-timeline-item");
        (selected, change)
    };

    let focused = selected.first().cloned();
    let project_state = project.borrow();
    crate::selection_state::set_selected_item_addresses(
        selection_state,
        &project_state,
        selected,
        focused,
    );
    drop(project_state);
    player_state::refresh_project(player_state, change);
    area.queue_render();
}

fn ripple_trim_selected_timeline_items(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
) {
    let selection = {
        let project = project.borrow();
        crate::selection_state::selected_item_addresses(selection_state, &project)
    };
    if selection.is_empty() {
        return;
    }
    let cut = player_state::snapshot(player_state).position;
    let (right, shifted_position, change) = {
        let mut project_state = project.borrow_mut();
        let context = SequenceTimeline::new(crate::selection_state::active_scope(selection_state));
        let Some(result) =
            ripple_trim_item_addresses(&context, &mut project_state, &selection, cut)
        else {
            return;
        };
        let duration = project_state.duration();
        project_state.normalize_clip_transitions();
        crate::project::commit_edit(&project_state, "ripple-trim-timeline-items");
        (
            result.selection,
            result.shifted_position.min(duration),
            ProjectChange {
                duration: Some(duration),
                audio: result.audio,
                audio_waveforms: result.audio,
                video: result.video,
                captions: result.captions,
                ..ProjectChange::default()
            },
        )
    };

    let project_state = project.borrow();
    let focused = right.first().cloned();
    crate::selection_state::set_selected_item_addresses(
        selection_state,
        &project_state,
        right,
        focused,
    );
    drop(project_state);
    player_state::refresh_project(player_state, change);
    player_state::seek_time(player_state, shifted_position);
    area.queue_render();
}

fn toggle_timeline_zoom(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) {
    let timeline_width = timeline_width(f64::from(area.width()));
    if timeline_width <= 0.0 {
        return;
    }
    let player = player_state::snapshot(player_state);
    let (duration, minimum_zoom) = {
        let project = project.borrow();
        (
            project
                .duration()
                .max(player.duration)
                .max(Time::from_seconds(1)),
            min_seconds_per_pixel(frame_step_seconds(&project)),
        )
    };
    let mut runtime = runtime.borrow_mut();
    shrimply_timeline_core::math::toggle_timeline_zoom(
        &mut runtime.view,
        duration,
        player.position,
        timeline_width,
        minimum_zoom,
    );
    runtime.horizontal_scrollbar.cancel_scroll();
    runtime.overscroll = None;
    let zoom = Time::from_seconds_f64(runtime.view.seconds_per_pixel);
    drop(runtime);

    let mut project = project.borrow_mut();
    if project.timeline_zoom != Some(zoom) {
        project.timeline_zoom = Some(zoom);
        crate::project::save_view_state(&project);
    }
    drop(project);
    area.queue_render();
}
