use super::*;
use crate::project::ItemAddress;
use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};

pub enum KeyAction {
    Copy,
    Cut,
    Paste,
    ReplaceProperties,
    Group,
    Ungroup,
    Split { select_left: bool },
    RippleTrim,
    Delete { ripple: bool },
    ToggleZoom,
}

/// An immutable selection captured before presenting a native confirmation dialog.
pub struct TrackDeletion {
    tracks: Vec<project::TrackAddress>,
}

impl KeyAction {
    /// GTK key bindings; the native adapter supplies its platform shortcut modifier.
    pub fn from_key(key: char, shortcut: bool, shift: bool) -> Option<Self> {
        Some(match (key.to_ascii_lowercase(), shortcut) {
            ('c', true) => Self::Copy,
            ('x', true) => Self::Cut,
            ('v', true) if shift => Self::ReplaceProperties,
            ('v', true) => Self::Paste,
            ('g', true) if shift => Self::Ungroup,
            ('g', true) => Self::Group,
            ('s', false) => Self::Split { select_left: shift },
            ('q', false) => Self::RippleTrim,
            ('z', false) => Self::ToggleZoom,
            ('d' | '\u{7f}' | '\u{8}' | '\u{f728}', false) => Self::Delete { ripple: shift },
            _ => return None,
        })
    }
}

impl Scene {
    pub fn key_action(&mut self, action: KeyAction) -> Result<Option<ContextMenuRequest>, String> {
        self.pointer_cancelled();
        let project = self.project.borrow();
        self.context.selected = selection_state::selected_item_addresses(&self.selection, &project);
        self.context.focus = selection_state::focused_item_address(&self.selection, &project);
        self.context.folded_track = None;
        self.context.scope = selection_state::active_scope(&self.selection);
        self.context.tracks = selection_state::selected_track_addresses(&self.selection, &project);
        drop(project);
        let context = match action {
            KeyAction::Copy => Some(ContextMenuAction::Copy),
            KeyAction::Cut => Some(ContextMenuAction::Cut),
            KeyAction::Paste => Some(ContextMenuAction::Paste),
            KeyAction::ReplaceProperties => Some(ContextMenuAction::ReplaceProperties),
            KeyAction::Group => Some(ContextMenuAction::Group),
            KeyAction::Ungroup => Some(ContextMenuAction::Ungroup),
            _ => None,
        };
        if let Some(action) = context {
            return self.dispatch_context_action(action);
        }
        if matches!(action, KeyAction::ToggleZoom) {
            self.toggle_zoom();
            return Ok(None);
        }
        let mut edited = self.project.borrow().clone();
        let scope = SequenceTimeline::new(self.context.scope.clone());
        let position = player_state::current_time(&self.player);
        let mut seek = None;
        let (message, selected) = match action {
            KeyAction::Split { select_left } => {
                let addresses = scope
                    .items(&edited)
                    .into_iter()
                    .filter(|address| {
                        scope
                            .timeline_item_times(&edited, address)
                            .is_some_and(|(start, end)| start < position && position < end)
                    })
                    .collect::<Vec<_>>();
                let (left, right) =
                    items::split_item_addresses(&scope, &mut edited, &addresses, position);
                let selected = if select_left { left } else { right };
                if selected.is_empty() {
                    return Ok(None);
                }
                ("split-timeline-item", selected)
            }
            KeyAction::RippleTrim => {
                let Some(result) = items::ripple_trim_item_addresses(
                    &scope,
                    &mut edited,
                    &self.context.selected,
                    position,
                ) else {
                    return Ok(None);
                };
                seek = Some(result.shifted_position);
                ("ripple-trim-timeline-items", result.selection)
            }
            KeyAction::Delete { ripple } => {
                if let Some(gap) = selection_state::selected_gap(&self.selection) {
                    if items::delete_track_gap(&mut edited, gap).is_none() {
                        return Ok(None);
                    }
                } else if !self.context.selected.is_empty() {
                    if ripple {
                        let Some(result) = items::ripple_delete_item_addresses(
                            &scope,
                            &mut edited,
                            &self.context.selected,
                            position,
                        ) else {
                            return Ok(None);
                        };
                        seek = Some(result.shifted_position);
                    } else {
                        for address in &self.context.selected {
                            edited
                                .take_item(address)
                                .ok_or("Selected item no longer exists")?;
                        }
                    }
                } else {
                    let clip_count = self
                        .context
                        .tracks
                        .iter()
                        .filter_map(|track| edited.track(track))
                        .map(|track| match track {
                            project::TrackRef::Caption(track) => track.items.len(),
                            project::TrackRef::Video(track) => track.items.len(),
                            project::TrackRef::Audio(track) => track.items.len(),
                        })
                        .sum();
                    if clip_count > 0 {
                        return Ok(Some(ContextMenuRequest::DeleteTracks { clip_count }));
                    }
                    for track in &self.context.tracks {
                        if !edited.remove_track(track) {
                            return Err("Selected track no longer exists".into());
                        }
                    }
                }
                edited.prune_folded_sequences();
                ("delete-timeline-selection", Vec::<ItemAddress>::new())
            }
            _ => unreachable!("context and zoom commands handled above"),
        };
        self.commit_context_edit(edited, message, Some(selected))?;
        selection_state::set_selected_gap(&self.selection, None);
        if let Some(time) = seek {
            player_state::seek_time(&self.player, time.min(self.project.borrow().duration()));
        }
        Ok(None)
    }

    pub fn track_deletion(&self) -> TrackDeletion {
        TrackDeletion {
            tracks: self.context.tracks.clone(),
        }
    }

    pub fn confirm_delete_selected_tracks(
        &mut self,
        deletion: TrackDeletion,
    ) -> Result<(), String> {
        let mut edited = self.project.borrow().clone();
        for track in &deletion.tracks {
            if !edited.remove_track(track) {
                return Err("Selected track no longer exists".into());
            }
        }
        edited.prune_folded_sequences();
        self.commit_context_edit(edited, "delete-selected-tracks", Some(Vec::new()))
    }

    fn toggle_zoom(&mut self) {
        let width = timeline_width(f64::from(self.viewport.width()));
        if width <= 0.0 {
            return;
        }
        let player = player_state::snapshot(&self.player);
        let project = self.project.borrow();
        let duration = project
            .duration()
            .max(player.duration)
            .max(Time::from_seconds(1));
        let minimum = min_seconds_per_pixel(frame_step_seconds(&project));
        crate::math::toggle_timeline_zoom(
            &mut self.view,
            duration,
            player.position,
            width,
            minimum,
        );
        self.horizontal_scrollbar.cancel_scroll();
        drop(project);
        self.save_zoom();
    }
}
