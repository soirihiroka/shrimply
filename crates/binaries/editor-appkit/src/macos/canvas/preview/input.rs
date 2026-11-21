use super::*;
use shrimply_editor_state::{player_state, preferences, preview_focus};
use shrimply_preview_interaction_skia::surface::{Clock, Configuration};
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, Key, KeyState, KeyboardEvent, Modifiers, PointerButton, PointerEvent,
    PointerInput, PointerSample, PointerTool, PreviewRefresh, PreviewResponse,
};
use shrimply_timeline_skia::selection_state;

impl CanvasView {
    pub(in crate::macos::canvas) fn preview_configuration(
        &self,
        state: &State,
        prefs: &preferences::PreferencesSnapshot,
    ) -> Configuration {
        let size = self.bounds().size;
        Configuration {
            size: glam::vec2(size.width.max(1.0) as f32, size.height.max(1.0) as f32),
            padding: prefs.preview_padding_px,
            guides_visible: state.guides_visible,
            fullscreen: state.fullscreen,
            snap_enabled: prefs.timeline_magnet == "true",
            snap_radius: prefs.timeline_snap_radius_px as f32,
            navigation_enabled: prefs.preview_zoom_pan_enabled,
        }
    }

    pub(in crate::macos::canvas) fn preview_clock(&self) -> Clock {
        let player = player_state::snapshot(&self.ivars().session.player_state);
        Clock {
            revision: player.revision,
            position: player.position,
            moving: player.playing || player.scrubbing,
        }
    }

