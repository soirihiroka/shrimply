use super::*;
use crate::interaction::{select_item_in_context, set_timeline_selection};
use crate::project::{fraction_as_f64, playback_speed_or_default};
use crate::timeline_operation::SequenceTimeline;

impl ToolkitTimeline {
    pub fn prepare_context_menu(&mut self, x: f32, y: f32) -> usize {
        self.context_menu = ContextMenu::default();
        self.context_track = None;
        self.context_folded_track = None;
        self.context_item = None;
        self.context_file_path = None;
        self.context_new_track_at_top = None;
        let x = f64::from(x);
        let y = f64::from(y);

        let folded_hit = {
            let project = self.project.borrow();
            let runtime = self.runtime.borrow();
            folded_sequence::hit_projected_item(&project, runtime.view, x, y)
        };
        if let Some(hit) = folded_hit {
            let project = self.project.borrow();
            let context = SequenceTimeline::for_item(&project, &hit.key)
                .expect("projected item must have a valid operation scope");
            select_item_in_context(
                &context,
                &project,
                &self.selection_state,
                hit.key.clone(),
                false,
                false,
            );
            drop(project);
            self.reset_context_interaction();
            self.context_item = Some(hit.key.clone());
            let (folder, groupable, ungroupable, can_replace_properties, can_paste_modifiers) = {
                let project = self.project.borrow();
                let selected =
                    selection_state::selected_item_addresses(&self.selection_state, &project);
                let folder = match project.item(&hit.key) {
                    Some(project::ItemRef::Video(item)) => {
                        matches!(item.content, project::VideoItemContent::FoldedSequence(_))
                    }
                    Some(project::ItemRef::Audio(item)) => {
                        matches!(item.source, project::AudioSource::FoldedSequence(_))
                    }
                    Some(project::ItemRef::Caption(_)) | None => false,
                };
                let clipboard = self.runtime.borrow().property_clipboard.clone();
                let clipboard = clipboard.borrow();
                (
                    folder,
                    selected.len() >= 2,
                    selected
                        .iter()
                        .any(|item| items::item_address_group_id(&project, item).is_some()),
                    clipboard.can_replace_properties(&project, &selected),
                    clipboard.can_append_modifiers(&project, &selected),
                )
            };
            self.context_menu =
                shrimply_timeline_core::folded_item_context_menu(FoldedItemMenuContext {
                    groupable,
                    ungroupable,
                    folder,
                    can_replace_properties,
                    can_paste_modifiers,
                });
            return self.context_menu_entry_count();
        }

        let hit = {
            let project = self.project.borrow();
            let runtime = self.runtime.borrow();
            items::hit_item_at(&project, runtime.view, x, y)
        };
        if let Some(hit) = hit {
            let (address, folder) = {
                let project = self.project.borrow();
                (
                    selection_state::item_address(&project, hit)
                        .expect("hit-tested root item must have an address"),
                    folded_sequence::reference(&project, hit)
                        .and_then(|_| selection_state::item_address(&project, hit)),
                )
            };
            let preserve_track_selection = {
                let project = self.project.borrow();
                let selected_items = selected_timeline_items(&self.selection_state);
                let selected_tracks = selected_timeline_tracks(&self.selection_state);
                let hit_track = TrackKey {
                    kind: hit.kind,
                    track_index: hit.track_index,
                };
                hit.kind == TrackKind::Audio
                    && selected_tracks.contains(&hit_track)
                    && silence::can_remove(&project, &selected_items, &selected_tracks, hit)
            };
            if !preserve_track_selection {
                select_item_in_context(
                    &SequenceTimeline::root(),
                    &self.project.borrow(),
                    &self.selection_state,
                    address.clone(),
                    false,
                    false,
                );
            }
            self.reset_context_interaction();
            self.context_item = Some(address);
            self.context_file_path = interaction::item_file_path(&self.project.borrow(), hit);
            self.add_selected_item_actions(hit, folder);
            return self.context_menu_entry_count();
        }

        let (track_row, key, empty_above, empty_below) = {
            let project = self.project.borrow();
            let runtime = self.runtime.borrow();
            let rows = items::track_rows(&project);
            let row = (y >= RULER_HEIGHT).then(|| {
                ((y + runtime.view.scroll_y - RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize
            });
            let over_handles = (0.0..timeline_x()).contains(&x);
            let track_row = over_handles
                .then_some(row)
                .flatten()
                .and_then(|row| rows.get(row).cloned());
            let key = track_label_action_at(&project, runtime.view, x, y).map(|(key, _)| key);
            (
                track_row,
                key,
                over_handles && (0.0..RULER_HEIGHT).contains(&y),
                over_handles
                    && y >= RULER_HEIGHT
                    && y + runtime.view.scroll_y >= RULER_HEIGHT + rows.len() as f64 * TRACK_HEIGHT,
            )
        };
        if empty_above || empty_below {
            set_timeline_selection(
                &self.project.borrow(),
                &self.selection_state,
                Vec::new(),
                None,
            );
            self.reset_context_interaction();
            self.context_menu = shrimply_timeline_core::empty_track_context_menu();
            self.context_new_track_at_top = Some(empty_above);
            return self.context_menu_entry_count();
        }
        if let Some(track_row) = track_row.filter(|row| row.root_key.is_none()) {
            self.reset_context_interaction();
            self.context_menu = shrimply_timeline_core::folded_track_context_menu();
            self.context_folded_track = Some(track_row.address.clone());
            tracing::debug!(track = ?track_row.address, "prepared folded timeline track context menu");
            return self.context_menu_entry_count();
        }
        if let Some(key) = key {
            if !selected_timeline_tracks(&self.selection_state).contains(&key) {
                selection_state::set_selected_tracks(&self.selection_state, vec![key], Some(key));
            }
            self.reset_context_interaction();
            self.context_track = Some(key);
            self.context_menu = match key.kind {
                TrackKind::Caption => {
                    shrimply_timeline_core::track_context_menu(TrackMenuContext::Caption)
                }
                TrackKind::Video => {
                    shrimply_timeline_core::track_context_menu(TrackMenuContext::Video)
                }
                TrackKind::Audio => {
                    let project = self.project.borrow();
                    shrimply_timeline_core::track_context_menu(TrackMenuContext::Audio {
                        can_remove_silences: silence::can_remove_track(
                            &project,
                            &selected_timeline_tracks(&self.selection_state),
                            key,
                        ),
                        gain_db: project.audio_tracks[key.track_index].gain_db,
                    })
                }
            };
        }
        self.context_menu_entry_count()
    }

    pub fn context_menu_item(&self, index: usize) -> Option<ContextMenuItem> {
        self.context_menu.actions().nth(index)
    }

    pub fn context_menu(&self) -> &ContextMenu {
        &self.context_menu
    }

    pub fn set_context_menu_control(&mut self, control: ContextMenuControl, value: f64) {
        match control {
            ContextMenuControl::PlaybackSpeed { .. } => self.set_selected_playback_speed(value),
            ContextMenuControl::AudioTrackGain { .. } => self.set_context_track_gain(value),
        }
    }

    pub fn activate_context_menu_action(
        &mut self,
        action: ContextMenuAction,
    ) -> Option<ContextMenuRequest> {
        let request = match action {
            ContextMenuAction::Copy => {
                copy_selected_timeline_items_core(
                    &self.project,
                    &self.player_state,
                    &self.selection_state,
                    &self.runtime,
                );
                self.runtime
                    .borrow()
                    .clipboard
                    .as_ref()
                    .map(|_| ContextMenuRequest::SetTimelineClipboardMarker)
            }
            ContextMenuAction::Cut => {
                copy_selected_timeline_items_core(
                    &self.project,
                    &self.player_state,
                    &self.selection_state,
                    &self.runtime,
                );
                if self.runtime.borrow().clipboard.is_some() {
                    interaction::delete_selected_addressed_items_core(
                        &self.project,
                        &self.player_state,
                        &self.selection_state,
                        false,
                    );
                }
                self.runtime
                    .borrow()
                    .clipboard
                    .as_ref()
                    .map(|_| ContextMenuRequest::SetTimelineClipboardMarker)
            }
            ContextMenuAction::Paste => Some(ContextMenuRequest::PasteFromClipboard),
            ContextMenuAction::ReplaceProperties => {
                interaction::paste_selected_item_properties_core(
                    &self.project,
                    &self.player_state,
                    &self.selection_state,
                    &self.runtime,
                    false,
                );
                None
            }
            ContextMenuAction::PasteModifiers => {
                interaction::paste_selected_item_properties_core(
                    &self.project,
                    &self.player_state,
                    &self.selection_state,
                    &self.runtime,
                    true,
                );
                None
            }
            ContextMenuAction::FoldSequence => {
                interaction::fold_selected_timeline_items_core(
                    &self.project,
                    &self.player_state,
                    &self.selection_state,
                );
                None
            }
            ContextMenuAction::UnlinkFolder => {
                interaction::ungroup_selected_timeline_items_core(
                    &self.project,
                    &self.selection_state,
                );
                None
            }
            ContextMenuAction::EnableBeatDetection => {
                set_selected_audio_beat_detection(
                    &self.project,
                    &self.player_state,
                    &self.selection_state,
                    true,
                );
                None
            }
            ContextMenuAction::DisableBeatDetection => {
                set_selected_audio_beat_detection(
                    &self.project,
                    &self.player_state,
                    &self.selection_state,
                    false,
                );
                None
            }
            ContextMenuAction::AddCaptionTrack => {
                self.create_track(TrackKind::Caption);
                None
            }
            ContextMenuAction::AddVideoTrack => {
                self.create_track(TrackKind::Video);
                None
            }
            ContextMenuAction::AddAudioTrack => {
                self.create_track(TrackKind::Audio);
                None
            }
            ContextMenuAction::AddFolderTrackTop | ContextMenuAction::AddFolderTrackBottom => {
                if let Some(folder) = self.context_item.as_ref() {
                    interaction::create_folded_track_core(
                        &self.project,
                        &self.player_state,
                        folder,
                        action == ContextMenuAction::AddFolderTrackTop,
                    );
                }
                None
            }
            ContextMenuAction::MoveOutOfSequence => {
                if let Some(address) = self.context_item.as_ref()
                    && let Some(context) =
                        SequenceTimeline::for_item(&self.project.borrow(), address)
                {
                    interaction::move_item_out_of_sequence_core(
                        &self.project,
                        &self.player_state,
                        &self.selection_state,
                        &context,
                        address,
                    );
                }
                None
            }
            ContextMenuAction::Group => {
                interaction::group_selected_timeline_items_core(
                    &self.project,
                    &self.selection_state,
                );
                None
            }
            ContextMenuAction::Ungroup => {
                interaction::ungroup_selected_timeline_items_core(
                    &self.project,
                    &self.selection_state,
                );
                None
            }
            ContextMenuAction::CopyFrame => Some(ContextMenuRequest::CopyFrame(
                self.context_video_frame_selection(),
            )),
            ContextMenuAction::SaveFrame => Some(ContextMenuRequest::SaveFrame(
                self.context_video_frame_selection(),
            )),
            ContextMenuAction::ShowInFolder => Some(ContextMenuRequest::ShowInFolder),
            ContextMenuAction::ExportAudio => Some(ContextMenuRequest::ExportAudio),
            ContextMenuAction::Transcribe => Some(ContextMenuRequest::Transcribe),
            ContextMenuAction::RemoveSilences => Some(ContextMenuRequest::RemoveSilences),
            ContextMenuAction::GenerateSpeech => Some(ContextMenuRequest::GenerateSpeech),
            ContextMenuAction::DeleteFoldedTrack => {
                let track = self.context_folded_track.clone()?;
                let clip_count = interaction::selected_track_clip_count(
                    &self.project.borrow(),
                    std::slice::from_ref(&track),
                );
                if clip_count == 0 {
                    interaction::delete_selected_tracks_now_core(
                        &self.project,
                        &self.player_state,
                        &self.selection_state,
                        vec![track],
                    );
                    None
                } else {
                    Some(ContextMenuRequest::DeleteFoldedTrack { clip_count })
                }
            }
        };
        self.context_menu = ContextMenu::default();
        request
    }

    pub fn render_context_video_frame(
        &self,
        selection: VideoFrameSelection,
    ) -> Result<RenderedVideoFrame, String> {
        let (project, position, item_ids) = prepare_selected_video_frame(
            &self.project,
            &self.player_state,
            &self.selection_state,
            selection,
        );
        render_video_frame(project, position, &item_ids)
    }

    pub fn context_file_path(&self) -> Option<&std::path::Path> {
        self.context_file_path.as_deref()
    }

    pub fn delete_context_folded_track(&self) {
        if let Some(track) = self.context_folded_track.clone() {
            interaction::delete_selected_tracks_now_core(
                &self.project,
                &self.player_state,
                &self.selection_state,
                vec![track],
            );
        }
    }

    pub fn paste_context_clipboard_text(&self, text: String) {
        if text == shrimply_timeline_core::TIMELINE_CLIPBOARD_MARKER {
            if let Some(clipboard) = self.runtime.borrow().clipboard.clone() {
                interaction::paste_timeline_clipboard_core(
                    &self.project,
                    &self.player_state,
                    &self.selection_state,
                    &clipboard,
                    &selection_state::active_scope(&self.selection_state),
                );
            }
        } else {
            crate::external_content::insert_text_at_playhead_core(
                &self.project,
                &self.player_state,
                &self.selection_state,
                &self.runtime,
                text,
            );
        }
    }

    fn add_selected_item_actions(&mut self, hit: ItemKey, folder: Option<project::ItemAddress>) {
        let (can_replace_properties, can_paste_modifiers) =
            if matches!(hit.kind, TrackKind::Video | TrackKind::Audio) {
                let targets = {
                    let project = self.project.borrow();
                    selection_state::selected_item_addresses(&self.selection_state, &project)
                };
                let clipboard = self.runtime.borrow().property_clipboard.clone();
                let clipboard = clipboard.borrow();
                (
                    clipboard.can_replace_properties(&self.project.borrow(), &targets),
                    clipboard.can_append_modifiers(&self.project.borrow(), &targets),
                )
            } else {
                (false, false)
            };
        let selected = selected_timeline_items(&self.selection_state);
        let foldable = selected.len() >= 2
            && selected
                .iter()
                .all(|key| matches!(key.kind, TrackKind::Video | TrackKind::Audio));
        let enable_beat_detection = if hit.kind == TrackKind::Audio {
            selected
                .iter()
                .filter(|key| key.kind == TrackKind::Audio)
                .any(|key| {
                    self.project
                        .borrow()
                        .audio_tracks
                        .get(key.track_index)
                        .and_then(|track| track.items.get(key.item_index))
                        .is_some_and(|item| !item.beat_detection)
                })
        } else {
            false
        };
        let selected_tracks = selected_timeline_tracks(&self.selection_state);
        let can_remove_silences = hit.kind == TrackKind::Audio
            && silence::can_remove(&self.project.borrow(), &selected, &selected_tracks, hit);
        let speeds = {
            let project = self.project.borrow();
            selected
                .iter()
                .filter_map(|key| match key.kind {
                    TrackKind::Audio => project
                        .audio_tracks
                        .get(key.track_index)
                        .and_then(|track| track.items.get(key.item_index))
                        .map(|item| playback_speed_or_default(item.playback_speed)),
                    TrackKind::Video => project
                        .video_tracks
                        .get(key.track_index)
                        .and_then(|track| track.items.get(key.item_index))
                        .map(|item| playback_speed_or_default(item.playback_speed)),
                    TrackKind::Caption => None,
                })
                .collect::<Vec<_>>()
        };
        let playback_speed = speeds
            .first()
            .map(|first| ContextMenuControl::PlaybackSpeed {
                position: shrimply_math_media::playback_speed_scale_position(fraction_as_f64(
                    *first,
                )),
                mixed: speeds.iter().any(|speed| speed != first),
            });
        let unlinkable_folder = folder
            .as_ref()
            .is_some_and(|_| items::item_group_id(&self.project.borrow(), hit).is_some());
        self.context_menu = shrimply_timeline_core::item_context_menu(ItemMenuContext {
            kind: match hit.kind {
                TrackKind::Caption => ContextItemKind::Caption,
                TrackKind::Video => ContextItemKind::Video,
                TrackKind::Audio => ContextItemKind::Audio,
            },
            can_replace_properties,
            can_paste_modifiers,
            has_file: self.context_file_path.is_some(),
            foldable,
            unlinkable_folder,
            folder: folder.is_some(),
            playback_speed,
            enable_beat_detection,
            can_remove_silences,
        });
    }

    fn context_menu_entry_count(&self) -> usize {
        self.context_menu.sections.iter().map(Vec::len).sum()
    }

    fn context_video_frame_selection(&self) -> VideoFrameSelection {
        if self.context_track.is_some() {
            VideoFrameSelection::Tracks
        } else {
            VideoFrameSelection::Items
        }
    }

    fn set_selected_playback_speed(&self, position: f64) {
        let speed = Fraction::from(
            (shrimply_math_media::playback_speed_from_scale_position(position) * 100.0).round()
                as i64,
        ) / Fraction::from(100);
        let selected = selected_timeline_items(&self.selection_state);
        let mut project = self.project.borrow_mut();
        let mut audio = false;
        let mut video = false;
        for key in selected {
            match key.kind {
                TrackKind::Audio => {
                    if let Some(item) = project
                        .audio_tracks
                        .get_mut(key.track_index)
                        .and_then(|track| track.items.get_mut(key.item_index))
                        && item.playback_speed != speed
                    {
                        item.playback_speed = speed;
                        audio = true;
                    }
                }
                TrackKind::Video => {
                    if let Some(item) = project
                        .video_tracks
                        .get_mut(key.track_index)
                        .and_then(|track| track.items.get_mut(key.item_index))
                        && item.playback_speed != speed
                    {
                        item.playback_speed = speed;
                        video = true;
                    }
                }
                TrackKind::Caption => {}
            }
        }
        if !audio && !video {
            return;
        }
        let duration = project.duration();
        project::commit_coalesced_edit(&project, "selected-item-speed");
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                duration: Some(duration),
                audio,
                audio_waveforms: audio,
                video,
                inspector: true,
                ..Default::default()
            },
        );
    }

