//! Fullscreen control reveal policy shared by native view adapters.
use glam::Vec2;
use shrimply_preview_core::math::fullscreen_pointer_positions_close;
use std::{cell::Cell, time::Duration};

pub const CONTROLS_HIDE_DELAY: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct ControlsMotion {
    pointer: Cell<Option<Vec2>>,
    controls_pointer: Cell<Option<Vec2>>,
    hidden_pointer: Cell<Option<Vec2>>,
    hidden_after_idle: Cell<bool>,
}

impl ControlsMotion {
    pub fn reset(&self) {
        self.pointer.set(None);
        self.controls_pointer.set(None);
        self.shown();
    }
    pub fn shown(&self) {
        self.hidden_pointer.set(None);
        self.hidden_after_idle.set(false);
    }

    pub fn hidden(&self, require_pointer_move: bool) {
        self.hidden_after_idle.set(require_pointer_move);
        self.hidden_pointer
            .set(require_pointer_move.then(|| self.pointer.get()).flatten());
    }

    pub fn pointer_motion(&self, position: Vec2) -> bool {
        if !pointer_moved(&self.pointer, position) {
            return false;
        }
        if !self.hidden_after_idle.get() {
            return true;
        }
        let Some(hidden) = self.hidden_pointer.get() else {
            self.hidden_pointer.set(Some(position));
            return false;
        };
        if fullscreen_pointer_positions_close(hidden, position) {
            return false;
        }
        self.shown();
        true
    }

    pub fn controls_enter(&self, position: Vec2) {
        self.controls_pointer.set(Some(position));
    }

    pub fn controls_motion(&self, position: Vec2) -> bool {
        pointer_moved(&self.controls_pointer, position)
    }
}

fn pointer_moved(position: &Cell<Option<Vec2>>, next: Vec2) -> bool {
    position
        .replace(Some(next))
        .is_none_or(|previous| !fullscreen_pointer_positions_close(previous, next))
}
