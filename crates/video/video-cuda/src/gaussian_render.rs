use std::rc::Rc;

use shrimply_project::project::{CanvasSize, VideoItem, VideoItemContent};
use uuid::Uuid;

use crate::gpu::CudaVideoCompositor;
use crate::layer::{GpuFrame, RasterVisual, Visual};
use crate::visual_source::{VisualElement, VisualRender, VisualRenderRequest, VisualSourceCache};

pub struct GaussianElement {
    session: shrimply_3dgs::RenderSession,
    expressions: shrimply_evaluation::TransformExpressionCache,
    cached: Option<CachedFrame>,
}

struct CachedFrame {
    renderer_generation: u64,
    width: u32,
    height: u32,
    params: shrimply_3dgs::RenderParams,
    layer: Rc<crate::gpu::VisualFrame>,
}

impl GaussianElement {
    pub fn new(item: &VideoItem) -> Result<Self, String> {
        Ok(Self {
            session: shrimply_3dgs::RenderSession::load(&item.file)
                .map_err(|error| error.to_string())?,
            expressions: Default::default(),
            cached: None,
        })
    }
}

impl VisualElement for GaussianElement {
    fn matches(&self, item: &VideoItem, _canvas_size: CanvasSize) -> bool {
        matches!(&item.content, VideoItemContent::Gaussian(_))
            && self.session.matches_asset(&item.file).unwrap_or(false)
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        compositor: &mut CudaVideoCompositor,
        track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let VideoItemContent::Gaussian(scene) = &request.item.content else {
            return Err("3DGS renderer received a non-3DGS visual".to_string());
        };
        let evaluation = shrimply_evaluation::VisualEvaluation::for_item_with_audio(
            request.project,
            request.item,
            request.position,
            request.audio_analysis,
        );
        let mut params =
            shrimply_evaluation::resolve_gaussian_scene(scene, &evaluation, &mut self.expressions);
        if let shrimply_3dgs::CameraSource::Tracking(source) = &scene.camera.source
            && source.track_id != track_id
            && request
                .project
                .video_tracks
                .iter()
                .any(|track| track.id == source.track_id)
            && let Some(camera) = crate::camera_reconstruction::sample(
                request.item.id,
                source,
                request
                    .position
                    .signed_sub(request.item.start)
                    .saturating_add(request.item.animation_time_offset),
            )
        {
            let camera = crate::camera_reconstruction::apply_custom_camera_offset(
                camera,
                params.camera.position,
                params.camera.rotation_degrees,
            );
            params.camera.position = camera.position;
            params.camera.rotation_degrees = shrimply_transform_3d::rotation_degrees(
                camera.rotation,
                shrimply_transform_3d::RotationOrder::Xyz,
            );
            params.camera.projection = camera.projection;
            params.camera.vertical_fov_degrees = camera.vertical_fov_degrees;
        }
        let canvas_size = request.render_canvas;
        let width = canvas_size.width.max(1);
        let height = canvas_size.height.max(1);
        let layer = if let Some(cached) = self.cached.as_ref().filter(|cached| {
            cached.renderer_generation == compositor.generated_renderer_generation()
                && cached.width == width
                && cached.height == height
                && cached.params == params
        }) {
            cached.layer.clone()
        } else {
            self.cached = None;
            let layer =
                Rc::new(compositor.render_gaussian_3d(&self.session, width, height, &params)?);
            self.cached = Some(CachedFrame {
                renderer_generation: compositor.generated_renderer_generation(),
                width,
                height,
                params,
                layer: layer.clone(),
            });
            layer
        };
        Ok(VisualRender::Ready(Visual::Raster(
            RasterVisual::materialized(GpuFrame::Rgba(layer), request.state.baked()),
        )))
    }
}
