use super::*;
use crate::project::{ItemAddress, ItemKind, ItemMut, TrackMut};
use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};

impl Scene {
    pub(super) fn commit_context_edit(
        &mut self,
        mut edited: Project,
        message: &str,
        selection: Option<Vec<ItemAddress>>,
    ) -> Result<(), String> {
        edited.normalize_clip_transitions();
        let audio_waveforms = items::audio_waveform_cache_signature(&self.project.borrow())
            != items::audio_waveform_cache_signature(&edited);
        project::commit_edit_checked(&edited, message)?;
        let duration = edited.duration();
        *self.project.borrow_mut() = edited;
        if let Some(selected) = selection {
            let focus = self
                .context
                .focus
                .clone()
                .filter(|focus| selected.contains(focus))
                .or_else(|| selected.first().cloned());
            selection_state::set_selected_item_addresses(
                &self.selection,
                &self.project.borrow(),
                selected,
                focus,
            );
        }
        player_state::refresh_project(
            &self.player,
            player_state::ProjectChange {
                duration: Some(duration),
                video: true,
                audio: true,
                audio_waveforms,
                audio_beats: true,
                captions: true,
                inspector: true,
                ..Default::default()
            },
        );
        Ok(())
    }

    pub(super) fn delete_context_items(&mut self) -> Result<(), String> {
        let mut edited = self.project.borrow().clone();
        for address in &self.context.selected {
            edited
                .take_item(address)
                .ok_or("Selected item was removed before cutting")?;
        }
        edited.prune_folded_sequences();
        self.commit_context_edit(edited, "cut-timeline-items", Some(Vec::new()))
    }

    pub fn paste_context_clipboard(&mut self) -> Result<(), String> {
        let clipboard = self
            .clipboard
            .as_ref()
            .ok_or("Timeline clipboard is empty")?;
        let mut edited = self.project.borrow().clone();
        let result = items::paste_items(
            &mut edited,
            clipboard,
            &self.context.scope,
            player_state::current_time(&self.player),
        );
        if result.selection.is_empty() {
            return Err("Timeline clipboard could not be pasted into this scope".into());
        }
        self.commit_context_edit(edited, "paste-timeline-items", Some(result.selection))
    }

    pub fn confirm_delete_context_track(&mut self) -> Result<(), String> {
        self.delete_context_folded_track()
    }

    pub fn delete_context_folded_track(&mut self) -> Result<(), String> {
        let track = self
            .context
            .folded_track
            .as_ref()
            .ok_or("No nested track is selected")?;
        let mut edited = self.project.borrow().clone();
        if !edited.remove_track(track) {
            return Err("Nested track was removed while the menu was open".into());
        }
        edited.prune_folded_sequences();
        self.commit_context_edit(edited, "delete-selected-tracks", Some(Vec::new()))
    }

