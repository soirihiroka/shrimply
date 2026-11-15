//! Toolkit-independent preview controls, guides, navigation and recorded overlay.
use std::cell::RefCell;

use glam::Vec2;
use shrimply_components_skia::input::{TouchAction, TouchChange, TouchInput};
pub use shrimply_paint_edit_skia::toolbar as paint_toolbar;
pub use shrimply_preview_provider_skia::math::{pen_tilt, preview_scroll_delta};
pub use shrimply_preview_provider_skia::{
    Color, Key, KeyState, KeyboardEvent, Modifiers, PointerButton, PointerEvent, PointerInput,
    PointerSample, PointerTool, PreviewRefresh, PreviewResponse, PreviewTarget, PreviewViewport,
    Rect,
};
use shrimply_project_document::project::{ItemAddress, PreviewGuides, Project, Time};
use shrimply_project_evaluation::{FrameAudioAnalysis, TransformExpressionCache};

use crate::{
    controller::{self, Controller, PointerSequence, Preparation},
    guides::{self, GuideInput},
    navigation::Navigation,
};

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
    clock: Clock,
    selected: Option<(ItemAddress, PreviewTarget)>,
    configuration: Configuration,
    audio_revision: u64,
    guides: PreviewGuides,
    guide_colors: [Color; 2],
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
    overlay: Option<Overlay>,
    committed: bool,
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
        }
    }

    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_ref()
    }

    pub fn focus(&mut self, focus: Option<(ItemAddress, PreviewTarget)>) {
        if self.focused != focus {
            self.focused = focus;
            self.core.context_invalidated = true;
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
        if self.core.accept_base_frame(revision, exclusion) {
            self.core.frame_pending = true;
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

    fn ensure(
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
        );
        let selected = selection.map(|(address, target)| (address.clone(), target));
        let changed = self.stamp.as_ref().is_some_and(|stamp| {
            stamp.selected != selected || stamp.configuration != configuration
        });
        let response = if changed && self.core.sequence != PointerSequence::Idle {
            self.cancel_pointer(project)
        } else {
            PreviewResponse::IGNORED
        };
        if !configuration.navigation_enabled {
            self.navigation.reset();
        }
        let viewport = self
            .navigation
            .viewport(configuration.fit(project), configuration.bounds());
        let position = self.displayed_position(clock);
        self.core.ensure(
            project,
            selected
                .as_ref()
                .map(|(address, target)| (address, *target)),
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
                camera_sampler: crate::provider::sample_camera,
            },
        )?;
        Ok(response)
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
        let fit = configuration.fit(project);
        let bounds = configuration.bounds();
        if let Some(navigation) = self.navigation.pointer(
            fit,
            bounds,
            event,
            configuration.navigation_enabled,
            self.core.sequence != PointerSequence::Idle,
        ) {
            self.core.context_invalidated |= navigation.redraw;
            self.core.frame_pending |= navigation.redraw;
            response.merge(navigation);
            return Ok(response);
        }
        let viewport = self.navigation.viewport(fit, bounds);
        let guide = match event {
            PointerEvent::Begin(input) if input.button == PointerButton::Primary => {
                self.guides.pointer_press(
                    &mut project.preview_guides,
                    viewport,
                    configuration.guides_visible,
                    input.sample.position,
                )
            }
            PointerEvent::Hover(input) | PointerEvent::Samples { input, .. } => {
                let active = self.guides.active();
                self.guides.pointer_move(
                    &mut project.preview_guides,
                    viewport,
                    configuration.guides_visible,
                    input.sample.position,
                );
                active
            }
            PointerEvent::End(input) => {
                let changed = self.guides.pointer_release(
                    &mut project.preview_guides,
                    viewport,
                    input.sample.position,
                );
                if changed == Some(true) {
                    guides::commit_edit(project);
                    self.committed = true;
                    self.core.project_committed(clock.revision);
                }
                changed.is_some()
            }
            _ => false,
        };
        if guide {
            self.core.sequence = if self.guides.active() {
                PointerSequence::Guide
            } else {
                PointerSequence::Idle
            };
            self.core.frame_pending = true;
            response.merge(PreviewResponse::handled().redraw());
            return Ok(response);
        }
        if let PointerEvent::Begin(input) = event
            && !self
                .navigation
                .clip_rect(bounds, configuration.size)
                .contains(input.sample.position)
        {
            return Ok(response);
        }
        let provider = self.core.pointer(project, &self.expressions, event);
        if provider.edit.commits() {
            controller::commit_response(project, provider, "preview-provider");
            self.core.project_committed(clock.revision);
            self.committed = true;
        }
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
        if let Some(navigation) = self.navigation.keyboard(event) {
            self.core.frame_pending |= navigation.redraw;
            response.merge(navigation);
            return Ok(response);
        }
        let keyboard = self.core.keyboard(project, &self.expressions, event);
        if keyboard.edit.commits() {
            controller::commit_response(project, keyboard, "preview-keyboard");
            self.core.project_committed(clock.revision);
            self.committed = true;
        }
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
                TouchAction::Cancel(_) => output.response.merge(self.cancel_pointer(project)),
                TouchAction::History { redo } => {
                    output.response.merge(self.cancel_pointer(project));
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
                        self.core.context_invalidated = true;
                        self.core.frame_pending = true;
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

    fn cancel_pointer(&mut self, project: &mut Project) -> PreviewResponse {
        let mut response = self.core.teardown(project, &self.expressions);
        response.redraw |= self.guides.pointer_cancel(&mut project.preview_guides);
        self.navigation.cancel();
        self.core.frame_pending |= response.redraw;
        response
    }

    pub fn cancel(&mut self, project: &mut Project) -> PreviewResponse {
        self.synchronize_history();
        self.touch.cancel();
        self.cancel_pointer(project)
    }

    pub fn exclusion_update(&mut self) -> Option<Option<uuid::Uuid>> {
        let exclusion = self
            .core
            .provider
            .as_ref()
            .and_then(|provider| provider.provider.base_frame_exclusion());
        if exclusion == self.core.base_exclusion {
            return None;
        }
        self.core.base_exclusion = exclusion;
        Some(exclusion)
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

    pub fn prepare_overlay(
        &mut self,
        project: &mut Project,
        selected: Option<&ItemAddress>,
        clock: Clock,
        configuration: Configuration,
    ) -> Result<PreviewResponse, String> {
        let response = self.ensure(project, selected, clock, configuration)?;
        let viewport = self
            .navigation
            .viewport(configuration.fit(project), configuration.bounds());
        let selection = controller::selection(
            project,
            selected,
            self.focused
                .as_ref()
                .map(|(address, target)| (address, *target)),
        )
        .map(|(address, target)| (address.clone(), target));
        let stamp = Stamp {
            clock: Clock {
                position: self.displayed_position(clock),
                ..clock
            },
            selected: selection,
            configuration,
            audio_revision: self.audio_revision,
            guides: project.preview_guides.as_ref().clone(),
            guide_colors: {
                let theme = shrimply_cross_ui_theme::current();
                [theme.sidebar_bg, theme.sidebar_fg]
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
        if configuration.guides_visible {
            guides::draw(
                &shrimply_components_skia::canvas::TimelinePainter::new(canvas),
                &project.preview_guides,
                viewport,
                Rect::from_min_size(Vec2::ZERO, configuration.size),
                Color::BLUE5,
            );
        }
        let clip = self
            .navigation
            .clip_rect(configuration.bounds(), configuration.size);
        canvas.save();
        canvas.clip_rect(
            skia_safe::Rect::from_xywh(clip.min.x, clip.min.y, clip.width(), clip.height()),
            None,
            true,
        );
        self.core.draw(canvas, &self.expressions);
        canvas.restore();
        let picture = recorder
            .finish_recording_as_picture(None)
            .expect("record shared preview controls");
        self.overlay = Some(Overlay {
            picture,
            viewport,
            clip,
            size: configuration.size,
        });
        self.stamp = Some(stamp);
        self.core.frame_pending = false;
        Ok(response)
    }
}
