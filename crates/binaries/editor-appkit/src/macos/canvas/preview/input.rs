use super::*;
use shrimply_editor_state::{player_state, preferences, preview_focus};
use shrimply_preview_interaction_skia::controller::Preparation;
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, Key, KeyState, KeyboardEvent, Modifiers, PointerButton, PointerEvent,
    PointerInput, PointerSample, PointerTool, PreviewRefresh, PreviewResponse,
};
use shrimply_timeline_skia::selection_state;

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
            state.sync_guides(&project.preview_guides, prefs.preview_guides_visible);
            let size = self.bounds().size;
            let viewport = guides::viewport(
                glam::IVec2::new(size.width as i32, size.height as i32),
                project.canvas_size,
                prefs.preview_padding_px,
                state.guides_visible,
                state.fullscreen,
            );
            if !prefs.preview_zoom_pan_enabled {
                if state.navigation.active() {
                    objc2_app_kit::NSCursor::arrowCursor().set();
                }
                state.navigation.reset();
            }
            let bounds = guides::bounds(
                glam::vec2(size.width as f32, size.height as f32),
                prefs.preview_padding_px,
                state.guides_visible,
                state.fullscreen,
            );
            let viewport = state.navigation.viewport(viewport, bounds);
            state.viewport = Some(viewport);
            // While time is moving, controls belong to the displayed frame, not the
            // advancing playback clock. Waiting for an exact clock match tears
            // down the provider (and its caches/base exclusion) between frames.
            if let Some((_, frame_time, audio_analysis)) =
                state.audio_analysis.as_ref().filter(|(revision, time, _)| {
                    *revision == player.revision
                        && (player.playing || player.scrubbing || *time == player.position)
                })
            {
                let had_provider = state.controller.provider.is_some();
                let invalidated = state.controller.context_invalidated
                    && state.controller.sequence == PointerSequence::Idle;
                state.controller.ensure(
                    &project,
                    selection,
                    *frame_time,
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
                        camera_sampler: |id, source, time| {
                            shrimply_visual_core::camera_reconstruction::sample(id, source, time)
                                .map(|camera| {
                                    shrimply_project_document::project::TrackedCameraPreview {
                                        position: camera.position,
                                        rotation: camera.rotation,
                                        projection: camera.projection,
                                        vertical_fov_degrees: camera.vertical_fov_degrees,
                                    }
                                })
                        },
                    },
                )?;
                if had_provider != state.controller.provider.is_some() || invalidated {
                    self.ivars().surface_dirty.set(true);
                }
            } else if state.controller.sequence == PointerSequence::Idle
                && state.controller.provider.take().is_some()
            {
                self.ivars().surface_dirty.set(true);
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

    fn preview_navigation(&self, event: PointerEvent<'_>) -> Option<PreviewResponse> {
        let prefs = preferences::snapshot(&self.ivars().session.preferences);
        let mut content = self.ivars().content.borrow_mut();
        let Content::Preview(state) = &mut *content else {
            return None;
        };
        let size = self.bounds().size;
        let fit = guides::viewport(
            glam::IVec2::new(size.width as i32, size.height as i32),
            self.ivars().session.project.borrow().canvas_size,
            prefs.preview_padding_px,
            state.guides_visible,
            state.fullscreen,
        );
        let bounds = guides::bounds(
            glam::vec2(size.width as f32, size.height as f32),
            prefs.preview_padding_px,
            state.guides_visible,
            state.fullscreen,
        );
        let response = state.navigation.pointer(
            fit,
            bounds,
            event,
            prefs.preview_zoom_pan_enabled,
            state.controller.sequence != PointerSequence::Idle || state.guide_input.active(),
        );
        if response.is_some_and(|response| response.redraw) {
            state.viewport = Some(state.navigation.viewport(fit, bounds));
            state.controller.context_invalidated = true;
            state.caption_split_hover = None;
        }
        response
    }

    pub(in crate::macos::canvas) fn preview_pointer_event(&self, event: PointerEvent<'_>) -> bool {
        if let Some(response) = self.preview_navigation(event) {
            if let Err(error) = self.apply_preview_response(response) {
                self.show_error(&error);
            }
            return true;
        }
        let result = self.prepare_preview().and_then(|()| {
            if self.preview_caption_pointer(&event)? {
                return Ok(true);
            }
            let response = {
                let mut content = self.ivars().content.borrow_mut();
                let Content::Preview(state) = &mut *content else {
                    return Ok(false);
                };
                if state.guide_input.active() || state.controller.sequence == PointerSequence::Guide
                {
                    return Ok(true);
                }
                if matches!(event, PointerEvent::Hover(_))
                    && state.guide_input.cursor() != GuideCursor::Default
                {
                    return Ok(true);
                }
                if let PointerEvent::Begin(input) = event {
                    let prefs = preferences::snapshot(&self.ivars().session.preferences);
                    let size = self.bounds().size;
                    if !state
                        .navigation
                        .clip_rect(
                            guides::bounds(
                                glam::vec2(size.width as f32, size.height as f32),
                                prefs.preview_padding_px,
                                state.guides_visible,
                                state.fullscreen,
                            ),
                            glam::vec2(size.width as f32, size.height as f32),
                        )
                        .contains(input.sample.position)
                    {
                        return Ok(false);
                    }
                }
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
                state.controller.pointer(
                    &mut self.ivars().session.project.borrow_mut(),
                    &state.expressions,
                    event,
                )
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
        let guide_active = matches!(&*self.ivars().content.borrow(), Content::Preview(preview) if preview.guide_input.active() || preview.navigation.active());
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
        if response.edit.commits() {
            shrimply_project_document::project::commit_edit_checked(
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
            Cursor::Crosshair => NSCursor::crosshairCursor().set(),
            Cursor::Move => NSCursor::openHandCursor().set(),
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
        (NSEventModifierFlags::Command, Modifiers::META),
        (NSEventModifierFlags::Option, Modifiers::ALT),
    ] {
        if flags.contains(native) {
            modifiers |= shared;
        }
    }
    modifiers
}
