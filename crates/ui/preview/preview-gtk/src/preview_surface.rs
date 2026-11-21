use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use glam::{IVec2, Vec2 as GlamVec2};
use gtk::prelude::*;
use gtk::{gdk, glib};
use shrimply_preview_provider_skia::{
    CursorUpdate, Key, KeyState, KeyboardEvent, Modifiers, PointerButton, PointerEvent,
    PointerInput, PointerSample, PointerTool, PreviewExtensionKey, PreviewRefresh, PreviewResponse,
};
use shrimply_preview_runtime_cuda::captions::{self, CaptionAppearance};
use shrimply_preview_runtime_cuda::renderer::{Appearance, VideoRenderer};
use shrimply_preview_runtime_cuda::surface::{Clock, Configuration, Surface};

use crate::player_state::{self, SharedPlayerState};
use crate::preferences::store as preferences_store;
use crate::preview_focus::{self, SharedPreviewFocus};
use crate::project::{ItemAddress, Project, Time};
use crate::selection_state::{self, SharedSelectionState};
use crate::timeline::renderer::{Color, Rect, vec2};
use crate::transform_eval::FrameAudioAnalysis;
use crate::video::compositor::{CompositeAccuracy, VideoCommand, VideoCommandSender};
use crate::video::gpu::CompositedVideoFrame;

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
#[path = "preview_surface/sanim_inspector.rs"]
mod sanim_inspector;
#[path = "preview_surface/touch.rs"]
mod touch;

#[derive(Clone)]
pub struct VideoSurface {
    area: gtk::GLArea,
    state: Rc<RefCell<VideoSurfaceState>>,
}

#[derive(Clone)]
pub struct PreviewController {
    surface: VideoSurface,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    preview_focus: SharedPreviewFocus,
}

impl std::ops::Deref for PreviewController {
    type Target = VideoSurface;

    fn deref(&self) -> &Self::Target {
        &self.surface
    }
}

struct VideoSurfaceState {
    sanim_inspector: sanim_inspector::Inspector,
    presentation: Option<Presentation>,
    renderer: Option<VideoRenderer>,
    frame: Option<CompositedVideoFrame>,
    surface: Surface,
    video_tx: VideoCommandSender,
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
    zoom_pan_enabled: bool,
}

impl VideoSurfaceState {
    fn configuration(&self, area: &gtk::GLArea) -> Configuration {
        Configuration {
            size: glam::vec2(area.width().max(1) as f32, area.height().max(1) as f32),
            padding: self.preview_padding_px,
            guides_visible: self.guides_visible,
            fullscreen: self.fullscreen,
            snap_enabled: self.snap_enabled,
            snap_radius: self.snap_radius_px as f32,
            navigation_enabled: self.zoom_pan_enabled,
        }
    }

    fn sync_focus(&mut self, preview_focus: &SharedPreviewFocus) {
        self.surface.focus(
            preview_focus::snapshot(preview_focus).map(|focused| (focused.item, focused.target)),
        );
    }

