use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use glam::{IVec2, Vec2 as GlamVec2};
use gtk::prelude::*;
use gtk::{gdk, glib};
use shrimply_preview_core::{
    CursorUpdate, Key, KeyState, KeyboardEvent, Modifiers, PointerButton, PointerEvent,
    PointerInput, PointerSample, PointerTool, PreviewExtensionKey, PreviewRefresh, PreviewResponse,
    PreviewTarget,
};

use crate::player_state::{self, SharedPlayerState};
use crate::preferences::store as preferences_store;
use crate::preview_focus::{self, FocusedPreview, SharedPreviewFocus};
use crate::project::{ItemAddress, PreviewGuides, Project, Time};
use crate::selection_state::{self, SharedSelectionState};
use crate::timeline::renderer::{Color, Rect, vec2};
use crate::transform_eval::{FrameAudioAnalysis, TransformExpressionCache};
use crate::video::compositor::{CompositeAccuracy, VideoCommand, VideoCommandSender};
use crate::video::gpu::CompositedVideoFrame;

use shrimply_preview_runtime::captions::{self, CaptionAppearance, draw_captions};
#[path = "preview_surface/dispatch.rs"]
mod dispatch;
use dispatch::{
    apply_response, attach_frame_scheduler, caption_split_at_pointer, split_caption_at_pointer,
};
#[path = "preview_surface/cursor.rs"]
mod cursor;
use cursor::name as cursor_name;
#[path = "preview_surface/geometry.rs"]
mod geometry;
#[path = "preview_surface/gtk_guides.rs"]
mod gtk_guides;
use geometry::surface_viewport;
use shrimply_preview_runtime::controller::PreparedProvider;
use shrimply_preview_runtime::geometry::preview_viewport;
use shrimply_preview_runtime::guides;
use shrimply_preview_runtime::renderer::{Appearance, VideoRenderer};

#[derive(Clone)]
pub struct VideoSurface {
    area: gtk::GLArea,
    state: Rc<RefCell<VideoSurfaceState>>,
}

#[derive(Clone)]
pub struct PreviewController {
    surface: VideoSurface,
    controller: Rc<RefCell<PreviewControllerState>>,
}

impl std::ops::Deref for PreviewController {
    type Target = VideoSurface;

    fn deref(&self) -> &Self::Target {
        &self.surface
    }
}

struct VideoSurfaceState {
    renderer: Option<VideoRenderer>,
    frame: Option<CompositedVideoFrame>,
    audio_analysis: FrameAudioAnalysis,
    expression_cache: RefCell<TransformExpressionCache>,
    guides_visible: bool,
    snap_enabled: bool,
    snap_radius_px: u32,
    caption_bottom_inset: f32,
    caption_font_size: f32,
    caption_background_color: Color<u8>,
    caption_split_hover: Option<GlamVec2>,
    preview_padding_px: u32,
    preview_shadow_size_px: u32,
    preview_upsample_method: preferences_store::PreviewUpsampleMethod,
    preview_downsample_method: preferences_store::PreviewDownsampleMethod,
    fullscreen: bool,
}

struct PreviewControllerState {
    core: shrimply_preview_runtime::controller::Controller,
    video_tx: VideoCommandSender,
}

use shrimply_preview_runtime::controller::PointerSequence;

fn set_base_exclusion(
    controller: &mut PreviewControllerState,
    item_id: Option<uuid::Uuid>,
    position: Time,
) {
    if controller.core.base_exclusion == item_id {
        return;
    }
    controller.core.base_exclusion = item_id;
    for command in [
        VideoCommand::SetPreviewExclusion(item_id),
        VideoCommand::Render {
            position,
            accuracy: CompositeAccuracy::FULLY_ACCURATE,
        },
    ] {
        if let Err(error) = controller.video_tx.send(command) {
            tracing::error!(%error, "could not update the preview base frame");
        }
    }
}

