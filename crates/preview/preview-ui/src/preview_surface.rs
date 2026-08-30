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
    Cursor, CursorUpdate, Key, KeyState, KeyboardEvent, Modifiers, PointerButton, PointerEvent,
    PointerInput, PointerSample, PointerTool, PreviewBuilder, PreviewContext, PreviewEditSink,
    PreviewExtensionKey, PreviewItemGeometry, PreviewProvider, PreviewRefresh, PreviewResponse,
    PreviewTarget, PreviewViewport, SnapScene,
};

use crate::player_state::{self, SharedPlayerState};
use crate::preferences::store as preferences_store;
use crate::preview_focus::{self, FocusedPreview, SharedPreviewFocus};
use crate::project::{ItemAddress, PreviewGuides, Project, Time};
use crate::selection_state::{self, SharedSelectionState};
use crate::timeline::renderer::{Color, Rect, vec2};
use crate::transform_eval::{FrameAudioAnalysis, TransformExpressionCache, VisualEvaluation};
use crate::video::compositor::{CompositeAccuracy, VideoCommand, VideoCommandSender};
use crate::video::gpu::CompositedVideoFrame;

#[path = "preview_surface/captions.rs"]
mod captions;
use captions::draw_captions;
#[path = "preview_surface/dispatch.rs"]
mod dispatch;
use dispatch::{
    apply_response, attach_frame_scheduler, caption_split_at_pointer, split_caption_at_pointer,
};
#[path = "preview_surface/geometry.rs"]
mod geometry;
#[path = "preview_surface/guides.rs"]
mod guides;
use geometry::{surface_viewport, video_content_rect};
#[path = "preview_surface/renderer.rs"]
mod renderer;
use renderer::{Appearance, VideoRenderer};

const FULLSCREEN_BACKGROUND_COLOR: Color = Color::BLACK;

const fn cursor_name(cursor: Cursor) -> &'static str {
    match cursor {
        Cursor::Default => "default",
        Cursor::Pointer => "pointer",
        Cursor::Crosshair => "crosshair",
        Cursor::Move => "move",
        Cursor::Grab => "grab",
        Cursor::Grabbing => "grabbing",
        Cursor::Text => "text",
        Cursor::ResizeHorizontal => "ew-resize",
        Cursor::ResizeVertical => "ns-resize",
        Cursor::ResizeDiagonalDown => "nwse-resize",
        Cursor::ResizeDiagonalUp => "nesw-resize",
        Cursor::Hidden => "none",
    }
}

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
    expression_revision: Option<u64>,
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
    provider: Option<PreparedProvider>,
    retiring_provider: Option<PreparedProvider>,
    extensions: HashMap<PreviewExtensionKey, Box<dyn Any>>,
    sequence: PointerSequence,
    context_invalidated: bool,
    frame_pending: bool,
    live_base_pending: bool,
    live_base_in_flight: Option<u64>,
    base_exclusion: Option<uuid::Uuid>,
    presented_base_exclusion: Option<uuid::Uuid>,
    video_tx: VideoCommandSender,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum PointerSequence {
    #[default]
    Idle,
    Active,
    Guide,
    Suppressed,
}

