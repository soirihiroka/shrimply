//! JNI translates native events; all interaction semantics live in the shared Scene.
use shrimply_components_skia::{
    canvas::vec2,
    input::{TouchChange, TouchPhase},
};
use shrimply_timeline_skia::{
    scene::{Event, KeyAction, PointerButton, TimelineModifiers},
    view::{TimelineScrollEvent, TimelineScrollInput},
};
use std::time::Instant;

pub const INPUT_STRIDE: usize = 6;
pub const MAX_CONTACTS: usize = 32;
pub const MAX_INPUT_VALUES: usize = INPUT_STRIDE * MAX_CONTACTS;
const MAX_PENDING_EVENTS: usize = 256;

pub enum Input {
    Scene(Event),
    Key(KeyAction),
    TogglePlaying,
    AdvancePlaybackSpeed,
    History { redo: bool },
}

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub density: f32,
    pub visible: bool,
}

#[derive(Default)]
pub struct Queue {
    pub events: Vec<Input>,
    pub at: Option<Instant>,
    pub overflow: bool,
    pub viewport: Viewport,
}
impl Queue {
    pub fn push(&mut self, input: Input) {
        if self.overflow {
            return;
        }
        // Only complete consecutive motion snapshots may be replaced. Begin/end
        // boundaries and separate contacts must retain their original ordering.
        if let Input::Scene(Event::Touch(next)) = &input
            && next.iter().all(|change| change.phase == TouchPhase::Move)
            && let Some(Input::Scene(Event::Touch(previous))) = self.events.last_mut()
            && previous.len() == next.len()
            && previous
                .iter()
                .zip(next)
                .all(|(a, b)| a.id == b.id && a.phase == TouchPhase::Move)
        {
            previous.clone_from(next);
            return;
        }
        if self.events.len() == MAX_PENDING_EVENTS {
            self.events.clear();
            self.events.push(Input::Scene(Event::Cancel));
            self.overflow = true;
            return;
        }
        self.events.push(input);
    }
}

pub fn decode(values: &[f32]) -> Option<Input> {
    assert!(!values.is_empty() && values.len().is_multiple_of(INPUT_STRIDE));
    assert!(
        values.iter().all(|value| value.is_finite()),
        "nonfinite native input"
    );
    if values[0] == 3.0 {
        return Some(Input::Scene(Event::Cancel));
    }
    if values[0] < 4.0 {
        return Some(Input::Scene(Event::Touch(
            values
                .chunks_exact(INPUT_STRIDE)
                .map(|value| {
                    let phase = match value[0] {
                        0.0 => TouchPhase::Begin,
                        1.0 => TouchPhase::Move,
                        2.0 => TouchPhase::End,
                        3.0 => TouchPhase::Cancel,
                        _ => panic!("invalid native touch phase"),
                    };
                    assert!(
                        value[3] >= 0.0
                            && value[3] < MAX_CONTACTS as f32
                            && value[3].fract() == 0.0
                    );
                    TouchChange {
                        id: value[3] as u64,
                        point: vec2(value[1], value[2]),
                        phase,
                    }
                })
                .collect(),
        )));
    }
    let [kind, x, y, detail, flags, extra] = *values else {
        panic!("invalid native pointer event")
    };
    let point = vec2(x, y);
    let modifiers = TimelineModifiers {
        ctrl: flags as u32 & 1 != 0,
        shift: flags as u32 & 2 != 0,
    };
    Some(Input::Scene(match kind {
        4.0 => Event::Motion { point, modifiers },
        5.0 => Event::Press {
            point,
            button: if detail == 1.0 {
                PointerButton::Middle
            } else {
                PointerButton::Primary
            },
            double: detail == 2.0,
            modifiers,
        },
        6.0 => Event::Release {
            point,
            button: if detail == 1.0 {
                PointerButton::Middle
            } else {
                PointerButton::Primary
            },
            modifiers,
        },
        7.0 => {
            let input = if flags as u32 & 4 != 0 {
                TimelineScrollInput::Surface
            } else {
                TimelineScrollInput::Wheel
            };
            let scale = if input == TimelineScrollInput::Wheel {
                shrimply_timeline_skia::metrics::SCROLL_PIXELS_PER_STEP as f32
            } else {
                1.0
            };
            Event::Scroll(TimelineScrollEvent {
                delta: vec2(detail, extra) * scale,
                ctrl: input == TimelineScrollInput::Wheel && modifiers.ctrl,
                pointer: Some(point),
                input,
            })
        }
        8.0 => {
            let key = char::from_u32(detail as u32).expect("native key character");
            return Some(match (key.to_ascii_lowercase(), modifiers.ctrl) {
                (' ', _) => Input::TogglePlaying,
                ('l', _) => Input::AdvancePlaybackSpeed,
                ('z', true) => Input::History {
                    redo: modifiers.shift,
                },
                ('y', true) => Input::History { redo: true },
                _ => Input::Key(KeyAction::from_key(key, modifiers.ctrl, modifiers.shift)?),
            });
        }
        9.0 => Event::Leave,
        10.0 => Event::ContextMenu {
            point,
            touch: false,
        },
        _ => panic!("unknown native input kind"),
    }))
}