impl PreviewController {
    pub fn new(
        project: Rc<RefCell<Project>>,
        player_state: SharedPlayerState,
        selection_state: SharedSelectionState,
        preview_focus: SharedPreviewFocus,
        preferences: preferences_store::SharedPreferences,
        video_tx: VideoCommandSender,
    ) -> Self {
        let area = gtk::GLArea::builder()
            .auto_render(false)
            .has_depth_buffer(false)
            .has_stencil_buffer(false)
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .width_request(640)
            .build();
        let preference = preferences_store::snapshot(&preferences);
        let state = Rc::new(RefCell::new(VideoSurfaceState {
            renderer: None,
            frame: None,
            audio_analysis: FrameAudioAnalysis::default(),
            expression_cache: RefCell::new(TransformExpressionCache::default()),
            guides_visible: preference.preview_guides_visible,
            snap_enabled: preference.timeline_magnet == "true",
            snap_radius_px: preference.timeline_snap_radius_px,
            caption_bottom_inset: 0.0,
            caption_font_size: preference.caption_font_size,
            caption_background_color: preference.caption_background_color,
            caption_split_hover: None,
            preview_padding_px: preference.preview_padding_px,
            preview_shadow_size_px: preference.preview_shadow_size_px,
            preview_upsample_method: preference.preview_upsample_method,
            preview_downsample_method: preference.preview_downsample_method,
            fullscreen: false,
        }));
        let controller = Rc::new(RefCell::new(PreviewControllerState {
            core: Default::default(),
            video_tx,
        }));

        attach_render(
            &area,
            project.clone(),
            player_state.clone(),
            selection_state.clone(),
            preview_focus.clone(),
            state.clone(),
            controller.clone(),
        );
        attach_input(
            &area,
            project.clone(),
            player_state.clone(),
            selection_state.clone(),
            preview_focus.clone(),
            state.clone(),
            controller.clone(),
        );
        attach_frame_scheduler(&area, player_state.clone(), controller.clone());

        let unrealize_state = state.clone();
        area.connect_unrealize(move |area| {
            area.make_current();
            if let Some(mut renderer) = unrealize_state.borrow_mut().renderer.take() {
                renderer.destroy();
            }
        });
        let selection_area = area.clone();
        let selection_project = project.clone();
        let selection_player = player_state.clone();
        let selection_surface = state.clone();
        let selection_controller = controller.clone();
        selection_state::connect_named(&selection_state, "preview provider selection", move || {
            let area = selection_area.clone();
            let project = selection_project.clone();
            let player = selection_player.clone();
            let surface = selection_surface.clone();
            let controller = selection_controller.clone();
            glib::idle_add_local_once(move || {
                cancel_provider(&area, &project, &player, &surface, &controller);
                area.queue_render();
            });
        });
        let focus_area = area.clone();
        let focus_project = project.clone();
        let focus_player = player_state.clone();
        let focus_surface = state.clone();
        let focus_controller = controller.clone();
        preview_focus::connect_named(&preview_focus, "preview provider focus", move || {
            cancel_provider(
                &focus_area,
                &focus_project,
                &focus_player,
                &focus_surface,
                &focus_controller,
            );
            focus_area.set_cursor_from_name(None);
            focus_area.queue_render();
        });
        let preference_area = area.clone();
        let preference_state = state.clone();
        let preference_controller = controller.clone();
        preferences_store::connect(&preferences, move |preference| {
            let mut state = preference_state.borrow_mut();
            state.caption_font_size = preference.caption_font_size;
            state.caption_background_color = preference.caption_background_color;
            state.preview_padding_px = preference.preview_padding_px;
            state.preview_shadow_size_px = preference.preview_shadow_size_px;
            state.preview_upsample_method = preference.preview_upsample_method;
            state.preview_downsample_method = preference.preview_downsample_method;
            state.guides_visible = preference.preview_guides_visible;
            state.snap_enabled = preference.timeline_magnet == "true";
            state.snap_radius_px = preference.timeline_snap_radius_px;
            drop(state);
            preference_controller.borrow_mut().core.context_invalidated = true;
            preference_area.queue_render();
        });
        let style = adw::StyleManager::for_display(&area.display());
        let dark_area = area.clone();
        style.connect_dark_notify(move |_| dark_area.queue_render());
        let contrast_area = area.clone();
        style.connect_high_contrast_notify(move |_| contrast_area.queue_render());

        Self {
            surface: VideoSurface { area, state },
            controller,
        }
    }

    pub(crate) fn preview_state<T: 'static, R>(
        &self,
        key: PreviewExtensionKey,
        read: impl FnOnce(&T) -> R,
    ) -> R {
        let controller = self.controller.borrow();
        let value = controller
            .core
            .extensions
            .get(&key)
            .and_then(|value| value.downcast_ref())
            .expect("preview state extension has the wrong type");
        read(value)
    }

    pub(crate) fn install_preview_state<T: 'static>(&self, key: PreviewExtensionKey, value: T) {
        assert!(
            self.controller
                .borrow_mut()
                .core
                .extensions
                .insert(key, Box::new(value))
                .is_none(),
            "preview state extension is already installed"
        );
    }

    pub(crate) fn update_preview_state<T: 'static, R>(
        &self,
        key: PreviewExtensionKey,
        update: impl FnOnce(&mut T) -> R,
    ) -> R {
        let result = {
            let mut controller = self.controller.borrow_mut();
            let value = controller
                .core
                .extensions
                .get_mut(&key)
                .and_then(|value| value.downcast_mut())
                .expect("preview state extension has the wrong type");
            update(value)
        };
        self.area.queue_render();
        result
    }

    pub fn set_frame(
        &self,
        frame: CompositedVideoFrame,
        audio_analysis: FrameAudioAnalysis,
        revision: u64,
        excluded_item_id: Option<uuid::Uuid>,
    ) {
        let mut controller = self.controller.borrow_mut();
        if !controller
            .core
            .accept_base_frame(revision, excluded_item_id)
        {
            return;
        }
        drop(controller);
        self.surface.set_frame(frame, audio_analysis);
    }

    pub fn clear_frame(
        &self,
        audio_analysis: FrameAudioAnalysis,
        revision: u64,
        excluded_item_id: Option<uuid::Uuid>,
    ) {
        let mut controller = self.controller.borrow_mut();
        if !controller
            .core
            .accept_base_frame(revision, excluded_item_id)
        {
            return;
        }
        drop(controller);
        self.surface.clear_frame(audio_analysis);
    }
}