    fn set_context_track_gain(&self, gain_db: f64) {
        let Some(TrackKey {
            kind: TrackKind::Audio,
            track_index,
        }) = self.context_track
        else {
            return;
        };
        let gain_db = (gain_db as f32).clamp(
            project::AUDIO_TRACK_GAIN_MIN_DB,
            project::AUDIO_TRACK_GAIN_MAX_DB,
        );
        let mut project = self.project.borrow_mut();
        let track = &mut project.audio_tracks[track_index];
        if track.gain_db == gain_db {
            return;
        }
        track.gain_db = gain_db;
        project::commit_coalesced_edit(&project, "audio-track-gain");
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                audio: true,
                ..Default::default()
            },
        );
    }

    fn reset_context_interaction(&self) {
        let mut runtime = self.runtime.borrow_mut();
        runtime.dragged_group = None;
        runtime.resize_drag = None;
        runtime.transition_drag = None;
        runtime.cut_preview = None;
        runtime.view.selection = None;
        runtime.view.drag_mode = DragMode::None;
    }

    fn create_track(&self, kind: TrackKind) {
        let at_top = self.context_new_track_at_top.unwrap_or(true);
        let index = match (kind, at_top) {
            (TrackKind::Caption | TrackKind::Video, true) | (TrackKind::Audio, false) => None,
            (TrackKind::Caption | TrackKind::Video, false) | (TrackKind::Audio, true) => Some(0),
        };
        interaction::create_track_core(
            &self.project,
            &self.player_state,
            &self.selection_state,
            kind,
            index,
        );
    }
}