fn set_base_exclusion(
    controller: &mut PreviewControllerState,
    item_id: Option<uuid::Uuid>,
    position: Time,
) {
    if controller.base_exclusion == item_id {
        return;
    }
    controller.base_exclusion = item_id;
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

impl VideoSurfaceState {
    fn padding_px(&self) -> u32 {
        let padding = if self.fullscreen {
            0
        } else {
            self.preview_padding_px
        };
        if self.guides_visible {
            padding.max(guides::MIN_PADDING_PX)
        } else {
            padding
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
            expression_revision: None,
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
            provider: None,
            retiring_provider: None,
            extensions: HashMap::new(),
            sequence: PointerSequence::Idle,
            context_invalidated: false,
            frame_pending: false,
            live_base_pending: false,
            live_base_in_flight: None,
            base_exclusion: None,
            presented_base_exclusion: None,
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
            preference_controller.borrow_mut().context_invalidated = true;
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
        if excluded_item_id != controller.base_exclusion {
            return;
        }
        controller.presented_base_exclusion = excluded_item_id;
        controller.retiring_provider = None;
        if controller
            .live_base_in_flight
            .is_some_and(|requested| revision >= requested)
        {
            controller.live_base_in_flight = None;
        }
        let sequence_idle = controller.sequence == PointerSequence::Idle;
        let mut context_invalidated = false;
        if let Some(provider) = controller.provider.as_mut() {
            provider.provider.on_base_frame_presented(revision);
            context_invalidated = sequence_idle && revision >= provider.project_revision;
        }
        controller.context_invalidated |= context_invalidated;
        drop(controller);
        self.surface.set_frame(frame, audio_analysis, revision);
    }

    pub fn clear_frame(
        &self,
        audio_analysis: FrameAudioAnalysis,
        revision: u64,
        excluded_item_id: Option<uuid::Uuid>,
    ) {
        let mut controller = self.controller.borrow_mut();
        if excluded_item_id != controller.base_exclusion {
            return;
        }
        controller.presented_base_exclusion = excluded_item_id;
        controller.retiring_provider = None;
        if controller
            .live_base_in_flight
            .is_some_and(|requested| revision >= requested)
        {
            controller.live_base_in_flight = None;
        }
        let sequence_idle = controller.sequence == PointerSequence::Idle;
        let mut context_invalidated = false;
        if let Some(provider) = controller.provider.as_mut() {
            provider.provider.on_base_frame_presented(revision);
            context_invalidated = sequence_idle && revision >= provider.project_revision;
        }
        controller.context_invalidated |= context_invalidated;
        drop(controller);
        self.surface.clear_frame(audio_analysis, revision);
    }
}

impl VideoSurface {
    fn set_frame(
        &self,
        frame: CompositedVideoFrame,
        audio_analysis: FrameAudioAnalysis,
        revision: u64,
    ) {
        let mut state = self.state.borrow_mut();
        if state.expression_revision != Some(revision) {
            state.expression_cache.get_mut().invalidate_values();
            state.expression_revision = Some(revision);
        }
        if state.frame.as_ref().map(|frame| frame.storage_key) != Some(frame.storage_key)
            || !state.audio_analysis.same_frame(&audio_analysis)
        {
            state.frame = Some(frame);
            state.audio_analysis = audio_analysis;
        }
        drop(state);
        self.area.queue_render();
    }

    fn clear_frame(&self, audio_analysis: FrameAudioAnalysis, revision: u64) {
        let mut state = self.state.borrow_mut();
        if state.expression_revision != Some(revision) {
            state.expression_cache.get_mut().invalidate_values();
            state.expression_revision = Some(revision);
        }
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

struct PreparedProvider {
    item: ItemAddress,
    project_revision: u64,
    context: PreparedContext,
    provider: Box<dyn PreviewProvider>,
    deferred_refresh: PreviewRefresh,
}

struct PreparedContext {
    evaluation: VisualEvaluation,
    timeline_position: Time,
    keyframe_time: Time,
    viewport: PreviewViewport,
    geometry: PreviewItemGeometry,
    source_sizes: HashMap<uuid::Uuid, GlamVec2>,
    snap_scene: Option<SnapScene>,
    tracked_camera: Option<crate::project::TrackedCameraPreview>,
    item_id: uuid::Uuid,
}

impl PreparedContext {
    fn context<'a>(
        &'a self,
        expression_cache: &'a RefCell<TransformExpressionCache>,
        extensions: Option<&'a HashMap<PreviewExtensionKey, Box<dyn Any>>>,
    ) -> HandlerContext<'a> {
        HandlerContext {
            evaluation: &self.evaluation,
            timeline_position: self.timeline_position,
            expression_cache,
            viewport: self.viewport,
            geometry: Some(self.geometry),
            source_sizes: &self.source_sizes,
            snap_scene: self.snap_scene.as_ref(),
            tracked_camera: self.tracked_camera.as_ref(),
            extensions,
            item_id: self.item_id,
        }
    }
}

#[derive(Clone, Copy)]
struct HandlerContext<'a> {
    evaluation: &'a VisualEvaluation,
    timeline_position: Time,
    expression_cache: &'a RefCell<TransformExpressionCache>,
    viewport: PreviewViewport,
    geometry: Option<PreviewItemGeometry>,
    source_sizes: &'a HashMap<uuid::Uuid, GlamVec2>,
    snap_scene: Option<&'a SnapScene>,
    tracked_camera: Option<&'a crate::project::TrackedCameraPreview>,
    extensions: Option<&'a HashMap<PreviewExtensionKey, Box<dyn Any>>>,
    item_id: uuid::Uuid,
}

impl PreviewContext for HandlerContext<'_> {
    fn timeline_position(&self) -> Time {
        self.timeline_position
    }

    fn local_time(&self) -> Time {
        self.evaluation.local_time()
    }

    fn viewport(&self) -> PreviewViewport {
        self.viewport
    }

    fn selection_color(&self) -> Color {
        Color::BLUE5
    }

    fn target_geometry(&self, _target: PreviewTarget) -> Option<PreviewItemGeometry> {
        self.geometry
    }

    fn source_size(&self, item_id: uuid::Uuid) -> Option<GlamVec2> {
        self.source_sizes.get(&item_id).copied()
    }

    fn item_geometry(&self, item_id: uuid::Uuid) -> Option<PreviewItemGeometry> {
        (item_id == self.item_id).then_some(self.geometry).flatten()
    }

    fn snapping(&self) -> Option<&SnapScene> {
        self.snap_scene
    }

    fn extension(&self, _target: PreviewTarget, key: PreviewExtensionKey) -> Option<&dyn Any> {
        if key == crate::project::TRACKED_CAMERA_PREVIEW {
            return self.tracked_camera.map(|camera| camera as &dyn Any);
        }
        self.extensions?.get(&key).map(|value| value.as_ref())
    }
}

