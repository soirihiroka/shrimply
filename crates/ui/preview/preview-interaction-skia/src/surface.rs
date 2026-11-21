//! Toolkit-independent preview lifecycle, controls, guides, navigation, and Skia overlay.
use std::cell::RefCell;

use glam::Vec2;
use shrimply_components_skia::{
    canvas::TimelinePainter,
    input::{TouchAction, TouchChange, TouchInput},
};
pub use shrimply_paint_edit_skia::toolbar as paint_toolbar;
pub use shrimply_preview_provider_skia::math::{pen_tilt, preview_scroll_delta};
pub use shrimply_preview_provider_skia::{
    Color, Cursor, CursorUpdate, Key, KeyState, KeyboardEvent, Modifiers, PointerButton,
    PointerEvent, PointerInput, PointerSample, PointerTool, PreviewExtensionKey, PreviewRefresh,
    PreviewResponse, PreviewTarget, PreviewViewport, Rect,
};
use shrimply_project_document::project::{ItemAddress, PreviewGuides, Project, Time};
use shrimply_project_evaluation::{FrameAudioAnalysis, TransformExpressionCache};

use crate::{
    captions::{self, CaptionAppearance},
    controller::{self, Controller, PointerSequence, Preparation, PreparedProvider},
    guides::{self, GuideCursor, GuideInput},
    navigation::Navigation,
};

pub fn background_color(window_color: Color, fullscreen: bool) -> Color {
    if fullscreen {
        Color::BLACK
    } else {
        window_color
    }
}

#[derive(Clone, Copy)]
pub struct BackgroundAppearance {
    pub content_rect: Rect,
    pub clip_rect: Rect,
    pub background: Color,
    pub shadow_size: u32,
    pub pixel_scale: f32,
}

pub fn draw_background(target: &skia_safe::Canvas, appearance: BackgroundAppearance) {
    shrimply_preview_provider_skia::canvas::draw_background(
        target,
        shrimply_preview_provider_skia::canvas::Appearance {
            content_rect: Rect::from_min_max(
                appearance.content_rect.min.max(appearance.clip_rect.min),
                appearance.content_rect.max.min(appearance.clip_rect.max),
            ),
            background: appearance.background,
            shadow_size: appearance.shadow_size,
            pixel_scale: appearance.pixel_scale,
        },
    );
}

#[derive(Clone, Copy, PartialEq)]
pub struct Configuration {
    pub size: Vec2,
    pub padding: u32,
    pub guides_visible: bool,
    pub fullscreen: bool,
    pub snap_enabled: bool,
    pub snap_radius: f32,
    pub navigation_enabled: bool,
}

impl Configuration {
    fn fit(self, project: &Project) -> PreviewViewport {
        guides::viewport(
            self.size.round().as_ivec2(),
            project.canvas_size,
            self.padding,
            self.guides_visible,
            self.fullscreen,
        )
    }

