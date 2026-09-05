use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_math_color::Color;
use shrimply_project::project::{
    BlenderPreviewDownsample, BlenderRenderMethod, CanvasSize, ResolvedTransform, VideoItem,
    VideoItemContent, generated_source_time_at,
};
use uuid::Uuid;

use crate::gpu::{CudaVideoCompositor, VisualFrame};
use crate::layer::{GpuFrame, RasterVisual, Visual};
use crate::visual_source::{VisualElement, VisualRender, VisualRenderRequest, VisualSourceCache};

const LOADING_COLOR: Color<u8> = Color::new(104, 51, 12, 255);
const LOADING_PROGRESS_COLOR: Color<u8> = Color::new(255, 174, 85, 255);

pub struct BlenderElement {
    file: Asset,
    snapshot: AssetSnapshot,
    scene: String,
    view_layer: String,
    camera: String,
    render_method: BlenderRenderMethod,
    preview_render_method: BlenderRenderMethod,
    preview_downsample: BlenderPreviewDownsample,
    canvas_size: CanvasSize,
    binary: Option<std::path::PathBuf>,
    session: SessionState,
    frame: Option<Rc<VisualFrame>>,
    rendered: Option<(
        shrimply_math_core::Fraction,
        shrimply_blender::RenderMethod,
        CanvasSize,
    )>,
}

enum SessionState {
    Idle,
    Opening(Receiver<Result<shrimply_blender::Session, String>>),
    Ready(shrimply_blender::Session),
    Failed(String),
}

impl BlenderElement {
    pub fn new(item: &VideoItem, canvas_size: CanvasSize) -> Result<Self, String> {
        let VideoItemContent::Blender(blender) = &item.content else {
            return Err("Blender renderer received a non-Blender visual".into());
        };
        Ok(Self {
            file: item.file.clone(),
            snapshot: item.file.snapshot()?,
            scene: blender.scene.clone(),
            view_layer: blender.view_layer.clone(),
            camera: blender.camera.clone(),
            render_method: blender.render_method,
            preview_render_method: blender.preview_render_method,
            preview_downsample: blender.preview_downsample,
            canvas_size,
            binary: shrimply_blender::binary(),
            session: SessionState::Idle,
            frame: None,
            rendered: None,
        })
    }

    fn reset(&mut self, item: &VideoItem, canvas_size: CanvasSize) -> Result<(), String> {
        let VideoItemContent::Blender(blender) = &item.content else {
            return Err("Blender renderer received a non-Blender visual".into());
        };
        self.file = item.file.clone();
        self.snapshot = item.file.snapshot()?;
        self.scene.clone_from(&blender.scene);
        self.view_layer.clone_from(&blender.view_layer);
        self.camera.clone_from(&blender.camera);
        self.render_method = blender.render_method;
        self.preview_render_method = blender.preview_render_method;
        self.preview_downsample = blender.preview_downsample;
        self.canvas_size = canvas_size;
        self.binary = shrimply_blender::binary();
        self.session = SessionState::Idle;
        self.frame = None;
        self.rendered = None;
        Ok(())
    }

    fn loading_placeholder(
        &mut self,
        compositor: &mut CudaVideoCompositor,
        state: crate::layer::VisualState,
    ) -> Result<VisualRender, String> {
        if self.frame.is_none() {
            let frame = Rc::new(compositor.allocate_cached_rgba_layer(
                self.canvas_size.width,
                self.canvas_size.height,
                "persistent Blender preview",
            )?);
            let pixels = shrimply_loading_screen::render(
                self.canvas_size.width,
                self.canvas_size.height,
                shrimply_i18n_core::text("Starting Blender…").as_ref(),
                LOADING_COLOR,
                LOADING_PROGRESS_COLOR,
            )?;
            compositor.upload_rgba_layer_into(&frame, &pixels)?;
            self.frame = Some(frame);
        }
        compositor.prepare_host_backed_frame(
            self.frame
                .as_ref()
                .expect("Blender loading screen was initialized"),
            "persistent Blender preview",
        )?;
        Ok(VisualRender::LoadingPlaceholder(Visual::Raster(
            RasterVisual::materialized(
                GpuFrame::Rgba(
                    self.frame
                        .as_ref()
                        .expect("Blender loading screen was initialized")
                        .clone(),
                ),
                state,
            ),
        )))
    }
}

