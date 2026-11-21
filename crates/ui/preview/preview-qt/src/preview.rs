use super::*;
use shrimply_editor_state::preview_focus::{self, SharedPreviewFocus};
use shrimply_paint_edit_skia::{PAINT_PREVIEW_STATE, PaintPreviewMode, PaintPreviewState};
use shrimply_preview_provider_skia::{
    Cursor, Key, KeyboardEvent, Modifiers, PointerButton, PointerEvent, PointerInput,
    PointerSample, PointerTool, PreviewRefresh, PreviewResponse,
};
use shrimply_preview_runtime_cuda::surface::{Clock, Configuration, Surface};
use shrimply_timeline_edit::selection_state::{self, SharedSelectionState};
use std::cell::Cell;

pub struct ToolkitPreview {
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    preview_focus: SharedPreviewFocus,
    preferences: preferences_store::SharedPreferences,
    media: PreviewMedia,
    video_tx: VideoCommandSender,
    audio_player: Rc<AudioPlayer>,
    renderer: Option<renderer::ToolkitPreviewRenderer>,
    surface: Surface,
    frame: Option<CompositedVideoFrame>,
    frame_rate_label: String,
    fullscreen: bool,
    provider_invalidated: Rc<Cell<bool>>,
}

impl ToolkitPreview {
    pub fn new(
        project: Rc<RefCell<Project>>,
        player_state: SharedPlayerState,
        selection_state: SharedSelectionState,
        preview_focus: SharedPreviewFocus,
        playback_performance: playback_performance::SharedCollector,
        preferences: preferences_store::SharedPreferences,
        audio_player: Rc<AudioPlayer>,
    ) -> Result<Self, String> {
        let media = PreviewMedia::new(
            project.clone(),
            player_state.clone(),
            playback_performance,
            preferences.clone(),
            shrimply_visual_cuda::compositor::spawn_worker_with_resources_and_observer,
        );
        let video_tx = media.sender();
        let provider_invalidated = Rc::new(Cell::new(false));
        let selection_invalidated = provider_invalidated.clone();
        selection_state::connect_named(
            &selection_state,
            "Qt preview provider selection",
            move || selection_invalidated.set(true),
        );
        let focus_invalidated = provider_invalidated.clone();
        preview_focus::connect_named(&preview_focus, "Qt preview provider focus", move || {
            focus_invalidated.set(true);
        });
        Ok(Self {
            project,
            player_state,
            selection_state,
            preview_focus,
            preferences,
            media,
            video_tx,
            audio_player,
            renderer: None,
            surface: Surface::default(),
            frame: None,
            frame_rate_label: String::from("--"),
            fullscreen: false,
            provider_invalidated,
        })
    }

    fn configuration(
        &self,
        preferences: &preferences_store::PreferencesSnapshot,
        size: glam::Vec2,
    ) -> Configuration {
        Configuration {
            size,
            padding: preferences.preview_padding_px,
            guides_visible: preferences.preview_guides_visible,
            fullscreen: self.fullscreen,
            snap_enabled: preferences.timeline_magnet == "true",
            snap_radius: preferences.timeline_snap_radius_px as f32,
            navigation_enabled: preferences.preview_zoom_pan_enabled,
        }
    }

    fn clock(&self) -> Clock {
        let snapshot = player_state::snapshot(&self.player_state);
        Clock {
            revision: snapshot.revision,
            position: snapshot.position,
            moving: snapshot.playing || snapshot.scrubbing,
        }
    }

    fn sync_focus(&mut self) {
        self.surface.focus(
            preview_focus::snapshot(&self.preview_focus)
                .map(|focused| (focused.item, focused.target)),
        );
    }

    fn prepare_invalidation(&mut self) {
        if self.provider_invalidated.replace(false) {
            if self.surface.active() {
                self.pointer_cancel();
            }
            self.surface.invalidate();
        }
    }

