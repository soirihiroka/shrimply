use super::*;
use shrimply_skia_adw_core::{Axis, Scrollbar, slider};

impl Scene {
    pub(super) fn scrollbar_at(&self, point: Vec2) -> Option<Axis> {
        if !self.pointer_in_viewport(point) {
            return None;
        }
        let (horizontal, vertical) = self.scrollbar_geometry();
        if vertical.is_some_and(|bar| self.vertical_scrollbar.hit_test(bar, point)) {
            Some(Axis::Vertical)
        } else if self.horizontal_scrollbar.hit_test(horizontal, point) {
            Some(Axis::Horizontal)
        } else {
            None
        }
    }

    pub(super) fn scrollbar_geometry(&self) -> (Scrollbar, Option<Scrollbar>) {
        let project = self.project.borrow();
        let virtual_tracks = drawing::active_virtual_tracks(
            self.dragged_group.as_ref(),
            self.import_preview.as_ref(),
        );
        let width = f64::from(self.viewport.width());
        let height = f64::from(self.viewport.height());
        let player = player_state::snapshot(&self.player);
        let duration = folded_sequence::expanded_timeline_end(&project)
            .max(player.duration)
            .max(player.position)
            .max(Time::from_seconds(1));
        (
            horizontal_scrollbar(
                self.view,
                timeline_width(width),
                height,
                duration.as_secs_f64(),
                slider::idle_state(),
            ),
            vertical_scrollbar(
                self.view,
                width,
                height,
                timeline_track_content_height(&project, &virtual_tracks),
                slider::idle_state(),
            ),
        )
    }
}

impl Scene {
    pub fn begin_pan(&mut self, point: Vec2) {
        self.pointer_cancelled();
        if self.pointer_in_viewport(point) {
            self.event(Event::Press {
                point,
                button: PointerButton::Middle,
                double: false,
                modifiers: self.modifiers,
            });
            self.update_input();
        }
    }
    pub fn pan_to(&mut self, point: Vec2) {
        if self.view.drag_mode == DragMode::MiddlePan {
            self.event(Event::Motion {
                point,
                modifiers: self.modifiers,
            });
            self.update_input();
        }
    }
    pub fn end_pan(&mut self, point: Vec2) {
        self.event(Event::Release {
            point,
            button: PointerButton::Middle,
            modifiers: self.modifiers,
        });
        self.update_input();
    }

    pub(super) fn save_zoom(&self) {
        let zoom = Time::from_seconds_f64(self.view.seconds_per_pixel);
        let mut project = self.project.borrow_mut();
        if project.timeline_zoom != Some(zoom) {
            project.timeline_zoom = Some(zoom);
            project::save_view_state(&project);
        }
    }
}