impl VideoSurface {
    fn set_frame(&self, frame: CompositedVideoFrame, audio_analysis: FrameAudioAnalysis) {
        let mut state = self.state.borrow_mut();
        if state.frame.as_ref().map(|frame| frame.storage_key) != Some(frame.storage_key)
            || !state.audio_analysis.same_frame(&audio_analysis)
        {
            state.frame = Some(frame);
            state.audio_analysis = audio_analysis;
        }
        drop(state);
        self.area.queue_render();
    }

    fn clear_frame(&self, audio_analysis: FrameAudioAnalysis) {
        let mut state = self.state.borrow_mut();
        state.frame = None;
        state.audio_analysis = audio_analysis;
        drop(state);
        self.area.queue_render();
    }

    pub fn widget(&self) -> &gtk::GLArea {
        &self.area
    }

    pub fn has_frame(&self) -> bool {
        self.state.borrow().frame.is_some()
    }

    pub fn current_frame_texture(&self) -> Result<Option<gdk::Texture>, String> {
        let state = self.state.borrow();
        let Some(frame) = &state.frame else {
            return Ok(None);
        };
        frame
            .buffer
            .context()
            .synchronize()
            .map_err(|error| format!("could not synchronize preview frame: {error}"))?;
        let stream = frame.buffer.context().default_stream();
        let pixels = frame
            .buffer
            .to_host_vec(&stream)
            .map_err(|error| format!("could not copy preview frame: {error}"))?;
        let width = i32::try_from(frame.width).map_err(|_| "preview width is too large")?;
        let height = i32::try_from(frame.height).map_err(|_| "preview height is too large")?;
        let stride = frame.width as usize * std::mem::size_of::<u32>();
        let mut bytes = Vec::with_capacity(pixels.len() * std::mem::size_of::<u32>());
        for pixel in pixels {
            bytes.extend_from_slice(&pixel.to_le_bytes());
        }
        Ok(Some(
            gdk::MemoryTexture::new(
                width,
                height,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_owned(bytes),
                stride,
            )
            .upcast(),
        ))
    }

    pub fn set_caption_bottom_inset(&self, inset: f32) {
        self.state.borrow_mut().caption_bottom_inset = inset.max(0.0);
        self.area.queue_render();
    }

    pub fn set_fullscreen(&self, fullscreen: bool) {
        self.state.borrow_mut().fullscreen = fullscreen;
        self.area.queue_render();
    }

    pub fn queue_render(&self) {
        self.area.queue_render();
    }
}

#[derive(Clone, Copy)]
struct Preparation<'a> {
    surface: IVec2,
    project_revision: u64,
    padding_px: u32,
    audio_analysis: &'a FrameAudioAnalysis,
    expression_cache: &'a RefCell<TransformExpressionCache>,
    extensions: &'a HashMap<PreviewExtensionKey, Box<dyn Any>>,
    snap_enabled: bool,
    snap_radius_px: f32,
    guides: Option<&'a PreviewGuides>,
}

fn prepare_current(
    project: &Project,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    preview_focus: &SharedPreviewFocus,
    preparation: Preparation<'_>,
) -> Option<PreparedProvider> {
    let item = selection_state::focused_video_address(selection_state, project)?;
    prepare(
        project,
        &item,
        preview_focus::snapshot(preview_focus).as_ref(),
        player_state::snapshot(player_state).position,
        preparation,
    )
}

fn prepare(
    project: &Project,
    key: &ItemAddress,
    focused: Option<&FocusedPreview>,
    position: Time,
    preparation: Preparation<'_>,
) -> Option<PreparedProvider> {
    let item = project.video_item(key)?;
    let target = focused
        .filter(|focused| &focused.item == key && item.owns_preview_target(focused.target))
        .map_or_else(|| item.default_preview_target(), |focused| focused.target);
    prepare_target(project, key, target, position, preparation)
}

fn prepare_target(
    project: &Project,
    key: &ItemAddress,
    target: PreviewTarget,
    position: Time,
    preparation: Preparation<'_>,
) -> Option<PreparedProvider> {
    shrimply_preview_runtime::controller::prepare_target(
        project,
        key,
        target,
        position,
        shrimply_preview_runtime::controller::Preparation {
            project_revision: preparation.project_revision,
            viewport: preview_viewport(
                preparation.surface,
                project.canvas_size,
                preparation.padding_px,
            ),
            audio_analysis: preparation.audio_analysis,
            expression_cache: preparation.expression_cache,
            snap_enabled: preparation.snap_enabled,
            snap_radius_px: preparation.snap_radius_px,
            guides: preparation.guides,
            camera_sampler: shrimply_preview_runtime::provider::sample_camera,
        },
        preparation.extensions,
    )
}