    fn send_exclusion_update(&mut self, position: Time) {
        let Some(item_id) = self.surface.exclusion_update() else {
            return;
        };
        for command in [
            VideoCommand::SetPreviewExclusion(item_id),
            VideoCommand::Render {
                position,
                accuracy: CompositeAccuracy::FULLY_ACCURATE,
            },
        ] {
            if let Err(error) = self.video_tx.send(command) {
                tracing::error!(%error, "could not update the preview base frame");
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Presentation {
    position: Time,
    revision: u64,
    settled: bool,
}

impl PreviewController {
    /// Widget-owned callbacks retain the shared state without retaining their widget.
    pub(crate) fn weak(&self) -> impl Fn() -> Option<Self> + 'static + use<> {
        let area = self.widget().downgrade();
        let state = self.surface.state.clone();
        let project = self.project.clone();
        let player_state = self.player_state.clone();
        let selection_state = self.selection_state.clone();
        let preview_focus = self.preview_focus.clone();
        move || {
            Some(Self {
                surface: VideoSurface {
                    area: area.upgrade()?,
                    state: state.clone(),
                },
                project: project.clone(),
                player_state: player_state.clone(),
                selection_state: selection_state.clone(),
                preview_focus: preview_focus.clone(),
            })
        }
    }

    pub fn new(
        project: Rc<RefCell<Project>>,
        player_state: SharedPlayerState,
        selection_state: SharedSelectionState,
        preview_focus: SharedPreviewFocus,
        preferences: preferences_store::SharedPreferences,
        video_tx: VideoCommandSender,
    ) -> Self {
        let area = gtk::GLArea::builder()
            .allowed_apis(gdk::GLAPI::GL)
            .auto_render(false)
            .has_depth_buffer(false)
            .has_stencil_buffer(false)
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .build();
        area.set_required_version(3, 3);
        let preference = preferences_store::snapshot(&preferences);
        let state = Rc::new(RefCell::new(VideoSurfaceState {
            sanim_inspector: Default::default(),
            presentation: None,
            renderer: None,
            frame: None,
            surface: Surface::default(),
            video_tx,
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
            zoom_pan_enabled: preference.preview_zoom_pan_enabled,
        }));

        let preview = Self {
            surface: VideoSurface {
                area: area.clone(),
                state: state.clone(),
            },
            project: project.clone(),
            player_state: player_state.clone(),
            selection_state: selection_state.clone(),
            preview_focus: preview_focus.clone(),
        };
        attach_render(
            &area,
            project.clone(),
            player_state.clone(),
            selection_state.clone(),
            preview_focus.clone(),
            state.clone(),
        );
        attach_input(&preview);
        attach_frame_scheduler(&area, player_state.clone(), state.clone());
        let resize_state = state.clone();
        area.connect_resize(move |_, _, _| resize_state.borrow().sanim_inspector.dismiss());

        let unrealize_state = state.clone();
        area.connect_unrealize(move |area| {
            area.make_current();
            let mut state = unrealize_state.borrow_mut();
            state.sanim_inspector.dismiss();
            if let Some(mut renderer) = state.renderer.take() {
                renderer.destroy();
            }
        });
        let unmap_state = state.clone();
        area.connect_unmap(move |area| {
            unmap_state.borrow().sanim_inspector.dismiss();
            area.set_cursor_from_name(None);
        });
        let selection_area = area.clone();
        let selection_project = project.clone();
        let selection_player = player_state.clone();
        let selection_surface = state.clone();
        selection_state::connect_named(&selection_state, "preview provider selection", move || {
            let area = selection_area.clone();
            let project = selection_project.clone();
            let player = selection_player.clone();
            let surface = selection_surface.clone();
            glib::idle_add_local_once(move || {
                surface.borrow().sanim_inspector.dismiss();
                cancel_provider(&area, &project, &player, &surface, false);
                area.queue_render();
            });
        });
        let focus_area = area.clone();
        let focus_project = project.clone();
        let focus_player = player_state.clone();
        let focus_surface = state.clone();
        preview_focus::connect_named(&preview_focus, "preview provider focus", move || {
            focus_surface.borrow().sanim_inspector.dismiss();
            cancel_provider(
                &focus_area,
                &focus_project,
                &focus_player,
                &focus_surface,
                false,
            );
            focus_area.set_cursor_from_name(None);
            focus_area.queue_render();
        });
        let inspection_state = state.clone();
        let inspection_area = area.downgrade();
        player_state::connect_while_alive_named(
            &player_state,
            "preview Sanim element inspection",
            move || inspection_area.upgrade().is_some(),
            move |event| {
                if matches!(event, player_state::PlayerEvent::Project(_))
                    || matches!(event, player_state::PlayerEvent::State(change) if change.position.is_some())
                {
                    inspection_state.borrow().sanim_inspector.dismiss();
                }
            },
        );
        let preference_area = area.clone();
        let preference_state = state.clone();
        preferences_store::connect(&preferences, move |preference| {
            let mut state = preference_state.borrow_mut();
            state.sanim_inspector.dismiss();
            state.zoom_pan_enabled = preference.preview_zoom_pan_enabled;
            if !state.zoom_pan_enabled {
                preference_area.set_cursor_from_name(None);
            }
            state.caption_font_size = preference.caption_font_size;
            state.caption_background_color = preference.caption_background_color;
            state.preview_padding_px = preference.preview_padding_px;
            state.preview_shadow_size_px = preference.preview_shadow_size_px;
            state.preview_upsample_method = preference.preview_upsample_method;
            state.preview_downsample_method = preference.preview_downsample_method;
            state.guides_visible = preference.preview_guides_visible;
            state.snap_enabled = preference.timeline_magnet == "true";
            state.snap_radius_px = preference.timeline_snap_radius_px;
            state.surface.invalidate();
            drop(state);
            preference_area.queue_render();
        });
        let style = adw::StyleManager::for_display(&area.display());
        let dark_area = area.clone();
        style.connect_dark_notify(move |_| dark_area.queue_render());
        let accent_area = area.clone();
        style.connect_accent_color_notify(move |_| accent_area.queue_render());
        let contrast_area = area.clone();
        style.connect_high_contrast_notify(move |_| contrast_area.queue_render());

        preview
    }

    pub(crate) fn can_inspect_sanim(&self) -> bool {
        let project = self.project.borrow();
        !player_state::snapshot(&self.player_state).playing
            && selection_state::focused_video_address(&self.selection_state, &project)
                .and_then(|address| project.video_item(&address))
                .is_some_and(|item| {
                    matches!(item.content, crate::project::VideoItemContent::Sanim(_))
                })
    }

    pub(crate) fn inspect_sanim(&self, point: GlamVec2) {
        sanim_inspector::inspect(
            ProviderDispatch {
                area: &self.surface.area,
                project: &self.project,
                player_state: &self.player_state,
                selection_state: &self.selection_state,
                preview_focus: &self.preview_focus,
                state: &self.surface.state,
            },
            point,
        );
    }

    pub(crate) fn preview_state<T: 'static, R>(
        &self,
        key: PreviewExtensionKey,
        read: impl FnOnce(&T) -> R,
    ) -> R {
        let state = self.surface.state.borrow();
        let value = state
            .surface
            .extension::<T>(key)
            .expect("preview state extension has the wrong type");
        read(value)
    }

    pub(crate) fn update_preview_state<T: 'static, R>(
        &self,
        key: PreviewExtensionKey,
        update: impl FnOnce(&mut T) -> R,
    ) -> R {
        let mut output = None;
        assert!(
            self.surface
                .state
                .borrow_mut()
                .surface
                .update_extension(key, |value: &mut T| output = Some(update(value))),
            "preview state extension has the wrong type"
        );
        self.area.queue_render();
        output.expect("preview state updated")
    }

    pub fn set_frame(
        &self,
        frame: CompositedVideoFrame,
        audio_analysis: FrameAudioAnalysis,
        position: Time,
        revision: u64,
        excluded_item_id: Option<uuid::Uuid>,
        settled: bool,
    ) {
        let mut state = self.surface.state.borrow_mut();
        if !state
            .surface
            .presented(revision, position, excluded_item_id, &audio_analysis)
        {
            return;
        }
        state.presentation = Some(Presentation {
            position,
            revision,
            settled,
        });
        if state.frame.as_ref().map(|item| item.storage_key) != Some(frame.storage_key) {
            state.frame = Some(frame);
        }
        drop(state);
        self.area.queue_render();
    }

    pub fn clear_frame(
        &self,
        audio_analysis: FrameAudioAnalysis,
        revision: u64,
        excluded_item_id: Option<uuid::Uuid>,
    ) {
        let mut state = self.surface.state.borrow_mut();
        if !state
            .surface
            .presented(revision, Time::ZERO, excluded_item_id, &audio_analysis)
        {
            return;
        }
        state.sanim_inspector.dismiss();
        state.presentation = None;
        state.frame = None;
        drop(state);
        self.area.queue_render();
    }
}

impl VideoSurface {
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
        let width = i32::try_from(frame.width).map_err(|_| "preview width is too large")?;
        let height = i32::try_from(frame.height).map_err(|_| "preview height is too large")?;
        let stride = frame.width as usize * std::mem::size_of::<u32>();
        let bytes = frame.rgba_bytes()?;
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
        let mut state = self.state.borrow_mut();
        state.sanim_inspector.dismiss();
        state.fullscreen = fullscreen;
        drop(state);
        self.area.queue_render();
    }