    pub(super) fn edit_context(&mut self, action: ContextMenuAction) -> Result<(), String> {
        use ContextMenuAction as A;
        let mut edited = self.project.borrow().clone();
        let mut selection = None;
        let mut selected_track = None;
        let message = match action {
            A::ReplaceProperties | A::PasteModifiers => {
                let result = if action == A::PasteModifiers {
                    self.property_clipboard
                        .borrow()
                        .append_modifiers(&mut edited, &self.context.selected)
                } else {
                    self.property_clipboard
                        .borrow()
                        .replace_properties(&mut edited, &self.context.selected)
                };
                if !result.changed {
                    return Err("Clipboard properties cannot be applied to this selection".into());
                }
                if result.stabilization {
                    return Err("Pasting stabilization requires the stabilization backend, which is unavailable in this editor".into());
                }
                "paste-item-properties"
            }
            A::Group | A::Ungroup | A::UnlinkFolder => {
                let first = self
                    .context
                    .selected
                    .first()
                    .ok_or("No timeline items are selected")?;
                let scope = SequenceTimeline::for_item(&edited, first)
                    .ok_or("Selected item scope no longer exists")?;
                selection = Some(
                    if action == A::Group {
                        items::group_item_addresses(&scope, &mut edited, &self.context.selected)
                    } else {
                        items::ungroup_item_addresses(&scope, &mut edited, &self.context.selected)
                    }
                    .ok_or("Selected items cannot be grouped or ungrouped")?,
                );
                "group-timeline-items"
            }
            A::FoldSequence => {
                let keys = self
                    .context
                    .selected
                    .iter()
                    .map(|address| {
                        selection_state::item_key(&edited, address)
                            .ok_or("Only top-level items can be folded here")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let folded = items::fold_items(&mut edited, &keys)
                    .ok_or("Selected items cannot be folded into a sequence")?;
                selection = Some(
                    folded
                        .into_iter()
                        .map(|key| {
                            selection_state::item_address(&edited, key)
                                .expect("new folded item has an address")
                        })
                        .collect(),
                );
                "fold-timeline-sequence"
            }
            A::MoveOutOfSequence => {
                let address = self
                    .context
                    .item
                    .as_ref()
                    .ok_or("No nested item is selected")?;
                let scope = SequenceTimeline::for_item(&edited, address)
                    .ok_or("Selected item scope no longer exists")?;
                selection = Some(vec![
                    scope
                        .move_item_out(&mut edited, address)
                        .ok_or("Item cannot be moved out of this sequence")?,
                ]);
                "move-timeline-item-out-of-sequence"
            }
            A::EnableBeatDetection | A::DisableBeatDetection => {
                let enabled = action == A::EnableBeatDetection;
                for address in &self.context.selected {
                    if let Some(item) = edited.audio_item_mut(address) {
                        item.beat_detection = enabled;
                    }
                }
                "toggle-beat-detection"
            }
            A::AddCaptionTrack | A::AddVideoTrack | A::AddAudioTrack => {
                let kind = match action {
                    A::AddCaptionTrack => TrackKind::Caption,
                    A::AddVideoTrack => TrackKind::Video,
                    _ => TrackKind::Audio,
                };
                let count = match kind {
                    TrackKind::Caption => edited.caption_tracks.len(),
                    TrackKind::Video => edited.video_tracks.len(),
                    TrackKind::Audio => edited.audio_tracks.len(),
                };
                let index = match (kind, self.context.at_top) {
                    (TrackKind::Audio, true) | (TrackKind::Caption | TrackKind::Video, false) => 0,
                    _ => count,
                };
                match kind {
                    TrackKind::Caption => edited.caption_tracks.insert(index, Default::default()),
                    TrackKind::Video => edited.video_tracks.insert(index, Default::default()),
                    TrackKind::Audio => edited.audio_tracks.insert(index, Default::default()),
                }
                selected_track = selection_state::track_address(
                    &edited,
                    TrackKey {
                        kind,
                        track_index: index,
                    },
                );
                "create-timeline-track"
            }
            A::AddFolderTrackTop | A::AddFolderTrackBottom => {
                let folder = self
                    .context
                    .item
                    .as_ref()
                    .ok_or("No folded sequence is selected")?;
                let (kind, reference) = match edited.item(folder) {
                    Some(project::ItemRef::Video(item)) => match item.content {
                        project::VideoItemContent::FoldedSequence(reference) => {
                            (ItemKind::Video, reference)
                        }
                        _ => return Err("Selected item is not a folded sequence".into()),
                    },
                    Some(project::ItemRef::Audio(item)) => match item.source {
                        project::AudioSource::FoldedSequence(reference) => {
                            (ItemKind::Audio, reference)
                        }
                        _ => return Err("Selected item is not a folded sequence".into()),
                    },
                    _ => return Err("Selected item is not a folded sequence".into()),
                };
                let sequence = edited
                    .folded_sequence_mut(reference.sequence_id)
                    .ok_or("Folded sequence was removed")?;
                let top = action == A::AddFolderTrackTop;
                match kind {
                    ItemKind::Video => {
                        let index = if top { sequence.video_tracks.len() } else { 0 };
                        sequence.video_tracks.insert(index, Default::default());
                    }
                    ItemKind::Audio => {
                        let index = if top { 0 } else { sequence.audio_tracks.len() };
                        sequence.audio_tracks.insert(index, Default::default());
                    }
                    ItemKind::Caption => unreachable!(),
                }
                let path = folder
                    .sequence_path()
                    .iter()
                    .copied()
                    .chain(std::iter::once(folder.item_id()))
                    .collect::<Vec<_>>();
                if !edited.expanded_sequence_paths.contains(&path) {
                    edited.expanded_sequence_paths.push(path);
                }
                "create-folded-sequence-track"
            }
            _ => return Err("Action requires a native editor handler".into()),
        };
        self.commit_context_edit(edited, message, selection)?;
        if let Some(track) = selected_track {
            selection_state::set_selected_track_addresses(
                &self.selection,
                &self.project.borrow(),
                vec![track.clone()],
                Some(track),
            );
        }
        Ok(())
    }

    pub fn set_context_menu_control(
        &mut self,
        control: ContextMenuControl,
        value: f64,
    ) -> Result<(), String> {
        if !value.is_finite() || value < control.minimum() || value > control.maximum() {
            return Err("Context-menu value is out of range".into());
        }
        if !self.context.menu.sections.iter().flatten().any(|entry| matches!(entry, ContextMenuEntry::Control(existing) if std::mem::discriminant(existing) == std::mem::discriminant(&control))) {
            return Err("Context-menu control is unavailable".into());
        }
        self.validate_context()?;
        let mut edited = self.project.borrow().clone();
        let mut changed = false;
        let message = match control {
            ContextMenuControl::PlaybackSpeed { .. } => {
                let speed = shrimply_math_core::fraction_snapped(
                    math::playback_speed_from_scale_position(value),
                    shrimply_math_core::FRACTION_ZERO,
                    Fraction::new(1_u64, 100_u64),
                );
                for address in &self.context.selected {
                    match edited.item_mut(address) {
                        Some(ItemMut::Video(item)) if item.playback_speed != speed => {
                            item.playback_speed = speed;
                            changed = true;
                        }
                        Some(ItemMut::Audio(item)) if item.playback_speed != speed => {
                            item.playback_speed = speed;
                            changed = true;
                        }
                        _ => {}
                    }
                }
                "selected-item-speed"
            }
            ContextMenuControl::AudioTrackGain { .. } => {
                let track = self
                    .context
                    .track
                    .as_ref()
                    .ok_or("No audio track is selected")?;
                let Some(TrackMut::Audio(track)) = edited.track_mut(track) else {
                    return Err("Audio track no longer exists".into());
                };
                let gain = value as f32;
                changed = track.gain_db != gain;
                track.gain_db = gain;
                "audio-track-gain"
            }
        };
        if !changed {
            return Ok(());
        }
        self.commit_context_edit(edited, message, None)
    }
}
