use super::*;
use crate::project::{ItemAddress, ItemKind, ItemRef, TrackAddress};
use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};
use std::path::PathBuf;

/// A single shared hit contract routes mouse and touch menus on every host.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineContextTarget {
    Header,
    Playhead {
        position: Time,
    },
    Item {
        address: ItemAddress,
        folded: bool,
    },
    Track {
        address: TrackAddress,
        key: Option<TrackKey>,
    },
    Gap,
}

#[derive(Default)]
pub(super) struct Context {
    pub menu: ContextMenu,
    pub touch: bool,
    pub point: Vec2,
    pub item: Option<ItemAddress>,
    pub track: Option<TrackAddress>,
    pub folded_track: Option<TrackAddress>,
    pub file: Option<PathBuf>,
    pub at_top: bool,
    pub selected: Vec<ItemAddress>,
    pub tracks: Vec<TrackAddress>,
    pub focus: Option<ItemAddress>,
    pub scope: project::SequenceScopeId,
    pub split_time: Option<Time>,
    pub playhead_track: Option<TrackAddress>,
    recording_sources: Option<(TrackKind, Vec<Option<String>>)>,
}

impl Scene {
    pub fn context_target(&self, point: Vec2) -> Option<TimelineContextTarget> {
        if !self.pointer_in_viewport(point) || self.scrollbar_at(point).is_some() {
            return None;
        }
        let x = f64::from(point.x);
        let y = f64::from(point.y);
        if y < RULER_HEIGHT && x < self.view.timeline_x() {
            return Some(TimelineContextTarget::Header);
        }
        let position = player_state::current_time(&self.player);
        if crate::math::playhead_handle_contains(self.view, position, x, y) {
            return Some(TimelineContextTarget::Playhead { position });
        }
        if y < RULER_HEIGHT {
            return None;
        }
        let project = self.project.borrow();
        if let Some(hit) = folded_sequence::hit_projected_item(&project, self.view, x, y) {
            return Some(TimelineContextTarget::Item {
                address: hit.key,
                folded: true,
            });
        }
        if let Some(hit) = items::hit_item_at(&project, self.view, x, y)
            && let Some(address) = selection_state::item_address(&project, hit)
        {
            return Some(TimelineContextTarget::Item {
                address,
                folded: false,
            });
        }
        if x < self.view.timeline_x() {
            let rows = items::track_rows(&project);
            if let Some(row) =
                crate::math::track_row_at_y(y + self.view.scroll_y).and_then(|row| rows.get(row))
            {
                return Some(TimelineContextTarget::Track {
                    address: row.address.clone(),
                    key: row.root_key,
                });
            }
        }
        Some(TimelineContextTarget::Gap)
    }

    pub fn context_menu_checked(&self, action: ContextMenuAction) -> Option<bool> {
        match action {
            ContextMenuAction::ShowTools => Some(self.show_tools),
            ContextMenuAction::ShowTrackControls => Some(self.show_track_controls),
            ContextMenuAction::ShowAudioMonitor => Some(self.show_audio_monitor),
            _ => None,
        }
    }

    pub fn prepare_context_menu(&mut self, point: Vec2) -> ContextMenu {
        self.prepare_context_menu_for_input(point, false)
    }

    pub fn prepare_context_menu_for_input(&mut self, point: Vec2, touch: bool) -> ContextMenu {
        self.prepare_context_menu_with_cameras(point, touch, Vec::new)
    }

