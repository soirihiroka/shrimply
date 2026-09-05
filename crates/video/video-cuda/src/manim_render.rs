use std::rc::Rc;
use std::sync::Arc;

use shrimply_manim_parser::Progress;
use shrimply_math_core::Fraction;
use shrimply_project::project::{CanvasSize, VideoItem, VideoItemContent};
use uuid::Uuid;

use crate::gpu::{CudaVideoCompositor, VisualFrame};
use crate::layer::{GpuFrame, RasterVisual, Visual};
use crate::visual_source::{VisualElement, VisualRender, VisualRenderRequest, VisualSourceCache};

pub struct ManimElement {
    item_id: Uuid,
    source: shrimply_manim_wgpu::Source,
    canvas_size: CanvasSize,
    render_slot: Arc<()>,
    frame: Option<Rc<VisualFrame>>,
    rendered_frame: Option<usize>,
    placeholder_progress: Option<Progress>,
    first_frame_reported: bool,
}

impl ManimElement {
    pub fn new(item: &VideoItem, canvas_size: CanvasSize) -> Result<Self, String> {
        Ok(Self {
            item_id: item.id,
            source: shrimply_manim_wgpu::Source::new(item, canvas_size, item.playback_fps)?,
            canvas_size,
            render_slot: Arc::new(()),
            frame: None,
            rendered_frame: None,
            placeholder_progress: None,
            first_frame_reported: false,
        })
    }

    fn loading_placeholder(
        &mut self,
        compositor: &mut CudaVideoCompositor,
        state: crate::layer::VisualState,
        progress: Option<Progress>,
    ) -> Result<VisualRender, String> {
        let refresh = self.frame.is_none() || self.placeholder_progress != progress;
        if self.frame.is_none() {
            self.frame = Some(Rc::new(compositor.allocate_cached_rgba_layer(
                self.canvas_size.width,
                self.canvas_size.height,
                "persistent Manim output",
            )?));
        }
        let frame = self
            .frame
            .as_ref()
            .expect("Manim preview image was allocated")
            .clone();
        compositor.prepare_host_backed_frame(&frame, "persistent Manim output")?;
        if refresh {
            let pixels = shrimply_manim_wgpu::loading_pixels(
                self.canvas_size.width,
                self.canvas_size.height,
                progress,
            )?;
            compositor.upload_rgba_layer_into(&frame, &pixels)?;
            self.rendered_frame = None;
            self.placeholder_progress = progress;
        }
        Ok(VisualRender::LoadingPlaceholder(Visual::Raster(
            RasterVisual::materialized(GpuFrame::Rgba(frame), state),
        )))
    }
}

impl VisualElement for ManimElement {
    fn matches(&self, item: &VideoItem, canvas_size: CanvasSize) -> bool {
        matches!(item.content, VideoItemContent::Manim(_)) && self.canvas_size == canvas_size
    }

    fn take_manim_updates(&mut self) -> Vec<shrimply_state::manim_status::Update> {
        let source_revision = self.source.source_revision();
        let scene = self.source.scene().to_string();
        let input_parameters = self.source.input_parameters().clone();
        let mut updates = Vec::new();
        if let Some(duration) = self.source.take_duration() {
            updates.push(shrimply_state::manim_status::Update::Duration {
                item_id: self.item_id,
                source_revision,
                scene: scene.clone(),
                input_parameters: input_parameters.clone(),
                duration,
            });
        }
        if let Some((parameters, render_is_current)) = self.source.take_parameters() {
            updates.push(shrimply_state::manim_status::Update::Parameters {
                item_id: self.item_id,
                source_revision,
                scene,
                input_parameters,
                parameters,
                render_is_current,
            });
        }
        updates
    }

    fn manim_status(&self, error: Option<String>) -> Option<shrimply_state::manim_status::Update> {
        Some(shrimply_state::manim_status::Update::Error {
            item_id: self.item_id,
            source_revision: self.source.source_revision(),
            scene: self.source.scene().to_string(),
            input_parameters: self.source.input_parameters().clone(),
            error,
        })
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        compositor: &mut CudaVideoCompositor,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let VideoItemContent::Manim(manim) = &request.item.content else {
            return Err("Manim renderer received a non-Manim visual".into());
        };
        if shrimply_project::project::generated_source_time_at(request.item, request.position)
            .is_none()
        {
            return Ok(VisualRender::Empty);
        }
        let fps: Fraction =
            if request.item.playback_fps == shrimply_project::project::native_playback_fps() {
                request.project.fps
            } else {
                request.item.playback_fps
            };
        let compiled =
            match self
                .source
                .poll(request.item, self.canvas_size, fps, request.position)?
            {
                Ok(frame) => frame,
                Err(shrimply_manim_wgpu::SourceStatus::Loading { progress, changed }) => {
                    if changed && progress.is_none() {
                        self.rendered_frame = None;
                        self.first_frame_reported = false;
                    }
                    return self.loading_placeholder(compositor, request.state, progress);
                }
                Err(shrimply_manim_wgpu::SourceStatus::NeedsParameters) => {
                    return self.loading_placeholder(compositor, request.state, None);
                }
            };
        let render_started = std::time::Instant::now();
        let frame = self
            .frame
            .as_ref()
            .expect("Manim preview image was allocated while loading");
        compositor.prepare_host_backed_frame(frame, "persistent Manim output")?;
        if self.rendered_frame != Some(compiled.frame_index) {
            let _measurement = shrimply_benchmarking::measure("Manim / WGPU");
            compositor.render_manim(
                &self.render_slot,
                &compiled.prepared,
                compiled.frame_index,
                frame,
            )?;
            self.rendered_frame = Some(compiled.frame_index);
        }
        if !self.first_frame_reported {
            tracing::info!(
                source = %request.item.file.path().display(),
                scene = %manim.scene,
                frame = compiled.frame_index,
                elapsed_ms = render_started.elapsed().as_millis(),
                "Manim first frame displayed",
            );
            self.first_frame_reported = true;
        }
        Ok(VisualRender::Ready(Visual::Raster(
            RasterVisual::materialized(GpuFrame::Rgba(frame.clone()), request.state),
        )))
    }
}
