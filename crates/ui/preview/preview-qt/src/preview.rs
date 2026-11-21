use super::*;
use shrimply_editor_state::preview_focus::{self, SharedPreviewFocus};
use shrimply_paint_edit_skia::{PAINT_PREVIEW_STATE, PaintPreviewMode, PaintPreviewState};
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, Key, KeyboardEvent, Modifiers, PointerButton, PointerEvent, PointerInput,
    PointerSample, PointerTool, PreviewRefresh, PreviewResponse, PreviewViewport,
};
use shrimply_preview_runtime_cuda::controller::{self, Controller, PointerSequence, Preparation};
use shrimply_project_evaluation::{FrameAudioAnalysis, TransformExpressionCache};
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
    navigation: shrimply_preview_runtime_cuda::navigation::Navigation,
    guide_input: guides::GuideInput,
    frame: Option<CompositedVideoFrame>,
    frame_rate_label: String,
    fullscreen: bool,
    expression_cache: RefCell<TransformExpressionCache>,
    audio_analysis: FrameAudioAnalysis,
    controller: Controller,
    provider_cursor: Cursor,
    guide_active: bool,
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
        let controller = Controller::default();
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
            navigation: Default::default(),
            guide_input: guides::GuideInput::default(),
            frame: None,
            frame_rate_label: String::from("--"),
            fullscreen: false,
            expression_cache: RefCell::new(TransformExpressionCache::default()),
            audio_analysis: FrameAudioAnalysis::default(),
            controller,
            provider_cursor: Cursor::Default,
            guide_active: false,
            provider_invalidated,
        })
    }

    pub fn render(
        &mut self,
        width: u32,
        height: u32,
        pixels_per_point: f32,
        background_color: Color,
        fullscreen: bool,
    ) -> Result<(), String> {
        self.synchronize_history();
        self.fullscreen = fullscreen;
        if !preferences_store::snapshot(&self.preferences).preview_zoom_pan_enabled {
            if self.navigation.active() {
                self.provider_cursor = Cursor::Default;
            }
            self.navigation.reset();
        }
        if self.provider_invalidated.replace(false) {
            if self.controller.sequence != PointerSequence::Idle {
                self.pointer_cancel();
            }
            self.controller.context_invalidated = true;
        }
        let background_color =
            shrimply_preview_runtime_cuda::background_color(background_color, fullscreen);
        self.schedule_live_base();
        let update = self.media.poll();
        // Media callbacks can adopt durable metadata/parameter edits before a
        // base-frame receipt enters the shared controller.
        self.synchronize_history();
        assert!(update.running, "video compositor stopped unexpectedly");
        if let Some(label) = update.render_elapsed.and_then(rendered_frame_rate_label) {
            self.frame_rate_label = label;
        }
        match update.visual {
            Some(VideoEvent::Frame {
                frame,
                audio_analysis,
                revision,
                excluded_item_id,
                ..
            }) if self
                .controller
                .accept_base_frame(revision, excluded_item_id) =>
            {
                self.frame = Some(frame);
                self.audio_analysis = audio_analysis;
            }
            Some(VideoEvent::Clear {
                audio_analysis,
                revision,
                excluded_item_id,
                ..
            }) if self
                .controller
                .accept_base_frame(revision, excluded_item_id) =>
            {
                self.frame = None;
                self.audio_analysis = audio_analysis;
            }
            Some(VideoEvent::Frame { .. } | VideoEvent::Clear { .. }) => {}
            Some(_) => unreachable!(),
            None => {}
        }
        self.schedule_live_base();
        let snapshot = player_state::snapshot(&self.player_state);
        if self.renderer.is_none() {
            self.renderer = Some(renderer::ToolkitPreviewRenderer::new()?);
        }
        let project_handle = self.project.clone();
        let project = project_handle.borrow();
        let preferences = preferences_store::snapshot(&self.preferences);
        let surface = glam::IVec2::new(width.max(1) as i32, height.max(1) as i32);
        let scale = pixels_per_point.max(1.0);
        let viewport_surface =
            glam::vec2(width.max(1) as f32 / scale, height.max(1) as f32 / scale);
        let viewport = renderer::toolkit_guide_viewport(
            &project,
            &preferences,
            width.max(1) as f32 / scale,
            height.max(1) as f32 / scale,
            fullscreen,
        );
        let bounds = guides::bounds(
            viewport_surface,
            preferences.preview_padding_px,
            preferences.preview_guides_visible,
            self.fullscreen,
        );
        let viewport = self.navigation.viewport(viewport, bounds);
        self.update_overlay(
            &project,
            &preferences,
            snapshot.position,
            snapshot.revision,
            viewport,
        )?;
        self.update_base_exclusion(snapshot.position);
        let clip_rect = self.navigation.clip_rect(bounds, viewport_surface);
        let Self {
            renderer,
            controller,
            expression_cache,
            frame,
            ..
        } = self;
        renderer
            .as_mut()
            .expect("toolkit preview renderer was initialized")
            .render(
                &project,
                snapshot.position,
                frame.as_ref(),
                surface,
                pixels_per_point,
                background_color,
                &preferences,
                viewport,
                clip_rect,
                |canvas| {
                    if let Some(retiring) = controller.retiring_provider.as_mut() {
                        retiring.context.viewport = viewport;
                    }
                    controller.draw(canvas, expression_cache);
                },
            )
    }

    fn update_base_exclusion(&mut self, position: Time) {
        let exclusion = self
            .controller
            .provider
            .as_ref()
            .and_then(|overlay| overlay.provider.base_frame_exclusion());
        if exclusion == self.controller.base_exclusion {
            return;
        }
        self.controller.base_exclusion = exclusion;
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
        if !self.controller.take_live_base_request() {
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
        self.controller
            .live_base_requested(player_state::snapshot(&self.player_state).revision);
    }

    fn synchronize_history(&mut self) {
        if self.controller.synchronize_history() {
            // These drafts belong to the old history owner. Never roll them back
            // into the replacement model through GuideInput/provider Cancel.
            self.guide_input = guides::GuideInput::default();
            self.guide_active = false;
            self.navigation.cancel();
            self.provider_cursor = Cursor::Default;
        }
    }

    fn prepare_input(&mut self) {
        self.synchronize_history();
        if self.provider_invalidated.get() {
            self.pointer_cancel();
            self.provider_invalidated.set(false);
            self.controller.context_invalidated = true;
        }
    }

    fn update_overlay(
        &mut self,
        project: &Project,
        preferences: &preferences_store::PreferencesSnapshot,
        position: Time,
        revision: u64,
        viewport: PreviewViewport,
    ) -> Result<(), String> {
        let selected = selection_state::focused_video_address(&self.selection_state, project);
        let focused = preview_focus::snapshot(&self.preview_focus);
        let selection = controller::selection(
            project,
            selected.as_ref(),
            focused
                .as_ref()
                .map(|focused| (&focused.item, focused.target)),
        );
        self.controller.ensure(
            project,
            selection,
            position,
            Preparation {
                project_revision: revision,
                viewport,
                audio_analysis: &self.audio_analysis,
                expression_cache: &self.expression_cache,
                snap_enabled: preferences.timeline_magnet == "true",
                snap_radius_px: preferences.timeline_snap_radius_px as f32,
                guides: preferences
                    .preview_guides_visible
                    .then_some(project.preview_guides.as_ref()),
                camera_sampler: shrimply_preview_runtime_cuda::provider::sample_camera,
            },
        )?;
        Ok(())
    }

    fn dispatch_provider(&mut self, event: PointerEvent<'_>) -> PreviewResponse {
        if self.provider_invalidated.get() {
            return PreviewResponse::IGNORED.redraw();
        }
        let response = self.controller.pointer(
            &mut self.project.borrow_mut(),
            &self.expression_cache,
            event,
        );
        self.apply_provider_response(response);
        response
    }

    fn apply_provider_response(&mut self, response: PreviewResponse) {
        match response.cursor {
            CursorUpdate::Keep => {}
            CursorUpdate::Set(cursor) => self.provider_cursor = cursor,
            CursorUpdate::Clear => self.provider_cursor = Cursor::Default,
        }
        controller::commit_response(&self.project.borrow(), response, "preview-provider");
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
            self.controller
                .project_committed(player_state::snapshot(&self.player_state).revision);
        }
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
        let state = self
            .controller
            .extensions
            .get(&PAINT_PREVIEW_STATE)
            .and_then(|value| value.downcast_ref::<PaintPreviewState>())
            .expect("Qt paint preview state is missing");
        read(state)
    }

    fn update_paint_state(&mut self, update: impl FnOnce(&mut PaintPreviewState)) {
        self.synchronize_history();
        assert!(
            self.controller
                .update_extension(PAINT_PREVIEW_STATE, update),
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
        self.synchronize_history();
        let preferences = preferences_store::snapshot(&self.preferences);
        let fit = renderer::toolkit_guide_viewport(
            &self.project.borrow(),
            &preferences,
            size.x,
            size.y,
            self.fullscreen,
        );
        let bounds = guides::bounds(
            size,
            preferences.preview_padding_px,
            preferences.preview_guides_visible,
            self.fullscreen,
        );
        let response = self.navigation.pointer(
            fit,
            bounds,
            event,
            preferences.preview_zoom_pan_enabled,
            self.controller.sequence != PointerSequence::Idle || self.guide_active,
        );
        if response.is_some_and(|response| response.redraw) {
            self.provider_cursor = if self.navigation.active() {
                Cursor::Grabbing
            } else {
                Cursor::Default
            };
        }
        response.is_some_and(|response| response.handled)
    }

    pub fn pointer_move(&mut self, width: f32, height: f32, x: f32, y: f32, modifiers: Modifiers) {
        self.synchronize_history();
        if self.navigation.active() {
            self.navigate(
                glam::vec2(width, height),
                PointerEvent::Hover(pointer_input(pointer_sample(x, y), modifiers)),
            );
            return;
        }
        if self.controller.sequence == PointerSequence::Active {
            let sample = pointer_sample(x, y);
            self.dispatch_provider(PointerEvent::Samples {
                input: pointer_input(sample, modifiers),
                samples: std::slice::from_ref(&sample),
            });
            return;
        }
        let preferences = preferences_store::snapshot(&self.preferences);
        let mut project = self.project.borrow_mut();
        let viewport_surface = glam::vec2(width, height);
        let viewport = renderer::toolkit_guide_viewport(
            &project,
            &preferences,
            width,
            height,
            self.fullscreen,
        );
        let bounds = guides::bounds(
            viewport_surface,
            preferences.preview_padding_px,
            preferences.preview_guides_visible,
            self.fullscreen,
        );
        let viewport = self.navigation.viewport(viewport, bounds);
        self.guide_input.pointer_move(
            &mut project.preview_guides,
            viewport,
            preferences.preview_guides_visible,
            glam::vec2(x, y),
        );
        drop(project);
        if self.guide_input.cursor() == guides::GuideCursor::Default {
            self.dispatch_provider(PointerEvent::Hover(pointer_input(
                pointer_sample(x, y),
                modifiers,
            )));
        } else {
            self.provider_cursor = Cursor::Default;
        }
    }

    pub fn pointer_press(
        &mut self,
        width: f32,
        height: f32,
        x: f32,
        y: f32,
        modifiers: Modifiers,
    ) -> bool {
        self.prepare_input();
        if self.navigation.active() {
            return false;
        }
        let preferences = preferences_store::snapshot(&self.preferences);
        let mut project = self.project.borrow_mut();
        let viewport_surface = glam::vec2(width, height);
        let viewport = renderer::toolkit_guide_viewport(
            &project,
            &preferences,
            width,
            height,
            self.fullscreen,
        );
        let bounds = guides::bounds(
            viewport_surface,
            preferences.preview_padding_px,
            preferences.preview_guides_visible,
            self.fullscreen,
        );
        let viewport = self.navigation.viewport(viewport, bounds);
        if self.guide_input.pointer_press(
            &mut project.preview_guides,
            viewport,
            preferences.preview_guides_visible,
            glam::vec2(x, y),
        ) {
            self.guide_active = true;
            self.controller.sequence = PointerSequence::Guide;
            return true;
        }
        drop(project);
        if !self
            .navigation
            .clip_rect(bounds, viewport_surface)
            .contains(glam::vec2(x, y))
        {
            return false;
        }
        let player = player_state::snapshot(&self.player_state);
        let project_handle = self.project.clone();
        if let Err(error) = self.update_overlay(
            &project_handle.borrow(),
            &preferences,
            player.position,
            player.revision,
            viewport,
        ) {
            tracing::error!(%error, "could not prepare Qt preview interaction");
            return false;
        }
        self.update_base_exclusion(player.position);
        let sample = pointer_sample(x, y);
        let response =
            self.dispatch_provider(PointerEvent::Begin(pointer_input(sample, modifiers)));
        if !response.handled {
            let canceled = self
                .controller
                .teardown(&mut self.project.borrow_mut(), &self.expression_cache);
            self.apply_provider_response(canceled);
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
        self.synchronize_history();
        let preferences = preferences_store::snapshot(&self.preferences);
        let mut project = self.project.borrow_mut();
        let viewport_surface = glam::vec2(width, height);
        let viewport = renderer::toolkit_guide_viewport(
            &project,
            &preferences,
            width,
            height,
            self.fullscreen,
        );
        let bounds = guides::bounds(
            viewport_surface,
            preferences.preview_padding_px,
            preferences.preview_guides_visible,
            self.fullscreen,
        );
        let viewport = self.navigation.viewport(viewport, bounds);
        let changed = self.guide_active.then(|| {
            self.guide_input.pointer_release(
                &mut project.preview_guides,
                viewport,
                glam::vec2(x, y),
            )
        });
        drop(project);
        if changed.flatten() == Some(true) {
            guides::commit_edit(&self.project.borrow());
            self.controller
                .project_committed(player_state::snapshot(&self.player_state).revision);
        }
        if self.guide_active {
            self.guide_active = false;
            self.controller.sequence = PointerSequence::Idle;
            self.controller.context_invalidated = true;
            return;
        }
        if self.controller.sequence == PointerSequence::Active {
            let sample = pointer_sample(x, y);
            self.dispatch_provider(PointerEvent::End(pointer_input(sample, modifiers)));
            self.provider_cursor = Cursor::Default;
        }
    }

    pub fn keyboard(&mut self, size: glam::Vec2, event: KeyboardEvent) -> bool {
        if !size.is_finite() || size.min_element() <= 0.0 {
            return false;
        }
        self.prepare_input();
        if let Some(response) = self.navigation.keyboard(event) {
            self.apply_provider_response(response);
            return response.handled;
        }
        let preferences = preferences_store::snapshot(&self.preferences);
        let project_handle = self.project.clone();
        let project = project_handle.borrow();
        let fit = renderer::toolkit_guide_viewport(
            &project,
            &preferences,
            size.x,
            size.y,
            self.fullscreen,
        );
        let bounds = guides::bounds(
            size,
            preferences.preview_padding_px,
            preferences.preview_guides_visible,
            self.fullscreen,
        );
        let viewport = self.navigation.viewport(fit, bounds);
        let player = player_state::snapshot(&self.player_state);
        let prepared = self.update_overlay(
            &project,
            &preferences,
            player.position,
            player.revision,
            viewport,
        );
        drop(project);
        if let Err(error) = prepared {
            tracing::error!(%error, "could not prepare Qt preview keyboard interaction");
            return false;
        }
        self.update_base_exclusion(player.position);
        let response = self.controller.keyboard(
            &mut self.project.borrow_mut(),
            &self.expression_cache,
            event,
        );
        self.apply_provider_response(response);
        response.handled
    }

    pub fn pointer_cancel(&mut self) {
        self.synchronize_history();
        self.navigation.cancel();
        if self.guide_active {
            self.guide_input
                .pointer_cancel(&mut self.project.borrow_mut().preview_guides);
            self.guide_active = false;
        }
        let response = self
            .controller
            .teardown(&mut self.project.borrow_mut(), &self.expression_cache);
        self.apply_provider_response(response);
        self.provider_cursor = Cursor::Default;
    }

    pub fn pointer_leave(&mut self) {
        self.synchronize_history();
        self.guide_input.pointer_leave();
        if self.controller.sequence != PointerSequence::Active {
            self.dispatch_provider(PointerEvent::Leave);
            self.provider_cursor = Cursor::Default;
        }
    }

    pub fn pointer_cursor(&self) -> u8 {
        if self.navigation.active() {
            return provider_cursor_code(Cursor::Grabbing);
        }
        match self.guide_input.cursor() {
            guides::GuideCursor::ResizeHorizontal => 1,
            guides::GuideCursor::ResizeVertical => 2,
            guides::GuideCursor::Default => provider_cursor_code(self.provider_cursor),
        }
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
        Cursor::ResizeHorizontal => 9,
        Cursor::ResizeVertical => 10,
        Cursor::ResizeDiagonalDown => 11,
        Cursor::ResizeDiagonalUp => 12,
        Cursor::Hidden => 13,
    }
}

impl Drop for ToolkitPreview {
    fn drop(&mut self) {
        self.media.stop();
        self.audio_player.stop();
    }
}