impl PreviewBuilder for HandlerContext<'_> {
    fn resolve<T: shrimply_core::timeline_value::TimelineExpressionValue>(
        &self,
        value: &shrimply_core::timeline_value::TimelineValue<T>,
    ) -> T {
        crate::transform_eval::resolve(
            value,
            self.evaluation,
            &mut self.expression_cache.borrow_mut(),
        )
    }

    fn resolve_at<T: shrimply_core::timeline_value::TimelineExpressionValue>(
        &self,
        value: &shrimply_core::timeline_value::TimelineValue<T>,
        time: Time,
    ) -> T {
        crate::transform_eval::resolve(
            value,
            &self.evaluation.at_local_time(time),
            &mut self.expression_cache.borrow_mut(),
        )
    }
}

struct Edits<'a> {
    project: &'a mut Project,
    extensions: &'a mut HashMap<PreviewExtensionKey, Box<dyn Any>>,
    item: &'a ItemAddress,
    keyframe_time: Time,
    context: HandlerContext<'a>,
}

impl PreviewEditSink for Edits<'_> {
    fn keyframe_time(&self) -> Time {
        self.keyframe_time
    }

    fn target_mut(&mut self, target: PreviewTarget) -> &mut dyn Any {
        self.project
            .preview_target_mut(target)
            .expect("preview target is missing")
    }

    fn updated_geometry(&self, _target: PreviewTarget) -> Option<PreviewItemGeometry> {
        let item = self.project.video_item(self.item)?;
        let mut source_sizes = self.context.source_sizes.clone();
        if let crate::project::VideoItemContent::Text(text) = &item.content {
            source_sizes.insert(
                item.id,
                text_source_size(text, self.context.evaluation, self.context.expression_cache),
            );
        }
        item.preview_geometry(&HandlerContext {
            source_sizes: &source_sizes,
            ..self.context
        })
    }

    fn extension_mut(
        &mut self,
        _target: PreviewTarget,
        key: PreviewExtensionKey,
    ) -> Option<&mut dyn Any> {
        self.extensions.get_mut(&key).map(|value| value.as_mut())
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
    let item = project.video_item(key)?;
    if !item.owns_preview_target(target) {
        return None;
    }
    let sequence_position = project.timeline_time_to_sequence(&key.track(), position)?;
    let keyframe_time = project.keyframe_time(key, position)?;
    let evaluation = VisualEvaluation::for_item_with_audio(
        project,
        item,
        sequence_position,
        preparation.audio_analysis,
    );
    let canvas_size = GlamVec2::new(
        project.canvas_size.width.max(1) as f32,
        project.canvas_size.height.max(1) as f32,
    );
    let content_rect = video_content_rect(
        preparation.surface.x,
        preparation.surface.y,
        project.canvas_size.width,
        project.canvas_size.height,
        preparation.padding_px,
    );
    let viewport = PreviewViewport::new(canvas_size, content_rect);
    let mut source_sizes = source_sizes(project);
    if let crate::project::VideoItemContent::Text(text) = &item.content {
        source_sizes.insert(
            item.id,
            text_source_size(text, &evaluation, preparation.expression_cache),
        );
    }
    let tracked_camera = item
        .tracking_camera_source()
        .filter(|source| {
            source.track_id != key.track_id()
                && project
                    .video_tracks
                    .iter()
                    .any(|track| track.id == source.track_id)
        })
        .and_then(|source| {
            crate::video::camera_reconstruction::sample(item.id, source, evaluation.local_time())
        })
        .map(|camera| crate::project::TrackedCameraPreview {
            position: camera.position,
            rotation: camera.rotation,
            projection: camera.projection,
            vertical_fov_degrees: camera.vertical_fov_degrees,
        });
    let geometry = {
        let context = HandlerContext {
            evaluation: &evaluation,
            timeline_position: position,
            expression_cache: preparation.expression_cache,
            viewport,
            geometry: None,
            source_sizes: &source_sizes,
            snap_scene: None,
            tracked_camera: tracked_camera.as_ref(),
            extensions: Some(preparation.extensions),
            item_id: item.id,
        };
        item.preview_geometry(&context)?
    };
    let snap_scene = preparation.snap_enabled.then(|| {
        let mut scene = SnapScene::new(viewport, preparation.snap_radius_px);
        if let Some(guides) = preparation.guides {
            scene.add_guides(&guides.vertical, &guides.horizontal);
        }
        add_snap_providers(
            &mut scene,
            project,
            key,
            position,
            preparation,
            viewport,
            &mut source_sizes,
        );
        scene
    });
    let context = HandlerContext {
        evaluation: &evaluation,
        timeline_position: position,
        expression_cache: preparation.expression_cache,
        viewport,
        geometry: Some(geometry),
        source_sizes: &source_sizes,
        snap_scene: snap_scene.as_ref(),
        tracked_camera: tracked_camera.as_ref(),
        extensions: Some(preparation.extensions),
        item_id: item.id,
    };
    let provider = item.preview_provider(target, &context)?;
    Some(PreparedProvider {
        item: key.clone(),
        project_revision: preparation.project_revision,
        context: PreparedContext {
            evaluation,
            timeline_position: position,
            keyframe_time,
            viewport,
            geometry,
            source_sizes,
            snap_scene,
            tracked_camera,
            item_id: item.id,
        },
        provider,
        deferred_refresh: PreviewRefresh::NONE,
    })
}