impl VisualElement for BlenderElement {
    fn matches(&self, item: &VideoItem, canvas_size: CanvasSize) -> bool {
        matches!(item.content, VideoItemContent::Blender(_)) && self.canvas_size == canvas_size
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        compositor: &mut CudaVideoCompositor,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let VideoItemContent::Blender(blender) = &request.item.content else {
            return Err("Blender renderer received a non-Blender visual".into());
        };
        if self.file != request.item.file
            || !self.snapshot.is_current()
            || self.scene != blender.scene
            || self.view_layer != blender.view_layer
            || self.camera != blender.camera
            || self.render_method != blender.render_method
            || self.preview_render_method != blender.preview_render_method
            || self.preview_downsample != blender.preview_downsample
            || self.binary != shrimply_blender::binary()
        {
            self.reset(request.item, self.canvas_size)?;
        }
        let Some(source_time) = generated_source_time_at(request.item, request.position) else {
            return Ok(VisualRender::Empty);
        };
        let binary = self
            .binary
            .clone()
            .ok_or_else(|| "Choose a compatible Blender binary in Preferences".to_string())?;
        if matches!(self.session, SessionState::Idle) {
            let blend = self.snapshot.path().to_path_buf();
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                let _ = sender.send(shrimply_blender::Session::open(&binary, &blend));
            });
            self.session = SessionState::Opening(receiver);
            return self.loading_placeholder(compositor, request.state);
        }
        let opened = match &self.session {
            SessionState::Opening(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => {
                    return self.loading_placeholder(compositor, request.state);
                }
                Err(TryRecvError::Disconnected) => Some(Err(
                    "Blender startup worker stopped unexpectedly".to_string(),
                )),
            },
            _ => None,
        };
        if let Some(opened) = opened {
            self.session = match opened {
                Ok(session) => SessionState::Ready(session),
                Err(error) => SessionState::Failed(error),
            };
        }
        if let SessionState::Failed(error) = &self.session {
            return Err(error.clone());
        }
        let SessionState::Ready(session) = &mut self.session else {
            return Err("Blender session entered an invalid state".to_string());
        };
        let scene = session
            .metadata()
            .scenes
            .iter()
            .find(|scene| self.scene.is_empty() || scene.name == self.scene)
            .ok_or_else(|| format!("Blender scene {:?} was not found", self.scene))?;
        let scene_name = if self.scene.is_empty() {
            scene.name.clone()
        } else {
            self.scene.clone()
        };
        let view_layer = if self.view_layer.is_empty() {
            scene.active_view_layer.clone()
        } else {
            self.view_layer.clone()
        };
        let camera = if self.camera.is_empty() {
            scene.active_camera.clone()
        } else {
            self.camera.clone()
        };
        if view_layer.is_empty() || camera.is_empty() {
            return Err(format!(
                "Blender scene {scene_name:?} has no view layer or camera"
            ));
        }
        let method = match if request.accuracy.content_accurate() {
            self.render_method
        } else {
            self.preview_render_method
        } {
            BlenderRenderMethod::Solid => shrimply_blender::RenderMethod::Solid,
            BlenderRenderMethod::MaterialPreview => shrimply_blender::RenderMethod::MaterialPreview,
            BlenderRenderMethod::SceneRenderer => shrimply_blender::RenderMethod::SceneRenderer,
        };
        let downsample = if request.accuracy.content_accurate() {
            1
        } else {
            self.preview_downsample.factor()
        };
        let render_size = CanvasSize {
            width: (self.canvas_size.width / downsample).max(1),
            height: (self.canvas_size.height / downsample).max(1),
        };
        let mut state = request.state;
        if render_size != self.canvas_size {
            state.transform = state.transform.compose(
                ResolvedTransform {
                    scale: glam::Vec2::new(
                        self.canvas_size.width as f32 / render_size.width as f32,
                        self.canvas_size.height as f32 / render_size.height as f32,
                    ),
                    ..ResolvedTransform::IDENTITY
                }
                .composed(),
            );
        }
        if self.rendered == Some((source_time.seconds, method, render_size))
            && let Some(frame) = &self.frame
        {
            compositor.prepare_host_backed_frame(frame, "persistent Blender preview")?;
            return Ok(VisualRender::Ready(Visual::Raster(
                RasterVisual::materialized(GpuFrame::Rgba(frame.clone()), state),
            )));
        }
        let rendered = match session.render(shrimply_blender::RenderRequest {
            scene: &scene_name,
            view_layer: &view_layer,
            camera: &camera,
            method,
            width: render_size.width,
            height: render_size.height,
            time: source_time.seconds,
        }) {
            Ok(rendered) => rendered,
            Err(error) => {
                self.session = SessionState::Idle;
                self.frame = None;
                self.rendered = None;
                return Err(error);
            }
        };
        if self.frame.as_ref().is_none_or(|frame| {
            frame.width() != render_size.width || frame.height() != render_size.height
        }) {
            self.frame = Some(Rc::new(compositor.allocate_cached_rgba_layer(
                render_size.width,
                render_size.height,
                "persistent Blender preview",
            )?));
        }
        if rendered.width != render_size.width || rendered.height != render_size.height {
            return Err("Blender returned a frame with the wrong dimensions".into());
        }
        let frame = self.frame.as_ref().expect("Blender frame was allocated");
        compositor.prepare_host_backed_frame(frame, "persistent Blender preview")?;
        compositor.upload_blender_frame(frame, &rendered.pixels)?;
        self.rendered = Some((source_time.seconds, method, render_size));
        Ok(VisualRender::Ready(Visual::Raster(
            RasterVisual::materialized(
                GpuFrame::Rgba(
                    self.frame
                        .as_ref()
                        .expect("Blender frame was allocated")
                        .clone(),
                ),
                state,
            ),
        )))
    }
}
