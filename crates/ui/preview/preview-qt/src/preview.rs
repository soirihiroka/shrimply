use super::*;
use shrimply_editor_state::preview_focus::{self, SharedPreviewFocus};
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, Modifiers, PointerButton, PointerEvent, PointerInput, PointerSample,
    PointerTool, PreviewEditSink, PreviewExtensionKey, PreviewItemGeometry, PreviewProvider,
    PreviewRefresh, PreviewResponse, PreviewTarget, PreviewViewport, SnapScene,
};
use shrimply_preview_runtime_cuda::provider::{
    BuildContext, PreparedGeometry, SnapPreparation, prepare_geometry, prepare_snap_scene,
};
use shrimply_project_document::project::{ItemAddress, PreviewGuides};
use shrimply_project_evaluation::{FrameAudioAnalysis, TransformExpressionCache};
use shrimply_timeline_edit::selection_state::{self, SharedSelectionState};
use std::{any::Any, cell::Cell, collections::HashMap};

#[derive(Clone, PartialEq)]
struct SnapConfiguration {
    enabled: bool,
    radius_px: u32,
    guides_visible: bool,
    vertical_guides: Vec<f32>,
    horizontal_guides: Vec<f32>,
}

struct OverlayCacheKey<'a> {
    address: &'a ItemAddress,
    target: PreviewTarget,
    revision: u64,
    position: Time,
    viewport: PreviewViewport,
    audio_revision: u64,
    preferences: &'a preferences_store::PreferencesSnapshot,
    guides: &'a PreviewGuides,
}

struct OverlayRequest {
    address: ItemAddress,
    target: PreviewTarget,
    revision: u64,
    position: Time,
    viewport: PreviewViewport,
    snap_configuration: SnapConfiguration,
}

impl SnapConfiguration {
    fn capture(
        preferences: &preferences_store::PreferencesSnapshot,
        guides: &PreviewGuides,
    ) -> Self {
        Self {
            enabled: preferences.timeline_magnet == "true",
            radius_px: preferences.timeline_snap_radius_px,
            guides_visible: preferences.preview_guides_visible,
            vertical_guides: guides.vertical.clone(),
            horizontal_guides: guides.horizontal.clone(),
        }
    }

    fn matches(
        &self,
        preferences: &preferences_store::PreferencesSnapshot,
        guides: &PreviewGuides,
    ) -> bool {
        self.enabled == (preferences.timeline_magnet == "true")
            && self.radius_px == preferences.timeline_snap_radius_px
            && self.guides_visible == preferences.preview_guides_visible
            && self.vertical_guides == guides.vertical
            && self.horizontal_guides == guides.horizontal
    }
}

struct PreparedOverlay {
    address: ItemAddress,
    target: PreviewTarget,
    revision: u64,
    position: Time,
    viewport: PreviewViewport,
    audio_revision: u64,
    snap_configuration: SnapConfiguration,
    prepared: PreparedGeometry,
    snap_scene: Option<SnapScene>,
    provider: Box<dyn PreviewProvider>,
    deferred_refresh: PreviewRefresh,
}

struct OverlayEdits<'a> {
    project: &'a mut Project,
    extensions: &'a mut HashMap<PreviewExtensionKey, Box<dyn Any>>,
    address: &'a ItemAddress,
    keyframe_time: Time,
    context: BuildContext<'a>,
}

impl PreviewEditSink for OverlayEdits<'_> {
    fn keyframe_time(&self) -> Time {
        self.keyframe_time
    }

    fn target_mut(&mut self, target: PreviewTarget) -> &mut dyn Any {
        self.project
            .preview_target_mut(target)
            .expect("Qt preview target is missing")
    }

    fn updated_geometry(&self, _target: PreviewTarget) -> Option<PreviewItemGeometry> {
        let item = self.project.video_item(self.address)?;
        let mut source_sizes = self.context.source_sizes().clone();
        shrimply_preview_runtime_cuda::provider::update_text_source_size(
            &mut source_sizes,
            item,
            self.context.evaluation(),
            self.context.expression_cache(),
        );
        item.preview_geometry(&self.context.with_source_sizes(&source_sizes))
    }

    fn extension_mut(
        &mut self,
        _target: PreviewTarget,
        key: PreviewExtensionKey,
    ) -> Option<&mut dyn Any> {
        self.extensions.get_mut(&key).map(|value| value.as_mut())
    }
}

