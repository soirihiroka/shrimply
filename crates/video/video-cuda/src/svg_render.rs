use std::rc::Rc;

use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_gpu_memory::{ResidentResource, ResourceKey, global as gpu_memory};
pub(crate) use shrimply_video_core::svg::PreparedSvg;
use skia_safe::Canvas;
use uuid::Uuid;

use crate::gpu::CudaVideoCompositor;
use crate::gpu::generated_gpu::GeneratedVisual;
use crate::layer::{VectorVisual, Visual, VisualData, VisualState};
use crate::svg_color;
use crate::visual_source::VisualSourceCache;
use crate::visual_source::{GeneratedTransition, VisualElement, VisualRender, VisualRenderRequest};
use shrimply_project::project::{CanvasSize, VideoItem};

pub struct SvgRenderSession {
    file: Asset,
    snapshot: AssetSnapshot,
    source_key: ResourceKey,
    root_width: u32,
    root_height: u32,
    surface_width: u32,
    surface_height: u32,
    color_overrides: Vec<shrimply_project::project::SvgColorOverride>,
}

struct DeferredSvgFrame {
    prepared_svg: ResidentResource<PreparedSvg>,
    root_width: u32,
    root_height: u32,
    surface_width: u32,
    surface_height: u32,
    canvas_size: CanvasSize,
    evaluation: shrimply_evaluation::VisualEvaluation,
    transition: Option<GeneratedTransition>,
}

pub(crate) struct SvgVectorVisualParams {
    pub prepared_svg: ResidentResource<PreparedSvg>,
    pub root_size: CanvasSize,
    pub surface_size: CanvasSize,
    pub canvas_size: CanvasSize,
    pub evaluation: shrimply_evaluation::VisualEvaluation,
    pub transition: Option<GeneratedTransition>,
}

pub(crate) fn svg_vector_visual(
    params: SvgVectorVisualParams,
    state: VisualState,
) -> Result<Visual, String> {
    let SvgVectorVisualParams {
        prepared_svg,
        root_size,
        surface_size,
        canvas_size,
        evaluation,
        transition,
    } = params;
    let frame = Box::new(DeferredSvgFrame {
        prepared_svg,
        root_width: root_size.width,
        root_height: root_size.height,
        surface_width: surface_size.width,
        surface_height: surface_size.height,
        canvas_size,
        evaluation,
        transition,
    });
    Ok(Visual::Vector(VectorVisual::prepared(frame, state)))
}

impl GeneratedVisual for DeferredSvgFrame {
    fn draw(
        &self,
        canvas: &Canvas,
        _evaluation: &shrimply_evaluation::TransformEvaluation,
        _expressions: &mut shrimply_evaluation::TransformExpressionCache,
        path_effect: Option<&skia_safe::PathEffect>,
    ) {
        self.prepared_svg.draw(
            canvas,
            CanvasSize {
                width: self.root_width,
                height: self.root_height,
            },
            self.transition,
            path_effect,
        );
    }
}

impl SvgRenderSession {
    pub fn new(item: &VideoItem, canvas_size: CanvasSize) -> Result<Self, String> {
        let snapshot = item.file.snapshot()?;
        let source = snapshot.read_to_string()?;
        let color_overrides = item.svg_color_overrides.clone();
        let svg = svg_color::apply_overrides(&source, &color_overrides);
        let source_bytes = u64::try_from(svg.len())
            .map_err(|_| format!("SVG {} source size exceeds u64", item.file.display()))?;
        let source_key = svg_source_key(&snapshot, &color_overrides)?;
        gpu_memory().insert_resource(
            source_key.clone(),
            source_bytes,
            PreparedSvg::new(svg).map_err(|error| format!("{} {}", item.file.display(), error))?,
        )?;
        Ok(Self {
            file: item.file.clone(),
            snapshot,
            source_key,
            root_width: item.source_width.max(1),
            root_height: item.source_height.max(1),
            surface_width: canvas_size.width.max(1),
            surface_height: canvas_size.height.max(1),
            color_overrides,
        })
    }

