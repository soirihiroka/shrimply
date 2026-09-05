use super::*;
use shrimply_gtk_components::tr;
use shrimply_gtk_components::ui::I18nAlertDialogExt;

pub(super) fn replace_selected_item_properties(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) {
    paste_selected_item_properties(area, project, player_state, selection_state, runtime, false);
}

pub(super) fn append_selected_item_modifiers(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) {
    paste_selected_item_properties(area, project, player_state, selection_state, runtime, true);
}

fn paste_selected_item_properties(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    modifiers_only: bool,
) {
    let Some(message) = paste_selected_item_properties_core(
        project,
        player_state,
        selection_state,
        runtime,
        modifiers_only,
    ) else {
        return;
    };
    shrimply_gtk_components::toast::show_confirmation_text_for_widget(area, &message);
    area.queue_render();
}

pub(crate) fn paste_selected_item_properties_core(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    modifiers_only: bool,
) -> Option<String> {
    let targets = {
        let project = project.borrow();
        selection_state::selected_item_addresses(selection_state, &project)
    };
    if targets.is_empty() {
        return None;
    }
    let clipboard = runtime.borrow().property_clipboard.clone();
    let result = {
        let mut project = project.borrow_mut();
        let result = if modifiers_only {
            clipboard.borrow().append_modifiers(&mut project, &targets)
        } else {
            clipboard
                .borrow()
                .replace_properties(&mut project, &targets)
        };
        if result.changed {
            crate::project::commit_edit(
                &project,
                if modifiers_only {
                    "paste-item-modifiers"
                } else {
                    "replace-item-properties"
                },
            );
        }
        result
    };
    if !result.changed {
        return None;
    }
    let message = if modifiers_only {
        if result.modifiers_added == 1 {
            tr!("1 effect pasted").into_owned()
        } else {
            shrimply_gtk_components::i18n::text_args(
                "%{count} effects pasted",
                &[("count", result.modifiers_added.to_string())],
            )
        }
    } else if result.changed_items == 1 {
        tr!("Properties replaced on 1 item").into_owned()
    } else {
        shrimply_gtk_components::i18n::text_args(
            "Properties replaced on %{count} items",
            &[("count", result.changed_items.to_string())],
        )
    };
    if result.stabilization {
        let project = project.borrow();
        for target in &targets {
            if let Some(item) = project.video_item(target)
                && item.stabilize_video
            {
                shrimply_video::video_stabilization::request(item);
            }
        }
    }
    player_state::refresh_project(
        player_state,
        ProjectChange {
            video: result.video,
            audio: result.audio,
            audio_waveforms: result.audio_waveforms,
            audio_beats: result.audio_beats,
            inspector: true,
            ..ProjectChange::default()
        },
    );
    Some(message)
}

pub(crate) fn paste_timeline_clipboard(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    clipboard: &TimelineClipboard,
    sequence_scope: &crate::project::SequenceScopeId,
) {
    paste_timeline_clipboard_core(
        project,
        player_state,
        selection_state,
        clipboard,
        sequence_scope,
    );
    area.queue_render();
}

pub(crate) fn paste_timeline_clipboard_core(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    clipboard: &TimelineClipboard,
    sequence_scope: &crate::project::SequenceScopeId,
) {
    let mut project_state = project.borrow_mut();
    let result = paste_items(
        &mut project_state,
        clipboard,
        sequence_scope,
        player_state::snapshot(player_state).position,
    );
    if result.selection.is_empty() {
        return;
    }

    let duration = project_state.duration();
    let selection = result.selection;
    let focused_item = selection.first().cloned();
    project_state.normalize_clip_transitions();
    crate::project::commit_edit(&project_state, "paste-timeline-items");
    drop(project_state);
    let project = project.borrow();
    selection_state::set_selected_item_addresses(
        selection_state,
        &project,
        selection,
        focused_item,
    );
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            audio: result.audio,
            audio_waveforms: false,
            video: result.video,
            captions: result.captions,
            inspector: true,
            ..ProjectChange::default()
        },
    );
}

