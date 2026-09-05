use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn show_folded_track_context_menu(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    track: crate::project::TrackAddress,
    x: f64,
    y: f64,
) {
    prepare_virtual_track_context_menu(runtime);
    let menu =
        crate::native_menu::menu_model(&shrimply_timeline_core::folded_track_context_menu()).menu;
    let actions = gio::SimpleActionGroup::new();
    add_menu_action(&actions, "delete-folded-track", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        move || {
            delete_tracks(
                &area,
                &project,
                &player_state,
                &selection_state,
                vec![track.clone()],
            )
        }
    });
    popup_timeline_context_menu(area, runtime, &menu, &actions, None, x, y);
    area.queue_render();
}

pub(super) fn create_folded_track(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    folder: &crate::project::ItemAddress,
    at_top: bool,
) {
    create_folded_track_core(project, player_state, folder, at_top);
    area.queue_render();
}

pub(crate) fn create_folded_track_core(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    folder: &crate::project::ItemAddress,
    at_top: bool,
) {
    let (kind, duration) = {
        let mut project = project.borrow_mut();
        let (kind, sequence_id) = match folder {
            crate::project::ItemAddress::Video { .. } => {
                let Some(item) = project.video_item(folder) else {
                    return;
                };
                let crate::project::VideoItemContent::FoldedSequence(reference) = item.content
                else {
                    return;
                };
                (TrackKind::Video, reference.sequence_id)
            }
            crate::project::ItemAddress::Audio { .. } => {
                let Some(item) = project.audio_item(folder) else {
                    return;
                };
                let crate::project::AudioSource::FoldedSequence(reference) = item.source else {
                    return;
                };
                (TrackKind::Audio, reference.sequence_id)
            }
            crate::project::ItemAddress::Caption { .. } => return,
        };
        let Some(sequence) = project.folded_sequence_mut(sequence_id) else {
            return;
        };
        match kind {
            TrackKind::Video => {
                let index = if at_top {
                    sequence.video_tracks.len()
                } else {
                    0
                };
                sequence.video_tracks.insert(index, Default::default());
            }
            TrackKind::Audio => {
                let index = if at_top {
                    0
                } else {
                    sequence.audio_tracks.len()
                };
                sequence.audio_tracks.insert(index, Default::default());
            }
            TrackKind::Caption => unreachable!(),
        }
        let path = folder
            .sequence_path()
            .iter()
            .copied()
            .chain(std::iter::once(folder.item_id()))
            .collect::<Vec<_>>();
        if !project.expanded_sequence_paths.contains(&path) {
            project.expanded_sequence_paths.push(path);
        }
        let duration = project.duration();
        crate::project::commit_edit(&project, "create-folded-sequence-track");
        (kind, duration)
    };

    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            audio: kind == TrackKind::Audio,
            audio_waveforms: kind == TrackKind::Audio,
            video: kind == TrackKind::Video,
            inspector: true,
            ..ProjectChange::default()
        },
    );
}

fn prepare_virtual_track_context_menu(runtime: &Rc<RefCell<TimelineRuntime>>) {
    let mut runtime = runtime.borrow_mut();
    runtime.scene.pointer_cancelled();
    if let Some(existing) = runtime.active_context_menu.take() {
        existing.popdown();
    }
}
