use super::*;
use shrimply_preview_core::PreviewViewport;
use shrimply_preview_interaction_core::controller::{Controller, PointerSequence};
use shrimply_preview_interaction_core::guides::{self, GuideCursor, GuideInput};
use shrimply_project::project::PreviewGuides;

pub(super) mod captions;
mod context_menu;
mod input;

#[derive(Default)]
pub struct State {
    pub renderer: shrimply_preview_metal::Renderer,
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
    pub loading_since: Option<std::time::Instant>,
    pub controller: Controller,
    pub expressions: RefCell<shrimply_evaluation::TransformExpressionCache>,
    pub audio_analysis: Option<(
        u64,
        shrimply_math_core::Time,
        shrimply_evaluation::FrameAudioAnalysis,
    )>,
    pub presented_frame: Option<u32>,
    pub cursor_hidden: bool,
    pub last_sample: Option<shrimply_preview_core::PointerSample>,
    pub modifiers: shrimply_preview_core::Modifiers,
    pub caption_split_hover: Option<glam::Vec2>,
}

impl State {
    pub fn new(
        guide_button: Retained<objc2_app_kit::NSButton>,
        loading_done: Retained<objc2_app_kit::NSButton>,
        loading_spinner: Retained<objc2_app_kit::NSProgressIndicator>,
        frame_rate_label: Retained<objc2_app_kit::NSTextField>,
    ) -> Self {
        use shrimply_paint_edit::{
            DEFAULT_PAINT_ERASER_SCALE, PAINT_PREVIEW_STATE, PaintPreviewState,
        };
        let mut state = Self::default();
        state.controller.extensions.insert(
            PAINT_PREVIEW_STATE,
            Box::new(PaintPreviewState {
                eraser_scale: DEFAULT_PAINT_ERASER_SCALE,
                ..Default::default()
            }),
        );
        state.guide_button = Some(guide_button);
        state.loading_done = Some(loading_done);
        state.loading_spinner = Some(loading_spinner);
        state.frame_rate_label = Some(frame_rate_label);
        state
    }

    pub fn sync_loading(&mut self, tolerance: shrimply_math_core::Time) {
        if let Some(label) = self
            .renderer
            .render_elapsed()
            .and_then(shrimply_preview_core::playback::rendered_frame_rate_label)
        {
            self.frame_rate_label
                .as_ref()
                .expect("preview frame-rate label installed")
                .setStringValue(&objc2_foundation::NSString::from_str(&label));
        }
        let loading = self.renderer.loading(tolerance);
        if loading {
            self.loading_since
                .get_or_insert_with(std::time::Instant::now);
        } else {
            self.loading_since = None;
        }
        let visible = self.loading_since.is_some_and(|started| {
            started.elapsed() >= shrimply_preview_core::playback::LOADING_INDICATOR_DELAY
        });
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
            button.setState(if visible {
                objc2_app_kit::NSControlStateValueOn
            } else {
                objc2_app_kit::NSControlStateValueOff
            });
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
            shrimply_project::project::commit_edit_checked(&project, "preview-guide")?;
            *self.ivars().session.project.borrow_mut() = project;
        }
        objc2_app_kit::NSCursor::arrowCursor().set();
        Ok(())
    }

    pub(super) fn cancel_preview_pointer(&self) {
        let reset_cursor = matches!(&*self.ivars().content.borrow(), Content::Preview(state)
            if state.controller.provider.is_some() || state.guide_input.active() || state.cursor_hidden || state.caption_split_hover.is_some());
        let response = if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut() {
            state.cancel_guides();
            state.last_sample = None;
            state.caption_split_hover = None;
            if state.controller.provider.is_some() {
                let response = state.controller.cancel(
                    &mut self.ivars().session.project.borrow_mut(),
                    &state.expressions,
                );
                state.controller.base_exclusion = None;
                state.renderer.set_exclusion(None);
                Some(response)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(response) = response
            && let Err(error) = self.apply_preview_response(response)
        {
            self.show_error(&error);
        }
        if reset_cursor {
            self.set_preview_cursor(shrimply_preview_core::CursorUpdate::Clear);
        }
    }

    pub fn set_preview_fullscreen(&self, fullscreen: bool) {
        if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut() {
            state.fullscreen = fullscreen;
        }
    }

    pub fn set_caption_bottom_inset(&self, inset: f32) {
        if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut()
            && state.caption_bottom_inset != inset
        {
            state.caption_bottom_inset = inset;
            self.setNeedsDisplay(true);
        }
    }
}

pub(super) fn draw_guides(
    canvas: &skia_safe::Canvas,
    state: &State,
    project: &shrimply_project::project::Project,
    size: NSSize,
) {
    if state.guides_visible
        && let Some(viewport) = state.viewport
    {
        guides::draw(
            &shrimply_skia_adw_core::canvas::TimelinePainter::new(canvas),
            state
                .edited_guides
                .as_deref()
                .unwrap_or(&project.preview_guides),
            viewport,
            shrimply_skia_adw_core::Rect::from_min_size(
                glam::Vec2::ZERO,
                glam::Vec2::new(size.width as f32, size.height as f32),
            ),
            shrimply_cross_ui_theme::current().accent_blue_standalone,
        );
    }
}
