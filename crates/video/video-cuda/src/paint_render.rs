use std::cell::RefCell;
use std::rc::Rc;

use shrimply_evaluation::{TransformExpressionCache, VisualEvaluation};
use shrimply_project::project::{CanvasSize, VideoItem, VideoItemContent};
use shrimply_video_core::generated::GeneratedVisual;
use uuid::Uuid;

use crate::gpu::CudaVideoCompositor;

use crate::layer::{VectorVisual, Visual, VisualData};
use crate::visual_source::{VisualElement, VisualRender, VisualRenderRequest, VisualSourceCache};

pub(crate) struct PaintElement {
    item_id: Uuid,
    expressions: TransformExpressionCache,
    cache: Rc<RefCell<shrimply_paint_skia::PaintCache>>,
}

impl PaintElement {
    pub(crate) fn new(item: &VideoItem) -> Self {
        Self {
            item_id: item.id,
            expressions: TransformExpressionCache::default(),
            cache: Rc::new(RefCell::new(shrimply_paint_skia::PaintCache::default())),
        }
    }
}

impl VisualElement for PaintElement {
    fn matches(&self, item: &VideoItem, _canvas_size: CanvasSize) -> bool {
        self.item_id == item.id && matches!(&item.content, VideoItemContent::Paint(_))
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        _compositor: &mut CudaVideoCompositor,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let Some(local_time) =
            shrimply_project::project::generated_item_time(request.item, request.position)
        else {
            return Ok(VisualRender::Empty);
        };
        let VideoItemContent::Paint(_) = &request.item.content else {
            return Err("paint renderer received a non-paint visual".to_string());
        };
        let evaluation = VisualEvaluation::for_item_at_local_time_with_audio(
            request.project,
            request.item,
            request.position,
            local_time,
            request.audio_analysis,
        );
        let prepared = shrimply_video_core::paint::prepare(
            request.project.canvas_size,
            request.render_canvas,
            request.item,
            evaluation,
            request.generated_transition,
            &mut self.expressions,
            Rc::clone(&self.cache),
        )?;
        Ok(VisualRender::Ready(Visual::Vector(VectorVisual::prepared(
            Box::new(DeferredPaintFrame(prepared)),
            request.state,
        ))))
    }
}

struct DeferredPaintFrame(shrimply_video_core::paint::PreparedPaint);
impl VisualData for DeferredPaintFrame {
    fn morph_scene(&self) -> Option<crate::vector_morph::MorphScene> {
        self.0.morph_scene()
    }

    fn rasterize(
        &self,
        compositor: &mut CudaVideoCompositor,
        drawing_strategy: shrimply_project::project::SkiaDrawingStrategy,
        operations: &[crate::layer::VectorOperation],
    ) -> Result<Rc<crate::gpu::VisualFrame>, String> {
        self.0.take_error();
        let rendered = compositor.render_generated_visual(
            self.0.surface_size,
            self.0.canvas_size,
            &self.0,
            &self.0.evaluation,
            operations,
            drawing_strategy,
        );
        if let Some(error) = self.0.take_error() {
            return Err(error);
        }
        rendered.map(Rc::new)
    }
}
