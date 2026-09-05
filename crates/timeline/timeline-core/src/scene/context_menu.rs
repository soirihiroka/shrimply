use super::*;
use crate::project::{ItemAddress, ItemKind, ItemRef, TrackAddress};
use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct Context {
    pub menu: ContextMenu,
    pub item: Option<ItemAddress>,
    pub track: Option<TrackAddress>,
    pub folded_track: Option<TrackAddress>,
    pub file: Option<PathBuf>,
    pub at_top: bool,
    pub selected: Vec<ItemAddress>,
    pub tracks: Vec<TrackAddress>,
    pub focus: Option<ItemAddress>,
    pub scope: project::SequenceScopeId,
}

impl Scene {
    pub fn prepare_context_menu(&mut self, point: Vec2) -> ContextMenu {
        self.pointer_cancelled();
        self.view.selection = None;
        self.context = Context::default();
        if !self.pointer_in_viewport(point) || self.scrollbar_at(point).is_some() {
            return ContextMenu::default();
        }
        let x = f64::from(point.x);
        let y = f64::from(point.y);
        let project = self.project.borrow();
        let folded = folded_sequence::hit_projected_item(&project, self.view, x, y);
        let hit = items::hit_item_at(&project, self.view, x, y);
        if let Some(address) = folded
            .as_ref()
            .map(|hit| hit.key.clone())
            .or_else(|| hit.and_then(|hit| selection_state::item_address(&project, hit)))
        {
            let preserve_tracks = hit.is_some_and(|hit| {
                hit.kind == TrackKind::Audio
                    && selection_state::selected_tracks(&self.selection).contains(&TrackKey {
                        kind: hit.kind,
                        track_index: hit.track_index,
                    })
            });
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
                Some(ItemRef::Video(item))
                    if item.is_media()
                        || matches!(item.content, project::VideoItemContent::Manim(_)) =>
                {
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
            self.context.menu = if folded.is_some() {
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
                        ItemKind::Caption => ContextItemKind::Caption,
                        ItemKind::Video => ContextItemKind::Video,
                        ItemKind::Audio => ContextItemKind::Audio,
                    },
                    can_replace_properties,
                    can_paste_modifiers,
                    has_file: self.context.file.is_some(),
                    foldable: selected.len() >= 2
                        && selected.iter().all(|item| item.kind() != ItemKind::Caption),
                    unlinkable_folder: folder
                        && items::item_address_group_id(&project, &address).is_some(),
                    folder,
                    playback_speed: speeds
                        .first()
                        .map(|first| ContextMenuControl::PlaybackSpeed {
                            position: math::playback_speed_scale_position(
                                project::fraction_as_f64(*first),
                            ),
                            mixed: speeds.iter().any(|speed| speed != first),
                        }),
                    enable_beat_detection: selected.iter().any(|address| {
                        project
                            .audio_item(address)
                            .is_some_and(|item| !item.beat_detection)
                    }),
                    can_remove_silences: address.kind() == ItemKind::Audio,
                })
            };
        } else if x < timeline_x() {
            let rows = items::track_rows(&project);
            let row = (y >= RULER_HEIGHT)
                .then(|| crate::math::track_row_at_y(y + self.view.scroll_y))
                .flatten();
            if let Some(row) = row.and_then(|row| rows.get(row)) {
                if let Some(key) = row.root_key {
                    if !selection_state::selected_track_addresses(&self.selection, &project)
                        .contains(&row.address)
                    {
                        selection_state::set_selected_tracks(&self.selection, vec![key], Some(key));
                    }
                    self.context.track = Some(row.address.clone());
                    self.context.menu = crate::track_context_menu(match key.kind {
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
                    self.context.folded_track = Some(row.address.clone());
                    self.context.menu = crate::folded_track_context_menu();
                }
            } else {
                selection_state::set_selected_items(&self.selection, Vec::new(), None);
                self.context.at_top = y < RULER_HEIGHT;
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
        self.validate_context()?;
        self.dispatch_context_action(action)
    }

    pub(super) fn dispatch_context_action(
        &mut self,
        action: ContextMenuAction,
    ) -> Result<Option<ContextMenuRequest>, String> {
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
