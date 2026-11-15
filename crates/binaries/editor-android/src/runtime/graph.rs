//! The native host forwards graph input and presents the shared Skia controller.
use serde_json::{Value, json};
#[cfg(target_os = "android")]
use shrimply_components_skia::canvas::TimelinePainter;
use shrimply_components_skia::canvas::Vec2;
use shrimply_framegraph_skia::{
    FrameGraphInputResult, FrameGraphKey, FrameGraphModifiers, FrameGraphPointerButton,
    FrameGraphPointerPosition, FrameGraphScrollInput,
};
use shrimply_inspector_document::remote::graph::GraphSession;

use super::{Runtime, player};

impl Runtime {
    pub(super) fn graph_request(&mut self, request: &Value) -> Result<Value, String> {
        match request["op"].as_str() {
            Some("framegraph_open") => {
                self.cancel_graph_edit()?;
                self.update_inspector();
                let controller = self.inspector.as_ref().ok_or("Open a project first")?;
                let document = self
                    .inspector_document
                    .as_ref()
                    .ok_or("Open the inspector first")?;
                self.context_serial = self
                    .context_serial
                    .checked_add(1)
                    .expect("editor menu serial overflow");
                self.framegraph = Some(GraphSession::open(
                    controller,
                    document,
                    request,
                    self.context_serial,
                )?);
                self.graph_stamp = None;
            }
            Some("framegraph_close") => {
                if self.framegraph.as_ref().map(|graph| graph.serial) != request["serial"].as_u64()
                {
                    return Err("Frame graph changed".into());
                }
                self.cancel_graph_edit()?;
                self.framegraph = None;
            }
            Some("framegraph_action") => {
                let controller = self.inspector.as_ref().ok_or("Open a project first")?;
                self.framegraph
                    .as_mut()
                    .ok_or("Frame graph is closed")?
                    .request(controller, request)?;
                self.inspector_stamp = None;
                self.timeline_changed()?;
            }
            _ => return Err("Unknown frame graph request".into()),
        }
        self.graph_dirty = true;
        Ok(self
            .framegraph
            .as_ref()
            .map(|graph| graph.snapshot())
            .unwrap_or(Value::Null))
    }

    pub fn cancel_graph_edit(&mut self) -> Result<(), String> {
        if let Some(graph) = &mut self.framegraph
            && let Some(controller) = &self.inspector
        {
            graph.cancel(controller)?;
            self.graph_dirty = true;
            self.graph_stamp = None;
        }
        Ok(())
    }

    pub fn graph_input(
        &mut self,
        event: crate::input::GraphInput,
        size: Vec2,
    ) -> Result<(), String> {
        if !size.is_finite() || size.min_element() <= 0.0 || self.framegraph.is_none() {
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
            self.framegraph = None;
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
        let graph = self.framegraph.as_mut().expect("open graph");
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
                self.graph_pointer = position(point);
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

    pub fn poll_graph(&mut self, size: Vec2) -> bool {
        if self.framegraph.is_none() || !size.is_finite() || size.min_element() <= 0.0 {
            return false;
        }
        self.graph_pointer.width = f64::from(size.x);
        self.graph_pointer.height = f64::from(size.y);
        let snapshot = player::snapshot(&self.player);
        let stamp = (snapshot.revision, snapshot.position);
        let controller = self.inspector.as_ref().expect("graph inspector exists");
        let graph = self.framegraph.as_mut().expect("graph exists");
        let result = (|| {
            if self.graph_stamp != Some(stamp) {
                graph.refresh(controller)?;
                self.graph_dirty = true;
                self.graph_stamp = Some(stamp);
            }
            let settings = shrimply_preferences_core::snapshot(&self.backend.preferences);
            graph.configure_snapping(
                settings.timeline_magnet == "true",
                f64::from(settings.timeline_snap_radius_px),
            );
            let event = graph.core.poll_long_press(self.graph_pointer);
            let changed = event.redraw;
            graph.apply(controller, event, true)?;
            self.graph_dirty |= changed;
            Ok::<_, String>(changed)
        })();
        match result {
            Ok(changed) => changed,
            Err(error) => {
                if let Err(cancel) = graph.cancel(controller) {
                    self.error = Some(format!("{error}; {cancel}"));
                } else {
                    self.error = Some(error);
                }
                self.framegraph = None;
                self.graph_dirty = true;
                true
            }
        }
    }

    #[cfg(target_os = "android")]
    pub fn graph_draw(&mut self, canvas: &skia_safe::Canvas, size: Vec2) {
        canvas.clear(skia_safe::Color4f::from(
            shrimply_cross_ui_theme::current().view_bg,
        ));
        if let Some(graph) = &self.framegraph {
            graph.core.draw(
                &TimelinePainter::new(canvas),
                f64::from(size.x),
                f64::from(size.y),
            );
        }
        self.graph_dirty = false;
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