pub(crate) fn prepare_selected_video_frame(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    selection: VideoFrameSelection,
) -> (Project, Time, Vec<uuid::Uuid>) {
    let project = project.borrow().clone();
    let item_ids = match selection {
        VideoFrameSelection::Items => selected_timeline_items(selection_state)
            .iter()
            .filter(|key| key.kind == TrackKind::Video)
            .filter_map(|key| {
                project
                    .video_tracks
                    .get(key.track_index)
                    .and_then(|track| track.items.get(key.item_index))
                    .map(|item| item.id)
            })
            .collect(),
        VideoFrameSelection::Tracks => selected_timeline_tracks(selection_state)
            .iter()
            .filter(|key| key.kind == TrackKind::Video)
            .filter_map(|key| project.video_tracks.get(key.track_index))
            .flat_map(|track| track.items.iter().map(|item| item.id))
            .collect(),
    };
    (
        project,
        player_state::snapshot(player_state).position,
        item_ids,
    )
}

pub(crate) fn render_video_frame(
    project: Project,
    position: Time,
    item_ids: &[uuid::Uuid],
) -> Result<RenderedVideoFrame, String> {
    let canvas_size = project.canvas_size;
    let mut renderer = shrimply_video::compositor::VideoExportRenderer::new(48_000)?;
    let frame = renderer.render_items(&project, position, 0, item_ids)?;
    let mut rgba = ffmpeg_next::frame::Video::new(
        ffmpeg_next::format::Pixel::RGBA,
        canvas_size.width,
        canvas_size.height,
    );
    renderer.copy_to_rgba_frame(frame, &mut rgba)?;
    let width = i32::try_from(canvas_size.width)
        .map_err(|_| "selected frame width is too large".to_string())?;
    let height = i32::try_from(canvas_size.height)
        .map_err(|_| "selected frame height is too large".to_string())?;
    let row_bytes = canvas_size.width as usize * std::mem::size_of::<u32>();
    let stride = rgba.stride(0);
    let mut pixels = Vec::with_capacity(row_bytes * canvas_size.height as usize);
    for row in rgba
        .data(0)
        .chunks_exact(stride)
        .take(canvas_size.height as usize)
    {
        pixels.extend_from_slice(&row[..row_bytes]);
    }
    Ok(RenderedVideoFrame {
        width,
        height,
        pixels,
    })
}