    pub fn queue_render(&self) {
        self.area.queue_render();
    }
}

fn attach_render(
    area: &gtk::GLArea,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    preview_focus: SharedPreviewFocus,
    state: Rc<RefCell<VideoSurfaceState>>,
) {
    area.connect_render(move |area, _| {
        if let Some(error) = area.error() {
            tracing::error!("Video GLArea error: {error}");
            return glib::Propagation::Stop;
        }
        let surface_scale = area.scale_factor().max(1) as f32;
        let surface_px = IVec2::new(
            (area.width().max(1) as f32 * surface_scale) as i32,
            (area.height().max(1) as f32 * surface_scale) as i32,
        );
        let project = project.borrow();
        let player = player_state::snapshot(&player_state);
        let clock = Clock {
            revision: player.revision,
            position: player.position,
            moving: player.playing || player.scrubbing,
        };
        let focused_video = selection_state::focused_video_address(&selection_state, &project);
        let focused_caption = selection_state::focused_item_address(&selection_state, &project)
            .filter(|address| project.caption_item(address).is_some());
        let mut state = state.borrow_mut();
        if state.renderer.is_none() {
            match VideoRenderer::new() {
                Ok(renderer) => state.renderer = Some(renderer),
                Err(error) => {
                    tracing::error!("Could not initialize preview renderer: {error}");
                    return glib::Propagation::Stop;
                }
            }
        }
        state.sync_focus(&preview_focus);
        let configuration = state.configuration(area);
        if let Err(error) =
            state
                .surface
                .ensure_readonly(&project, focused_video.as_ref(), clock, configuration)
        {
            tracing::error!(%error, "Could not prepare preview provider");
            return glib::Propagation::Stop;
        }
        state.send_exclusion_update(clock.position);
        let layout = state.surface.layout(&project, configuration);
        let background_color = shrimply_preview_runtime_cuda::background_color(
            geometry::theme_window_color(area),
            state.fullscreen,
        );
        let captions = CaptionAppearance {
            preview_rect: layout.surface_rect,
            font_size: state.caption_font_size,
            background_color: state.caption_background_color,
            bottom_inset: state.caption_bottom_inset,
        };
        let caption_split_hover = focused_caption.as_ref().zip(state.caption_split_hover);
        let appearance = Appearance {
            content_rect: layout.viewport.content_rect,
            clip_rect: layout.clip_rect,
            shadow_size_px: state.preview_shadow_size_px,
            background_color,
            upsample_method: state.preview_upsample_method,
            downsample_method: state.preview_downsample_method,
        };
        let sanim_inspector = state.sanim_inspector.clone();
        let sanim_sources = state
            .frame
            .as_ref()
            .map(|frame| frame.sanim_sources.clone());
        let VideoSurfaceState {
            renderer,
            frame,
            surface,
            ..
        } = &mut *state;
        let result = renderer
            .as_mut()
            .expect("preview renderer was initialized")
            .render(
                surface_px,
                surface_scale,
                frame.as_ref(),
                appearance,
                |timeline_painter| {
                    if let Err(error) = surface.draw_overlay(
                        timeline_painter.canvas(),
                        &project,
                        focused_video.as_ref(),
                        clock,
                        configuration,
                        captions,
                        caption_split_hover,
                        |canvas| sanim_inspector.draw(canvas, sanim_sources.as_deref()),
                    ) {
                        tracing::error!(%error, "Could not draw preview overlay");
                    }
                },
            );
        if let Err(error) = result {
            tracing::error!("Could not render video preview: {error}");
        }
        glib::Propagation::Stop
    });
}