pub(super) fn group_selected_timeline_items(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    selection_state: &SharedSelectionState,
) {
    group_selected_timeline_items_core(project, selection_state);
    area.queue_render();
}

pub(crate) fn group_selected_timeline_items_core(
    project: &Rc<RefCell<Project>>,
    selection_state: &SharedSelectionState,
) {
    let (selected, focused) = {
        let mut project_state = project.borrow_mut();
        let selection = selection_state::selected_item_addresses(selection_state, &project_state);
        let focused = selection_state::focused_item_address(selection_state, &project_state);
        let Some(selected) = group_selected_item_addresses(&mut project_state, &selection) else {
            return;
        };
        project_state.normalize_clip_transitions();
        crate::project::commit_edit(&project_state, "group-timeline-items");
        (selected, focused)
    };

    let project = project.borrow();
    selection_state::set_selected_item_addresses(selection_state, &project, selected, focused);
}

pub(super) fn fold_selected_timeline_items(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
) {
    fold_selected_timeline_items_core(project, player_state, selection_state);
    area.queue_render();
}

pub(crate) fn fold_selected_timeline_items_core(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
) {
    let selection = selected_timeline_items(selection_state);
    let selected = {
        let mut project = project.borrow_mut();
        let Some(selected) = fold_items(&mut project, &selection) else {
            return;
        };
        project.normalize_clip_transitions();
        crate::project::commit_edit(&project, "fold-timeline-sequence");
        selected
    };
    let focused = selected.first().copied();
    let project_state = project.borrow();
    set_timeline_selection(&project_state, selection_state, selected, focused);
    let duration = project_state.duration();
    drop(project_state);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            audio: selection.iter().any(|key| key.kind == TrackKind::Audio),
            audio_waveforms: selection.iter().any(|key| key.kind == TrackKind::Audio),
            video: selection.iter().any(|key| key.kind == TrackKind::Video),
            inspector: true,
            ..ProjectChange::default()
        },
    );
}

pub(super) fn ungroup_selected_timeline_items(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    selection_state: &SharedSelectionState,
) {
    ungroup_selected_timeline_items_core(project, selection_state);
    area.queue_render();
}

pub(crate) fn ungroup_selected_timeline_items_core(
    project: &Rc<RefCell<Project>>,
    selection_state: &SharedSelectionState,
) {
    let (selected, focused) = {
        let mut project_state = project.borrow_mut();
        let selection = selection_state::selected_item_addresses(selection_state, &project_state);
        let focused = selection_state::focused_item_address(selection_state, &project_state);
        let Some(selected) = ungroup_selected_item_addresses(&mut project_state, &selection) else {
            return;
        };
        project_state.normalize_clip_transitions();
        crate::project::commit_edit(&project_state, "ungroup-timeline-items");
        (selected, focused)
    };

    let project = project.borrow();
    selection_state::set_selected_item_addresses(selection_state, &project, selected, focused);
}

fn group_selected_item_addresses(
    project: &mut Project,
    selection: &[crate::project::ItemAddress],
) -> Option<Vec<crate::project::ItemAddress>> {
    let first = selection.first()?;
    let context = crate::timeline_operation::SequenceTimeline::for_item(project, first)?;
    group_item_addresses(&context, project, selection)
}

fn ungroup_selected_item_addresses(
    project: &mut Project,
    selection: &[crate::project::ItemAddress],
) -> Option<Vec<crate::project::ItemAddress>> {
    let first = selection.first()?;
    let context = crate::timeline_operation::SequenceTimeline::for_item(project, first)?;
    ungroup_item_addresses(&context, project, selection)
}