fn source_sizes(project: &Project) -> HashMap<uuid::Uuid, GlamVec2> {
    let fallback = GlamVec2::new(
        project.canvas_size.width.max(1) as f32,
        project.canvas_size.height.max(1) as f32,
    );
    project
        .video_tracks
        .iter()
        .flat_map(|track| &track.items)
        .chain(
            project
                .folded_sequences
                .iter()
                .flat_map(|sequence| &sequence.video_tracks)
                .flat_map(|track| &track.items),
        )
        .map(|item| {
            let size = GlamVec2::new(item.source_width as f32, item.source_height as f32);
            (
                item.id,
                if size.min_element() > 0.0 {
                    size
                } else {
                    fallback
                },
            )
        })
        .collect()
}

fn text_source_size(
    text: &crate::project::TextItem,
    evaluation: &VisualEvaluation,
    expression_cache: &RefCell<TransformExpressionCache>,
) -> GlamVec2 {
    let mut expressions = expression_cache.borrow_mut();
    let content = crate::transform_eval::resolve_text(&text.text, evaluation, &mut expressions);
    let font_size =
        crate::transform_eval::resolve_scalar(&text.font_size, evaluation, &mut expressions)
            .max(1.0);
    let font_weight =
        crate::transform_eval::resolve_scalar(&text.font_weight, evaluation, &mut expressions);
    let tracking =
        crate::transform_eval::resolve_scalar(&text.tracking, evaluation, &mut expressions);
    let line_height =
        crate::transform_eval::resolve_scalar(&text.line_height, evaluation, &mut expressions)
            .max(f32::EPSILON);
    crate::video::text_layout::layout(
        text,
        &content,
        font_size,
        font_weight,
        tracking,
        line_height,
        evaluation.local_time(),
    )
    .size
    .max(GlamVec2::ONE)
}

