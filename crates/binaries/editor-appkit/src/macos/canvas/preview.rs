use super::*;
use shrimply_preview_interaction_skia::controller::{Controller, PointerSequence};
use shrimply_preview_interaction_skia::guides::{self, GuideCursor, GuideInput};
use shrimply_preview_provider_skia::PreviewViewport;
use shrimply_project_document::project::PreviewGuides;

pub(super) mod captions;
mod context_menu;
mod input;

pub struct State {
    pub renderer: shrimply_preview_render_metal::Renderer,
    pub viewport: Option<PreviewViewport>,
    pub guides_visible: bool,
    pub fullscreen: bool,
    pub caption_bottom_inset: f32,
    pub guide_input: GuideInput,
    pub edited_guides: Option<Box<PreviewGuides>>,
    pub baseline_guides: Option<Box<PreviewGuides>>,
    pub guide_button: Option<Retained<objc2_app_kit::NSButton>>,
    pub loading_done: Option<Retained<objc2_app_kit::NSButton>>,
    pub loading_spinner: Option<Retained<objc2_app_kit::NSProgressIndicator>>,
    pub frame_rate_label: Option<Retained<objc2_app_kit::NSTextField>>,
    pub loading_indicator: shrimply_preview_provider_skia::playback::LoadingIndicator,
    pub controller: Controller,
    pub expressions: RefCell<shrimply_project_evaluation::TransformExpressionCache>,
    pub audio_analysis: Option<(
        u64,
        shrimply_math_core::Time,
        shrimply_project_evaluation::FrameAudioAnalysis,
    )>,
    pub presented_frame: Option<u32>,
    pub cursor_hidden: bool,
    pub last_sample: Option<shrimply_preview_provider_skia::PointerSample>,
    pub modifiers: shrimply_preview_provider_skia::Modifiers,
    pub caption_split_hover: Option<glam::Vec2>,
}

impl State {
    pub fn new(
        guide_button: Retained<objc2_app_kit::NSButton>,
        loading_done: Retained<objc2_app_kit::NSButton>,
        loading_spinner: Retained<objc2_app_kit::NSProgressIndicator>,
        frame_rate_label: Retained<objc2_app_kit::NSTextField>,
        playback_performance: shrimply_playback_performance::SharedCollector,
    ) -> Self {
        use shrimply_paint_edit_skia::{
            DEFAULT_PAINT_ERASER_SCALE, PAINT_PREVIEW_STATE, PaintPreviewState,
        };
        let renderer =
            shrimply_preview_render_metal::Renderer::new(Some(std::sync::Arc::new(move |event| {
                shrimply_playback_performance::record_render_event(&playback_performance, event);
            })));
        let mut state = Self {
            renderer,
            viewport: None,
            guides_visible: false,
            fullscreen: false,
            caption_bottom_inset: 0.0,
            guide_input: GuideInput::default(),
            edited_guides: None,
            baseline_guides: None,
            guide_button: Some(guide_button),
            loading_done: Some(loading_done),
            loading_spinner: Some(loading_spinner),
            frame_rate_label: Some(frame_rate_label),
            loading_indicator: Default::default(),
            controller: Controller::default(),
            expressions: RefCell::default(),
            audio_analysis: None,
            presented_frame: None,
            cursor_hidden: false,
            last_sample: None,
            modifiers: shrimply_preview_provider_skia::Modifiers::NONE,
            caption_split_hover: None,
        };
        state.controller.extensions.insert(
            PAINT_PREVIEW_STATE,
            Box::new(PaintPreviewState {
                eraser_scale: DEFAULT_PAINT_ERASER_SCALE,
                ..Default::default()
            }),
        );
        state
    }