    pub fn prepare_context_menu_with_cameras(
        &mut self,
        point: Vec2,
        touch: bool,
        cameras: impl FnOnce() -> Vec<(String, String)>,
    ) -> ContextMenu {
        self.pointer_cancelled();
        self.view.selection = None;
        self.context = Context {
            point,
            touch,
            ..Context::default()
        };
        let button = {
            let project = self.project.borrow();
            track_controls::track_button_at(
                &project,
                self.view,
                f64::from(point.x),
                f64::from(point.y),
            )
        };
        let recording = match button {
            Some((_, TrackLabelAction::AudioRecord)) if self.recording_available.0 => Some((
                TrackKind::Audio,
                "Default Audio Input",
                &self.audio_source,
                shrimply_audio_engine::recording::input_devices().unwrap_or_else(|error| {
                    tracing::warn!(%error, "Could not list audio inputs");
                    Vec::new()
                }),
            )),
            Some((_, TrackLabelAction::VideoRecord)) if self.recording_available.1 => Some((
                TrackKind::Video,
                "Screen Recording",
                &self.video_source,
                cameras(),
            )),
            _ => None,
        };
        if let Some((kind, default, selected, devices)) = recording {
            let mut names = vec![default.into()];
            let mut sources = vec![None];
            for (id, name) in devices {
                sources.push(Some(id));
                names.push(name);
            }
            let items = sources
                .iter()
                .enumerate()
                .map(|(index, source)| {
                    ContextMenuEntry::Action(ContextMenuItem {
                        action: ContextMenuAction::SelectRecordingSource(index),
                        enabled: true,
                        checked: Some(source == selected),
                    })
                })
                .collect();
            self.context.recording_sources = Some((kind, sources));
            self.context.menu = ContextMenu {
                sections: vec![items],
                source_names: names,
            };
            return self.context.menu.clone();
        }
        let Some(target) = self.context_target(point) else {
            return ContextMenu::default();
        };
        let project = self.project.borrow();
        match target {
            TimelineContextTarget::Header => {
                self.context.at_top = true;
                self.context.menu = crate::empty_track_context_menu();
                let visibility = [
                    ContextMenuAction::ShowTools,
                    ContextMenuAction::ShowTrackControls,
                    ContextMenuAction::ShowAudioMonitor,
                ]
                .into_iter()
                .map(|action| {
                    ContextMenuEntry::Action(ContextMenuItem {
                        action,
                        enabled: true,
                        checked: self.context_menu_checked(action),
                    })
                })
                .collect();
                self.context.menu.sections.push(visibility);
            }
            TimelineContextTarget::Playhead { position } => {
                let scope = SequenceTimeline::new(selection_state::active_scope(&self.selection));
                let selected = selection_state::selected_item_addresses(&self.selection, &project);
                let grouped = items::expand_grouped_item_addresses(&scope, &project, &selected);
                let spanning = |address: &ItemAddress| {
                    scope
                        .timeline_item_times(&project, address)
                        .is_some_and(|(start, end)| start < position && position < end)
                };
                self.context.split_time = Some(position);
                self.context.menu.sections.push(vec![
                    ContextMenuEntry::Action(ContextMenuItem::enabled(
                        ContextMenuAction::SplitSelected,
                        grouped.iter().any(spanning),
                    )),
                    ContextMenuEntry::Action(ContextMenuItem::enabled(
                        ContextMenuAction::SplitAll,
                        scope.items(&project).iter().any(spanning),
                    )),
                ]);
                let mut tracks =
                    selection_state::selected_track_addresses(&self.selection, &project);
                if tracks.is_empty() {
                    for address in &selected {
                        let track = address.track();
                        if !tracks.contains(&track) {
                            tracks.push(track);
                        }
                    }
                    if tracks.is_empty()
                        && let Some(track) =
                            selection_state::focused_track_address(&self.selection, &project)
                    {
                        tracks.push(track);
                    }
                }
                if let [address] = tracks.as_slice()
                    && let Some(key) = selection_state::track_key(&project, address)
                {
                    self.context.playhead_track = Some(address.clone());
                    let mut actions = vec![ContextMenuEntry::Action(ContextMenuItem::new(
                        ContextMenuAction::AddToTrack,
                    ))];
                    if key.kind == TrackKind::Audio && self.recording_available.0 {
                        actions.push(ContextMenuEntry::Action(ContextMenuItem::new(
                            ContextMenuAction::RecordAudio,
                        )));
                    }
                    if key.kind == TrackKind::Video && self.recording_available.1 {
                        actions.push(ContextMenuEntry::Action(ContextMenuItem::new(
                            ContextMenuAction::RecordVideo,
                        )));
                    }
                    self.context.menu.sections.push(actions);
                }
            }
            TimelineContextTarget::Item { address, folded } => {
                let preserve_tracks = !folded
                    && address.kind() == ItemKind::Audio
                    && selection_state::selected_track_addresses(&self.selection, &project)
                        .contains(&address.track());
                if !preserve_tracks {
                    let scope = SequenceTimeline::for_item(&project, &address)
                        .expect("hit item has a valid scope");
                    let mut selected =
                        selection_state::selected_item_addresses(&self.selection, &project)
                            .into_iter()
                            .filter(|item| scope.contains_item(&project, item))
                            .collect::<Vec<_>>();
                    if !selected.contains(&address) {
                        selected = items::expand_grouped_item_addresses(
                            &scope,
                            &project,
                            std::slice::from_ref(&address),
                        );
                    }
                    selection_state::set_selected_item_addresses(
                        &self.selection,
                        &project,
                        selected,
                        Some(address.clone()),
                    );
                }
                let selected = selection_state::selected_item_addresses(&self.selection, &project);
                let clipboard = self.property_clipboard.borrow();
                let can_replace_properties = clipboard.can_replace_properties(&project, &selected);
                let can_paste_modifiers = clipboard.can_append_modifiers(&project, &selected);
                let folder = match project.item(&address) {
                    Some(ItemRef::Video(item)) => {
                        matches!(item.content, project::VideoItemContent::FoldedSequence(_))
                    }
                    Some(ItemRef::Audio(item)) => {
                        matches!(item.source, project::AudioSource::FoldedSequence(_))
                    }
                    _ => false,
                };
                self.context.file = match project.item(&address) {
                    Some(ItemRef::Video(item)) if item.uses_file_asset() => {
                        Some(item.file.path().to_owned())
                    }
                    Some(ItemRef::Audio(item))
                        if matches!(
                            item.source,
                            project::AudioSource::Media | project::AudioSource::Tts(_)
                        ) && !item.file.as_os_str().is_empty() =>
                    {
                        Some(item.file.path().to_owned())
                    }
                    _ => None,
                };
                self.context.item = Some(address.clone());
                self.context.menu = if folded {
                    crate::folded_item_context_menu(FoldedItemMenuContext {
                        groupable: selected.len() >= 2,
                        ungroupable: selected
                            .iter()
                            .any(|item| items::item_address_group_id(&project, item).is_some()),
                        folder,
                        can_replace_properties,
                        can_paste_modifiers,
                    })
                } else {
                    let speeds = selected
                        .iter()
                        .filter_map(|address| match project.item(address) {
                            Some(ItemRef::Video(item)) => {
                                Some(project::playback_speed_or_default(item.playback_speed))
                            }
                            Some(ItemRef::Audio(item)) => {
                                Some(project::playback_speed_or_default(item.playback_speed))
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    crate::item_context_menu(ItemMenuContext {
                        kind: match address.kind() {
                            ItemKind::Comment => ContextItemKind::Comment,
                            ItemKind::Caption => ContextItemKind::Caption,
                            ItemKind::Video => ContextItemKind::Video,
                            ItemKind::Audio => ContextItemKind::Audio,
                        },
                        can_replace_properties,
                        can_paste_modifiers,
                        has_file: self.context.file.is_some(),
                        foldable: selected.len() >= 2
                            && selected.iter().all(|item| {
                                !matches!(item.kind(), ItemKind::Comment | ItemKind::Caption)
                            }),
                        unlinkable_folder: folder
                            && items::item_address_group_id(&project, &address).is_some(),
                        folder,
                        playback_speed: speeds.first().map(|first| {
                            ContextMenuControl::PlaybackSpeed {
                                position: math::playback_speed_scale_position(
                                    project::fraction_as_f64(*first),
                                ),
                                mixed: speeds.iter().any(|speed| speed != first),
                            }
                        }),
                        enable_beat_detection: selected.iter().any(|address| {
                            project
                                .audio_item(address)
                                .is_some_and(|item| !item.beat_detection)
                        }),
                        can_remove_silences: address.kind() == ItemKind::Audio,
                    })
                };
            }
            TimelineContextTarget::Track { address, key } => {
                if let Some(key) = key {
                    if !selection_state::selected_track_addresses(&self.selection, &project)
                        .contains(&address)
                    {
                        selection_state::set_selected_tracks(&self.selection, vec![key], Some(key));
                    }
                    self.context.track = Some(address);
                    self.context.menu = crate::track_context_menu(match key.kind {
                        TrackKind::Comment => TrackMenuContext::Comment,
                        TrackKind::Caption => TrackMenuContext::Caption,
                        TrackKind::Video => TrackMenuContext::Video,
                        TrackKind::Audio => TrackMenuContext::Audio {
                            can_remove_silences: selection_state::selected_tracks(&self.selection)
                                .iter()
                                .any(|key| {
                                    key.kind == TrackKind::Audio
                                        && project
                                            .audio_tracks
                                            .get(key.track_index)
                                            .is_some_and(|track| !track.items.is_empty())
                                }),
                            gain_db: project.audio_tracks[key.track_index].gain_db,
                        },
                    });
                } else {
                    self.context.folded_track = Some(address);
                    self.context.menu = crate::folded_track_context_menu();
                }
            }
            TimelineContextTarget::Gap => {
                if f64::from(point.x) < self.view.timeline_x() {
                    selection_state::set_selected_items(&self.selection, Vec::new(), None);
                }
                self.context.menu = crate::empty_track_context_menu();
            }
        }
        self.context.selected = selection_state::selected_item_addresses(&self.selection, &project);
        self.context.tracks = selection_state::selected_track_addresses(&self.selection, &project);
        self.context.focus = selection_state::focused_item_address(&self.selection, &project);
        self.context.scope = selection_state::active_scope(&self.selection);
        self.context.menu.clone()
    }

    pub fn context_file_path(&self) -> Option<PathBuf> {
        self.context.file.clone()
    }

    pub(super) fn validate_context(&self) -> Result<(), String> {
        let project = self.project.borrow();
        if self
            .context
            .selected
            .iter()
            .chain(self.context.item.iter())
            .any(|address| project.item(address).is_none())
            || self
                .context
                .tracks
                .iter()
                .chain(self.context.track.iter())
                .chain(self.context.folded_track.iter())
                .chain(self.context.playhead_track.iter())
                .any(|address| project.track(address).is_none())
        {
            return Err("The context-menu selection was removed while the menu was open".into());
        }
        Ok(())
    }

    pub fn activate_context_menu_action(
        &mut self,
        action: ContextMenuAction,
    ) -> Result<Option<ContextMenuRequest>, String> {
        if !self
            .context
            .menu
            .actions()
            .any(|item| item.action == action && item.enabled)
        {
            return Err("Context-menu action is unavailable".into());
        }
        if let ContextMenuAction::SelectRecordingSource(index) = action {
            let (kind, sources) = self
                .context
                .recording_sources
                .take()
                .ok_or("The recording source menu is closed")?;
            let source = sources
                .into_iter()
                .nth(index)
                .ok_or("Invalid recording source")?;
            match kind {
                TrackKind::Audio => self.audio_source = source,
                TrackKind::Video => self.video_source = source,
                _ => unreachable!("recording track kind"),
            }
            return Ok(None);
        }
        if !matches!(
            action,
            ContextMenuAction::ShowTools
                | ContextMenuAction::ShowTrackControls
                | ContextMenuAction::ShowAudioMonitor
        ) {
            self.validate_context()?;
        }
        self.dispatch_context_action(action)
    }

    pub(super) fn dispatch_context_action(
        &mut self,
        action: ContextMenuAction,
    ) -> Result<Option<ContextMenuRequest>, String> {
        if matches!(
            action,
            ContextMenuAction::ShowTools
                | ContextMenuAction::ShowTrackControls
                | ContextMenuAction::ShowAudioMonitor
        ) {
            preferences::update_editor_layout(&self.preferences, |layout| match action {
                ContextMenuAction::ShowTools => {
                    layout.timeline_tools_visible = !layout.timeline_tools_visible
                }
                ContextMenuAction::ShowTrackControls => {
                    layout.timeline_track_controls_visible = !layout.timeline_track_controls_visible
                }
                ContextMenuAction::ShowAudioMonitor => {
                    layout.timeline_audio_monitor_visible = !layout.timeline_audio_monitor_visible
                }
                _ => unreachable!("timeline visibility action"),
            })?;
            self.apply_preferences(&preferences::snapshot(&self.preferences));
            self.media_refresh.redraw.set(true);
            return Ok(None);
        }
        // Modal native menus keep processing timers. Restore the captured selection
        // before dispatch so a completed import cannot redirect a menu action.
        if !self.context.tracks.is_empty() {
            selection_state::set_selected_track_addresses(
                &self.selection,
                &self.project.borrow(),
                self.context.tracks.clone(),
                self.context.tracks.first().cloned(),
            );
        } else {
            selection_state::set_selected_item_addresses(
                &self.selection,
                &self.project.borrow(),
                self.context.selected.clone(),
                self.context.focus.clone(),
            );
        }
        use ContextMenuAction as A;
        use ContextMenuRequest as R;
        let request = match action {
            A::AddToTrack | A::RecordAudio | A::RecordVideo => {
                let position = self
                    .context
                    .split_time
                    .ok_or("playhead insertion position is unavailable")?;
                let address = self
                    .context
                    .playhead_track
                    .as_ref()
                    .ok_or("no single track is active")?;
                let key = selection_state::track_key(&self.project.borrow(), address)
                    .ok_or("playhead target track is unavailable")?;
                match action {
                    A::AddToTrack => {
                        self.pending_track_add_menu = Some(TrackAddMenuRequest {
                            key,
                            touch: self.context.touch,
                            point: self.context.point,
                            button_bounds: None,
                            import_targets: vec![key],
                            position,
                        })
                    }
                    A::RecordAudio if self.recording_available.0 => {
                        self.pending_audio_record = Some(TrackRecordingRequest { key, position })
                    }
                    A::RecordVideo if self.recording_available.1 => {
                        self.pending_video_record = Some(TrackRecordingRequest { key, position })
                    }
                    _ => return Err("recording service is unavailable".into()),
                }
                self.media_refresh.redraw.set(true);
                None
            }
            A::SplitSelected | A::SplitAll => {
                self.split_context_items(action == A::SplitAll)?;
                None
            }
            A::InspectProperties => {
                player_state::refresh_project(
                    &self.player,
                    ProjectChange {
                        inspector: true,
                        ..Default::default()
                    },
                );
                None
            }
            A::DeleteTrack => self.key_action(KeyAction::Delete { ripple: false })?,
            A::Copy | A::Cut => {
                let project = self.project.borrow();
                self.clipboard = items::copy_items(&project, &self.context.selected);
                if self.clipboard.is_none() {
                    return Err("No timeline items are selected to copy".into());
                }
                if let Some(focus) = &self.context.focus {
                    self.property_clipboard
                        .borrow_mut()
                        .copy_item(&project, focus);
                } else {
                    self.property_clipboard.borrow_mut().clear();
                }
                drop(project);
                if action == A::Cut {
                    self.delete_context_items()?;
                }
                player_state::refresh_project(
                    &self.player,
                    ProjectChange {
                        inspector: true,
                        ..Default::default()
                    },
                );
                Some(R::SetTimelineClipboardMarker)
            }
            A::Paste => Some(R::PasteFromClipboard),
            A::ShowInFolder => Some(R::ShowInFolder),
            A::CopyFrame | A::SaveFrame => {
                let selection = if self.context.track.is_some() {
                    VideoFrameSelection::Tracks
                } else {
                    VideoFrameSelection::Items
                };
                Some(if action == A::CopyFrame {
                    R::CopyFrame(selection)
                } else {
                    R::SaveFrame(selection)
                })
            }
            A::ExportAudio => Some(R::ExportAudio),
            A::Transcribe => Some(R::Transcribe),
            A::RemoveSilences => Some(R::RemoveSilences),
            A::GenerateSpeech => Some(R::GenerateSpeech),
            A::DeleteFoldedTrack => {
                let track = self
                    .context
                    .folded_track
                    .as_ref()
                    .ok_or("No nested track is selected")?;
                let project = self.project.borrow();
                let count = match project.track(track) {
                    Some(project::TrackRef::Comment(track)) => track.items.len(),
                    Some(project::TrackRef::Caption(track)) => track.items.len(),
                    Some(project::TrackRef::Video(track)) => track.items.len(),
                    Some(project::TrackRef::Audio(track)) => track.items.len(),
                    None => return Err("Nested track was removed".into()),
                };
                drop(project);
                if count == 0 {
                    self.delete_context_folded_track()?;
                    None
                } else {
                    Some(R::DeleteFoldedTrack { clip_count: count })
                }
            }
            _ => {
                self.edit_context(action)?;
                None
            }
        };
        Ok(request)
    }
}