    pub fn render(
        &mut self,
        width: u32,
        height: u32,
        pixels_per_point: f32,
        background_color: Color,
        fullscreen: bool,
    ) -> Result<(), String> {
        self.fullscreen = fullscreen;
        self.prepare_invalidation();
        let background_color =
            shrimply_preview_runtime_cuda::background_color(background_color, fullscreen);
        self.schedule_live_base();
        let update = self.media.poll();
        assert!(update.running, "video compositor stopped unexpectedly");
        if let Some(label) = update.render_elapsed.and_then(rendered_frame_rate_label) {
            self.frame_rate_label = label;
        }
        match update.visual {
            Some(VideoEvent::Frame {
                frame,
                audio_analysis,
                position,
                revision,
                excluded_item_id,
                ..
            }) if self
                .surface
                .presented(revision, position, excluded_item_id, &audio_analysis) =>
            {
                self.frame = Some(frame);
            }
            Some(VideoEvent::Clear {
                audio_analysis,
                position,
                revision,
                excluded_item_id,
                ..
            }) if self
                .surface
                .presented(revision, position, excluded_item_id, &audio_analysis) =>
            {
                self.frame = None;
            }
            Some(VideoEvent::Frame { .. } | VideoEvent::Clear { .. }) => {}
            Some(_) => unreachable!(),
            None => {}
        }
        self.schedule_live_base();
        if self.renderer.is_none() {
            self.renderer = Some(renderer::ToolkitPreviewRenderer::new()?);
        }
        self.sync_focus();
        let clock = self.clock();
        let project_handle = self.project.clone();
        let project = project_handle.borrow();
        let selected = selection_state::focused_video_address(&self.selection_state, &project);
        let preferences = preferences_store::snapshot(&self.preferences);
        let surface_px = glam::IVec2::new(width.max(1) as i32, height.max(1) as i32);
        let scale = pixels_per_point.max(1.0);
        let logical_surface = glam::vec2(width.max(1) as f32 / scale, height.max(1) as f32 / scale);
        let configuration = self.configuration(&preferences, logical_surface);
        self.surface
            .ensure_readonly(&project, selected.as_ref(), clock, configuration)?;
        self.update_base_exclusion(clock.position);
        let layout = self.surface.layout(&project, configuration);
        let appearance = Appearance {
            content_rect: layout.viewport.content_rect,
            clip_rect: layout.clip_rect,
            shadow_size_px: preferences.preview_shadow_size_px,
            background_color,
            upsample_method: preferences.preview_upsample_method,
            downsample_method: preferences.preview_downsample_method,
        };
        let captions = CaptionAppearance {
            preview_rect: layout.surface_rect,
            font_size: preferences.caption_font_size,
            background_color: preferences.caption_background_color,
            bottom_inset: 0.0,
        };
        let mut draw_result = Ok(());
        let Self {
            renderer,
            surface,
            frame,
            ..
        } = self;
        renderer
            .as_mut()
            .expect("toolkit preview renderer was initialized")
            .render(
                frame.as_ref(),
                surface_px,
                pixels_per_point,
                appearance,
                |canvas| {
                    draw_result = surface.draw_overlay(
                        canvas,
                        &project,
                        selected.as_ref(),
                        clock,
                        configuration,
                        captions,
                        None,
                        |_| {},
                    );
                },
            )?;
        draw_result
    }

    fn update_base_exclusion(&mut self, position: Time) {
        let Some(exclusion) = self.surface.exclusion_update() else {
            return;
        };
        for command in [
            VideoCommand::SetPreviewExclusion(exclusion),
            VideoCommand::Render {
                position,
                accuracy: CompositeAccuracy::FULLY_ACCURATE,
            },
        ] {
            if let Err(error) = self.video_tx.send(command) {
                tracing::error!(%error, "could not update the Qt preview base frame");
            }
        }
    }