fn delete_item_addresses(
    context: &impl crate::timeline_operation::TimelineOperationContext,
    project: &mut Project,
    selection: &[crate::project::ItemAddress],
) -> Option<(bool, bool, bool)> {
    assert!(
        selection
            .iter()
            .all(|item| context.contains_item(project, item)),
        "deleted items must belong to their operation context"
    );
    let mut captions = false;
    let mut video = false;
    let mut audio = false;
    let mut changed = false;
    for address in selection {
        if project.take_item(address).is_none() {
            continue;
        }
        changed = true;
        match address.kind() {
            crate::project::ItemKind::Caption => captions = true,
            crate::project::ItemKind::Video => video = true,
            crate::project::ItemKind::Audio => audio = true,
        }
    }
    changed.then_some((captions, video, audio))
}

pub(super) fn delete_selected_gap(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
) {
    let Some(gap) = selection_state::selected_gap(selection_state) else {
        return;
    };
    let changed = {
        let mut project = project.borrow_mut();
        let changed = delete_track_gap(&mut project, gap);
        if changed.is_some() {
            project.normalize_clip_transitions();
            crate::project::commit_edit(&project, "delete-track-gap");
        }
        changed.map(|(captions, video, audio)| (captions, video, audio, project.duration()))
    };

    selection_state::set_selected_gap(selection_state, None);
    if let Some((captions, video, audio, duration)) = changed {
        player_state::refresh_project(
            player_state,
            ProjectChange {
                duration: Some(duration),
                frame_rate: None,
                audio,
                video,
                captions,
                ..ProjectChange::default()
            },
        );
    }
    area.queue_render();
}

pub(super) fn delete_selected_addressed_items(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    ripple: bool,
) {
    delete_selected_addressed_items_core(project, player_state, selection_state, ripple);
    area.queue_render();
}

pub(crate) fn delete_selected_addressed_items_core(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    ripple: bool,
) {
    let selection = {
        let project = project.borrow();
        selection_state::selected_item_addresses(selection_state, &project)
    };
    if selection.is_empty() {
        return;
    }
    let context = crate::timeline_operation::SequenceTimeline::new(selection_state::active_scope(
        selection_state,
    ));

    let (has_captions, has_videos, has_audio, duration, shifted_position, changed) = {
        let mut project_state = project.borrow_mut();
        let (has_captions, has_videos, has_audio, shifted_position, changed) = if ripple {
            let result = crate::items::ripple_delete_item_addresses(
                &context,
                &mut project_state,
                &selection,
                player_state::snapshot(player_state).position,
            );
            match result {
                Some(result) => (
                    result.captions,
                    result.video,
                    result.audio,
                    Some(result.shifted_position),
                    true,
                ),
                None => (false, false, false, None, false),
            }
        } else {
            let deleted = delete_item_addresses(&context, &mut project_state, &selection);
            let (captions, video, audio) = deleted.unwrap_or_default();
            (captions, video, audio, None, deleted.is_some())
        };
        if changed {
            project_state.prune_folded_sequences();
            project_state.normalize_clip_transitions();
            crate::project::commit_edit(
                &project_state,
                if ripple {
                    "ripple-delete-timeline-items"
                } else {
                    "delete-timeline-items"
                },
            );
        }
        (
            has_captions,
            has_videos,
            has_audio,
            project_state.duration(),
            shifted_position,
            changed,
        )
    };

    let project_state = project.borrow();
    selection_state::set_selected_item_addresses(selection_state, &project_state, Vec::new(), None);
    drop(project_state);
    if changed {
        player_state::refresh_project(
            player_state,
            ProjectChange {
                duration: Some(duration),
                frame_rate: None,
                audio: has_audio,
                audio_beats: has_audio,
                audio_waveforms: has_audio,
                video: has_videos,
                live_preview: false,
                captions: has_captions,
                inspector: true,
            },
        );
        if let Some(position) = shifted_position {
            player_state::seek_time(player_state, position);
        }
    }
}