#[derive(Clone, Copy)]
struct ProviderDispatch<'a> {
    area: &'a gtk::GLArea,
    project: &'a Rc<RefCell<Project>>,
    player_state: &'a SharedPlayerState,
    selection_state: &'a SharedSelectionState,
    preview_focus: &'a SharedPreviewFocus,
    state: &'a Rc<RefCell<VideoSurfaceState>>,
}

impl ProviderDispatch<'_> {
    fn clock(&self) -> Clock {
        let player = player_state::snapshot(self.player_state);
        Clock {
            revision: player.revision,
            position: player.position,
            moving: player.playing || player.scrubbing,
        }
    }
}

fn dispatch_pointer(surface: ProviderDispatch<'_>, event: PointerEvent<'_>) -> PreviewResponse {
    let clock = surface.clock();
    let response = {
        let mut project = surface.project.borrow_mut();
        let selected = selection_state::focused_video_address(surface.selection_state, &project);
        let mut state = surface.state.borrow_mut();
        let config = state.configuration(surface.area);
        state.sync_focus(surface.preview_focus);
        let result = state
            .surface
            .pointer(&mut project, selected.as_ref(), clock, config, event);
        state.send_exclusion_update(clock.position);
        match result {
            Ok(response) => response,
            Err(error) => {
                tracing::error!(%error, "Could not dispatch preview pointer event");
                return PreviewResponse::IGNORED;
            }
        }
    };
    if response.redraw && surface.state.borrow().surface.navigation_active() {
        surface.state.borrow().sanim_inspector.dismiss();
    }
    apply_response(surface.area, surface.player_state, surface.state, response);
    response
}