pub(crate) fn copy_selected_timeline_items_core(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) -> bool {
    let (copied, focused) = {
        let project = project.borrow();
        (
            items::copy_items(
                &project,
                &selection_state::selected_item_addresses(selection_state, &project),
            ),
            selection_state::focused_item_address(selection_state, &project),
        )
    };
    let copied_any = copied.is_some();
    let property_clipboard = runtime.borrow().property_clipboard.clone();
    if let Some(focused) = focused {
        property_clipboard
            .borrow_mut()
            .copy_item(&project.borrow(), &focused);
    } else {
        property_clipboard.borrow_mut().clear();
    }
    runtime.borrow_mut().clipboard = copied;
    player_state::refresh_project(
        player_state,
        player_state::ProjectChange {
            inspector: true,
            ..Default::default()
        },
    );
    copied_any
}

pub(crate) fn set_selected_audio_beat_detection(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    enabled: bool,
) {
    let selected = selected_timeline_items(selection_state);
    let mut project = project.borrow_mut();
    let mut changed = false;
    for key in selected.iter().filter(|key| key.kind == TrackKind::Audio) {
        if let Some(item) = project
            .audio_tracks
            .get_mut(key.track_index)
            .and_then(|track| track.items.get_mut(key.item_index))
            && item.beat_detection != enabled
        {
            item.beat_detection = enabled;
            changed = true;
        }
    }
    if !changed {
        return;
    }
    project::commit_edit(&project, "toggle-beat-detection");
    drop(project);
    player_state::refresh_project(
        player_state,
        player_state::ProjectChange {
            audio_beats: true,
            inspector: true,
            ..Default::default()
        },
    );
}