impl PreparedOverlay {
    fn current(&self, key: OverlayCacheKey<'_>) -> bool {
        self.address == *key.address
            && self.target == key.target
            && self.revision == key.revision
            && self.position == key.position
            && self.viewport == key.viewport
            && self.audio_revision == key.audio_revision
            && self.snap_configuration.matches(key.preferences, key.guides)
    }

    fn draw(
        &mut self,
        canvas: &skia_safe::Canvas,
        cache: &RefCell<TransformExpressionCache>,
        extensions: &HashMap<shrimply_preview_provider_skia::PreviewExtensionKey, Box<dyn Any>>,
    ) {
        let context = self
            .prepared
            .context(self.position, cache, self.viewport)
            .snapping(self.snap_scene.as_ref())
            .extensions(Some(extensions));
        self.provider.on_draw(canvas, &context);
    }
}

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
    audio_revision: u64,
    extensions: HashMap<shrimply_preview_provider_skia::PreviewExtensionKey, Box<dyn Any>>,
    base_exclusion: Option<uuid::Uuid>,
    presented_base_exclusion: Option<uuid::Uuid>,
    overlay: Option<PreparedOverlay>,
    retiring_overlay: Option<PreparedOverlay>,
    provider_active: bool,
    provider_cursor: Cursor,
    guide_active: bool,
    provider_origin: Option<PointerSample>,
    provider_moved: bool,
    provider_invalidated: Rc<Cell<bool>>,
    live_base_pending: bool,
    live_base_in_flight: Option<u64>,
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
        );
        let video_tx = media.sender();
        let mut extensions = HashMap::<_, Box<dyn Any>>::new();
        extensions.insert(
            shrimply_paint_edit_skia::PAINT_PREVIEW_STATE,
            Box::new(shrimply_paint_edit_skia::PaintPreviewState::default()),
        );
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
            audio_revision: 0,
            extensions,
            base_exclusion: None,
            presented_base_exclusion: None,
            overlay: None,
            retiring_overlay: None,
            provider_active: false,
            provider_cursor: Cursor::Default,
            guide_active: false,
            provider_origin: None,
            provider_moved: false,
            provider_invalidated,
            live_base_pending: false,
            live_base_in_flight: None,
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
        self.fullscreen = fullscreen;
        if !preferences_store::snapshot(&self.preferences).preview_zoom_pan_enabled {
            if self.navigation.active() {
                self.provider_cursor = Cursor::Default;
            }
            self.navigation.reset();
        }
        if self.provider_invalidated.replace(false) && self.provider_active {
            self.pointer_cancel();
            self.provider_cursor = Cursor::Default;
        }
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
                revision,
                excluded_item_id,
                ..
            }) if excluded_item_id == self.base_exclusion => {
                self.frame = Some(frame);
                self.presented_base_exclusion = excluded_item_id;
                self.retiring_overlay = None;
                if self
                    .live_base_in_flight
                    .is_some_and(|requested| revision >= requested)
                {
                    self.live_base_in_flight = None;
                }
                if !self.audio_analysis.same_frame(&audio_analysis) {
                    self.audio_analysis = audio_analysis;
                    self.audio_revision = self.audio_revision.wrapping_add(1);
                }
                if let Some(overlay) = self.overlay.as_mut() {
                    overlay.provider.on_base_frame_presented(revision);
                }
            }
            Some(VideoEvent::Clear {
                audio_analysis,
                revision,
                excluded_item_id,
                ..
            }) if excluded_item_id == self.base_exclusion => {
                self.frame = None;
                self.presented_base_exclusion = excluded_item_id;
                self.retiring_overlay = None;
                if self
                    .live_base_in_flight
                    .is_some_and(|requested| revision >= requested)
                {
                    self.live_base_in_flight = None;
                }
                if !self.audio_analysis.same_frame(&audio_analysis) {
                    self.audio_analysis = audio_analysis;
                    self.audio_revision = self.audio_revision.wrapping_add(1);
                }
                if let Some(overlay) = self.overlay.as_mut() {
                    overlay.provider.on_base_frame_presented(revision);
                }
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
        );
        self.update_base_exclusion(snapshot.position);
        let clip_rect = self.navigation.clip_rect(bounds, viewport_surface);
        let Self {
            renderer,
            overlay,
            retiring_overlay,
            expression_cache,
            extensions,
            presented_base_exclusion,
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
                    let current_covers_base = overlay.as_ref().is_some_and(|overlay| {
                        overlay.provider.base_frame_exclusion() == *presented_base_exclusion
                    });
                    if !current_covers_base
                        && let Some(retiring) = retiring_overlay.as_mut()
                        && retiring.provider.base_frame_exclusion() == *presented_base_exclusion
                    {
                        retiring.viewport = viewport;
                        retiring.draw(canvas, expression_cache, extensions);
                    }
                    if let Some(overlay) = overlay.as_mut()
                        && overlay.provider.base_frame_exclusion() == *presented_base_exclusion
                    {
                        overlay.draw(canvas, expression_cache, extensions);
                    }
                },
            )
    }

    fn update_base_exclusion(&mut self, position: Time) {
        let exclusion = self
            .overlay
            .as_ref()
            .and_then(|overlay| overlay.provider.base_frame_exclusion());
        if exclusion == self.base_exclusion {
            return;
        }
        self.base_exclusion = exclusion;
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
        if !self.live_base_pending || self.live_base_in_flight.is_some() {
            return;
        }
        self.live_base_pending = false;
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                video: true,
                live_preview: true,
                ..Default::default()
            },
        );
        let revision = player_state::snapshot(&self.player_state).revision;
        self.live_base_in_flight = Some(revision);
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.revision = revision;
        }
    }

    fn update_overlay(
        &mut self,
        project: &Project,
        preferences: &preferences_store::PreferencesSnapshot,
        position: Time,
        revision: u64,
        viewport: PreviewViewport,
    ) {
        if self.provider_active {
            return;
        }
        let Some(address) = selection_state::focused_video_address(&self.selection_state, project)
        else {
            self.replace_overlay(None);
            return;
        };
        let Some(item) = project.video_item(&address) else {
            self.replace_overlay(None);
            return;
        };
        let target = preview_focus::snapshot(&self.preview_focus)
            .filter(|focused| focused.item == address && item.owns_preview_target(focused.target))
            .map_or_else(|| item.default_preview_target(), |focused| focused.target);
        if self.overlay.as_ref().is_some_and(|overlay| {
            overlay.current(OverlayCacheKey {
                address: &address,
                target,
                revision,
                position,
                viewport,
                audio_revision: self.audio_revision,
                preferences,
                guides: &project.preview_guides,
            })
        }) {
            return;
        }
        let overlay = self.prepare_overlay(
            project,
            OverlayRequest {
                address,
                target,
                revision,
                position,
                viewport,
                snap_configuration: SnapConfiguration::capture(
                    preferences,
                    &project.preview_guides,
                ),
            },
        );
        self.replace_overlay(overlay);
    }

    fn replace_overlay(&mut self, overlay: Option<PreparedOverlay>) {
        let previous = std::mem::replace(&mut self.overlay, overlay);
        if let Some(previous) = previous
            && previous.provider.base_frame_exclusion().is_some()
            && !self.retiring_overlay.as_ref().is_some_and(|retiring| {
                retiring.provider.base_frame_exclusion() == self.presented_base_exclusion
            })
        {
            self.retiring_overlay = Some(previous);
        }
    }

    fn prepare_overlay(
        &self,
        project: &Project,
        request: OverlayRequest,
    ) -> Option<PreparedOverlay> {
        let item = project.video_item(&request.address)?;
        let mut prepared = prepare_geometry(
            project,
            &request.address,
            request.position,
            &self.audio_analysis,
            &self.expression_cache,
            request.viewport,
            Some(&self.extensions),
        )?;
        let snap_scene = request.snap_configuration.enabled.then(|| {
            prepare_snap_scene(
                project,
                &request.address,
                request.position,
                request.viewport,
                &mut prepared.source_sizes,
                SnapPreparation {
                    audio_analysis: &self.audio_analysis,
                    expression_cache: &self.expression_cache,
                    extensions: &self.extensions,
                    guides: request
                        .snap_configuration
                        .guides_visible
                        .then_some(project.preview_guides.as_ref()),
                    radius_px: request.snap_configuration.radius_px as f32,
                },
            )
        });
        let provider = item.preview_provider(
            request.target,
            &prepared
                .context(request.position, &self.expression_cache, request.viewport)
                .snapping(snap_scene.as_ref())
                .extensions(Some(&self.extensions)),
        )?;
        Some(PreparedOverlay {
            address: request.address,
            target: request.target,
            revision: request.revision,
            position: request.position,
            viewport: request.viewport,
            audio_revision: self.audio_revision,
            snap_configuration: request.snap_configuration,
            prepared,
            snap_scene,
            provider,
            deferred_refresh: PreviewRefresh::NONE,
        })
    }

    fn dispatch_provider(&mut self, event: PointerEvent<'_>) -> PreviewResponse {
        let cancel = matches!(event, PointerEvent::Cancel);
        let terminal = matches!(event, PointerEvent::End(_) | PointerEvent::Cancel);
        let Some(mut overlay) = self.overlay.take() else {
            return PreviewResponse::IGNORED;
        };
        let project_handle = self.project.clone();
        let mut response = {
            let mut project = project_handle.borrow_mut();
            let context = overlay
                .prepared
                .context(overlay.position, &self.expression_cache, overlay.viewport)
                .snapping(overlay.snap_scene.as_ref());
            overlay.provider.on_pointer(
                event,
                &context,
                &mut OverlayEdits {
                    project: &mut project,
                    extensions: &mut self.extensions,
                    address: &overlay.address,
                    keyframe_time: overlay.prepared.keyframe_time,
                    context,
                },
            )
        };
        if cancel {
            response.edit = response.edit.canceled();
        }
        if response.edit.changed() && !response.edit.commits() && !terminal {
            if response.edit.refresh.contains(PreviewRefresh::PREVIEW) {
                self.live_base_pending = true;
            }
            overlay.deferred_refresh |= response.edit.refresh;
            response.edit.refresh = PreviewRefresh::NONE;
        } else if response.edit.commits() || terminal {
            response.edit.refresh |= overlay.deferred_refresh;
            overlay.deferred_refresh = PreviewRefresh::NONE;
            self.live_base_pending = false;
        }
        self.overlay = Some(overlay);
        self.apply_provider_response(response);
        if response.edit.commits() {
            let revision = player_state::snapshot(&self.player_state).revision;
            if let Some(overlay) = self.overlay.as_mut() {
                overlay.provider.on_project_committed(revision);
                if overlay.provider.keeps_frame_until_base() {
                    overlay.revision = revision;
                }
            }
        }
        response
    }

    fn apply_provider_response(&mut self, response: PreviewResponse) {
        match response.cursor {
            CursorUpdate::Keep => {}
            CursorUpdate::Set(cursor) => self.provider_cursor = cursor,
            CursorUpdate::Clear => self.provider_cursor = Cursor::Default,
        }
        if response.edit.commits() {
            shrimply_project_document::project::commit_edit(
                &self.project.borrow(),
                "qt-preview-edit",
            );
        }
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
    }

    pub fn destroy(&mut self) {
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

    pub fn navigate(&mut self, size: glam::Vec2, event: PointerEvent<'_>) -> bool {
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
            self.provider_active || self.guide_active,
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
        if self.navigation.active() {
            self.navigate(
                glam::vec2(width, height),
                PointerEvent::Hover(pointer_input(pointer_sample(x, y), modifiers)),
            );
            return;
        }
        if self.provider_active {
            let sample = pointer_sample(x, y);
            self.provider_moved |= self.provider_origin != Some(sample);
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
        self.update_overlay(
            &project_handle.borrow(),
            &preferences,
            player.position,
            player.revision,
            viewport,
        );
        self.update_base_exclusion(player.position);
        self.provider_invalidated.set(false);
        self.live_base_pending = false;
        self.live_base_in_flight = None;
        let sample = pointer_sample(x, y);
        let response =
            self.dispatch_provider(PointerEvent::Begin(pointer_input(sample, modifiers)));
        self.provider_active = response.handled;
        self.provider_origin = response.handled.then_some(sample);
        self.provider_moved = false;
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
        }
        if self.guide_active {
            self.guide_active = false;
            return;
        }
        if self.provider_active {
            let sample = pointer_sample(x, y);
            self.provider_moved |= self.provider_origin != Some(sample);
            if self.provider_moved {
                self.dispatch_provider(PointerEvent::Samples {
                    input: pointer_input(sample, modifiers),
                    samples: std::slice::from_ref(&sample),
                });
            }
            self.provider_active = false;
            self.dispatch_provider(PointerEvent::End(pointer_input(sample, modifiers)));
            self.provider_cursor = Cursor::Default;
            self.provider_origin = None;
            self.provider_moved = false;
        }
    }

    pub fn pointer_cancel(&mut self) {
        self.navigation.cancel();
        self.provider_cursor = Cursor::Default;
        if self.guide_active {
            self.guide_input
                .pointer_cancel(&mut self.project.borrow_mut().preview_guides);
            self.guide_active = false;
        } else if self.provider_active {
            self.provider_active = false;
            self.dispatch_provider(PointerEvent::Cancel);
            self.provider_cursor = Cursor::Default;
            self.provider_origin = None;
            self.provider_moved = false;
        }
    }

    pub fn pointer_leave(&mut self) {
        self.guide_input.pointer_leave();
        if !self.provider_active {
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