    pub fn sync_loading(&mut self, tolerance: shrimply_math_core::Time) {
        if let Some(label) = self
            .renderer
            .render_elapsed()
            .and_then(shrimply_preview_provider_skia::playback::rendered_frame_rate_label)
        {
            let field = self.frame_rate_label
                .as_ref()
                .expect("preview frame-rate label installed");
            let label = objc2_foundation::NSString::from_str(&label);
            if field.stringValue() != label {
                field.setStringValue(&label);
            }
        }
        let visible = self
            .loading_indicator
            .update(self.renderer.loading(tolerance));
        let spinner = self
            .loading_spinner
            .as_ref()
            .expect("preview loading spinner installed");
        let done = self
            .loading_done
            .as_ref()
            .expect("preview loading indicator installed");
        if spinner.isHidden() == visible {
            spinner.setHidden(!visible);
            done.setHidden(visible);
            unsafe {
                if visible {
                    spinner.startAnimation(None);
                } else {
                    spinner.stopAnimation(None);
                }
            }
        }
    }

    pub fn sync_guides(&mut self, guides: &PreviewGuides, visible: bool) {
        if self.guides_visible != visible
            || self.baseline_guides.as_ref().is_some_and(|baseline| {
                baseline.vertical != guides.vertical || baseline.horizontal != guides.horizontal
            })
        {
            self.cancel_guides();
        }
        self.guides_visible = visible;
        if let Some(button) = &self.guide_button {
            super::super::layout::set_toggle_selected(
                button,
                visible,
                super::super::layout::ToggleStyle::Solid,
            );
        }
    }

    fn cancel_guides(&mut self) {
        if self.guide_input.active() || self.edited_guides.is_some() {
            objc2_app_kit::NSCursor::arrowCursor().set();
        }
        self.guide_input = GuideInput::default();
        self.edited_guides = None;
        self.baseline_guides = None;
        if self.controller.sequence == PointerSequence::Guide {
            self.controller.sequence = PointerSequence::Idle;
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if self.cursor_hidden {
            objc2_app_kit::NSCursor::unhide();
        }
    }
}

impl CanvasView {
    pub(super) fn connect_preview_redraw(&self) {
        use shrimply_editor_state::{player_state, preferences, preview_focus};
        let session = &self.ivars().session;
        let dirty = Rc::downgrade(&self.ivars().surface_dirty);
        let alive = dirty.clone();
        player_state::connect_while_alive_named(
            &session.player_state,
            "preview redraw",
            move || alive.strong_count() > 0,
            move |_| {
                if let Some(dirty) = dirty.upgrade() {
                    dirty.set(true);
                }
            },
        );
        let dirty = Rc::downgrade(&self.ivars().surface_dirty);
        shrimply_timeline_skia::selection_state::connect_named(
            &session.selection_state,
            "preview redraw",
            move || {
                if let Some(dirty) = dirty.upgrade() {
                    dirty.set(true);
                }
            },
        );
        let dirty = Rc::downgrade(&self.ivars().surface_dirty);
        preferences::connect(&session.preferences, move |_| {
            if let Some(dirty) = dirty.upgrade() {
                dirty.set(true);
            }
        });
        let dirty = Rc::downgrade(&self.ivars().surface_dirty);
        let alive = dirty.clone();
        preview_focus::connect_while_alive_named(
            &session.preview_focus,
            "preview redraw",
            move || alive.strong_count() > 0,
            move || {
                if let Some(dirty) = dirty.upgrade() {
                    dirty.set(true);
                }
            },
        );
    }