    pub(in crate::macos::canvas) fn prepare_preview(&self) -> Result<(), String> {
        if self.window().is_none_or(|window| !window.isKeyWindow())
            || self.isHiddenOrHasHiddenAncestor()
        {
            return Ok(());
        }
        let session = &self.ivars().session;
        let clock = self.preview_clock();
        let prefs = preferences::snapshot(&session.preferences);
        let focus = preview_focus::snapshot(&session.preview_focus);
        let response = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Preview(state) = &mut *content else {
                return Ok(());
            };
            let mut project = session.project.borrow_mut();
            let selected =
                selection_state::focused_video_address(&session.selection_state, &project);
            state.sync_guides(prefs.preview_guides_visible);
            state
                .surface
                .focus(focus.map(|focus| (focus.item, focus.target)));
            let configuration = self.preview_configuration(state, &prefs);
            let had_provider = state.surface.provider().is_some();
            let invalidated = state.surface.context_invalidated() && state.surface.is_idle();
            let response =
                state
                    .surface
                    .ensure(&mut project, selected.as_ref(), clock, configuration)?;
            if had_provider != state.surface.provider().is_some() || invalidated {
                self.ivars().surface_dirty.set(true);
            }
            if let Some(exclusion) = state.surface.exclusion_update() {
                state.renderer.set_exclusion(exclusion);
            }
            state.renderer.set_project_revision(clock.revision);
            response
        };
        if response.redraw
            || response.cursor != CursorUpdate::Keep
            || response.edit.refresh != PreviewRefresh::NONE
        {
            self.apply_preview_response(response)?;
        }
        Ok(())
    }

    pub(in crate::macos::canvas) fn preview_pointer_event(&self, event: PointerEvent<'_>) -> bool {
        let result = self.prepare_preview().and_then(|()| {
            if self.preview_caption_pointer(&event)? {
                return Ok(true);
            }
            let session = &self.ivars().session;
            let clock = self.preview_clock();
            let prefs = preferences::snapshot(&session.preferences);
            let response = {
                let mut content = self.ivars().content.borrow_mut();
                let Content::Preview(state) = &mut *content else {
                    return Ok(false);
                };
                match event {
                    PointerEvent::Begin(input) => state.last_sample = Some(input.sample),
                    PointerEvent::Samples { input, .. } => {
                        if state.last_sample.is_some_and(|previous| {
                            previous.position == input.sample.position
                                && previous.pressure == input.sample.pressure
                                && previous.tilt == input.sample.tilt
                        }) {
                            return Ok(true);
                        }
                        state.last_sample = Some(input.sample);
                    }
                    PointerEvent::End(_) | PointerEvent::Cancel => state.last_sample = None,
                    _ => {}
                }
                let mut project = session.project.borrow_mut();
                let selected =
                    selection_state::focused_video_address(&session.selection_state, &project);
                let configuration = self.preview_configuration(state, &prefs);
                let response = state.surface.pointer(
                    &mut project,
                    selected.as_ref(),
                    clock,
                    configuration,
                    event,
                )?;
                if let Some(exclusion) = state.surface.exclusion_update() {
                    state.renderer.set_exclusion(exclusion);
                }
                if response.redraw && state.surface.navigation_active() {
                    state.caption_split_hover = None;
                }
                response
            };
            self.apply_preview_response(response)?;
            Ok(response.handled)
        });
        match result {
            Ok(handled) => handled,
            Err(error) => {
                self.show_error(&error);
                true
            }
        }
    }

    pub(in crate::macos::canvas) fn preview_keyboard(
        &self,
        event: &NSEvent,
        state: KeyState,
    ) -> bool {
        let key = match event
            .charactersIgnoringModifiers()
            .and_then(|text| text.to_string().chars().next())
        {
            Some('\u{1b}') => Key::Escape,
            Some('\u{7f}') | Some('\u{8}') => Key::Backspace,
            Some('\u{f728}') => Key::Delete,
            Some('\r') | Some('\n') => Key::Enter,
            Some('\t') => Key::Tab,
            Some(' ') => Key::Space,
            Some(key) => Key::Character(key.to_ascii_lowercase()),
            None => Key::Unknown,
        };
        self.preview_key_event(KeyboardEvent {
            key,
            state,
            repeat: event.isARepeat(),
            modifiers: modifiers(event.modifierFlags()),
        })
    }

    fn preview_key_event(&self, event: KeyboardEvent) -> bool {
        if let Err(error) = self.prepare_preview() {
            self.show_error(&error);
            return true;
        }
        let session = &self.ivars().session;
        let clock = self.preview_clock();
        let prefs = preferences::snapshot(&session.preferences);
        let response = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Preview(preview) = &mut *content else {
                return false;
            };
            let mut project = session.project.borrow_mut();
            let selected =
                selection_state::focused_video_address(&session.selection_state, &project);
            let configuration = self.preview_configuration(preview, &prefs);
            let response = match preview.surface.keyboard(
                &mut project,
                selected.as_ref(),
                clock,
                configuration,
                event,
            ) {
                Ok(response) => response,
                Err(error) => {
                    drop(project);
                    drop(content);
                    self.show_error(&error);
                    return true;
                }
            };
            if let Some(exclusion) = preview.surface.exclusion_update() {
                preview.renderer.set_exclusion(exclusion);
            }
            response
        };
        if let Err(error) = self.apply_preview_response(response) {
            self.show_error(&error);
        }
        response.handled
    }

    pub(in crate::macos::canvas) fn preview_modifiers_changed(&self, event: &NSEvent) {
        let modifiers = modifiers(event.modifierFlags());
        let previous = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Preview(state) = &mut *content else {
                return;
            };
            std::mem::replace(&mut state.modifiers, modifiers)
        };
        for (flag, key) in [
            (Modifiers::CONTROL, Key::Control),
            (Modifiers::SHIFT, Key::Shift),
            (Modifiers::ALT, Key::Alt),
        ] {
            if previous.contains(flag) != modifiers.contains(flag) {
                self.preview_key_event(KeyboardEvent {
                    key,
                    state: if modifiers.contains(flag) {
                        KeyState::Pressed
                    } else {
                        KeyState::Released
                    },
                    repeat: false,
                    modifiers,
                });
            }
        }
    }

    pub(in crate::macos::canvas) fn preview_input(&self, event: &NSEvent) -> PointerInput {
        use objc2_app_kit::{NSEventSubtype, NSEventType, NSPointingDeviceType};
        let tablet = event.r#type() == NSEventType::TabletPoint
            || event.subtype() == NSEventSubtype::TabletPoint;
        let tool = if tablet {
            match event.pointingDeviceType() {
                NSPointingDeviceType::Pen => PointerTool::Pen,
                NSPointingDeviceType::Eraser => PointerTool::Eraser,
                _ => PointerTool::Mouse,
            }
        } else {
            PointerTool::Mouse
        };
        let tilt = tablet.then(|| {
            let tilt = event.tilt();
            glam::Vec2::new(tilt.x as f32, tilt.y as f32)
        });
        PointerInput {
            sample: PointerSample {
                position: self.point(event),
                pressure: tablet.then(|| event.pressure()),
                tilt,
                time_millis: std::time::Duration::from_secs_f64(event.timestamp().max(0.0))
                    .as_millis() as u32,
            },
            tool,
            button: match event.buttonNumber() {
                0 => PointerButton::Primary,
                1 => PointerButton::Secondary,
                2 => PointerButton::Middle,
                other => PointerButton::Other(other as u32),
            },
            modifiers: modifiers(event.modifierFlags()),
        }
    }

    pub(in crate::macos::canvas) fn apply_preview_response(
        &self,
        response: PreviewResponse,
    ) -> Result<(), String> {
        if response.redraw {
            self.ivars().surface_dirty.set(true);
        }
        self.set_preview_cursor(response.cursor);
        let session = &self.ivars().session;
        if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut() {
            let _ = state.surface.take_committed();
        }
        if response.edit.refresh != PreviewRefresh::NONE {
            player_state::refresh_project(
                &session.player_state,
                player_state::ProjectChange {
                    video: response.edit.refresh.contains(PreviewRefresh::PREVIEW),
                    live_preview: response.edit.is_live(),
                    inspector: response.edit.refresh.contains(PreviewRefresh::INSPECTOR),
                    ..Default::default()
                },
            );
        }
        if response.edit.commits()
            && let Content::Preview(state) = &mut *self.ivars().content.borrow_mut()
        {
            state
                .surface
                .project_committed(player_state::snapshot(&session.player_state).revision);
        }
        Ok(())
    }

    pub(in crate::macos::canvas) fn refresh_live_preview(&self) {
        let requested = match &mut *self.ivars().content.borrow_mut() {
            Content::Preview(state) => state.surface.take_live_base_request(),
            _ => false,
        };
        if requested {
            let player = &self.ivars().session.player_state;
            player_state::refresh_project(
                player,
                player_state::ProjectChange {
                    video: true,
                    live_preview: true,
                    ..Default::default()
                },
            );
            if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut() {
                state
                    .surface
                    .live_base_requested(player_state::snapshot(player).revision);
            }
        }
    }

    pub(super) fn set_preview_cursor(&self, update: CursorUpdate) {
        let cursor = match update {
            CursorUpdate::Keep => return,
            CursorUpdate::Clear => Cursor::Default,
            CursorUpdate::Set(cursor) => cursor,
        };
        let mut content = self.ivars().content.borrow_mut();
        let Content::Preview(state) = &mut *content else {
            return;
        };
        if state.cursor_hidden && cursor != Cursor::Hidden {
            objc2_app_kit::NSCursor::unhide();
            state.cursor_hidden = false;
        }
        use objc2_app_kit::{
            NSCursor, NSCursorFrameResizeDirections, NSCursorFrameResizePosition,
            NSHorizontalDirections, NSVerticalDirections,
        };
        match cursor {
            Cursor::Default => NSCursor::arrowCursor().set(),
            Cursor::Pointer => NSCursor::pointingHandCursor().set(),
            Cursor::Crosshair => NSCursor::crosshairCursor().set(),
            Cursor::Move | Cursor::Grab => NSCursor::openHandCursor().set(),
            Cursor::Grabbing => NSCursor::closedHandCursor().set(),
            Cursor::Text => NSCursor::IBeamCursor().set(),
            Cursor::ResizeHorizontal => {
                NSCursor::columnResizeCursorInDirections(NSHorizontalDirections::All).set()
            }
            Cursor::ResizeVertical => {
                NSCursor::rowResizeCursorInDirections(NSVerticalDirections::All).set()
            }
            Cursor::ResizeDiagonalDown => NSCursor::frameResizeCursorFromPosition_inDirections(
                NSCursorFrameResizePosition::TopLeft,
                NSCursorFrameResizeDirections::All,
            )
            .set(),
            Cursor::ResizeDiagonalUp => NSCursor::frameResizeCursorFromPosition_inDirections(
                NSCursorFrameResizePosition::TopRight,
                NSCursorFrameResizeDirections::All,
            )
            .set(),
            Cursor::Hidden if !state.cursor_hidden => {
                NSCursor::hide();
                state.cursor_hidden = true;
            }
            Cursor::Hidden => {}
        }
    }
}

fn modifiers(flags: NSEventModifierFlags) -> Modifiers {
    let mut modifiers = Modifiers::NONE;
    for (native, shared) in [
        (NSEventModifierFlags::Shift, Modifiers::SHIFT),
        (NSEventModifierFlags::Control, Modifiers::CONTROL),
        (NSEventModifierFlags::Command, Modifiers::META),
        (NSEventModifierFlags::Option, Modifiers::ALT),
    ] {
        if flags.contains(native) {
            modifiers |= shared;
        }
    }
    modifiers
}