    fn source(&mut self) -> Result<ResidentResource<PreparedSvg>, String> {
        if let Some(source) = gpu_memory().get_resource(&self.source_key)? {
            return Ok(source);
        }
        let source = self.snapshot.read_to_string()?;
        self.snapshot.verify_current()?;
        let svg = svg_color::apply_overrides(&source, &self.color_overrides);
        let source_bytes = u64::try_from(svg.len())
            .map_err(|_| format!("SVG {} source size exceeds u64", self.file.display()))?;
        gpu_memory().insert_resource(
            self.source_key.clone(),
            source_bytes,
            PreparedSvg::new(svg).map_err(|error| format!("{} {}", self.file.display(), error))?,
        )?;
        gpu_memory()
            .get_resource(&self.source_key)?
            .ok_or_else(|| "reconstructed SVG source disappeared".to_string())
    }

    fn matches_item(&self, item: &VideoItem, canvas_size: CanvasSize) -> bool {
        self.file == item.file
            && self.snapshot.is_current()
            && self.root_width == item.source_width.max(1)
            && self.root_height == item.source_height.max(1)
            && self.surface_width == canvas_size.width.max(1)
            && self.surface_height == canvas_size.height.max(1)
            && self.color_overrides == item.svg_color_overrides
    }
}

impl VisualData for DeferredSvgFrame {
    fn morph_scene(&self) -> Option<crate::vector_morph::MorphScene> {
        self.prepared_svg.morph_scene(
            CanvasSize {
                width: self.root_width,
                height: self.root_height,
            },
            self.canvas_size,
            &self.evaluation,
        )
    }

    fn rasterize(
        &self,
        compositor: &mut CudaVideoCompositor,
        drawing_strategy: shrimply_project::project::SkiaDrawingStrategy,
        operations: &[crate::layer::VectorOperation],
    ) -> Result<Rc<crate::gpu::VisualFrame>, String> {
        let frame = Rc::new(compositor.render_generated_visual(
            CanvasSize {
                width: self.surface_width,
                height: self.surface_height,
            },
            self.canvas_size,
            self,
            &self.evaluation,
            operations,
            drawing_strategy,
        )?);
        Ok(frame)
    }
}

impl VisualElement for SvgRenderSession {
    fn matches(&self, item: &VideoItem, canvas_size: CanvasSize) -> bool {
        self.matches_item(item, canvas_size)
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        _compositor: &mut CudaVideoCompositor,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let source = self.source()?;
        let evaluation = shrimply_evaluation::VisualEvaluation::for_item_with_audio(
            request.project,
            request.item,
            request.position,
            request.audio_analysis,
        );
        svg_vector_visual(
            SvgVectorVisualParams {
                prepared_svg: source,
                root_size: CanvasSize {
                    width: self.root_width,
                    height: self.root_height,
                },
                surface_size: CanvasSize {
                    width: self.surface_width,
                    height: self.surface_height,
                },
                canvas_size: request.project.canvas_size,
                evaluation,
                transition: request.generated_transition,
            },
            request.state,
        )
        .map(VisualRender::Ready)
    }
}

fn svg_source_key(
    snapshot: &AssetSnapshot,
    color_overrides: &[shrimply_project::project::SvgColorOverride],
) -> Result<ResourceKey, String> {
    let mut discriminator = b"svg-source\0".to_vec();
    discriminator.extend_from_slice(snapshot.cache_key().as_bytes());
    discriminator.extend_from_slice(
        &serde_json::to_vec(color_overrides)
            .map_err(|error| format!("serialize SVG color overrides: {error}"))?,
    );
    Ok(ResourceKey::new(
        snapshot.path().to_path_buf(),
        discriminator,
    ))
}