    // Worker progress and native loading indicators must not depend on painting a drawable.
    pub(super) fn poll_preview(&self) -> Result<(), String> {
        let session = &self.ivars().session;
        let (result, updates) = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Preview(preview) = &mut *content else {
                return Ok(());
            };
            let _timing = shrimply_process_reporting::diagnostics::timing("Preview frame polling");
            let player = shrimply_editor_state::player_state::snapshot(&session.player_state);
            let prefs = shrimply_editor_state::preferences::snapshot(&session.preferences);
            let project = session.project.borrow();
            preview.renderer.set_interaction(player.playing, player.scrubbing);
            preview.renderer.set_project_revision(player.revision);
            preview.renderer.set_decoder_limit(prefs.temporal_decoder_pool_size as usize);
            let previous = preview.renderer.presented_frame();
            let result = preview.renderer.prepare(&project, player.position);
            if previous != preview.renderer.presented_frame() {
                self.ivars().surface_dirty.set(true);
            }
            if let Some((id, revision, exclusion)) = preview.renderer.presented_frame()
                && preview.presented_frame != Some(id)
                && preview.controller.accept_base_frame(revision, exclusion)
            {
                preview.presented_frame = Some(id);
                let (time, audio) = preview.renderer.presented_audio()
                    .expect("accepted frame has audio analysis");
                preview.audio_analysis = Some((revision, time, audio.clone()));
                preview.controller.context_invalidated = true;
                self.ivars().surface_dirty.set(true);
            }
            preview.sync_loading(shrimply_project_document::project::scaled_time_delta(
                project.frame_step(), player.playback_speed,
            ));
            (result, preview.renderer.take_manim_updates())
        };
        for update in updates {
            shrimply_editor_state::manim_status::apply(
                &session.project, &session.player_state, update,
            );
        }
        result
    }

    pub(super) fn preview_pointer_move(&self, point: glam::Vec2) {
        let mut content = self.ivars().content.borrow_mut();
        let Content::Preview(state) = &mut *content else {
            return;
        };
        let Some(viewport) = state.viewport else {
            return;
        };
        let project = self.ivars().session.project.borrow();
        state.sync_guides(&project.preview_guides, state.guides_visible);
        let mut guides = state
            .edited_guides
            .take()
            .unwrap_or_else(|| project.preview_guides.clone());
        state
            .guide_input
            .pointer_move(&mut guides, viewport, state.guides_visible, point);
        if state.guide_input.active() {
            state.edited_guides = Some(guides);
            self.ivars().surface_dirty.set(true);
        }
        match state.guide_input.cursor() {
            GuideCursor::Default => objc2_app_kit::NSCursor::arrowCursor().set(),
            GuideCursor::ResizeHorizontal => {
                objc2_app_kit::NSCursor::columnResizeCursorInDirections(
                    objc2_app_kit::NSHorizontalDirections::All,
                )
                .set()
            }
            GuideCursor::ResizeVertical => objc2_app_kit::NSCursor::rowResizeCursorInDirections(
                objc2_app_kit::NSVerticalDirections::All,
            )
            .set(),
        }
    }

    pub(super) fn preview_pointer_down(&self, point: glam::Vec2) {
        let mut content = self.ivars().content.borrow_mut();
        let Content::Preview(state) = &mut *content else {
            return;
        };
        let Some(viewport) = state.viewport else {
            return;
        };
        let mut guides = self.ivars().session.project.borrow().preview_guides.clone();
        let baseline = guides.clone();
        if state
            .guide_input
            .pointer_press(&mut guides, viewport, state.guides_visible, point)
        {
            state.edited_guides = Some(guides);
            state.baseline_guides = Some(baseline);
            state.controller.sequence = PointerSequence::Guide;
            self.ivars().surface_dirty.set(true);
        }
    }

