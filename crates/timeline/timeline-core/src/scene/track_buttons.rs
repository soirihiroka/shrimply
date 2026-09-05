use super::*;
use shrimply_skia_adw_core::button::Event;

pub(super) struct PressedTrackLabel {
    id: TrackButtonId,
    address: project::TrackAddress,
    toggle: bool,
    extend: bool,
}

impl Scene {
    pub(super) fn pointer_in_viewport(&self, point: Vec2) -> bool {
        self.viewport.contains(point) && point.cmplt(self.viewport.max).all()
    }

    pub fn pointer_moved(&mut self, point: Vec2) {
        self.pointer = Some(point);
        let hovered = self
            .pointer_in_viewport(point)
            .then_some(())
            .filter(|_| self.scrollbar_at(point).is_none())
            .and_then(|_| {
                track_button_at(
                    &self.project.borrow(),
                    self.view,
                    point.x.into(),
                    point.y.into(),
                )
            });
        if self.hovered_button != hovered {
            if let Some(previous) = self.hovered_button.take() {
                self.buttons
                    .entry(previous)
                    .or_default()
                    .event(Event::PointerLeft);
            }
            if let Some(hovered) = hovered {
                self.buttons
                    .entry(hovered)
                    .or_default()
                    .event(Event::PointerEntered);
            }
            self.hovered_button = hovered;
        }
        self.update_cut_preview(point);
    }

    pub fn pointer_exited(&mut self) {
        self.pointer = None;
        self.cut_preview = None;
        if let Some(previous) = self.hovered_button.take() {
            self.buttons
                .entry(previous)
                .or_default()
                .event(Event::PointerLeft);
        }
    }

    pub fn pointer_cancelled(&mut self) {
        self.horizontal_scrollbar.end_drag();
        self.vertical_scrollbar.end_drag();
        self.horizontal_scrollbar.cancel_scroll();
        self.vertical_scrollbar.cancel_scroll();
        self.scrollbar_drag = None;
        if let Some(pressed) = self.pressed_label.take()
            && pressed.id.1 != TrackLabelAction::Select
        {
            self.buttons
                .entry(pressed.id)
                .or_default()
                .event(Event::Cancelled);
        }
        if let Some(hovered) = self.hovered_button.take() {
            self.buttons
                .entry(hovered)
                .or_default()
                .event(Event::Cancelled);
        }
        if self.seeking {
            player_state::set_scrubbing(&self.player, false);
        }
        self.seeking = false;
        self.dragged_group = None;
        self.resize_drag = None;
        self.folded_drag = None;
        self.cut_preview = None;
        self.cutting = false;
        self.double_click = false;
        self.transition = None;
        self.view.selection = None;
        self.drag_moved = false;
        self.view.drag_mode = DragMode::None;
    }

    pub(super) fn press_track_label(&mut self, point: Vec2, toggle: bool, extend: bool) -> bool {
        let project = self.project.borrow();
        let Some(id) = track_label_action_at(&project, self.view, point.x.into(), point.y.into())
        else {
            return false;
        };
        let Some(address) = selection_state::track_address(&project, id.0) else {
            return false;
        };
        if id.1 != TrackLabelAction::Select {
            self.buttons.entry(id).or_default().event(Event::Pressed);
        }
        self.pressed_label = Some(PressedTrackLabel {
            id,
            address,
            toggle,
            extend,
        });
        true
    }

    pub(super) fn release_track_label(&mut self) -> Result<Option<TrackButtonId>, String> {
        let Some(PressedTrackLabel {
            id: pressed,
            address,
            toggle,
            extend,
        }) = self.pressed_label.take()
        else {
            return Ok(None);
        };
        let project = self.project.borrow();
        let released = self
            .pointer
            .filter(|point| self.pointer_in_viewport(*point))
            .and_then(|point| {
                track_label_action_at(&project, self.view, point.x.into(), point.y.into())
            });
        let same_track =
            selection_state::track_address(&project, pressed.0).as_ref() == Some(&address);
        let clicked = if pressed.1 == TrackLabelAction::Select {
            !self.drag_moved
        } else {
            self.buttons
                .entry(pressed)
                .or_default()
                .event(Event::Released)
                .clicked
        };
        drop(project);
        if !clicked || !same_track || released != Some(pressed) {
            return Ok(None);
        }
        match pressed.1 {
            TrackLabelAction::Select => {
                track_controls::select_track(&self.selection, pressed.0, toggle, extend);
                Ok(None)
            }
            TrackLabelAction::Toggle => {
                let mut project = self.project.borrow_mut();
                let mut edited = project.clone();
                if !track_controls::toggle_track_enabled(&mut edited, pressed.0) {
                    return Err("Track was removed before it could be toggled".into());
                }
                project::commit_edit_checked(&edited, "toggle-track-enabled")
                    .map_err(|error| format!("Could not save track enabled state: {error}"))?;
                let change = player_state::ProjectChange {
                    duration: Some(edited.duration()),
                    audio: pressed.0.kind == TrackKind::Audio,
                    video: pressed.0.kind == TrackKind::Video,
                    captions: pressed.0.kind == TrackKind::Caption,
                    inspector: true,
                    ..player_state::ProjectChange::default()
                };
                *project = edited;
                drop(project);
                player_state::refresh_project(&self.player, change);
                Ok(None)
            }
            TrackLabelAction::Add
            | TrackLabelAction::AudioRecord
            | TrackLabelAction::VideoRecord => Ok(Some(pressed)),
        }
    }
}
