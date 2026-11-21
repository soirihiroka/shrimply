//! The native host forwards graph input and presents the shared Skia controller.
use serde_json::{Value, json};
#[cfg(target_os = "android")]
use shrimply_components_skia::canvas::TimelinePainter;
use shrimply_components_skia::canvas::Vec2;
use shrimply_framegraph_skia::{
    FrameGraphInputResult, FrameGraphKey, FrameGraphModifiers, FrameGraphPointerButton,
    FrameGraphPointerPosition, FrameGraphScrollInput,
};

use super::Runtime;

impl Runtime {
    pub(super) fn graph_request(&mut self, request: &Value) -> Result<Value, String> {
        self.update_inspector();
        let controller = self.inspector.as_ref().ok_or("Open a project first")?;
        self.framegraphs.request(controller, request)?;
        self.inspector_stamp = None;
        self.graph_dirty = true;
        self.timeline_changed()?;
        Ok(Value::Null)
    }

    pub fn cancel_graph_edit(&mut self) -> Result<(), String> {
        if let Some(controller) = &self.inspector {
            self.framegraphs.cancel(controller)?;
            self.graph_dirty = true;
        }
        Ok(())
    }

    pub fn graph_input(
        &mut self,
        surface: usize,
        event: crate::input::GraphInput,
        size: Vec2,
    ) -> Result<(), String> {
        if let crate::input::GraphInput::Pointer { kind: 29, .. } = &event {
            if let (Some(graph), Some(controller)) =
                (self.framegraphs.sessions.get_mut(&surface), &self.inspector)
            {
                graph.cancel(controller)?;
                self.graph_dirty = true;
                self.inspector_stamp = None;
            }
            return Ok(());
        }
        if !size.is_finite()
            || size.min_element() <= 0.0
            || !self.framegraphs.sessions.contains_key(&surface)
        {
            return Ok(());
        }
        if self
            .framegraphs
            .sessions
            .get(&surface)
            .is_some_and(|graph| !graph.is_editable())
        {
            return Ok(());
        }
        self.cancel_numeric()?;
        const ESCAPE: i32 = 111;
        if let crate::input::GraphInput::Pointer {
            kind: 28, point, ..
        } = &event
            && point.x as i32 == ESCAPE
        {
            self.cancel_graph_edit()?;
            return Ok(());
        }
        if let crate::input::GraphInput::Pointer {
            kind: 28,
            detail,
            flags,
            ..
        } = &event
            && flags & 1 != 0
            && let Some(character) = char::from_u32(*detail as u32)
            && matches!(character.to_ascii_lowercase(), 'z' | 'y')
        {
            let redo = character.eq_ignore_ascii_case(&'y') || flags & 2 != 0;
            self.request(&json!({"op":if redo {"redo"} else {"undo"}}))?;
            return Ok(());
        }
        let controller = self.inspector.as_ref().ok_or("Open a project first")?;
        let graph = self
            .framegraphs
            .sessions
            .get_mut(&surface)
            .expect("open graph");
        let position = |point: Vec2| FrameGraphPointerPosition {
            x: f64::from(point.x),
            y: f64::from(point.y),
            width: f64::from(size.x),
            height: f64::from(size.y),
        };
        let history = match event {
            crate::input::GraphInput::Touch(changes) => {
                graph.touch(controller, &changes, position(Vec2::ZERO))?
            }
            crate::input::GraphInput::Pointer {
                kind,
                point,
                detail,
                flags,
                extra,
            } => {
                graph.pointer = position(point);
                let modifiers = FrameGraphModifiers {
                    control: flags & 1 != 0,
                    shift: flags & 2 != 0,
                };
                let result = match kind {
                    24 => {
                        if detail >= 0.0 {
                            graph.core.update_pointer(position(point))
                        } else {
                            graph
                                .core
                                .pointer_moved(f64::from(point.x), f64::from(point.y))
                        }
                    }
                    25 => graph.core.begin_pointer(
                        match detail as i32 {
                            0 => FrameGraphPointerButton::Primary,
                            1 => FrameGraphPointerButton::Middle,
                            2 => FrameGraphPointerButton::Secondary,
                            _ => return Err("Invalid frame graph pointer button".into()),
                        },
                        position(point),
                        modifiers,
                    ),
                    26 => graph.core.end_pointer(),
                    27 => graph.core.scroll(
                        f64::from(detail),
                        f64::from(extra),
                        position(point),
                        modifiers.control,
                        if flags & 4 != 0 {
                            FrameGraphScrollInput::Surface
                        } else {
                            FrameGraphScrollInput::Wheel
                        },
                    ),
                    28 => match native_key(point.x as i32, detail as u32, modifiers.control) {
                        Some(key) => graph.core.key(key),
                        None => FrameGraphInputResult::default(),
                    },
                    30 => graph.core.pointer_left(),
                    29 => {
                        graph.cancel(controller)?;
                        FrameGraphInputResult::default()
                    }
                    _ => return Err("Invalid frame graph event".into()),
                };
                graph.apply(controller, result, false)?
            }
        };
        self.graph_dirty = true;
        self.inspector_stamp = None;
        if let Some(redo) = history {
            self.request(&json!({"op":if redo {"redo"} else {"undo"}}))?;
        }
        Ok(())
    }