fn add_snap_providers(
    scene: &mut SnapScene,
    project: &Project,
    selected: &ItemAddress,
    position: Time,
    preparation: Preparation<'_>,
    viewport: PreviewViewport,
    source_sizes: &mut HashMap<uuid::Uuid, GlamVec2>,
) {
    let Some(tracks) = project.video_tracks_for_path(selected.sequence_path()) else {
        return;
    };
    for track in tracks.iter().filter(|track| track.enabled) {
        for item in &track.items {
            let key = ItemAddress::Video {
                sequence_path: selected.sequence_path().to_vec(),
                track_id: track.id,
                item_id: item.id,
            };
            if &key == selected {
                continue;
            }
            let Some((start, end)) = project.projected_item_times(&key) else {
                continue;
            };
            if position < start || position >= end {
                continue;
            }
            let Some(sequence_position) = project.timeline_time_to_sequence(&key.track(), position)
            else {
                continue;
            };
            let evaluation = VisualEvaluation::for_item_with_audio(
                project,
                item,
                sequence_position,
                preparation.audio_analysis,
            );
            if !crate::transform_eval::resolve_bool(
                &item.visibility,
                &evaluation,
                &mut preparation.expression_cache.borrow_mut(),
            ) {
                continue;
            }
            if let crate::project::VideoItemContent::Text(text) = &item.content {
                source_sizes.insert(
                    item.id,
                    text_source_size(text, &evaluation, preparation.expression_cache),
                );
            }
            let context = HandlerContext {
                evaluation: &evaluation,
                timeline_position: position,
                expression_cache: preparation.expression_cache,
                viewport,
                geometry: None,
                source_sizes,
                snap_scene: None,
                tracked_camera: None,
                extensions: Some(preparation.extensions),
                item_id: item.id,
            };
            scene.add_provider(item, &context);
        }
    }
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
        let surface = IVec2::new(area.width().max(1), area.height().max(1));
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
        let content_rect = video_content_rect(
            surface.x,
            surface.y,
            project.canvas_size.width,
            project.canvas_size.height,
            padding_px,
        );
        let viewport = PreviewViewport::new(
            GlamVec2::new(
                project.canvas_size.width.max(1) as f32,
                project.canvas_size.height.max(1) as f32,
            ),
            content_rect,
        );
        let stale_provider = controller.context_invalidated
            || controller.provider.as_ref().is_some_and(|prepared| {
                prepared.project_revision != player.revision
                    || prepared.context.timeline_position != position
                    || prepared.context.viewport != viewport
            });
        if stale_provider && controller.sequence == PointerSequence::Idle {
            controller.provider = None;
            controller.context_invalidated = false;
        }
        let background_color = if state.fullscreen {
            FULLSCREEN_BACKGROUND_COLOR
        } else {
            geometry::theme_window_color(area)
        };
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
        let PreviewControllerState {
            provider,
            retiring_provider,
            presented_base_exclusion,
            extensions,
            ..
        } = &mut *controller;
        let result = renderer
            .as_mut()
            .expect("preview renderer was initialized")
            .render(
                surface,
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
                        surface_rect,
                        caption_font_size,
                        caption_background_color,
                        caption_bottom_inset,
                        focused_caption
                            .as_ref()
                            .zip(caption_split_hover)
                            .map(|(address, point)| (address, point, selection_color)),
                    );
                    let canvas_size = GlamVec2::new(
                        project.canvas_size.width.max(1) as f32,
                        project.canvas_size.height.max(1) as f32,
                    );
                    if let Some(guides) = &guides {
                        guides::draw(
                            timeline_painter,
                            guides,
                            canvas_size,
                            content_rect,
                            surface_rect,
                            selection_color,
                        );
                    }
                    if provider.is_none()
                        && let Some(key) = focused_video.as_ref()
                    {
                        *provider = prepare(
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
                                extensions,
                                snap_enabled,
                                snap_radius_px,
                                guides,
                            },
                        );
                    }
                    let current_covers_base = provider.as_ref().is_some_and(|prepared| {
                        prepared.provider.base_frame_exclusion() == *presented_base_exclusion
                    });
                    if !current_covers_base
                        && let Some(prepared) = retiring_provider.as_mut()
                        && prepared.provider.base_frame_exclusion() == *presented_base_exclusion
                    {
                        let context = prepared.context.context(expression_cache, Some(extensions));
                        prepared
                            .provider
                            .on_draw(timeline_painter.canvas(), &context);
                    }
                    let Some(prepared) = provider.as_mut() else {
                        return;
                    };
                    if prepared.provider.base_frame_exclusion() != *presented_base_exclusion {
                        return;
                    }
                    let context = prepared.context.context(expression_cache, Some(extensions));
                    prepared
                        .provider
                        .on_draw(timeline_painter.canvas(), &context);
                },
            );
        let exclusion = provider
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
        let surface_size = IVec2::new(surface.area.width().max(1), surface.area.height().max(1));
        let viewport = PreviewViewport::new(
            GlamVec2::new(
                project.canvas_size.width.max(1) as f32,
                project.canvas_size.height.max(1) as f32,
            ),
            video_content_rect(
                surface_size.x,
                surface_size.y,
                project.canvas_size.width,
                project.canvas_size.height,
                state.padding_px(),
            ),
        );
        let mut controller = surface.controller.borrow_mut();
        let stale = controller.context_invalidated
            || controller.provider.as_ref().is_some_and(|prepared| {
                prepared.project_revision != player.revision
                    || prepared.context.timeline_position != player.position
                    || prepared.context.viewport != viewport
            });
        if stale && controller.sequence == PointerSequence::Idle {
            controller.provider = None;
            controller.context_invalidated = false;
        }
        if controller.provider.is_some() {
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
                extensions: &controller.extensions,
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
    controller.provider = prepared;
    set_base_exclusion(&mut controller, exclusion, player.position);
    drop(controller);
    surface.controller.borrow().provider.is_some()
}

