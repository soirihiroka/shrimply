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

pub enum GraphInput {
    Touch(Vec<TouchChange>),
    Pointer {
        kind: u8,
        point: glam::Vec2,
        detail: f32,
        flags: u32,
        extra: f32,
    },
}

pub enum Input {
    Graph(GraphInput),
    Scene(Event),
    Key(KeyAction),
    TogglePlaying,
    AdvancePlaybackSpeed,
    History {
        redo: bool,
    },
    PreviewTouch {
        changes: Vec<TouchChange>,
        size: glam::Vec2,
    },
    PreviewContext {
        point: glam::Vec2,
        touch: bool,
    },
    PreviewKey {
        event: shrimply_preview_interaction_skia::surface::KeyboardEvent,
        character: Option<char>,
    },
    PreviewPen {
        phase: u8,
        flags: u32,
        size: glam::Vec2,
        samples: Vec<shrimply_preview_interaction_skia::surface::PointerSample>,
    },
    PreviewScroll {
        position: glam::Vec2,
        delta: glam::Vec2,
        flags: u32,
    },
    Preview {
        flags: u32,
        phase: u8,
        position: glam::Vec2,
        size: glam::Vec2,
    },
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
    pub graph_viewport: Viewport,
    pub preview_viewport: Viewport,
}
impl Queue {
    pub fn push(&mut self, input: Input) {
        if self.overflow {
            return;
        }
        if let Input::Preview {
            phase: 1,
            position,
            size,
            flags,
        } = &input
            && let Some(Input::Preview {
                phase: 1,
                position: previous,
                size: previous_size,
                flags: previous_flags,
            }) = self.events.last_mut()
        {
            *previous = *position;
            *previous_size = *size;
            *previous_flags = *flags;
            return;
        }
        // Only complete consecutive motion snapshots may be replaced. Begin/end
        // boundaries and separate contacts must retain their original ordering.
        let touches = match (&input, self.events.last_mut()) {
            (
                Input::Graph(GraphInput::Touch(next)),
                Some(Input::Graph(GraphInput::Touch(previous))),
            ) => Some((next, previous)),
            (Input::Scene(Event::Touch(next)), Some(Input::Scene(Event::Touch(previous)))) => {
                Some((next, previous))
            }
            (
                Input::PreviewTouch {
                    changes: next,
                    size,
                },
                Some(Input::PreviewTouch {
                    changes: previous,
                    size: previous_size,
                }),
            ) if size == previous_size => Some((next, previous)),
            _ => None,
        };
        if let Some((next, previous)) = touches
            && next.iter().all(|change| change.phase == TouchPhase::Move)
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
            self.events.push(Input::Graph(GraphInput::Pointer {
                kind: 29,
                point: glam::Vec2::ZERO,
                detail: 0.0,
                flags: 0,
                extra: 0.0,
            }));
            self.events.push(Input::Preview {
                flags: 0,
                phase: 3,
                position: glam::Vec2::ZERO,
                size: glam::Vec2::ZERO,
            });
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
    if (36.0..=41.0).contains(&values[0]) {
        assert!(
            values.len().is_multiple_of(INPUT_STRIDE * 2),
            "invalid pen sample packet"
        );
        let mut samples = Vec::with_capacity(values.len() / (INPUT_STRIDE * 2));
        let mut phase = 0;
        let mut flags = 0;
        let mut size = glam::Vec2::ZERO;
        for record in values.chunks_exact(INPUT_STRIDE * 2) {
            let [
                kind,
                x,
                y,
                pressure,
                tilt,
                orientation,
                width,
                height,
                buttons,
                time_low,
                time_high,
                reserved,
            ] = *record
            else {
                unreachable!()
            };
            assert!(
                (36.0..=41.0).contains(&kind) && kind.fract() == 0.0 && reserved == 0.0,
                "invalid pen event"
            );
            phase = (kind - 36.0) as u8;
            flags = buttons as u32;
            size = vec2(width, height);
            samples.push(shrimply_preview_interaction_skia::surface::PointerSample {
                position: vec2(x, y),
                pressure: Some(pressure),
                tilt: Some(shrimply_preview_interaction_skia::surface::pen_tilt(
                    tilt,
                    orientation,
                )),
                time_millis: (time_low as u32) | ((time_high as u32) << 16),
            });
        }
        return Some(Input::PreviewPen {
            phase,
            flags,
            size,
            samples,
        });
    }
    if (20.0..=23.0).contains(&values[0]) {
        return Some(Input::Graph(GraphInput::Touch(touch_changes(values, 20.0))));
    }
    if (24.0..=30.0).contains(&values[0]) {
        let [kind, x, y, detail, flags, extra] = *values else {
            panic!("invalid graph pointer event")
        };
        assert_eq!(kind.fract(), 0.0, "invalid graph pointer phase");
        return Some(Input::Graph(GraphInput::Pointer {
            kind: kind as u8,
            point: vec2(x, y),
            detail,
            flags: flags as u32,
            extra,
        }));
    }
    if values[0] == 3.0 {
        return Some(Input::Scene(Event::Cancel));
    }
    if values[0] < 4.0 {
        return Some(Input::Scene(Event::Touch(touch_changes(values, 0.0))));
    }
    if (15.0..=18.0).contains(&values[0]) {
        return Some(Input::PreviewTouch {
            changes: touch_changes(values, 15.0),
            size: vec2(values[4], values[5]),
        });
    }
    let [kind, x, y, detail, flags, extra] = *values else {
        panic!("invalid native pointer event")
    };
    let point = vec2(x, y);
    if kind == 42.0 {
        use shrimply_preview_interaction_skia::surface::{Key, KeyState, KeyboardEvent};
        // Android KEYCODE_* values are native metadata, not editor shortcuts.
        const BACKSPACE: i32 = 67;
        const DELETE: i32 = 112;
        const ESCAPE: i32 = 111;
        const ENTER: i32 = 66;
        const SPACE: i32 = 62;
        const TAB: i32 = 61;
        const CONTROL_LEFT: i32 = 113;
        const CONTROL_RIGHT: i32 = 114;
        const SHIFT_LEFT: i32 = 59;
        const SHIFT_RIGHT: i32 = 60;
        const ALT_LEFT: i32 = 57;
        const ALT_RIGHT: i32 = 58;
        let character = char::from_u32(y as u32).filter(|character| *character != '\0');
        let key = match x as i32 {
            BACKSPACE => Key::Backspace,
            DELETE => Key::Delete,
            ESCAPE => Key::Escape,
            ENTER => Key::Enter,
            SPACE => Key::Space,
            TAB => Key::Tab,
            CONTROL_LEFT | CONTROL_RIGHT => Key::Control,
            SHIFT_LEFT | SHIFT_RIGHT => Key::Shift,
            ALT_LEFT | ALT_RIGHT => Key::Alt,
            _ => character.map_or(Key::Unknown, Key::Character),
        };
        let character = match key {
            Key::Backspace => Some('\u{8}'),
            Key::Delete => Some('\u{7f}'),
            Key::Escape => Some('\u{1b}'),
            Key::Enter => Some('\n'),
            Key::Space => Some(' '),
            Key::Tab => Some('\t'),
            _ => character,
        };
        let modifiers = preview_modifiers(detail as u32);
        return Some(Input::PreviewKey {
            event: KeyboardEvent {
                key,
                state: if flags == 0.0 {
                    KeyState::Pressed
                } else {
                    KeyState::Released
                },
                repeat: extra != 0.0,
                modifiers,
            },
            character,
        });
    }
    if kind == 19.0 {
        return Some(Input::PreviewContext {
            point,
            touch: extra as u32 & 1 != 0,
        });
    }
    if kind == 31.0 {
        return Some(Input::PreviewScroll {
            position: point,
            delta: vec2(detail, flags),
            flags: extra as u32,
        });
    }
    if (11.0..=14.0).contains(&kind) || kind == 32.0 || kind == 33.0 {
        assert_eq!(kind.fract(), 0.0, "invalid preview input phase");
        return Some(Input::Preview {
            flags: extra as u32,
            phase: if kind == 32.0 {
                4
            } else if kind == 33.0 {
                5
            } else {
                (kind - 11.0) as u8
            },
            position: point,
            size: vec2(detail, flags),
        });
    }
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
            return key_input(key, modifiers.ctrl, modifiers.shift);
        }
        9.0 => Event::Leave,
        10.0 => Event::ContextMenu {
            point,
            touch: false,
        },
        _ => panic!("unknown native input kind"),
    }))
}

