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

    pub(super) fn begin_scrollbar_drag(&mut self, point: Vec2) -> bool {
        let Some(axis) = self.scrollbar_at(point) else {
            return false;
        };
        let (horizontal, vertical) = self.scrollbar_geometry();
        if matches!(axis, Axis::Vertical)
            && let Some(vertical) = vertical
            && self
                .vertical_scrollbar
                .begin(vertical, point, |value| self.view.scroll_y = value)
                == slider::Begin::Drag
        {
            self.scrollbar_drag = Some(Axis::Vertical);
            return true;
        }
        if matches!(axis, Axis::Horizontal)
            && self
                .horizontal_scrollbar
                .begin(horizontal, point, |value| self.view.scroll_seconds = value)
                == slider::Begin::Drag
        {
            self.scrollbar_drag = Some(Axis::Horizontal);
            return true;
        }
        // Even a thumb with no available travel owns its hit region.
        true
    }

    pub(super) fn drag_scrollbar(&mut self, point: Vec2) -> bool {
        let Some(axis) = self.scrollbar_drag else {
            return false;
        };
        let (horizontal, vertical) = self.scrollbar_geometry();
        match axis {
            Axis::Horizontal => {
                self.horizontal_scrollbar.drag_by(
                    horizontal,
                    f64::from(point.x - self.drag_origin.x),
                    |value| self.view.scroll_seconds = value,
                );
            }
            Axis::Vertical => {
                if let Some(vertical) = vertical {
                    self.vertical_scrollbar.drag_by(
                        vertical,
                        f64::from(point.y - self.drag_origin.y),
                        |value| self.view.scroll_y = value,
                    );
                }
            }
        }
        true
    }

    pub(super) fn scroll_over_scrollbar(&mut self, point: Vec2, delta: Vec2) -> bool {
        if !self.pointer_in_viewport(point) {
            return false;
        }
        let (horizontal, vertical) = self.scrollbar_geometry();
        let horizontal_delta = if delta.x.abs() > f32::EPSILON {
            delta.x
        } else {
            delta.y
        };
        let event = self.horizontal_scrollbar.scroll_pages_at(
            horizontal,
            Some(point),
            crate::math::scrollbar_wheel_pages(-f64::from(horizontal_delta)),
            |value| self.view.scroll_seconds = value,
        );
        if event.handled {
            self.vertical_scrollbar.cancel_scroll();
            return true;
        }
        if let Some(vertical) = vertical {
            let vertical_delta = if delta.y.abs() > f32::EPSILON {
                delta.y
            } else {
                delta.x
            };
            let event = self.vertical_scrollbar.scroll_units_at(
                vertical,
                Some(point),
                -f64::from(vertical_delta),
                |value| self.view.scroll_y = value,
            );
            if event.handled {
                self.horizontal_scrollbar.cancel_scroll();
                return true;
            }
        }
        false
    }
}

impl Scene {
    pub fn begin_pan(&mut self, point: Vec2) {
        self.pointer_cancelled();
        if self.pointer_in_viewport(point) {
            self.view.begin_pan(point.as_dvec2());
        }
    }

    pub fn pan_to(&mut self, point: Vec2) {
        if self.view.drag_mode != DragMode::MiddlePan {
            return;
        }
        self.pointer_moved(point);
        self.view.pan_to(point.as_dvec2());
        let project = self.project.borrow();
        let player = player_state::snapshot(&self.player);
        let virtual_tracks = drawing::active_virtual_tracks(None, self.import_preview.as_ref());
        self.view.clamp(
            folded_sequence::expanded_timeline_end(&project)
                .max(player.duration)
                .max(player.position)
                .as_secs_f64(),
            timeline_width(self.viewport.width().into()),
            min_seconds_per_pixel(frame_step_seconds(&project)),
            timeline_track_content_height(&project, &virtual_tracks),
            self.viewport.height().into(),
        );
    }

    pub fn end_pan(&mut self, point: Vec2) {
        self.pan_to(point);
        self.view.drag_mode = DragMode::None;
        self.view.drag_moved = false;
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