    fn bounds(self) -> Rect {
        guides::bounds(
            self.size,
            self.padding,
            self.guides_visible,
            self.fullscreen,
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct Layout {
    pub fit: PreviewViewport,
    pub bounds: Rect,
    pub viewport: PreviewViewport,
    pub clip_rect: Rect,
    pub surface_rect: Rect,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Clock {
    pub revision: u64,
    pub position: Time,
    pub moving: bool,
}

#[derive(Clone)]
pub struct Overlay {
    pub picture: skia_safe::Picture,
    pub viewport: PreviewViewport,
    pub clip: Rect,
    pub size: Vec2,
}

impl PartialEq for Overlay {
    fn eq(&self, other: &Self) -> bool {
        self.picture.unique_id() == other.picture.unique_id()
    }
}

#[derive(Default)]
pub struct TouchResponse {
    pub response: PreviewResponse,
    pub history: Option<bool>,
}

#[derive(PartialEq)]
struct Stamp {
    revision: u64,
    animation: Option<(Clock, u64)>,
    selected: Option<(ItemAddress, PreviewTarget)>,
    configuration: Configuration,
    viewport: PreviewViewport,
    clip: Rect,
    guides: PreviewGuides,
    theme_colors: [Color; 4],
}

#[derive(Default)]
pub struct Surface {
    core: Controller,
    expressions: RefCell<TransformExpressionCache>,
    audio: FrameAudioAnalysis,
    audio_revision: u64,
    presented: Option<(u64, Time)>,
    focused: Option<(ItemAddress, PreviewTarget)>,
    guides: GuideInput,
    touch: TouchInput,
    navigation: Navigation,
    stamp: Option<Stamp>,
    last_selection: Option<(ItemAddress, PreviewTarget)>,
    last_configuration: Option<Configuration>,
    overlay: Option<Overlay>,
    committed: bool,
    cursor: Cursor,
}

impl Surface {
    pub fn paint_toolbar(
        &self,
        paint: &shrimply_project_document::project::PaintItem,
        time: Time,
    ) -> paint_toolbar::Presentation {
        use shrimply_project_document::project::{PAINT_PREVIEW_STATE, PaintPreviewState};
        paint_toolbar::Presentation::new(
            self.core
                .extension::<PaintPreviewState>(PAINT_PREVIEW_STATE)
                .expect("paint preview state"),
            paint,
            time,
        )
    }

    pub fn paint_toolbar_action(
        &mut self,
        paint: &shrimply_project_document::project::PaintItem,
        action: paint_toolbar::Action,
        value: Option<f32>,
    ) -> Result<(), String> {
        use shrimply_project_document::project::{PAINT_PREVIEW_STATE, PaintPreviewState};
        let mut result = Ok(());
        assert!(
            self.core
                .update_extension(PAINT_PREVIEW_STATE, |state: &mut PaintPreviewState| {
                    result = paint_toolbar::apply(state, paint, action, value)
                }),
            "paint preview state"
        );
        self.stamp = None;
        result
    }

    pub fn extension<T: 'static>(&self, key: PreviewExtensionKey) -> Option<&T> {
        self.core.extension(key)
    }

    pub fn update_extension<T: 'static>(
        &mut self,
        key: PreviewExtensionKey,
        update: impl FnOnce(&mut T),
    ) -> bool {
        self.synchronize_history();
        self.stamp = None;
        self.core.update_extension(key, update)
    }

    fn synchronize_history(&mut self) {
        if self.core.synchronize_history() {
            // The model already belongs to another edit. Dropping these drafts must
            // never run provider/guide Cancel against that replacement model.
            self.guides = GuideInput::default();
            self.touch.cancel();
            self.navigation.cancel();
            self.stamp = None;
            self.overlay = None;
            self.committed = false;
            self.cursor = Cursor::Default;
        }
    }

    pub fn layout(&self, project: &Project, configuration: Configuration) -> Layout {
        let fit = configuration.fit(project);
        let bounds = configuration.bounds();
        Layout {
            fit,
            bounds,
            viewport: self.navigation.viewport(fit, bounds),
            clip_rect: self.navigation.clip_rect(bounds, configuration.size),
            surface_rect: Rect::from_min_size(Vec2::ZERO, configuration.size),
        }
    }

    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_ref()
    }

    pub fn provider(&self) -> Option<&PreparedProvider> {
        self.core.provider.as_ref()
    }

    pub fn context_invalidated(&self) -> bool {
        self.core.context_invalidated
    }

    pub fn audio_analysis(&self) -> &FrameAudioAnalysis {
        &self.audio
    }

    pub fn expressions(&self) -> &RefCell<TransformExpressionCache> {
        &self.expressions
    }

    pub fn extensions(
        &self,
    ) -> &std::collections::HashMap<PreviewExtensionKey, Box<dyn std::any::Any>> {
        &self.core.extensions
    }

    pub fn focus(&mut self, focus: Option<(ItemAddress, PreviewTarget)>) {
        if self.focused != focus {
            self.focused = focus;
            self.invalidate();
        }
    }

    pub fn invalidate(&mut self) {
        self.core.context_invalidated = true;
        self.core.frame_pending = true;
        self.stamp = None;
    }

    pub fn take_frame_pending(&mut self) -> bool {
        std::mem::take(&mut self.core.frame_pending)
    }

    pub fn navigation_active(&self) -> bool {
        self.navigation.active()
    }

    pub fn guides_active(&self) -> bool {
        self.guides.active()
    }

    pub fn guide_cursor(&self) -> GuideCursor {
        self.guides.cursor()
    }

    pub fn is_idle(&self) -> bool {
        self.core.sequence == PointerSequence::Idle
            && !self.guides.active()
            && !self.navigation.active()
    }

    pub fn cursor(&self) -> Cursor {
        if self.navigation.active() {
            return Cursor::Grabbing;
        }
        match self.guides.cursor() {
            GuideCursor::ResizeHorizontal => Cursor::ResizeHorizontal,
            GuideCursor::ResizeVertical => Cursor::ResizeVertical,
            GuideCursor::Default => self.cursor,
        }
    }

    pub fn presented(
        &mut self,
        revision: u64,
        position: Time,
        exclusion: Option<uuid::Uuid>,
        audio: &FrameAudioAnalysis,
    ) -> bool {
        self.synchronize_history();
        let overlay_changed = self.core.provider.is_some() || self.core.retiring_provider.is_some();
        if self.core.accept_base_frame(revision, exclusion) {
            self.core.frame_pending |= overlay_changed;
            self.presented = Some((revision, position));
            if !self.audio.same_frame(audio) {
                self.audio = audio.clone();
                self.audio_revision = self
                    .audio_revision
                    .checked_add(1)
                    .expect("preview audio revision overflow");
            }
            true
        } else {
            false
        }
    }

    pub fn displayed_position(&self, clock: Clock) -> Time {
        self.presented
            .filter(|(revision, _)| *revision == clock.revision && clock.moving)
            .map_or(clock.position, |(_, position)| position)
    }

    pub fn ensure_readonly(
        &mut self,
        project: &Project,
        selected: Option<&ItemAddress>,
        clock: Clock,
        configuration: Configuration,
    ) -> Result<bool, String> {
        self.synchronize_history();
        if !configuration.size.is_finite() || configuration.size.min_element() <= 0.0 {
            return Err("Preview surface is empty".into());
        }
        if self
            .focused
            .as_ref()
            .is_some_and(|(address, _)| Some(address) != selected)
        {
            self.focused = None;
        }
        if !configuration.navigation_enabled {
            self.navigation.reset();
        }
        let selection = controller::selection(
            project,
            selected,
            self.focused
                .as_ref()
                .map(|(address, target)| (address, *target)),
        );
        self.last_selection = selection.map(|(address, target)| (address.clone(), target));
        self.last_configuration = Some(configuration);
        let viewport = self.layout(project, configuration).viewport;
        let position = self.displayed_position(clock);
        self.core.ensure(
            project,
            selection,
            position,
            Preparation {
                project_revision: clock.revision,
                viewport,
                audio_analysis: &self.audio,
                expression_cache: &self.expressions,
                snap_enabled: configuration.snap_enabled,
                snap_radius_px: configuration.snap_radius,
                guides: configuration
                    .guides_visible
                    .then_some(project.preview_guides.as_ref()),
            },
        )
    }

    pub fn ensure(
        &mut self,
        project: &mut Project,
        selected: Option<&ItemAddress>,
        clock: Clock,
        configuration: Configuration,
    ) -> Result<PreviewResponse, String> {
        self.synchronize_history();
        if !configuration.size.is_finite() || configuration.size.min_element() <= 0.0 {
            return Err("Preview surface is empty".into());
        }
        if self
            .focused
            .as_ref()
            .is_some_and(|(address, _)| Some(address) != selected)
        {
            self.focused = None;
        }
        let selection = controller::selection(
            project,
            selected,
            self.focused
                .as_ref()
                .map(|(address, target)| (address, *target)),
        )
        .map(|(address, target)| (address.clone(), target));
        let changed = self
            .last_configuration
            .is_some_and(|last| self.last_selection != selection || last != configuration);
        let response = if changed && self.core.sequence != PointerSequence::Idle {
            self.cancel_pointer(project, true)
        } else {
            PreviewResponse::IGNORED
        };
        self.ensure_readonly(project, selected, clock, configuration)?;
        Ok(response)
    }

    pub fn pointer_leave(&mut self, project: &mut Project) -> PreviewResponse {
        self.guides.pointer_leave();
        self.cursor = Cursor::Default;
        let mut response = if self.core.sequence == PointerSequence::Idle {
            self.core
                .pointer(project, &self.expressions, PointerEvent::Leave)
        } else {
            PreviewResponse::IGNORED
        };
        response.cursor = CursorUpdate::Clear;
        response
    }

    fn apply_cursor(&mut self, update: CursorUpdate) {
        match update {
            CursorUpdate::Keep => {}
            CursorUpdate::Set(cursor) => self.cursor = cursor,
            CursorUpdate::Clear => self.cursor = Cursor::Default,
        }
    }

    pub fn pointer(
        &mut self,
        project: &mut Project,
        selected: Option<&ItemAddress>,
        clock: Clock,
        configuration: Configuration,
        event: PointerEvent<'_>,
    ) -> Result<PreviewResponse, String> {
        let mut response = self.ensure(project, selected, clock, configuration)?;
        let layout = self.layout(project, configuration);
        if let Some(navigation) = self.navigation.pointer(
            layout.fit,
            layout.bounds,
            event,
            configuration.navigation_enabled,
            self.core.sequence != PointerSequence::Idle || self.guides.active(),
        ) {
            self.core.context_invalidated |= navigation.redraw;
            self.core.frame_pending |= navigation.redraw;
            self.apply_cursor(navigation.cursor);
            response.merge(navigation);
            return Ok(response);
        }
        if matches!(event, PointerEvent::Leave) {
            response.merge(self.pointer_leave(project));
            return Ok(response);
        }
        let was_guide_cursor = self.guides.cursor() != GuideCursor::Default;
        let guide = match event {
            PointerEvent::Begin(input)
                if input.button == PointerButton::Primary
                    && self.core.sequence == PointerSequence::Idle =>
            {
                self.guides.pointer_press(
                    &mut project.preview_guides,
                    layout.viewport,
                    configuration.guides_visible,
                    input.sample.position,
                )
            }
            PointerEvent::Hover(input) => {
                self.guides.pointer_move(
                    &mut project.preview_guides,
                    layout.viewport,
                    configuration.guides_visible,
                    input.sample.position,
                );
                if self.guides.active() {
                    true
                } else if let Some(cursor) = guide_cursor(self.guides.cursor()) {
                    response.merge(PreviewResponse {
                        handled: true,
                        cursor: CursorUpdate::Set(cursor),
                        ..PreviewResponse::IGNORED
                    });
                    return Ok(response);
                } else {
                    false
                }
            }
            PointerEvent::Samples { input, .. } => {
                let active = self.guides.active();
                if active {
                    self.guides.pointer_move(
                        &mut project.preview_guides,
                        layout.viewport,
                        configuration.guides_visible,
                        input.sample.position,
                    );
                }
                active
            }
            PointerEvent::End(input) => {
                let changed = self.guides.pointer_release(
                    &mut project.preview_guides,
                    layout.viewport,
                    input.sample.position,
                );
                if changed == Some(true) {
                    guides::commit_edit(project);
                    self.committed = true;
                    self.core.project_committed(clock.revision);
                    self.core.context_invalidated = true;
                }
                changed.is_some()
            }
            PointerEvent::Cancel => self.guides.pointer_cancel(&mut project.preview_guides),
            _ => false,
        };
        if guide {
            self.core.sequence = if self.guides.active() {
                PointerSequence::Guide
            } else {
                PointerSequence::Idle
            };
            self.core.frame_pending = true;
            let cursor =
                guide_cursor(self.guides.cursor()).map_or(CursorUpdate::Clear, CursorUpdate::Set);
            self.apply_cursor(cursor);
            response.merge(PreviewResponse {
                handled: true,
                redraw: true,
                cursor,
                edit: Default::default(),
            });
            return Ok(response);
        }
        if let PointerEvent::Begin(input) = event
            && !layout.clip_rect.contains(input.sample.position)
        {
            return Ok(response);
        }
        let mut provider = self.core.pointer(project, &self.expressions, event);
        if was_guide_cursor && provider.cursor == CursorUpdate::Keep {
            provider.cursor = CursorUpdate::Clear;
        }
        if provider.edit.commits() {
            controller::commit_response(project, provider, "preview-provider");
            self.core.project_committed(clock.revision);
            self.committed = true;
        }
        self.apply_cursor(provider.cursor);
        response.merge(provider);
        Ok(response)
    }

    pub fn keyboard(
        &mut self,
        project: &mut Project,
        selected: Option<&ItemAddress>,
        clock: Clock,
        configuration: Configuration,
        event: KeyboardEvent,
    ) -> Result<PreviewResponse, String> {
        let mut response = self.ensure(project, selected, clock, configuration)?;
        if event.key == Key::Escape && self.guides.active() {
            let canceled = self.cancel_pointer(project, true);
            response.merge(canceled);
            return Ok(response);
        }
        if let Some(navigation) = self.navigation.keyboard(event) {
            self.core.frame_pending |= navigation.redraw;
            self.apply_cursor(navigation.cursor);
            response.merge(navigation);
            return Ok(response);
        }
        let keyboard = self.core.keyboard(project, &self.expressions, event);
        if keyboard.edit.commits() {
            controller::commit_response(project, keyboard, "preview-keyboard");
            self.core.project_committed(clock.revision);
            self.committed = true;
        }
        self.apply_cursor(keyboard.cursor);
        response.merge(keyboard);
        Ok(response)
    }

    pub fn touch(
        &mut self,
        project: &mut Project,
        selected: Option<&ItemAddress>,
        clock: Clock,
        configuration: Configuration,
        changes: &[TouchChange],
    ) -> Result<TouchResponse, String> {
        self.synchronize_history();
        let mut output = TouchResponse::default();
        for action in self.touch.update(changes) {
            let pointer = |position| PointerInput {
                sample: PointerSample {
                    position,
                    ..Default::default()
                },
                tool: PointerTool::Touch,
                ..Default::default()
            };
            match action {
                TouchAction::Press(point) => output.response.merge(self.pointer(
                    project,
                    selected,
                    clock,
                    configuration,
                    PointerEvent::Begin(pointer(point)),
                )?),
                TouchAction::Move(point) => output.response.merge(self.pointer(
                    project,
                    selected,
                    clock,
                    configuration,
                    PointerEvent::Samples {
                        input: pointer(point),
                        samples: &[],
                    },
                )?),
                TouchAction::Release(point) => output.response.merge(self.pointer(
                    project,
                    selected,
                    clock,
                    configuration,
                    PointerEvent::End(pointer(point)),
                )?),
                TouchAction::Cancel(_) => output.response.merge(self.cancel_pointer(project, true)),
                TouchAction::History { redo } => {
                    output.response.merge(self.cancel_pointer(project, true));
                    output.history = Some(redo);
                }
                TouchAction::Transform {
                    previous_center,
                    center,
                    scale,
                } => {
                    if configuration.navigation_enabled
                        && self.navigation.transform(
                            configuration.fit(project),
                            configuration.bounds(),
                            previous_center,
                            center,
                            scale,
                        )
                    {
                        self.invalidate();
                        output.response.redraw = true;
                    }
                }
            }
        }
        Ok(output)
    }

    pub fn poll_context(&mut self) -> Option<Vec2> {
        self.synchronize_history();
        self.touch.poll_long_press()
    }

    fn cancel_pointer(&mut self, project: &mut Project, teardown: bool) -> PreviewResponse {
        let mut response = if teardown {
            self.core.teardown(project, &self.expressions)
        } else {
            self.core.cancel(project, &self.expressions)
        };
        response.redraw |= self.guides.pointer_cancel(&mut project.preview_guides);
        self.navigation.cancel();
        self.cursor = Cursor::Default;
        self.core.frame_pending |= response.redraw;
        response
    }

    pub fn cancel(&mut self, project: &mut Project) -> PreviewResponse {
        self.cancel_with(project, true)
    }

    pub fn cancel_with(&mut self, project: &mut Project, teardown: bool) -> PreviewResponse {
        self.synchronize_history();
        self.touch.cancel();
        self.cancel_pointer(project, teardown)
    }

    pub fn exclusion_update(&mut self) -> Option<Option<uuid::Uuid>> {
        self.core.exclusion_update()
    }

    pub fn take_committed(&mut self) -> bool {
        std::mem::take(&mut self.committed)
    }

    pub fn active(&self) -> bool {
        self.core.sequence == PointerSequence::Active
            || self.guides.active()
            || self.navigation.active()
    }

    pub fn take_live_base_request(&mut self) -> bool {
        self.core.take_live_base_request()
    }

    pub fn live_base_requested(&mut self, revision: u64) {
        self.core.live_base_requested(revision);
    }

    pub fn project_committed(&mut self, revision: u64) {
        self.core.project_committed(revision);
    }

    /// Live Skia overlay draw used directly by GTK, Qt, and AppKit.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_overlay(
        &mut self,
        canvas: &skia_safe::Canvas,
        project: &Project,
        selected: Option<&ItemAddress>,
        clock: Clock,
        configuration: Configuration,
        captions: CaptionAppearance,
        caption_split_hover: Option<(&ItemAddress, Vec2)>,
        after_controls: impl FnOnce(&skia_safe::Canvas),
    ) -> Result<(), String> {
        self.ensure_readonly(project, selected, clock, configuration)?;
        let layout = self.layout(project, configuration);
        let accent = shrimply_preview_provider_skia::selection_color();
        captions::draw_captions(
            &TimelinePainter::new(canvas),
            project,
            self.displayed_position(clock),
            captions,
            caption_split_hover.map(|(address, point)| (address, point, accent)),
        );
        self.draw_interaction(canvas, project, configuration, layout, after_controls);
        Ok(())
    }

