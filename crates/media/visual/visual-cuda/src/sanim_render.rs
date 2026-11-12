use crate::{
    gpu::CudaVideoCompositor,
    layer::{VectorVisual, Visual, VisualData},
    visual_source::{VisualElement, VisualRender, VisualRenderRequest, VisualSourceCache},
};
use shrimply_project_document::project::{CanvasSize, VideoItem, VideoItemContent};
use shrimply_project_evaluation::VisualEvaluation;
use std::{rc::Rc, sync::Arc};

pub struct SanimElement {
    source: shrimply_visual_core::sanim::Source,
}
impl SanimElement {
    pub fn new(item: &VideoItem) -> Result<Self, String> {
        Ok(Self {
            source: shrimply_visual_core::sanim::Source::new(item)?,
        })
    }
}
impl VisualElement for SanimElement {
    fn set_waker(&mut self, wake: Option<shrimply_visual_core::generated::SourceWake>) {
        self.source.set_waker(wake);
    }
    fn matches(&self, item: &VideoItem, _canvas: CanvasSize) -> bool {
        matches!(item.content, VideoItemContent::Sanim(_))
    }
    fn needs_update(&self) -> bool {
        self.source.needs_update()
    }
    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        _compositor: &mut CudaVideoCompositor,
        _track: uuid::Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let status = self.source.poll(
            request.item,
            request.position,
            request.project.canvas_size,
            request.project.fps,
            request.decode_control.is_none(),
        )?;
        if let Some(error) = self.source.take_error() {
            tracing::warn!(%error,"Sanim reload failed; showing last good animation");
        }
        match status {
            shrimply_visual_core::sanim::Status::Empty => Ok(VisualRender::Empty),
            shrimply_visual_core::sanim::Status::Loading => {
                Ok(VisualRender::Loading(request.render_canvas))
            }
            shrimply_visual_core::sanim::Status::Ready(recording) => {
                let prepared = DeferredSanim {
                    recording,
                    evaluation: VisualEvaluation::for_item_with_audio(
                        request.project,
                        request.item,
                        request.position,
                        request.audio_analysis,
                    ),
                    canvas: request.project.canvas_size,
                };
                Ok(VisualRender::Ready(Visual::Vector(VectorVisual::prepared(
                    Box::new(prepared),
                    request.state,
                ))))
            }
        }
    }
}
struct DeferredSanim {
    recording: Arc<sanim_runtime::sanim_render::Recording>,
    evaluation: VisualEvaluation,
    canvas: CanvasSize,
}
impl VisualData for DeferredSanim {
    fn rasterize(
        &self,
        compositor: &mut CudaVideoCompositor,
        strategy: shrimply_project_document::project::SkiaDrawingStrategy,
        operations: &[crate::layer::VectorOperation],
    ) -> Result<Rc<crate::gpu::VisualFrame>, String> {
        compositor
            .render_generated_visual(
                self.canvas,
                self.canvas,
                &shrimply_visual_core::sanim::RecordedVisual(self.recording.clone()),
                &self.evaluation,
                operations,
                strategy,
            )
            .map(Rc::new)
    }
}