fn attach_render(
    area: &gtk::GLArea,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    preview_focus: SharedPreviewFocus,
    state: Rc<RefCell<VideoSurfaceState>>,
    controller: Rc<RefCell<PreviewControllerState>>,
) {
    area.connect_render(move |area, _| {
        if let Some(error) = area.error() {
            tracing::error!("Video GLArea error: {error}");
            return glib::Propagation::Stop;
        }
        let surface_scale = area.scale_factor().max(1) as f32;
        let surface = IVec2::new(
            (area.width().max(1) as f32 * surface_scale) as i32,
            (area.height().max(1) as f32 * surface_scale) as i32,
        );
        let project = project.borrow();
        let player = player_state::snapshot(&player_state);
        let position = player.position;
        let focused_video = selection_state::focused_video_address(&selection_state, &project);
        let focused_caption = selection_state::focused_item_address(&selection_state, &project)
            .filter(|address| project.caption_item(address).is_some());
        let focused_preview = preview_focus::snapshot(&preview_focus);
        let mut state = state.borrow_mut();
        let mut controller = controller.borrow_mut();
        if state.renderer.is_none() {
            match VideoRenderer::new() {
                Ok(renderer) => state.renderer = Some(renderer),
                Err(error) => {
                    tracing::error!("Could not initialize preview renderer: {error}");
                    return glib::Propagation::Stop;
                }
            }
        }
        let padding_px = state.padding_px();
        let viewport = guides::viewport(
            surface,
            project.canvas_size,
            state.preview_padding_px,
            state.guides_visible,
            state.fullscreen,
        );
        let content_rect = viewport.content_rect;
        let stale_provider = controller.core.context_invalidated
            || controller.core.provider.as_ref().is_some_and(|prepared| {
                prepared.project_revision != player.revision
                    || prepared.context.timeline_position != position
                    || prepared.context.viewport != viewport
            });
        if stale_provider && controller.core.sequence == PointerSequence::Idle {
            controller.core.provider = None;
            controller.core.context_invalidated = false;
        }
        let background_color = shrimply_preview_runtime::background_color(
            geometry::theme_window_color(area),
            state.fullscreen,
        );
        let selection_color = Color::BLUE5;
        let guides = state
            .guides_visible
            .then_some(project.preview_guides.as_ref());
        let caption_bottom_inset = state.caption_bottom_inset;
        let caption_font_size = state.caption_font_size;
        let caption_background_color = state.caption_background_color;
        let caption_split_hover = state.caption_split_hover;
        let shadow_size_px = state.preview_shadow_size_px;
        let upsample_method = state.preview_upsample_method;
        let downsample_method = state.preview_downsample_method;
        let snap_enabled = state.snap_enabled;
        let snap_radius_px = state.snap_radius_px as f32;
        let VideoSurfaceState {
            renderer,
            frame,
            audio_analysis,
            expression_cache,
            ..
        } = &mut *state;
        let core = &mut controller.core;
        let result = renderer
            .as_mut()
            .expect("preview renderer was initialized")
            .render(
                surface,
                surface_scale,
                frame.as_ref(),
                Appearance {
                    content_rect,
                    shadow_size_px,
                    background_color,
                    upsample_method,
                    downsample_method,
                },
                |timeline_painter| {
                    let surface_rect = Rect::from_min_size(
                        vec2(0.0, 0.0),
                        vec2(surface.x as f32, surface.y as f32),
                    );
                    draw_captions(
                        timeline_painter,
                        &project,
                        position,
                        CaptionAppearance {
                            preview_rect: surface_rect,
                            font_size: caption_font_size,
                            background_color: caption_background_color,
                            bottom_inset: caption_bottom_inset,
                        },
                        focused_caption
                            .as_ref()
                            .zip(caption_split_hover)
                            .map(|(address, point)| (address, point, selection_color)),
                    );
                    if let Some(guides) = &guides {
                        guides::draw(
                            timeline_painter,
                            guides,
                            viewport,
                            surface_rect,
                            selection_color,
                        );
                    }
                    if core.provider.is_none()
                        && let Some(key) = focused_video.as_ref()
                    {
                        core.provider = prepare(
                            &project,
                            key,
                            focused_preview.as_ref(),
                            position,
                            Preparation {
                                surface,
                                project_revision: player.revision,
                                padding_px,
                                audio_analysis,
                                expression_cache,
                                extensions: &core.extensions,
                                snap_enabled,
                                snap_radius_px,
                                guides,
                            },
                        );
                    }
                    core.draw(timeline_painter.canvas(), expression_cache);
                },
            );
        let exclusion = core
            .provider
            .as_ref()
            .and_then(|prepared| prepared.provider.base_frame_exclusion());
        set_base_exclusion(&mut controller, exclusion, position);
        if let Err(error) = result {
            tracing::error!("Could not render video preview: {error}");
        }
        glib::Propagation::Stop
    });
}

struct ProviderDispatch<'a> {
    area: &'a gtk::GLArea,
    project: &'a Rc<RefCell<Project>>,
    player_state: &'a SharedPlayerState,
    selection_state: &'a SharedSelectionState,
    preview_focus: &'a SharedPreviewFocus,
    state: &'a Rc<RefCell<VideoSurfaceState>>,
    controller: &'a Rc<RefCell<PreviewControllerState>>,
}