fn dispatch_keyboard(surface: ProviderDispatch<'_>, event: KeyboardEvent) -> PreviewResponse {
    let clock = surface.clock();
    let response = {
        let mut project = surface.project.borrow_mut();
        let selected = selection_state::focused_video_address(surface.selection_state, &project);
        let mut state = surface.state.borrow_mut();
        let config = state.configuration(surface.area);
        state.sync_focus(surface.preview_focus);
        let result = state
            .surface
            .keyboard(&mut project, selected.as_ref(), clock, config, event);
        state.send_exclusion_update(clock.position);
        match result {
            Ok(response) => response,
            Err(error) => {
                tracing::error!(%error, "Could not dispatch preview keyboard event");
                return PreviewResponse::IGNORED;
            }
        }
    };
    apply_response(surface.area, surface.player_state, surface.state, response);
    response
}

fn cancel_provider(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    state: &Rc<RefCell<VideoSurfaceState>>,
    teardown: bool,
) {
    let position = player_state::snapshot(player_state).position;
    let response = {
        let mut project = project.borrow_mut();
        let mut state = state.borrow_mut();
        let response = state.surface.cancel_with(&mut project, teardown);
        state.send_exclusion_update(position);
        response
    };
    apply_response(area, player_state, state, response);
}

fn attach_input(preview: &PreviewController) {
    let area = preview.widget();
    let project = preview.project.clone();
    let player_state = preview.player_state.clone();
    let selection_state = preview.selection_state.clone();
    let preview_focus = preview.preview_focus.clone();
    let state = preview.surface.state.clone();
    let sequence_button = Rc::new(Cell::new(0));
    let sequence_origin = Rc::new(Cell::new(None::<PointerSample>));
    let sequence_moved = Rc::new(Cell::new(false));
    let sequence_inspect = Rc::new(Cell::new(false));
    touch::attach(preview);
    let caption_hover = Cell::new(false);

    let motion = gtk::EventControllerMotion::new();
    {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preview_focus = preview_focus.clone();
        let state = state.clone();
        motion.connect_motion(move |source, x, y| {
            let position = GlamVec2::new(x as f32, y as f32);
            if !state.borrow().surface.is_idle() {
                if state.borrow_mut().caption_split_hover.take().is_some() {
                    area.queue_render();
                }
                return;
            }
            let dispatch = ProviderDispatch {
                area: &area,
                project: &project,
                player_state: &player_state,
                selection_state: &selection_state,
                preview_focus: &preview_focus,
                state: &state,
            };
            let split_active = caption_split_at_pointer(
                &area,
                &project,
                &player_state,
                &selection_state,
                &state,
                position,
            )
            .is_some();
            if split_active {
                if state.borrow().caption_split_hover != Some(position) {
                    state.borrow_mut().caption_split_hover = Some(position);
                    area.queue_render();
                }
                caption_hover.set(true);
                area.set_cursor_from_name(Some("text"));
                return;
            }
            if state.borrow_mut().caption_split_hover.take().is_some() {
                area.queue_render();
            }
            if caption_hover.replace(false) {
                area.set_cursor_from_name(None);
            }
            dispatch_pointer(
                dispatch,
                PointerEvent::Hover(pointer_input(source, position, 0)),
            );
        });
    }
    {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preview_focus = preview_focus.clone();
        let state = state.clone();
        motion.connect_leave(move |_| {
            if state.borrow_mut().caption_split_hover.take().is_some() {
                area.queue_render();
            }
            if !state.borrow().surface.is_idle() {
                return;
            }
            dispatch_pointer(
                ProviderDispatch {
                    area: &area,
                    project: &project,
                    player_state: &player_state,
                    selection_state: &selection_state,
                    preview_focus: &preview_focus,
                    state: &state,
                },
                PointerEvent::Leave,
            );
            area.set_cursor_from_name(None);
        });
    }
    area.add_controller(motion);

    let pointer = gtk::GestureStylus::new();
    pointer.set_button(0);
    pointer.set_stylus_only(false);
    {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preview_focus = preview_focus.clone();
        let state = state.clone();
        let sequence_button = sequence_button.clone();
        let sequence_origin = sequence_origin.clone();
        let sequence_moved = sequence_moved.clone();
        let sequence_inspect = sequence_inspect.clone();
        pointer.connect_down(move |source, x, y| {
            let button = source.current_button();
            if (!state.borrow().surface.is_idle() && button == 2)
                || state.borrow().surface.navigation_active()
            {
                return;
            }
            area.grab_focus();
            state.borrow().sanim_inspector.dismiss();
            sequence_button.set(button);
            let input = pointer_input(source, GlamVec2::new(x as f32, y as f32), button);
            sequence_origin.set(Some(input.sample));
            sequence_moved.set(false);
            sequence_inspect.set(false);
            if button == 1
                && split_caption_at_pointer(
                    &area,
                    &project,
                    &player_state,
                    &selection_state,
                    &state,
                    input.sample.position,
                )
            {
                return;
            }
            let response = dispatch_pointer(
                ProviderDispatch {
                    area: &area,
                    project: &project,
                    player_state: &player_state,
                    selection_state: &selection_state,
                    preview_focus: &preview_focus,
                    state: &state,
                },
                PointerEvent::Begin(input),
            );
            if button == 1
                && input.modifiers == Modifiers::NONE
                && !player_state::snapshot(&player_state).playing
            {
                let project = project.borrow();
                let state = state.borrow();
                sequence_inspect.set(
                    selection_state::focused_video_address(&selection_state, &project)
                        .and_then(|address| project.video_item(&address))
                        .is_some_and(|item| {
                            matches!(item.content, crate::project::VideoItemContent::Sanim(_))
                                && state.surface.provider().is_none_or(|provider| {
                                    provider.target == item.default_preview_target()
                                })
                        }),
                );
            }
            if button == 2 && response.handled {
                source.set_state(gtk::EventSequenceState::Claimed);
            }
        });
    }
    let update_pointer = {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preview_focus = preview_focus.clone();
        let state = state.clone();
        let sequence_button = sequence_button.clone();
        let sequence_origin = sequence_origin.clone();
        let sequence_moved = sequence_moved.clone();
        let sequence_inspect = sequence_inspect.clone();
        move |source: &gtk::GestureStylus, x: f64, y: f64| {
            let input = pointer_input(
                source,
                GlamVec2::new(x as f32, y as f32),
                sequence_button.get(),
            );
            let dispatch = ProviderDispatch {
                area: &area,
                project: &project,
                player_state: &player_state,
                selection_state: &selection_state,
                preview_focus: &preview_focus,
                state: &state,
            };
            if state.borrow().surface.navigation_active() {
                dispatch_pointer(dispatch, PointerEvent::Hover(input));
                return;
            }
            let mut samples = pointer_backlog(source);
            if samples.last().copied() != Some(input.sample) {
                samples.push(input.sample);
            }
            let changed = sequence_origin.get().is_some_and(|origin| {
                samples.iter().any(|sample| {
                    pointer_sample_changed(
                        &area,
                        *sample,
                        origin,
                        sequence_inspect.get() && !sequence_moved.get(),
                    )
                })
            });
            if !changed {
                return;
            }
            sequence_moved.set(true);
            dispatch_pointer(
                dispatch,
                PointerEvent::Samples {
                    input,
                    samples: &samples,
                },
            );
        }
    };
    pointer.connect_motion(update_pointer.clone());
    let proximity_state = state.clone();
    pointer.connect_proximity(move |source, x, y| {
        if proximity_state.borrow().surface.navigation_active() {
            update_pointer(source, x, y);
        }
    });
    {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preview_focus = preview_focus.clone();
        let state = state.clone();
        let sequence_button = sequence_button.clone();
        let sequence_origin = sequence_origin.clone();
        let sequence_moved = sequence_moved.clone();
        let sequence_inspect = sequence_inspect.clone();
        pointer.connect_up(move |source, x, y| {
            let input = pointer_input(
                source,
                GlamVec2::new(x as f32, y as f32),
                sequence_button.get(),
            );
            let mut samples = pointer_backlog(source);
            if sequence_origin.get().is_some_and(|origin| {
                samples
                    .iter()
                    .chain(std::iter::once(&input.sample))
                    .any(|sample| {
                        pointer_sample_changed(
                            &area,
                            *sample,
                            origin,
                            sequence_inspect.get() && !sequence_moved.get(),
                        )
                    })
            }) {
                sequence_moved.set(true);
            }
            let dispatch = ProviderDispatch {
                area: &area,
                project: &project,
                player_state: &player_state,
                selection_state: &selection_state,
                preview_focus: &preview_focus,
                state: &state,
            };
            let was_guide = state.borrow().surface.guides_active();
            if !was_guide && !samples.is_empty() && sequence_moved.get() {
                if samples.last().copied() != Some(input.sample) {
                    samples.push(input.sample);
                }
                dispatch_pointer(
                    dispatch,
                    PointerEvent::Samples {
                        input,
                        samples: &samples,
                    },
                );
            }
            let provider_input = if !was_guide && sequence_inspect.get() && !sequence_moved.get() {
                PointerInput {
                    sample: sequence_origin
                        .get()
                        .expect("inspection press has an origin"),
                    ..input
                }
            } else {
                input
            };
            dispatch_pointer(dispatch, PointerEvent::End(provider_input));
            if !was_guide
                && sequence_button.get() == 1
                && !sequence_moved.get()
                && input.modifiers == Modifiers::NONE
            {
                sanim_inspector::inspect(dispatch, input.sample.position);
            }
            sequence_button.set(0);
            sequence_origin.set(None);
            sequence_moved.set(false);
            area.set_cursor_from_name(None);
        });
    }
    {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preview_focus = preview_focus.clone();
        let state = state.clone();
        pointer.connect_cancel(move |_, _| {
            dispatch_pointer(
                ProviderDispatch {
                    area: &area,
                    project: &project,
                    player_state: &player_state,
                    selection_state: &selection_state,
                    preview_focus: &preview_focus,
                    state: &state,
                },
                PointerEvent::Cancel,
            );
            sequence_button.set(0);
            sequence_origin.set(None);
            sequence_moved.set(false);
            area.set_cursor_from_name(None);
        });
    }
    area.add_controller(pointer);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preview_focus = preview_focus.clone();
        let state = state.clone();
        scroll.connect_scroll(move |source, dx, dy| {
            let position = source
                .current_event()
                .and_then(|event| event.position())
                .map_or_else(
                    || GlamVec2::new(area.width() as f32 * 0.5, area.height() as f32 * 0.5),
                    |(x, y)| {
                        let native = area.native().expect("preview must have a native surface");
                        let (offset_x, offset_y) = native.surface_transform();
                        let native_widget = native
                            .dynamic_cast::<gtk::Widget>()
                            .expect("native surface must be a widget");
                        native_widget
                            .compute_point(
                                &area,
                                &gtk::graphene::Point::new(
                                    (x - offset_x) as f32,
                                    (y - offset_y) as f32,
                                ),
                            )
                            .map(|point| GlamVec2::new(point.x(), point.y()))
                            .expect("preview must share its native coordinate space")
                    },
                );
            let delta = shrimply_preview_provider_skia::math::preview_scroll_delta(
                GlamVec2::new(dx as f32, dy as f32),
                source.unit() == gdk::ScrollUnit::Surface,
                state.borrow().zoom_pan_enabled,
            );
            let response = dispatch_pointer(
                ProviderDispatch {
                    area: &area,
                    project: &project,
                    player_state: &player_state,
                    selection_state: &selection_state,
                    preview_focus: &preview_focus,
                    state: &state,
                },
                PointerEvent::Scroll {
                    input: pointer_input(source, position, 0),
                    delta,
                },
            );
            if response.handled {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    area.add_controller(scroll);

    let key = gtk::EventControllerKey::new();
    {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preview_focus = preview_focus.clone();
        let state = state.clone();
        key.connect_key_pressed(move |_, key, _, modifiers| {
            let response = dispatch_keyboard(
                ProviderDispatch {
                    area: &area,
                    project: &project,
                    player_state: &player_state,
                    selection_state: &selection_state,
                    preview_focus: &preview_focus,
                    state: &state,
                },
                keyboard_event(key, KeyState::Pressed, modifiers),
            );
            if response.handled {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    {
        let area = area.clone();
        key.connect_key_released(move |_, key, _, modifiers| {
            dispatch_keyboard(
                ProviderDispatch {
                    area: &area,
                    project: &project,
                    player_state: &player_state,
                    selection_state: &selection_state,
                    preview_focus: &preview_focus,
                    state: &state,
                },
                keyboard_event(key, KeyState::Released, modifiers),
            );
        });
    }
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

fn pointer_sample_changed(
    area: &gtk::GLArea,
    sample: PointerSample,
    previous: PointerSample,
    inspecting: bool,
) -> bool {
    if inspecting {
        area.drag_check_threshold(
            previous.position.x.round() as i32,
            previous.position.y.round() as i32,
            sample.position.x.round() as i32,
            sample.position.y.round() as i32,
        )
    } else {
        sample.position != previous.position
            || sample.pressure != previous.pressure
            || sample.tilt != previous.tilt
    }
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
