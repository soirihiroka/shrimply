use super::*;
use shrimply_preview_core::{
    Cursor, CursorUpdate, Key, KeyState, KeyboardEvent, Modifiers, PointerButton, PointerEvent,
    PointerInput, PointerSample, PointerTool, PreviewRefresh, PreviewResponse,
};
use shrimply_preview_interaction_core::controller::Preparation;
use shrimply_state::{player_state, preferences, preview_focus};
use shrimply_timeline_core::selection_state;

impl CanvasView {
    pub(in crate::macos::canvas) fn prepare_preview(&self) -> Result<(), String> {
        if self.window().is_none_or(|window| !window.isKeyWindow())
            || self.isHiddenOrHasHiddenAncestor()
        {
            return Ok(());
        }
        let session = &self.ivars().session;
        let player = player_state::snapshot(&session.player_state);
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
            let target = selected
                .as_ref()
                .and_then(|address| project.video_item(address))
                .map(|item| {
                    focus
                        .as_ref()
                        .filter(|focus| {
                            Some(&focus.item) == selected.as_ref()
                                && item.owns_preview_target(focus.target)
                        })
                        .map_or_else(|| item.default_preview_target(), |focus| focus.target)
                });
            let selection = selected.as_ref().zip(target);
            let changed_selection = state
                .controller
                .provider
                .as_ref()
                .is_some_and(|prepared| selection != Some((&prepared.item, prepared.target)));
            let response = if changed_selection {
                state.controller.cancel(&mut project, &state.expressions)
            } else {
                PreviewResponse::IGNORED
            };
            if let Some(address) = &selected
                && project
                    .video_item(address)
                    .is_some_and(|item| item.tracking_camera_source().is_some())
            {
                return Err("Tracked camera preview requires the camera reconstruction backend, which is not connected to Metal yet".into());
            }
            state.sync_guides(&project.preview_guides, prefs.preview_guides_visible);
            let size = self.bounds().size;
            let viewport = guides::viewport(
                glam::IVec2::new(size.width as i32, size.height as i32),
                project.canvas_size,
                prefs.preview_padding_px,
                state.guides_visible,
                state.fullscreen,
            );
            state.viewport = Some(viewport);
            if let Some((_, _, audio_analysis)) =
                state.audio_analysis.as_ref().filter(|(revision, time, _)| {
                    *revision == player.revision && *time == player.position
                })
            {
                state.controller.ensure(
                    &project,
                    selection,
                    player.position,
                    Preparation {
                        project_revision: player.revision,
                        viewport,
                        audio_analysis,
                        expression_cache: &state.expressions,
                        snap_enabled: prefs.timeline_magnet == "true",
                        snap_radius_px: prefs.timeline_snap_radius_px as f32,
                        guides: state
                            .guides_visible
                            .then_some(project.preview_guides.as_ref()),
                        camera_sampler: |_, _, _| {
                            unreachable!("tracked camera was checked before provider preparation")
                        },
                    },
                )?;
            } else if state.controller.sequence == PointerSequence::Idle {
                state.controller.provider = None;
            }
            let excluded = state
                .controller
                .provider
                .as_ref()
                .and_then(|prepared| prepared.provider.base_frame_exclusion());
            state.controller.base_exclusion = excluded;
            state.renderer.set_exclusion(excluded);
            state.renderer.set_project_revision(player.revision);
            response
        };
        self.apply_preview_response(response)
    }

    pub(in crate::macos::canvas) fn preview_pointer_event(&self, event: PointerEvent<'_>) {
        let result = self.prepare_preview().and_then(|()| {
            if self.preview_caption_pointer(&event)? {
                return Ok(());
            }
            let response = {
                let mut content = self.ivars().content.borrow_mut();
                let Content::Preview(state) = &mut *content else {
                    return Ok(());
                };
                if state.guide_input.active() || state.controller.sequence == PointerSequence::Guide
                {
                    return Ok(());
                }
                if matches!(event, PointerEvent::Hover(_))
                    && state.guide_input.cursor() != GuideCursor::Default
                {
                    return Ok(());
                }
                match event {
                    PointerEvent::Begin(input) => state.last_sample = Some(input.sample),
                    PointerEvent::Samples { input, .. } => {
                        if state.last_sample.is_some_and(|previous| {
                            previous.position == input.sample.position
                                && previous.pressure == input.sample.pressure
                                && previous.tilt == input.sample.tilt
                        }) {
                            return Ok(());
                        }
                        state.last_sample = Some(input.sample);
                    }
                    PointerEvent::End(_) | PointerEvent::Cancel => state.last_sample = None,
                    _ => {}
                }
                state.controller.pointer(
                    &mut self.ivars().session.project.borrow_mut(),
                    &state.expressions,
                    event,
                )
            };
            self.apply_preview_response(response)
        });
        if let Err(error) = result {
            self.show_error(&error);
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
        let guide_active = matches!(&*self.ivars().content.borrow(), Content::Preview(preview) if preview.guide_input.active());
        if key == Key::Escape && guide_active {
            self.cancel_preview_pointer();
            return true;
        }
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
        let response = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Preview(preview) = &mut *content else {
                return false;
            };
            preview.controller.keyboard(
                &mut self.ivars().session.project.borrow_mut(),
                &preview.expressions,
                event,
            )
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
        PointerInput {
            sample: PointerSample {
                position: self.point(event),
                pressure: (event.pressure() > 0.0).then_some(event.pressure()),
                tilt: None,
                time_millis: std::time::Duration::from_secs_f64(event.timestamp().max(0.0))
                    .as_millis() as u32,
            },
            tool: PointerTool::Mouse,
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
        self.set_preview_cursor(response.cursor);
        let session = &self.ivars().session;
        if response.edit.commits() {
            shrimply_project::project::commit_edit_checked(
                &session.project.borrow(),
                "preview-provider",
            )?;
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
                .controller
                .project_committed(player_state::snapshot(&session.player_state).revision);
        }
        Ok(())
    }

    pub(in crate::macos::canvas) fn refresh_live_preview(&self) {
        let requested = match &mut *self.ivars().content.borrow_mut() {
            Content::Preview(state) => state.controller.take_live_base_request(),
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
                    .controller
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
            Cursor::Crosshair | Cursor::Move => NSCursor::crosshairCursor().set(),
            Cursor::Grab => NSCursor::openHandCursor().set(),
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
        (
            NSEventModifierFlags::Command,
            Modifiers::CONTROL | Modifiers::META,
        ),
        (NSEventModifierFlags::Option, Modifiers::ALT),
    ] {
        if flags.contains(native) {
            modifiers |= shared;
        }
    }
    modifiers
}