fn ensure_provider(surface: &ProviderDispatch<'_>) -> bool {
    let player = player_state::snapshot(surface.player_state);
    {
        let project = surface.project.borrow();
        let state = surface.state.borrow();
        let viewport = geometry::surface_viewport(surface.area, &project, &state);
        let mut controller = surface.controller.borrow_mut();
        let stale = controller.core.context_invalidated
            || controller.core.provider.as_ref().is_some_and(|prepared| {
                prepared.project_revision != player.revision
                    || prepared.context.timeline_position != player.position
                    || prepared.context.viewport != viewport
            });
        if stale && controller.core.sequence == PointerSequence::Idle {
            controller.core.provider = None;
            controller.core.context_invalidated = false;
        }
        if controller.core.provider.is_some() {
            return true;
        }
    }
    let prepared = {
        let project = surface.project.borrow();
        let state = surface.state.borrow();
        let controller = surface.controller.borrow();
        prepare_current(
            &project,
            surface.player_state,
            surface.selection_state,
            surface.preview_focus,
            Preparation {
                surface: IVec2::new(surface.area.width().max(1), surface.area.height().max(1)),
                project_revision: player.revision,
                padding_px: state.padding_px(),
                audio_analysis: &state.audio_analysis,
                expression_cache: &state.expression_cache,
                extensions: &controller.core.extensions,
                snap_enabled: state.snap_enabled,
                snap_radius_px: state.snap_radius_px as f32,
                guides: state
                    .guides_visible
                    .then_some(project.preview_guides.as_ref()),
            },
        )
    };
    let exclusion = prepared
        .as_ref()
        .and_then(|prepared| prepared.provider.base_frame_exclusion());
    let mut controller = surface.controller.borrow_mut();
    controller.core.provider = prepared;
    set_base_exclusion(&mut controller, exclusion, player.position);
    drop(controller);
    surface.controller.borrow().core.provider.is_some()
}

fn dispatch_pointer(surface: ProviderDispatch<'_>, event: PointerEvent<'_>) -> PreviewResponse {
    if !surface.controller.borrow_mut().core.accepts_pointer(event) {
        return PreviewResponse::IGNORED;
    }
    ensure_provider(&surface);
    let response = {
        let mut project = surface.project.borrow_mut();
        let state = surface.state.borrow();
        surface
            .controller
            .borrow_mut()
            .core
            .pointer(&mut project, &state.expression_cache, event)
    };
    apply_response(
        surface.area,
        surface.project,
        surface.player_state,
        response,
        "preview-provider",
    );
    if response.edit.commits() {
        surface
            .controller
            .borrow_mut()
            .core
            .project_committed(player_state::snapshot(surface.player_state).revision);
    }
    response
}

fn dispatch_keyboard(surface: ProviderDispatch<'_>, event: KeyboardEvent) -> PreviewResponse {
    if !ensure_provider(&surface) {
        return PreviewResponse::IGNORED;
    }
    let response = {
        let mut project = surface.project.borrow_mut();
        let state = surface.state.borrow();
        surface
            .controller
            .borrow_mut()
            .core
            .keyboard(&mut project, &state.expression_cache, event)
    };
    apply_response(
        surface.area,
        surface.project,
        surface.player_state,
        response,
        "preview-keyboard",
    );
    if response.edit.commits() {
        surface
            .controller
            .borrow_mut()
            .core
            .project_committed(player_state::snapshot(surface.player_state).revision);
    }
    response
}

fn cancel_provider(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    state: &Rc<RefCell<VideoSurfaceState>>,
    controller: &Rc<RefCell<PreviewControllerState>>,
) {
    let response = {
        let mut project = project.borrow_mut();
        let state = state.borrow();
        controller
            .borrow_mut()
            .core
            .cancel(&mut project, &state.expression_cache)
    };
    apply_response(area, project, player_state, response, "preview-cancel");
    set_base_exclusion(
        &mut controller.borrow_mut(),
        None,
        player_state::snapshot(player_state).position,
    );
}