    fn schedule_live_base(&mut self) {
        if !self.surface.take_live_base_request() {
            return;
        }
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                video: true,
                live_preview: true,
                ..Default::default()
            },
        );
        self.surface
            .live_base_requested(player_state::snapshot(&self.player_state).revision);
    }

    fn apply_provider_response(&mut self, response: PreviewResponse) {
        let _ = self.surface.take_committed();
        if response.edit.refresh != PreviewRefresh::NONE {
            player_state::refresh_project(
                &self.player_state,
                player_state::ProjectChange {
                    video: response.edit.refresh.contains(PreviewRefresh::PREVIEW),
                    live_preview: response.edit.is_live(),
                    inspector: response.edit.refresh.contains(PreviewRefresh::INSPECTOR),
                    ..Default::default()
                },
            );
        }
        if response.edit.commits() {
            self.surface
                .project_committed(player_state::snapshot(&self.player_state).revision);
        }
    }

    fn dispatch_pointer(&mut self, size: glam::Vec2, event: PointerEvent<'_>) -> PreviewResponse {
        if !size.is_finite() || size.min_element() <= 0.0 {
            return PreviewResponse::IGNORED;
        }
        self.prepare_invalidation();
        self.sync_focus();
        let clock = self.clock();
        let preferences = preferences_store::snapshot(&self.preferences);
        let configuration = self.configuration(&preferences, size);
        let project_handle = self.project.clone();
        let mut project = project_handle.borrow_mut();
        let selected = selection_state::focused_video_address(&self.selection_state, &project);
        let response =
            match self
                .surface
                .pointer(&mut project, selected.as_ref(), clock, configuration, event)
            {
                Ok(response) => response,
                Err(error) => {
                    tracing::error!(%error, "could not handle Qt preview pointer event");
                    return PreviewResponse::IGNORED;
                }
            };
        drop(project);
        self.update_base_exclusion(clock.position);
        self.apply_provider_response(response);
        response
    }

    pub fn destroy(&mut self) {
        self.pointer_cancel();
        self.media.stop();
        self.audio_player.stop();
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.destroy();
        }
        self.renderer = None;
    }

    pub fn mark_step(&self, delta: i32) {
        self.media.mark_step(if delta < 0 {
            StepDirection::Backward
        } else {
            StepDirection::Forward
        });
    }

    pub fn frame_rate_label(&self) -> &str {
        &self.frame_rate_label
    }

    pub fn guides_visible(&self) -> bool {
        preferences_store::snapshot(&self.preferences).preview_guides_visible
    }

    pub fn set_guides_visible(&self, visible: bool) {
        preferences_store::set_preview_guides_visible(&self.preferences, visible);
    }

    fn paint_state<R>(&self, read: impl FnOnce(&PaintPreviewState) -> R) -> R {
        read(
            self.surface
                .extension::<PaintPreviewState>(PAINT_PREVIEW_STATE)
                .expect("Qt paint preview state is missing"),
        )
    }

    fn update_paint_state(&mut self, update: impl FnOnce(&mut PaintPreviewState)) {
        assert!(
            self.surface.update_extension(PAINT_PREVIEW_STATE, update),
            "Qt paint preview state is missing"
        );
    }

    pub fn pen_tool_active(&self) -> bool {
        self.paint_state(|state| state.mode == PaintPreviewMode::Pen && !state.eraser)
    }

    pub fn fill_tool_active(&self) -> bool {
        self.paint_state(|state| state.mode == PaintPreviewMode::Fill)
    }

    pub fn transform_tool_active(&self) -> bool {
        self.paint_state(|state| state.mode == PaintPreviewMode::StrokeTransform)
    }

    pub fn eraser_tool_active(&self) -> bool {
        self.paint_state(|state| state.eraser)
    }

    pub fn select_pen_tool(&mut self) {
        self.update_paint_state(|state| {
            state.set_eraser(false);
            state.set_mode(PaintPreviewMode::Pen);
        });
    }

    pub fn select_fill_tool(&mut self) {
        self.update_paint_state(|state| {
            state.set_eraser(false);
            state.set_mode(PaintPreviewMode::Fill);
        });
    }

    pub fn select_transform_tool(&mut self) {
        self.update_paint_state(|state| state.set_mode(PaintPreviewMode::StrokeTransform));
    }

    pub fn set_eraser_tool_active(&mut self, active: bool) {
        self.update_paint_state(|state| state.set_eraser(active));
    }

    pub fn navigate(&mut self, size: glam::Vec2, event: PointerEvent<'_>) -> bool {
        self.dispatch_pointer(size, event).handled
    }

    pub fn pointer_move(&mut self, width: f32, height: f32, x: f32, y: f32, modifiers: Modifiers) {
        let sample = pointer_sample(x, y);
        let event = if self.surface.active() && !self.surface.navigation_active() {
            PointerEvent::Samples {
                input: pointer_input(sample, modifiers),
                samples: std::slice::from_ref(&sample),
            }
        } else {
            PointerEvent::Hover(pointer_input(sample, modifiers))
        };
        self.dispatch_pointer(glam::vec2(width, height), event);
    }

    pub fn pointer_press(
        &mut self,
        width: f32,
        height: f32,
        x: f32,
        y: f32,
        modifiers: Modifiers,
    ) -> bool {
        let sample = pointer_sample(x, y);
        let response = self.dispatch_pointer(
            glam::vec2(width, height),
            PointerEvent::Begin(pointer_input(sample, modifiers)),
        );
        if !response.handled {
            self.pointer_cancel();
        }
        response.handled
    }

    pub fn pointer_release(
        &mut self,
        width: f32,
        height: f32,
        x: f32,
        y: f32,
        modifiers: Modifiers,
    ) {
        let sample = pointer_sample(x, y);
        self.dispatch_pointer(
            glam::vec2(width, height),
            PointerEvent::End(pointer_input(sample, modifiers)),
        );
    }

    pub fn keyboard(&mut self, size: glam::Vec2, event: KeyboardEvent) -> bool {
        if !size.is_finite() || size.min_element() <= 0.0 {
            return false;
        }
        self.prepare_invalidation();
        self.sync_focus();
        let clock = self.clock();
        let preferences = preferences_store::snapshot(&self.preferences);
        let configuration = self.configuration(&preferences, size);
        let project_handle = self.project.clone();
        let mut project = project_handle.borrow_mut();
        let selected = selection_state::focused_video_address(&self.selection_state, &project);
        let response = match self.surface.keyboard(
            &mut project,
            selected.as_ref(),
            clock,
            configuration,
            event,
        ) {
            Ok(response) => response,
            Err(error) => {
                tracing::error!(%error, "could not handle Qt preview keyboard event");
                return false;
            }
        };
        drop(project);
        self.update_base_exclusion(clock.position);
        self.apply_provider_response(response);
        response.handled
    }

    pub fn pointer_cancel(&mut self) {
        let clock = self.clock();
        let project_handle = self.project.clone();
        let response = self.surface.cancel(&mut project_handle.borrow_mut());
        self.update_base_exclusion(clock.position);
        self.apply_provider_response(response);
    }

    pub fn pointer_leave(&mut self) {
        let project_handle = self.project.clone();
        let response = self.surface.pointer_leave(&mut project_handle.borrow_mut());
        self.apply_provider_response(response);
    }

    pub fn pointer_cursor(&self) -> u8 {
        provider_cursor_code(self.surface.cursor())
    }
}

