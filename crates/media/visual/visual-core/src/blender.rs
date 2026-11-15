use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_math_core::Fraction;
use shrimply_project_document::project::{
    BlenderPreviewDownsample, BlenderRenderMethod, CanvasSize, VideoItem, VideoItemContent,
    generated_source_time_at,
};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
};

pub struct Frame {
    pub pixels: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub display_scale: glam::Vec2,
}

pub enum Status {
    Empty,
    Loading,
    Ready(Arc<Frame>),
}

pub struct Source {
    file: Asset,
    snapshot: AssetSnapshot,
    scene: String,
    view_layer: String,
    camera: String,
    render_method: BlenderRenderMethod,
    preview_render_method: BlenderRenderMethod,
    preview_downsample: BlenderPreviewDownsample,
    canvas_size: CanvasSize,
    binary: Option<PathBuf>,
    session: SessionState,
    frame: Option<Arc<Frame>>,
    rendered: Option<(Fraction, shrimply_blender_core::RenderMethod, CanvasSize)>,
}

enum SessionState {
    Idle,
    Opening(mpsc::Receiver<Result<shrimply_blender_core::Session, String>>),
    Ready(shrimply_blender_core::Session),
    Failed(String),
}

impl Source {
    pub fn new(item: &VideoItem, canvas_size: CanvasSize) -> Result<Self, String> {
        shrimply_blender_core::require_support()?;
        let VideoItemContent::Blender(blender) = &item.content else {
            return Err("Blender source received a non-Blender visual".into());
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
            binary: shrimply_blender_core::binary(),
            session: SessionState::Idle,
            frame: None,
            rendered: None,
        })
    }

    pub fn poll(
        &mut self,
        item: &VideoItem,
        canvas_size: CanvasSize,
        position: shrimply_math_core::Time,
        content_accurate: bool,
    ) -> Result<Status, String> {
        if !self.matches(item, canvas_size) {
            *self = Self::new(item, canvas_size)?;
        }
        let VideoItemContent::Blender(blender) = &item.content else {
            return Err("Blender source received a non-Blender visual".into());
        };
        let Some(source_time) = generated_source_time_at(item, position) else {
            return Ok(Status::Empty);
        };
        let binary = self
            .binary
            .clone()
            .ok_or("Choose a compatible Blender binary in Preferences")?;
        if matches!(self.session, SessionState::Idle) {
            let blend = self.snapshot.path().to_path_buf();
            let (sender, receiver) = mpsc::channel();
            std::thread::Builder::new()
                .name("blender-startup".into())
                .spawn(move || {
                    let _ = sender.send(shrimply_blender_core::Session::open(&binary, &blend));
                })
                .map_err(|error| format!("Could not start Blender worker: {error}"))?;
            self.session = SessionState::Opening(receiver);
            return Ok(Status::Loading);
        }
        let opened = match &self.session {
            SessionState::Opening(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => return Ok(Status::Loading),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Blender startup worker stopped unexpectedly".into()))
                }
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
            return Err("Blender session entered an invalid state".into());
        };
        let scene = session
            .metadata()
            .scenes
            .iter()
            .find(|scene| blender.scene.is_empty() || scene.name == blender.scene)
            .ok_or_else(|| format!("Blender scene {:?} was not found", blender.scene))?;
        let scene_name = if blender.scene.is_empty() {
            scene.name.clone()
        } else {
            blender.scene.clone()
        };
        let view_layer = if blender.view_layer.is_empty() {
            scene.active_view_layer.clone()
        } else {
            blender.view_layer.clone()
        };
        let camera = if blender.camera.is_empty() {
            scene.active_camera.clone()
        } else {
            blender.camera.clone()
        };
        if view_layer.is_empty() || camera.is_empty() {
            return Err(format!(
                "Blender scene {scene_name:?} has no view layer or camera"
            ));
        }
        let method = match if content_accurate {
            blender.render_method
        } else {
            blender.preview_render_method
        } {
            BlenderRenderMethod::Solid => shrimply_blender_core::RenderMethod::Solid,
            BlenderRenderMethod::MaterialPreview => {
                shrimply_blender_core::RenderMethod::MaterialPreview
            }
            BlenderRenderMethod::SceneRenderer => {
                shrimply_blender_core::RenderMethod::SceneRenderer
            }
        };
        let downsample = if content_accurate {
            1
        } else {
            blender.preview_downsample.factor()
        };
        let render_size = CanvasSize {
            width: (canvas_size.width / downsample).max(1),
            height: (canvas_size.height / downsample).max(1),
        };
        if self.rendered == Some((source_time.seconds, method, render_size))
            && let Some(frame) = &self.frame
        {
            return Ok(Status::Ready(frame.clone()));
        }
        let rendered = match session.render(shrimply_blender_core::RenderRequest {
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
        if rendered.width != render_size.width || rendered.height != render_size.height {
            return Err("Blender returned a frame with the wrong dimensions".into());
        }
        let expected = usize::try_from(u64::from(rendered.width) * u64::from(rendered.height))
            .ok()
            .and_then(|pixels| pixels.checked_mul(size_of::<u32>()))
            .ok_or("Blender frame size overflow")?;
        if rendered.pixels.len() != expected {
            return Err("Blender returned a frame with the wrong byte length".into());
        }
        let frame = Arc::new(Frame {
            pixels: rendered.pixels.into(),
            width: rendered.width,
            height: rendered.height,
            display_scale: glam::Vec2::new(
                canvas_size.width as f32 / rendered.width as f32,
                canvas_size.height as f32 / rendered.height as f32,
            ),
        });
        self.rendered = Some((source_time.seconds, method, render_size));
        self.frame = Some(frame.clone());
        Ok(Status::Ready(frame))
    }

    fn matches(&self, item: &VideoItem, canvas_size: CanvasSize) -> bool {
        let VideoItemContent::Blender(blender) = &item.content else {
            return false;
        };
        self.file == item.file
            && self.snapshot.is_current()
            && self.scene == blender.scene
            && self.view_layer == blender.view_layer
            && self.camera == blender.camera
            && self.render_method == blender.render_method
            && self.preview_render_method == blender.preview_render_method
            && self.preview_downsample == blender.preview_downsample
            && self.canvas_size == canvas_size
            && self.binary == shrimply_blender_core::binary()
    }
}
