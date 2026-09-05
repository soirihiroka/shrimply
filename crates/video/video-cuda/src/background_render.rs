use std::rc::Rc;

use shrimply_evaluation::{TransformExpressionCache, VisualEvaluation};
use shrimply_project::project::{CanvasSize, VideoItem, VideoItemContent};
use uuid::Uuid;

use crate::gpu::CudaVideoCompositor;
use crate::layer::{GpuFrame, RasterVisual, Visual};
use crate::visual_source::{VisualElement, VisualRender, VisualRenderRequest, VisualSourceCache};

pub struct BackgroundElement {
    canvas_size: CanvasSize,
    expressions: TransformExpressionCache,
}

impl BackgroundElement {
    pub fn new(canvas_size: CanvasSize) -> Self {
        Self {
            canvas_size,
            expressions: Default::default(),
        }
    }
}

impl VisualElement for BackgroundElement {
    fn matches(&self, item: &VideoItem, canvas_size: CanvasSize) -> bool {
        self.canvas_size == canvas_size && matches!(&item.content, VideoItemContent::Background(_))
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        compositor: &mut CudaVideoCompositor,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let Some(local_time) =
            shrimply_project::project::generated_item_time(request.item, request.position)
        else {
            return Ok(VisualRender::Empty);
        };
        let VideoItemContent::Background(background) = &request.item.content else {
            return Err("background renderer received a non-background visual".to_string());
        };
        let evaluation = VisualEvaluation::for_item_with_audio(
            request.project,
            request.item,
            request.position,
            request.audio_analysis,
        );
        let background = shrimply_video_core::background::resolve(
            background,
            &evaluation,
            &mut self.expressions,
        );
        let canvas = request.render_canvas;
        let layer = Rc::new(compositor.render_background(
            canvas.width.max(1),
            canvas.height.max(1),
            local_time,
            &background,
        )?);
        Ok(VisualRender::Ready(Visual::Raster(
            RasterVisual::materialized(GpuFrame::Rgba(layer), request.state.baked()),
        )))
    }
}