fn dispatch_pointer(surface: ProviderDispatch<'_>, event: PointerEvent<'_>) -> PreviewResponse {
    {
        let mut controller = surface.controller.borrow_mut();
        match event {
            PointerEvent::Begin(_) if controller.sequence != PointerSequence::Idle => {
                return PreviewResponse::IGNORED;
            }
            PointerEvent::Samples { .. } if controller.sequence != PointerSequence::Active => {
                return PreviewResponse::IGNORED;
            }
            PointerEvent::End(_) | PointerEvent::Cancel
                if controller.sequence == PointerSequence::Suppressed =>
            {
                controller.sequence = PointerSequence::Idle;
                if controller.context_invalidated {
                    controller.provider = None;
                    controller.context_invalidated = false;
                }
                return PreviewResponse::IGNORED;
            }
            PointerEvent::End(_) | PointerEvent::Cancel
                if controller.sequence != PointerSequence::Active =>
            {
                return PreviewResponse::IGNORED;
            }
            _ => {}
        }
    }
    if !ensure_provider(&surface) {
        if matches!(event, PointerEvent::Begin(_)) {
            surface.controller.borrow_mut().sequence = PointerSequence::Suppressed;
        }
        return PreviewResponse::IGNORED;
    }
    if matches!(event, PointerEvent::Begin(_)) {
        let mut controller = surface.controller.borrow_mut();
        controller.sequence = PointerSequence::Active;
        controller.live_base_pending = false;
        controller.live_base_in_flight = None;
    }
    let ProviderDispatch {
        area,
        project,
        player_state,
        state,
        controller,
        ..
    } = surface;
    let terminal = matches!(event, PointerEvent::End(_) | PointerEvent::Cancel);
    let mut prepared = controller
        .borrow_mut()
        .provider
        .take()
        .expect("preview provider disappeared during pointer dispatch");
    let mut response = {
        let mut project = project.borrow_mut();
        let state = state.borrow();
        let mut controller = controller.borrow_mut();
        let context = prepared.context.context(&state.expression_cache, None);
        prepared.provider.on_pointer(
            event,
            &context,
            &mut Edits {
                project: &mut project,
                extensions: &mut controller.extensions,
                item: &prepared.item,
                keyframe_time: prepared.context.keyframe_time,
                context,
            },
        )
    };
    if matches!(event, PointerEvent::Cancel) {
        response.edit = response.edit.canceled();
    }
    if response.edit.changed() && !response.edit.commits() && !terminal {
        if response.edit.refresh.contains(PreviewRefresh::PREVIEW) {
            controller.borrow_mut().live_base_pending = true;
        }
        prepared.deferred_refresh |= response.edit.refresh;
        response.edit.refresh = PreviewRefresh::NONE;
    } else if response.edit.commits() || terminal {
        response.edit.refresh |= prepared.deferred_refresh;
        prepared.deferred_refresh = PreviewRefresh::NONE;
        let mut controller = controller.borrow_mut();
        controller.live_base_pending = false;
    }
    let mut controller_state = controller.borrow_mut();
    controller_state.provider = Some(prepared);
    controller_state.frame_pending |= response.redraw;
    if terminal {
        controller_state.sequence = PointerSequence::Idle;
    }
    drop(controller_state);
    apply_response(area, project, player_state, response, "preview-provider");
    if response.edit.commits() {
        let revision = player_state::snapshot(player_state).revision;
        let mut controller = controller.borrow_mut();
        if let Some(provider) = controller.provider.as_mut() {
            provider.project_revision = revision;
            provider.provider.on_project_committed(revision);
            let invalidate = !provider.provider.keeps_frame_until_base();
            controller.context_invalidated = invalidate;
        }
    }
    response
}

