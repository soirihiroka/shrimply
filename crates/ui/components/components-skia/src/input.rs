//! Toolkit-independent touch recognition. Hosts forward stable contact IDs and
//! positions in logical pixels; gestures never depend on an operating system.
use crate::{
    Vec2,
    math::{touch_dragged, touch_geometry},
};
use std::time::{Duration, Instant};

pub const LONG_PRESS_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchPhase {
    Begin,
    Move,
    End,
    Cancel,
}

#[derive(Clone, Copy, Debug)]
pub struct TouchChange {
    pub id: u64,
    pub point: Vec2,
    pub phase: TouchPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchCancelReason {
    Native,
    GestureChange,
}

#[derive(Clone, Copy, Debug)]
pub enum TouchAction {
    Press(Vec2),
    Move(Vec2),
    Release(Vec2),
    Cancel(TouchCancelReason),
    Transform {
        previous_center: Vec2,
        center: Vec2,
        scale: f32,
    },
}

#[derive(Default)]
pub struct TouchInput {
    contacts: Vec<(u64, Vec2)>,
    mode: TouchMode,
}

#[derive(Default)]
enum TouchMode {
    #[default]
    Idle,
    Single {
        id: u64,
        origin: Vec2,
        point: Vec2,
        pressed: bool,
        began: Instant,
    },
    Transform {
        ids: Vec<u64>,
        center: Vec2,
        span: f32,
    },
    // A finger remaining after a multi-touch gesture cannot start an item drag.
    AwaitLift,
}

impl TouchInput {
    pub fn waiting_long_press(&self) -> bool {
        matches!(self.mode, TouchMode::Single { pressed: false, .. })
    }

    pub fn poll_long_press(&mut self) -> Option<Vec2> {
        if let TouchMode::Single {
            origin,
            pressed: false,
            began,
            ..
        } = self.mode
            && began.elapsed() >= LONG_PRESS_DELAY
        {
            self.mode = TouchMode::AwaitLift;
            return Some(origin);
        }
        None
    }
    pub fn active(&self) -> bool {
        !self.contacts.is_empty()
    }

    pub fn cancel(&mut self) -> Vec<TouchAction> {
        let active = !matches!(self.mode, TouchMode::Idle);
        self.contacts.clear();
        self.mode = TouchMode::Idle;
        if active {
            vec![TouchAction::Cancel(TouchCancelReason::Native)]
        } else {
            Vec::new()
        }
    }

    /// Apply a complete native event batch before calculating its gesture. This
    /// prevents artificial pinches when two contacts move in the same frame.
    pub fn update(&mut self, changes: &[TouchChange]) -> Vec<TouchAction> {
        let cancelled = changes
            .iter()
            .any(|change| change.phase == TouchPhase::Cancel);
        let mut released = None;
        for change in changes {
            assert!(change.point.is_finite(), "nonfinite touch position");
            let index = self.contacts.iter().position(|(id, _)| *id == change.id);
            match change.phase {
                TouchPhase::Begin => {
                    assert!(index.is_none(), "touch contact began twice");
                    self.contacts.push((change.id, change.point));
                }
                TouchPhase::Move => {
                    // Native queues can outlive a focus/lifecycle cancellation.
                    // A new begin, not queued motion, starts the next gesture.
                    let Some(index) = index else {
                        continue;
                    };
                    self.contacts[index].1 = change.point;
                }
                TouchPhase::End => {
                    let Some(index) = index else {
                        continue;
                    };
                    self.contacts.remove(index);
                    released = Some((change.id, change.point));
                }
                TouchPhase::Cancel => {
                    if let Some(index) = index {
                        self.contacts.remove(index);
                    }
                }
            }
        }
        if cancelled {
            self.mode = if self.contacts.is_empty() {
                TouchMode::Idle
            } else {
                TouchMode::AwaitLift
            };
            return vec![TouchAction::Cancel(TouchCancelReason::Native)];
        }
        self.contacts.sort_unstable_by_key(|(id, _)| *id);
        let previous = std::mem::take(&mut self.mode);
        let mut actions = Vec::new();
        match self.contacts.as_slice() {
            [] => {
                if let TouchMode::Single {
                    id,
                    origin,
                    point,
                    pressed,
                    ..
                } = previous
                {
                    if !pressed {
                        actions.push(TouchAction::Press(origin));
                    }
                    actions.push(TouchAction::Release(
                        released
                            .filter(|(released, _)| *released == id)
                            .map_or(point, |(_, point)| point),
                    ));
                }
            }
            [(id, point)] => {
                self.mode = match previous {
                    TouchMode::Idle => {
                        // A second contact must be able to start a pan without
                        // selecting a clip, seeking or pausing playback first.
                        actions.push(TouchAction::Move(*point));
                        TouchMode::Single {
                            id: *id,
                            origin: *point,
                            point: *point,
                            pressed: false,
                            began: Instant::now(),
                        }
                    }
                    TouchMode::Single {
                        id: previous_id,
                        origin,
                        point: previous_point,
                        mut pressed,
                        began,
                    } if previous_id == *id => {
                        if !pressed && touch_dragged(origin, *point) {
                            actions.push(TouchAction::Press(origin));
                            pressed = true;
                        }
                        if previous_point != *point {
                            actions.push(TouchAction::Move(*point));
                        }
                        TouchMode::Single {
                            id: *id,
                            origin,
                            point: *point,
                            pressed,
                            began,
                        }
                    }
                    TouchMode::Single { .. } => {
                        actions.push(TouchAction::Cancel(TouchCancelReason::GestureChange));
                        TouchMode::AwaitLift
                    }
                    TouchMode::Transform { .. } | TouchMode::AwaitLift => TouchMode::AwaitLift,
                };
            }
            contacts => {
                let ids: Vec<_> = contacts.iter().map(|(id, _)| *id).collect();
                let (center, span) = touch_geometry(contacts);
                match previous {
                    TouchMode::Single { .. } => {
                        actions.push(TouchAction::Cancel(TouchCancelReason::GestureChange))
                    }
                    TouchMode::Transform {
                        ids: previous_ids,
                        center: previous_center,
                        span: previous_span,
                    } if previous_ids == ids => {
                        let scale = if previous_span > f32::EPSILON && span > f32::EPSILON {
                            span / previous_span
                        } else {
                            1.0
                        };
                        if center != previous_center || scale != 1.0 {
                            actions.push(TouchAction::Transform {
                                previous_center,
                                center,
                                scale,
                            });
                        }
                    }
                    TouchMode::Idle | TouchMode::Transform { .. } | TouchMode::AwaitLift => {}
                }
                self.mode = TouchMode::Transform { ids, center, span };
            }
        }
        actions
    }
}