pub(super) fn delete_selected_tracks(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
) {
    let selected_tracks = {
        let project = project.borrow();
        selection_state::selected_track_addresses(selection_state, &project)
    };
    if selected_tracks.is_empty() {
        return;
    }

    delete_tracks(
        area,
        project,
        player_state,
        selection_state,
        selected_tracks,
    );
}

pub(super) fn delete_tracks(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    selected_tracks: Vec<crate::project::TrackAddress>,
) {
    let clip_count = {
        let project = project.borrow();
        selected_track_clip_count(&project, &selected_tracks)
    };
    if clip_count == 0 {
        delete_selected_tracks_now(
            area,
            project,
            player_state,
            selection_state,
            selected_tracks,
        );
        return;
    }

    let dialog = adw::AlertDialog::new(
        Some("Delete Tracks?"),
        Some(&format!(
            "{clip_count} clips are about to be deleted, are you sure?"
        )),
    );
    dialog.add_responses_i18n(&[("cancel", "Cancel"), ("delete", "Delete")]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("delete"));
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);

    let area_for_render = area.clone();
    let area_for_dialog = area.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    let selection_state = selection_state.clone();
    dialog.choose(
        Some(area_for_dialog.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |response| {
            if response.as_str() == "delete" {
                delete_selected_tracks_now(
                    &area_for_render,
                    &project,
                    &player_state,
                    &selection_state,
                    selected_tracks,
                );
            }
        },
    );
}

fn delete_selected_tracks_now(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    selected_tracks: Vec<crate::project::TrackAddress>,
) {
    delete_selected_tracks_now_core(project, player_state, selection_state, selected_tracks);
    area.queue_render();
}

pub(crate) fn delete_selected_tracks_now_core(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    mut selected_tracks: Vec<crate::project::TrackAddress>,
) {
    let mut seen = hashbrown::HashSet::new();
    selected_tracks.retain(|track| seen.insert(track.clone()));
    if selected_tracks.is_empty() {
        return;
    }

    let (has_captions, has_videos, has_audio, duration, changed) = {
        let mut project_state = project.borrow_mut();
        let has_captions = selected_tracks
            .iter()
            .any(|track| track.kind() == crate::project::ItemKind::Caption);
        let has_videos = selected_tracks
            .iter()
            .any(|track| track.kind() == crate::project::ItemKind::Video);
        let has_audio = selected_tracks
            .iter()
            .any(|track| track.kind() == crate::project::ItemKind::Audio);
        let mut changed = false;
        for track in &selected_tracks {
            changed |= project_state.remove_track(track);
        }

        if changed {
            project_state.prune_folded_sequences();
            project_state.normalize_clip_transitions();
            crate::project::commit_edit(&project_state, "delete-selected-tracks");
        }
        (
            has_captions && changed,
            has_videos && changed,
            has_audio && changed,
            project_state.duration(),
            changed,
        )
    };

    if !changed {
        let project = project.borrow();
        selection_state::set_selected_track_addresses(selection_state, &project, Vec::new(), None);
        return;
    }

    let project = project.borrow();
    selection_state::set_selected_track_addresses(selection_state, &project, Vec::new(), None);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            frame_rate: None,
            audio: has_audio,
            audio_beats: has_audio,
            audio_waveforms: has_audio,
            video: has_videos,
            live_preview: false,
            captions: has_captions,
            inspector: true,
        },
    );
}

pub(crate) fn selected_track_clip_count(
    project: &Project,
    selected_tracks: &[crate::project::TrackAddress],
) -> usize {
    selected_tracks
        .iter()
        .map(|track| match project.track(track) {
            Some(crate::project::TrackRef::Caption(track)) => track.items.len(),
            Some(crate::project::TrackRef::Video(track)) => track.items.len(),
            Some(crate::project::TrackRef::Audio(track)) => track.items.len(),
            None => 0,
        })
        .sum()
}