    pub fn poll_graph(&mut self, surface: usize, size: Vec2) -> bool {
        let Some(graph) = self.framegraphs.sessions.get_mut(&surface) else {
            return false;
        };
        if !size.is_finite() || size.min_element() <= 0.0 {
            return false;
        }
        graph.pointer.width = f64::from(size.x);
        graph.pointer.height = f64::from(size.y);
        let controller = self.inspector.as_ref().expect("graph inspector exists");
        let settings = shrimply_preferences_core::snapshot(&self.backend.preferences);
        graph.configure_snapping(
            settings.timeline_magnet == "true",
            f64::from(settings.timeline_snap_radius_px),
        );
        let event = graph.core.poll_long_press(graph.pointer);
        let changed = event.redraw;
        match graph.apply(controller, event, true) {
            Ok(_) => {
                if changed {
                    self.inspector_stamp = None;
                }
                self.graph_dirty |= changed;
                changed
            }
            Err(error) => {
                self.error = Some(match graph.cancel(controller) {
                    Ok(()) => error,
                    Err(cancel) => format!("{error}; {cancel}"),
                });
                self.inspector_stamp = None;
                self.graph_dirty = true;
                true
            }
        }
    }

    #[cfg(target_os = "android")]
    pub fn graph_draw(&self, surface: usize, canvas: &skia_safe::Canvas, size: Vec2) {
        canvas.clear(skia_safe::Color4f::from(
            shrimply_cross_ui_theme::current().view_bg,
        ));
        if let Some(graph) = self.framegraphs.sessions.get(&surface) {
            graph.core.draw(
                &TimelinePainter::new(canvas),
                f64::from(size.x),
                f64::from(size.y),
            );
        }
    }
}

// Android keycodes are translated here; all commands and edit behavior live in FrameGraphCore.
fn native_key(keycode: i32, unicode: u32, control: bool) -> Option<FrameGraphKey> {
    const LEFT: i32 = 21;
    const RIGHT: i32 = 22;
    const HOME: i32 = 122;
    const END: i32 = 123;
    const BACKSPACE: i32 = 67;
    const DELETE: i32 = 112;
    const SPACE: i32 = 62;
    if control {
        return match char::from_u32(unicode)?.to_ascii_lowercase() {
            'c' => Some(FrameGraphKey::Copy),
            'v' => Some(FrameGraphKey::Paste),
            _ => None,
        };
    }
    match keycode {
        LEFT => Some(FrameGraphKey::PreviousFrame),
        RIGHT => Some(FrameGraphKey::NextFrame),
        HOME => Some(FrameGraphKey::Start),
        END => Some(FrameGraphKey::End),
        BACKSPACE | DELETE => Some(FrameGraphKey::Delete),
        SPACE => Some(FrameGraphKey::TogglePlayback),
        _ => match char::from_u32(unicode)? {
            '+' => Some(FrameGraphKey::ZoomIn),
            '-' => Some(FrameGraphKey::ZoomOut),
            _ => None,
        },
    }
}