    /// Off-thread recorded Skia overlay used by Android's separate Vulkan render thread.
    pub fn prepare_overlay(
        &mut self,
        project: &mut Project,
        selected: Option<&ItemAddress>,
        clock: Clock,
        configuration: Configuration,
    ) -> Result<PreviewResponse, String> {
        let response = self.ensure(project, selected, clock, configuration)?;
        let layout = self.layout(project, configuration);
        let selection = controller::selection(
            project,
            selected,
            self.focused
                .as_ref()
                .map(|(address, target)| (address, *target)),
        )
        .map(|(address, target)| (address.clone(), target));
        let stamp = Stamp {
            revision: clock.revision,
            animation: selection.as_ref().map(|_| {
                (
                    Clock {
                        position: self.displayed_position(clock),
                        ..clock
                    },
                    self.audio_revision,
                )
            }),
            selected: selection,
            configuration,
            viewport: layout.viewport,
            clip: layout.clip_rect,
            guides: project.preview_guides.as_ref().clone(),
            theme_colors: {
                let theme = shrimply_cross_ui_theme::current();
                [
                    theme.sidebar_bg,
                    theme.sidebar_fg,
                    theme.accent_bg,
                    theme.accent_fg,
                ]
            },
        };
        if self.stamp.as_ref() == Some(&stamp) && !self.core.frame_pending && self.overlay.is_some()
        {
            return Ok(response);
        }
        let mut recorder = skia_safe::PictureRecorder::new();
        let canvas = recorder.begin_recording(
            skia_safe::Rect::from_wh(configuration.size.x, configuration.size.y),
            false,
        );
        self.draw_interaction(canvas, project, configuration, layout, |_| {});
        let picture = recorder
            .finish_recording_as_picture(None)
            .expect("record shared preview controls");
        self.overlay = Some(Overlay {
            picture,
            viewport: layout.viewport,
            clip: layout.clip_rect,
            size: configuration.size,
        });
        self.stamp = Some(stamp);
        self.core.frame_pending = false;
        Ok(response)
    }

    fn draw_interaction(
        &mut self,
        canvas: &skia_safe::Canvas,
        project: &Project,
        configuration: Configuration,
        layout: Layout,
        after_controls: impl FnOnce(&skia_safe::Canvas),
    ) {
        if configuration.guides_visible {
            guides::draw(
                &TimelinePainter::new(canvas),
                &project.preview_guides,
                layout.viewport,
                layout.surface_rect,
            );
        }
        canvas.save();
        canvas.clip_rect(skia_safe::Rect::from(layout.clip_rect), None, false);
        if let Some(retiring) = self.core.retiring_provider.as_mut() {
            retiring.context.viewport = layout.viewport;
        }
        self.core.draw(canvas, &self.expressions);
        after_controls(canvas);
        canvas.restore();
    }
}

fn guide_cursor(cursor: GuideCursor) -> Option<Cursor> {
    match cursor {
        GuideCursor::Default => None,
        GuideCursor::ResizeHorizontal => Some(Cursor::ResizeHorizontal),
        GuideCursor::ResizeVertical => Some(Cursor::ResizeVertical),
    }
}