fn pointer_sample(x: f32, y: f32) -> PointerSample {
    PointerSample {
        position: glam::vec2(x, y),
        ..PointerSample::default()
    }
}

fn pointer_input(sample: PointerSample, modifiers: Modifiers) -> PointerInput {
    PointerInput {
        sample,
        tool: PointerTool::Mouse,
        button: PointerButton::Primary,
        modifiers,
    }
}

pub(crate) fn keyboard_key(key: u32, text: &str) -> Key {
    const QT_ESCAPE: u32 = 0x0100_0000;
    const QT_TAB: u32 = 0x0100_0001;
    const QT_BACKTAB: u32 = 0x0100_0002;
    const QT_BACKSPACE: u32 = 0x0100_0003;
    const QT_RETURN: u32 = 0x0100_0004;
    const QT_ENTER: u32 = 0x0100_0005;
    const QT_DELETE: u32 = 0x0100_0007;
    const QT_SHIFT: u32 = 0x0100_0020;
    const QT_CONTROL: u32 = 0x0100_0021;
    const QT_ALT: u32 = 0x0100_0023;
    match key {
        QT_ESCAPE => Key::Escape,
        QT_TAB | QT_BACKTAB => Key::Tab,
        QT_BACKSPACE => Key::Backspace,
        QT_RETURN | QT_ENTER => Key::Enter,
        QT_DELETE => Key::Delete,
        QT_SHIFT => Key::Shift,
        QT_CONTROL => Key::Control,
        QT_ALT => Key::Alt,
        _ => text
            .chars()
            .next()
            .filter(|value| !value.is_control())
            .or_else(|| char::from_u32(key).filter(|value| !value.is_control()))
            .map_or(Key::Unknown, |value| {
                if value == ' ' {
                    Key::Space
                } else {
                    Key::Character(value)
                }
            }),
    }
}

pub(crate) fn pointer_modifiers(control: bool, shift: bool, alt: bool) -> Modifiers {
    let mut modifiers = Modifiers::NONE;
    if control {
        modifiers |= Modifiers::CONTROL;
    }
    if shift {
        modifiers |= Modifiers::SHIFT;
    }
    if alt {
        modifiers |= Modifiers::ALT;
    }
    modifiers
}

fn provider_cursor_code(cursor: Cursor) -> u8 {
    match cursor {
        Cursor::Default => 0,
        Cursor::Pointer => 3,
        Cursor::Crosshair => 4,
        Cursor::Move => 5,
        Cursor::Grab => 6,
        Cursor::Grabbing => 7,
        Cursor::Text => 8,
        Cursor::ResizeHorizontal => 1,
        Cursor::ResizeVertical => 2,
        Cursor::ResizeDiagonalDown => 9,
        Cursor::ResizeDiagonalUp => 10,
        Cursor::Hidden => 11,
    }
}
