use crate::gpu::CudaVideoCompositor;
use crate::layer::{VectorVisual, Visual, VisualData};
use crate::visual_source::{VisualElement, VisualRender, VisualRenderRequest, VisualSourceCache};
use shrimply_evaluation::TransformExpressionCache;
use shrimply_project::project::{CanvasSize, VideoItem};
pub use shrimply_video_core::text::decoration_outset;
use std::rc::Rc;
use uuid::Uuid;

pub struct TextElement {
    canvas_size: CanvasSize,
    expressions: TransformExpressionCache,
}

impl TextElement {
    pub fn new(canvas_size: CanvasSize) -> Self {
        Self {
            canvas_size,
            expressions: Default::default(),
        }
    }
}

impl VisualElement for TextElement {
    fn matches(&self, item: &VideoItem, canvas_size: CanvasSize) -> bool {
        self.canvas_size == canvas_size
            && matches!(
                &item.content,
                shrimply_project::project::VideoItemContent::Text(_)
            )
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
        let evaluation = shrimply_evaluation::VisualEvaluation::for_item_at_local_time_with_audio(
            request.project,
            request.item,
            request.position,
            local_time,
            request.audio_analysis,
        );
        let prepared = shrimply_video_core::text::prepare(
            request.project.canvas_size,
            self.canvas_size,
            request.item,
            evaluation,
            request.generated_transition,
            &mut self.expressions,
        );
        let mut state = request.state;
        state.transform = state.transform.compose(prepared.source_offset);
        Ok(VisualRender::Ready(Visual::Vector(VectorVisual::prepared(
            Box::new(DeferredTextFrame(prepared)),
            state,
        ))))
    }
}

struct DeferredTextFrame(shrimply_video_core::text::PreparedText);

impl VisualData for DeferredTextFrame {
    fn morph_scene(&self) -> Option<crate::vector_morph::MorphScene> {
        self.0.morph_scene()
    }

    fn rasterize(
        &self,
        compositor: &mut CudaVideoCompositor,
        drawing_strategy: shrimply_project::project::SkiaDrawingStrategy,
        operations: &[crate::layer::VectorOperation],
    ) -> Result<Rc<crate::gpu::VisualFrame>, String> {
        let mut operations = operations.to_vec();
        let masks = shrimply_video_core::text::take_masks(&mut operations);
        compositor
            .render_generated_visual(
                self.0.surface_size,
                self.0.canvas_size,
                &shrimply_video_core::text::MaskedTextFrame {
                    frame: &self.0,
                    masks: &masks,
                },
                &self.0.evaluation,
                &operations,
                drawing_strategy,
            )
            .map(Rc::new)
    }
}