pub fn preview_modifiers(flags: u32) -> shrimply_preview_interaction_skia::surface::Modifiers {
    use shrimply_preview_interaction_skia::surface::Modifiers;
    let mut modifiers = Modifiers::NONE;
    for (flag, modifier) in [
        (4, Modifiers::SHIFT),
        (8, Modifiers::CONTROL),
        (16, Modifiers::ALT),
        (32, Modifiers::META),
    ] {
        if flags & flag != 0 {
            modifiers |= modifier;
        }
    }
    modifiers
}

pub fn key_input(key: char, control: bool, shift: bool) -> Option<Input> {
    Some(match (key.to_ascii_lowercase(), control) {
        (' ', _) => Input::TogglePlaying,
        ('l', _) => Input::AdvancePlaybackSpeed,
        ('z', true) => Input::History { redo: shift },
        ('y', true) => Input::History { redo: true },
        _ => Input::Key(KeyAction::from_key(key, control, shift)?),
    })
}

fn touch_changes(values: &[f32], offset: f32) -> Vec<TouchChange> {
    values
        .chunks_exact(INPUT_STRIDE)
        .map(|value| {
            let phase = match value[0] - offset {
                0.0 => TouchPhase::Begin,
                1.0 => TouchPhase::Move,
                2.0 => TouchPhase::End,
                3.0 => TouchPhase::Cancel,
                _ => panic!("invalid native touch phase"),
            };
            assert!(value[3] >= 0.0 && value[3] < MAX_CONTACTS as f32 && value[3].fract() == 0.0);
            TouchChange {
                id: value[3] as u64,
                point: vec2(value[1], value[2]),
                phase,
            }
        })
        .collect()
}