fn dispatch_keyboard(surface: ProviderDispatch<'_>, event: KeyboardEvent) -> PreviewResponse {
    if !ensure_provider(&surface) {
        return PreviewResponse::IGNORED;
    }
    let ProviderDispatch {
        area,
        project,
        player_state,
        state,
        controller,
        ..
    } = surface;
    let mut prepared = controller
        .borrow_mut()
        .provider
        .take()
        .expect("preview provider disappeared during keyboard dispatch");
    let response = {
        let mut project = project.borrow_mut();
        let state = state.borrow();
        let mut controller = controller.borrow_mut();
        let context = prepared.context.context(&state.expression_cache, None);
        prepared.provider.on_keyboard(
            event,
            &context,
            &mut Edits {
                project: &mut project,
                extensions: &mut controller.extensions,
                item: &prepared.item,
                keyframe_time: prepared.context.keyframe_time,
                context,
            },
        )
    };
    let mut controller_state = controller.borrow_mut();
    controller_state.provider = Some(prepared);
    controller_state.frame_pending |= response.redraw;
    drop(controller_state);
    apply_response(area, project, player_state, response, "preview-keyboard");
    if response.edit.commits() {
        let revision = player_state::snapshot(player_state).revision;
        let mut controller = controller.borrow_mut();
        if let Some(provider) = controller.provider.as_mut() {
            provider.project_revision = revision;
            provider.provider.on_project_committed(revision);
            let invalidate = !provider.provider.keeps_frame_until_base();
            controller.context_invalidated = invalidate;
        }
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
    let mut controller_state = controller.borrow_mut();
    controller_state.sequence = if controller_state.sequence == PointerSequence::Active {
        PointerSequence::Suppressed
    } else {
        PointerSequence::Idle
    };
    controller_state.context_invalidated = false;
    let Some(mut prepared) = controller_state.provider.take() else {
        let position = player_state::snapshot(player_state).position;
        set_base_exclusion(&mut controller_state, None, position);
        return;
    };
    drop(controller_state);
    let mut response = {
        let mut project = project.borrow_mut();
        let state = state.borrow();
        let mut controller = controller.borrow_mut();
        let context = prepared.context.context(&state.expression_cache, None);
        prepared.provider.on_cancel(
            &context,
            &mut Edits {
                project: &mut project,
                extensions: &mut controller.extensions,
                item: &prepared.item,
                keyframe_time: prepared.context.keyframe_time,
                context,
            },
        )
    };
    response.edit.refresh |= prepared.deferred_refresh;
    response.edit = response.edit.canceled();
    apply_response(area, project, player_state, response, "preview-cancel");
    let position = player_state::snapshot(player_state).position;
    let mut controller = controller.borrow_mut();
    controller.retiring_provider = prepared
        .provider
        .base_frame_exclusion()
        .is_some()
        .then_some(prepared);
    set_base_exclusion(&mut controller, None, position);
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
    let guide_drag = Rc::new(Cell::new(None::<guides::GuideDrag>));
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
    motion.connect_motion(move |source, x, y| {
        let position = GlamVec2::new(x as f32, y as f32);
        if motion_controller.borrow().sequence != PointerSequence::Idle {
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
        if let Some(cursor) =
            guides::hover_cursor(&motion_area, &motion_project, &motion_state, position)
        {
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
            motion_area.set_cursor_from_name(Some(cursor));
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
    motion.connect_leave(move |_| {
        if leave_state
            .borrow_mut()
            .caption_split_hover
            .take()
            .is_some()
        {
            leave_area.queue_render();
        }
        if leave_controller.borrow().sequence != PointerSequence::Idle {
            return;
        }
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
    let begin_guide = guide_drag.clone();
    pointer.connect_down(move |source, x, y| {
        begin_area.grab_focus();
        let button = source.current_button();
        begin_button.set(button);
        let input = pointer_input(source, GlamVec2::new(x as f32, y as f32), button);
        begin_origin.set(Some(input.sample));
        begin_moved.set(false);
        if button == 1
            && let Some(guide) = guides::begin_drag(
                &begin_area,
                &begin_project,
                &begin_state,
                &begin_controller,
                input.sample.position,
            )
        {
            begin_guide.set(Some(guide));
            return;
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
    let update_guide = guide_drag.clone();
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
        if let Some(guide) = update_guide.get() {
            guides::update_drag(
                &update_area,
                &update_project,
                &update_state,
                guide,
                input.sample.position,
            );
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
    let end_guide = guide_drag.clone();
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
        if let Some(guide) = end_guide.take() {
            guides::finish_drag(
                &end_area,
                &end_project,
                &end_state,
                &end_controller,
                guide,
                input.sample.position,
                end_moved.get(),
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
    let cancel_guide = guide_drag;
    pointer.connect_cancel(move |_, _| {
        if let Some(guide) = cancel_guide.take() {
            guides::cancel_drag(&cancel_area, &cancel_project, &cancel_controller, guide);
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