    pub(super) fn preview_pointer_up(&self, point: glam::Vec2) -> Result<(), String> {
        self.preview_pointer_move(point);
        let edited = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Preview(state) = &mut *content else {
                return Ok(());
            };
            let Some(viewport) = state.viewport else {
                return Ok(());
            };
            state.baseline_guides = None;
            if state.controller.sequence == PointerSequence::Guide {
                state.controller.sequence = PointerSequence::Idle;
                state.controller.context_invalidated = true;
            }
            state.edited_guides.take().and_then(|mut guides| {
                state
                    .guide_input
                    .pointer_release(&mut guides, viewport, point)
                    .unwrap_or(false)
                    .then_some(guides)
            })
        };
        if let Some(guides) = edited {
            let mut project = self.ivars().session.project.borrow().clone();
            project.preview_guides = guides;
            shrimply_project_document::project::commit_edit_checked(&project, "preview-guide")?;
            *self.ivars().session.project.borrow_mut() = project;
        }
        objc2_app_kit::NSCursor::arrowCursor().set();
        Ok(())
    }

    pub(super) fn cancel_preview_pointer(&self) {
        self.finish_preview_pointer(false);
    }

    pub(super) fn teardown_preview_pointer(&self) {
        self.finish_preview_pointer(true);
    }

    fn finish_preview_pointer(&self, teardown: bool) {
        self.ivars().surface_dirty.set(true);
        let reset_cursor = matches!(&*self.ivars().content.borrow(), Content::Preview(state)
            if state.controller.provider.is_some() || state.guide_input.active() || state.cursor_hidden || state.caption_split_hover.is_some());
        let response = if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut() {
            state.cancel_guides();
            state.last_sample = None;
            state.caption_split_hover = None;
            if state.controller.provider.is_some() {
                let response = if teardown {
                    state.controller.teardown(
                        &mut self.ivars().session.project.borrow_mut(),
                        &state.expressions,
                    )
                } else {
                    state.controller.cancel(
                        &mut self.ivars().session.project.borrow_mut(),
                        &state.expressions,
                    )
                };
                state.controller.base_exclusion = None;
                state.renderer.set_exclusion(None);
                Some(response)
            } else {
                if teardown {
                    state.controller.sequence = PointerSequence::Idle;
                }
                None
            }
        } else {
            None
        };
        if teardown {
            self.ivars().secondary_preview_active.set(false);
            self.ivars().suppress_primary.set(false);
        }
        if let Some(response) = response
            && let Err(error) = self.apply_preview_response(response)
        {
            self.show_error(&error);
        }
        if reset_cursor {
            self.set_preview_cursor(shrimply_preview_provider_skia::CursorUpdate::Clear);
        }
    }

    pub fn set_preview_fullscreen(&self, fullscreen: bool) {
        if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut()
            && state.fullscreen != fullscreen
        {
            state.fullscreen = fullscreen;
            self.ivars().surface_dirty.set(true);
        }
    }

    pub fn set_caption_bottom_inset(&self, inset: f32) {
        if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut()
            && state.caption_bottom_inset != inset
        {
            state.caption_bottom_inset = inset;
            self.ivars().surface_dirty.set(true);
        }
    }
}

pub(super) fn draw_guides(
    canvas: &skia_safe::Canvas,
    state: &State,
    project: &shrimply_project_document::project::Project,
    size: NSSize,
) {
    if state.guides_visible
        && let Some(viewport) = state.viewport
    {
        guides::draw(
            &shrimply_components_skia::canvas::TimelinePainter::new(canvas),
            state
                .edited_guides
                .as_deref()
                .unwrap_or(&project.preview_guides),
            viewport,
            shrimply_components_skia::Rect::from_min_size(
                glam::Vec2::ZERO,
                glam::Vec2::new(size.width as f32, size.height as f32),
            ),
            shrimply_cross_ui_theme::current().accent_blue_standalone,
        );
    }
}

impl CanvasView {
    pub fn poll_startup(&self) -> Result<shrimply_preview_render_metal::StartupStatus, String> {
        use shrimply_editor_state::player_state;
        use shrimply_preview_render_metal::StartupStatus;

        let session = &self.ivars().session;
        let player = player_state::snapshot(&session.player_state);
        let (result, updates) = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Preview(preview) = &mut *content else {
                return Ok(StartupStatus::Ready);
            };
            preview.renderer.set_decoder_limit(
                shrimply_editor_state::preferences::snapshot(&session.preferences)
                    .temporal_decoder_pool_size as usize,
            );
            preview.renderer.set_project_revision(player.revision);
            let result = preview
                .renderer
                .prepare(&session.project.borrow(), player.position);
            (result, preview.renderer.take_manim_updates())
        };
        for update in updates {
            shrimply_editor_state::manim_status::apply(
                &session.project,
                &session.player_state,
                update,
            );
        }
        result?;
        let mut content = self.ivars().content.borrow_mut();
        let Content::Preview(preview) = &mut *content else {
            unreachable!()
        };
        // Manim preparation can revise the project; wait for the matching frame.
        preview
            .renderer
            .set_project_revision(player_state::snapshot(&session.player_state).revision);
        preview.renderer.startup_status()
    }
}