fn attach_input(
    area: &gtk::GLArea,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    preview_focus: SharedPreviewFocus,
    state: Rc<RefCell<VideoSurfaceState>>,
    controller: Rc<RefCell<PreviewControllerState>>,
) {
    let sequence_button = Rc::new(Cell::new(0));
    let sequence_origin = Rc::new(Cell::new(None::<PointerSample>));
    let sequence_moved = Rc::new(Cell::new(false));
    let guide_input = Rc::new(RefCell::new(guides::GuideInput::default()));
    let ruler_hover = Cell::new(false);
    let caption_hover = Cell::new(false);

    let motion = gtk::EventControllerMotion::new();
    let motion_area = area.clone();
    let motion_project = project.clone();
    let motion_player = player_state.clone();
    let motion_selection = selection_state.clone();
    let motion_focus = preview_focus.clone();
    let motion_state = state.clone();
    let motion_controller = controller.clone();
    let motion_guides = guide_input.clone();
    motion.connect_motion(move |source, x, y| {
        let position = GlamVec2::new(x as f32, y as f32);
        if motion_controller.borrow().core.sequence != PointerSequence::Idle {
            if motion_state
                .borrow_mut()
                .caption_split_hover
                .take()
                .is_some()
            {
                motion_area.queue_render();
            }
            return;
        }
        let guide_cursor = gtk_guides::move_to(
            &motion_guides,
            &motion_area,
            &motion_project,
            &motion_state,
            position,
        );
        if guide_cursor != guides::GuideCursor::Default {
            if motion_state
                .borrow_mut()
                .caption_split_hover
                .take()
                .is_some()
            {
                motion_area.queue_render();
            }
            caption_hover.set(false);
            ruler_hover.set(true);
            motion_area.set_cursor_from_name(guide_cursor.name());
            return;
        }
        if ruler_hover.replace(false) {
            motion_area.set_cursor_from_name(None);
        }
        let split_active = caption_split_at_pointer(
            &motion_area,
            &motion_project,
            &motion_player,
            &motion_selection,
            &motion_state,
            position,
        )
        .is_some();
        let hover = split_active.then_some(position);
        if motion_state.borrow().caption_split_hover != hover {
            motion_state.borrow_mut().caption_split_hover = hover;
            motion_area.queue_render();
        }
        if split_active {
            caption_hover.set(true);
            motion_area.set_cursor_from_name(Some("text"));
            return;
        }
        if caption_hover.replace(false) {
            motion_area.set_cursor_from_name(None);
        }
        dispatch_pointer(
            ProviderDispatch {
                area: &motion_area,
                project: &motion_project,
                player_state: &motion_player,
                selection_state: &motion_selection,
                preview_focus: &motion_focus,
                state: &motion_state,
                controller: &motion_controller,
            },
            PointerEvent::Hover(pointer_input(source, position, 0)),
        );
    });
    let leave_area = area.clone();
    let leave_project = project.clone();
    let leave_player = player_state.clone();
    let leave_selection = selection_state.clone();
    let leave_focus = preview_focus.clone();
    let leave_state = state.clone();
    let leave_controller = controller.clone();
    let leave_guides = guide_input.clone();
    motion.connect_leave(move |_| {
        if leave_state
            .borrow_mut()
            .caption_split_hover
            .take()
            .is_some()
        {
            leave_area.queue_render();
        }
        if leave_controller.borrow().core.sequence != PointerSequence::Idle {
            return;
        }
        leave_guides.borrow_mut().pointer_leave();
        dispatch_pointer(
            ProviderDispatch {
                area: &leave_area,
                project: &leave_project,
                player_state: &leave_player,
                selection_state: &leave_selection,
                preview_focus: &leave_focus,
                state: &leave_state,
                controller: &leave_controller,
            },
            PointerEvent::Leave,
        );
        leave_area.set_cursor_from_name(None);
    });
    area.add_controller(motion);

    let pointer = gtk::GestureStylus::new();
    pointer.set_button(0);
    pointer.set_stylus_only(false);
    let begin_area = area.clone();
    let begin_project = project.clone();
    let begin_player = player_state.clone();
    let begin_selection = selection_state.clone();
    let begin_focus = preview_focus.clone();
    let begin_state = state.clone();
    let begin_controller = controller.clone();
    let begin_button = sequence_button.clone();
    let begin_origin = sequence_origin.clone();
    let begin_moved = sequence_moved.clone();
    let begin_guides = guide_input.clone();
    pointer.connect_down(move |source, x, y| {
        begin_area.grab_focus();
        let button = source.current_button();
        begin_button.set(button);
        let input = pointer_input(source, GlamVec2::new(x as f32, y as f32), button);
        begin_origin.set(Some(input.sample));
        begin_moved.set(false);
        if button == 1 {
            let began_guide = gtk_guides::press(
                &begin_guides,
                &begin_area,
                &begin_project,
                &begin_state,
                input.sample.position,
            );
            if began_guide {
                begin_controller.borrow_mut().core.sequence = PointerSequence::Guide;
                begin_area.set_cursor_from_name(begin_guides.borrow().cursor().name());
                begin_area.queue_render();
                return;
            }
        }
        if button == 1
            && split_caption_at_pointer(
                &begin_area,
                &begin_project,
                &begin_player,
                &begin_selection,
                &begin_state,
                input.sample.position,
            )
        {
            return;
        }
        dispatch_pointer(
            ProviderDispatch {
                area: &begin_area,
                project: &begin_project,
                player_state: &begin_player,
                selection_state: &begin_selection,
                preview_focus: &begin_focus,
                state: &begin_state,
                controller: &begin_controller,
            },
            PointerEvent::Begin(input),
        );
    });
    let update_area = area.clone();
    let update_project = project.clone();
    let update_player = player_state.clone();
    let update_selection = selection_state.clone();
    let update_focus = preview_focus.clone();
    let update_state = state.clone();
    let update_controller = controller.clone();
    let update_button = sequence_button.clone();
    let update_origin = sequence_origin.clone();
    let update_moved = sequence_moved.clone();
    let update_guides = guide_input.clone();
    pointer.connect_motion(move |source, x, y| {
        let input = pointer_input(
            source,
            GlamVec2::new(x as f32, y as f32),
            update_button.get(),
        );
        let mut samples = pointer_backlog(source);
        if samples.last().copied() != Some(input.sample) {
            samples.push(input.sample);
        }
        let changed = update_origin.get().is_some_and(|origin| {
            samples
                .iter()
                .any(|sample| pointer_sample_changed(*sample, origin))
        });
        if !changed {
            return;
        }
        update_moved.set(true);
        if update_guides.borrow().active() {
            let cursor = gtk_guides::move_to(
                &update_guides,
                &update_area,
                &update_project,
                &update_state,
                input.sample.position,
            );
            update_area.set_cursor_from_name(cursor.name());
            update_area.queue_render();
            return;
        }
        dispatch_pointer(
            ProviderDispatch {
                area: &update_area,
                project: &update_project,
                player_state: &update_player,
                selection_state: &update_selection,
                preview_focus: &update_focus,
                state: &update_state,
                controller: &update_controller,
            },
            PointerEvent::Samples {
                input,
                samples: &samples,
            },
        );
    });
    let end_area = area.clone();
    let end_project = project.clone();
    let end_player = player_state.clone();
    let end_selection = selection_state.clone();
    let end_focus = preview_focus.clone();
    let end_state = state.clone();
    let end_controller = controller.clone();
    let end_button = sequence_button.clone();
    let end_origin = sequence_origin.clone();
    let end_moved = sequence_moved.clone();
    let end_guides = guide_input.clone();
    pointer.connect_up(move |source, x, y| {
        let input = pointer_input(source, GlamVec2::new(x as f32, y as f32), end_button.get());
        let mut samples = pointer_backlog(source);
        if samples.last().copied() != Some(input.sample) {
            samples.push(input.sample);
        }
        if end_origin.get().is_some_and(|origin| {
            samples
                .iter()
                .any(|sample| pointer_sample_changed(*sample, origin))
        }) {
            end_moved.set(true);
        }
        if end_guides.borrow().active() {
            gtk_guides::finish(
                &end_guides,
                &end_area,
                &end_project,
                &end_state,
                &end_controller,
                input.sample.position,
            );
            end_button.set(0);
            end_origin.set(None);
            end_moved.set(false);
            return;
        }
        if end_moved.get() {
            dispatch_pointer(
                ProviderDispatch {
                    area: &end_area,
                    project: &end_project,
                    player_state: &end_player,
                    selection_state: &end_selection,
                    preview_focus: &end_focus,
                    state: &end_state,
                    controller: &end_controller,
                },
                PointerEvent::Samples {
                    input,
                    samples: &samples,
                },
            );
        }
        dispatch_pointer(
            ProviderDispatch {
                area: &end_area,
                project: &end_project,
                player_state: &end_player,
                selection_state: &end_selection,
                preview_focus: &end_focus,
                state: &end_state,
                controller: &end_controller,
            },
            PointerEvent::End(input),
        );
        end_button.set(0);
        end_origin.set(None);
        end_moved.set(false);
        end_area.set_cursor_from_name(None);
    });
    let cancel_area = area.clone();
    let cancel_project = project.clone();
    let cancel_player = player_state.clone();
    let cancel_selection = selection_state.clone();
    let cancel_focus = preview_focus.clone();
    let cancel_state = state.clone();
    let cancel_controller = controller.clone();
    let cancel_button = sequence_button.clone();
    let cancel_origin = sequence_origin.clone();
    let cancel_moved = sequence_moved.clone();
    let cancel_guides = guide_input;
    pointer.connect_cancel(move |_, _| {
        if gtk_guides::cancel(
            &cancel_guides,
            &cancel_area,
            &cancel_project,
            &cancel_controller,
        ) {
            cancel_button.set(0);
            cancel_origin.set(None);
            cancel_moved.set(false);
            return;
        }
        dispatch_pointer(
            ProviderDispatch {
                area: &cancel_area,
                project: &cancel_project,
                player_state: &cancel_player,
                selection_state: &cancel_selection,
                preview_focus: &cancel_focus,
                state: &cancel_state,
                controller: &cancel_controller,
            },
            PointerEvent::Cancel,
        );
        cancel_button.set(0);
        cancel_origin.set(None);
        cancel_moved.set(false);
        cancel_area.set_cursor_from_name(None);
    });
    area.add_controller(pointer);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    let scroll_area = area.clone();
    let scroll_project = project.clone();
    let scroll_player = player_state.clone();
    let scroll_selection = selection_state.clone();
    let scroll_focus = preview_focus.clone();
    let scroll_state = state.clone();
    let scroll_controller = controller.clone();
    scroll.connect_scroll(move |source, dx, dy| {
        let position = source
            .current_event()
            .and_then(|event| event.position())
            .map_or_else(
                || {
                    GlamVec2::new(
                        scroll_area.width() as f32 * 0.5,
                        scroll_area.height() as f32 * 0.5,
                    )
                },
                |(x, y)| GlamVec2::new(x as f32, y as f32),
            );
        let response = dispatch_pointer(
            ProviderDispatch {
                area: &scroll_area,
                project: &scroll_project,
                player_state: &scroll_player,
                selection_state: &scroll_selection,
                preview_focus: &scroll_focus,
                state: &scroll_state,
                controller: &scroll_controller,
            },
            PointerEvent::Scroll {
                input: pointer_input(source, position, 0),
                delta: GlamVec2::new(dx as f32, dy as f32),
            },
        );
        if response.handled {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    area.add_controller(scroll);

    let key = gtk::EventControllerKey::new();
    let key_area = area.clone();
    let key_project = project.clone();
    let key_player = player_state.clone();
    let key_selection = selection_state.clone();
    let key_focus = preview_focus.clone();
    let key_state = state.clone();
    let key_controller = controller.clone();
    key.connect_key_pressed(move |_, key, _, modifiers| {
        let response = dispatch_keyboard(
            ProviderDispatch {
                area: &key_area,
                project: &key_project,
                player_state: &key_player,
                selection_state: &key_selection,
                preview_focus: &key_focus,
                state: &key_state,
                controller: &key_controller,
            },
            keyboard_event(key, KeyState::Pressed, modifiers),
        );
        if response.handled {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    let release_area = area.clone();
    let release_project = project;
    let release_player = player_state;
    let release_selection = selection_state;
    let release_focus = preview_focus;
    let release_state = state;
    let release_controller = controller;
    key.connect_key_released(move |_, key, _, modifiers| {
        dispatch_keyboard(
            ProviderDispatch {
                area: &release_area,
                project: &release_project,
                player_state: &release_player,
                selection_state: &release_selection,
                preview_focus: &release_focus,
                state: &release_state,
                controller: &release_controller,
            },
            keyboard_event(key, KeyState::Released, modifiers),
        );
    });
    area.add_controller(key);
}

fn pointer_input(
    controller: &impl IsA<gtk::EventController>,
    position: GlamVec2,
    button: u32,
) -> PointerInput {
    let event = controller.current_event();
    let time_millis = event.as_ref().map_or(0, |event| event.time());
    let tool = event
        .as_ref()
        .and_then(|event| event.device_tool())
        .map_or_else(
            || match event
                .as_ref()
                .and_then(|event| event.device())
                .map(|device| device.source())
            {
                Some(gdk::InputSource::Pen) => PointerTool::Pen,
                Some(gdk::InputSource::Touchscreen) => PointerTool::Touch,
                _ => PointerTool::Mouse,
            },
            |tool| match tool.tool_type() {
                gdk::DeviceToolType::Eraser => PointerTool::Eraser,
                gdk::DeviceToolType::Mouse => PointerTool::Mouse,
                _ => PointerTool::Pen,
            },
        );
    let pressure = event
        .as_ref()
        .and_then(|event| event.axis(gdk::AxisUse::Pressure))
        .map(|value| value as f32);
    let tilt = event.as_ref().and_then(|event| {
        Some(GlamVec2::new(
            event.axis(gdk::AxisUse::Xtilt)? as f32,
            event.axis(gdk::AxisUse::Ytilt)? as f32,
        ))
    });
    PointerInput {
        sample: PointerSample {
            position,
            pressure,
            tilt,
            time_millis,
        },
        tool,
        button: match button {
            1 => PointerButton::Primary,
            2 => PointerButton::Middle,
            3 => PointerButton::Secondary,
            button => PointerButton::Other(button),
        },
        modifiers: preview_modifiers(controller.current_event_state()),
    }
}

fn pointer_backlog(pointer: &gtk::GestureStylus) -> Vec<PointerSample> {
    const X: usize = gdk::ffi::GDK_AXIS_X as usize;
    const Y: usize = gdk::ffi::GDK_AXIS_Y as usize;
    const PRESSURE: usize = gdk::ffi::GDK_AXIS_PRESSURE as usize;
    const X_TILT: usize = gdk::ffi::GDK_AXIS_XTILT as usize;
    const Y_TILT: usize = gdk::ffi::GDK_AXIS_YTILT as usize;

    pointer
        .backlog()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|coord| {
            let flags = coord.flags();
            if !flags.contains(gdk::AxisFlags::X | gdk::AxisFlags::Y) {
                return None;
            }
            let axes = coord.axes();
            Some(PointerSample {
                position: GlamVec2::new(axes[X] as f32, axes[Y] as f32),
                pressure: flags
                    .contains(gdk::AxisFlags::PRESSURE)
                    .then(|| axes[PRESSURE] as f32),
                tilt: flags
                    .contains(gdk::AxisFlags::XTILT | gdk::AxisFlags::YTILT)
                    .then(|| GlamVec2::new(axes[X_TILT] as f32, axes[Y_TILT] as f32)),
                time_millis: coord.time(),
            })
        })
        .collect()
}

fn pointer_sample_changed(sample: PointerSample, previous: PointerSample) -> bool {
    sample.position != previous.position
        || sample.pressure != previous.pressure
        || sample.tilt != previous.tilt
}

fn keyboard_event(key: gdk::Key, state: KeyState, modifiers: gdk::ModifierType) -> KeyboardEvent {
    KeyboardEvent {
        key: match key {
            gdk::Key::BackSpace => Key::Backspace,
            gdk::Key::Delete | gdk::Key::KP_Delete => Key::Delete,
            gdk::Key::Escape => Key::Escape,
            gdk::Key::Return | gdk::Key::KP_Enter => Key::Enter,
            gdk::Key::space => Key::Space,
            gdk::Key::Tab => Key::Tab,
            gdk::Key::Control_L | gdk::Key::Control_R => Key::Control,
            gdk::Key::Shift_L | gdk::Key::Shift_R => Key::Shift,
            gdk::Key::Alt_L | gdk::Key::Alt_R => Key::Alt,
            key => key.to_unicode().map_or(Key::Unknown, Key::Character),
        },
        state,
        repeat: false,
        modifiers: preview_modifiers(modifiers),
    }
}

fn preview_modifiers(modifiers: gdk::ModifierType) -> Modifiers {
    let mut result = Modifiers::NONE;
    if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
        result |= Modifiers::SHIFT;
    }
    if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
        result |= Modifiers::CONTROL;
    }
    if modifiers.contains(gdk::ModifierType::ALT_MASK) {
        result |= Modifiers::ALT;
    }
    if modifiers.contains(gdk::ModifierType::SUPER_MASK) {
        result |= Modifiers::META;
    }
    result
}
