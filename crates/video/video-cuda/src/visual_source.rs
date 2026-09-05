use std::path::Path;
use std::rc::Rc;

use hashbrown::{HashMap, hash_map::Entry};
use shrimply_asset::Asset;
use uuid::Uuid;

use crate::background_render::BackgroundElement;
use crate::blender_render::BlenderElement;
use crate::decode::DecodeControl;
use crate::gaussian_render::GaussianElement;
use crate::gpu::{CudaVideoCompositor, VisualFrame};
use crate::image_decode::ImageDecodeSession;
use crate::layer::{Visual, VisualState};
use crate::layered_image_render::{LayeredImageAsset, LayeredImageRenderSession};
use crate::manim_render::ManimElement;
use crate::obj_render::ObjElement;
use crate::paint_render::PaintElement;
use crate::pdf_render::PdfRenderSession;
use crate::shape_render::ShapeElement;
use crate::svg_render::SvgRenderSession;
use crate::text_render::TextElement;
use shrimply_evaluation::{FrameAudioAnalysis, TransformExpressionCache, VisualEvaluation};
use shrimply_project::project::{CanvasSize, Project, Time, VideoItem, VideoItemContent};

pub use shrimply_preview_core::accuracy::{Accuracy, CompositeAccuracy};

#[derive(Clone, Copy)]
pub struct VisualRenderRequest<'a> {
    pub project: &'a Project,
    pub item: &'a VideoItem,
    pub position: Time,
    pub audio_analysis: &'a FrameAudioAnalysis,
    pub state: VisualState,
    pub render_canvas: CanvasSize,
    pub generated_transition: Option<GeneratedTransition>,
    pub accuracy: CompositeAccuracy,
    pub transmission_background: Option<&'a VisualFrame>,
    pub decode_control: Option<&'a DecodeControl>,
}

#[derive(Clone, Copy)]
pub struct VisualPrepareRequest<'a> {
    pub item: &'a VideoItem,
    pub position: Time,
    pub accuracy: CompositeAccuracy,
    pub decode_control: Option<&'a DecodeControl>,
    pub prefetch: bool,
}

pub enum VisualRender {
    Ready(Visual),
    Loading(CanvasSize),
    LoadingPlaceholder(Visual),
    Empty,
    Superseded,
}

pub use shrimply_video_core::generated::GeneratedTransition;

pub struct VisualSourceCache {
    layered_image_files: HashMap<Asset, LayeredImageAsset>,
}

impl Default for VisualSourceCache {
    fn default() -> Self {
        Self {
            layered_image_files: HashMap::new(),
        }
    }
}

impl VisualSourceCache {
    pub(crate) fn layered_image(&mut self, file: &Asset) -> &mut LayeredImageAsset {
        match self.layered_image_files.entry(file.clone()) {
            Entry::Occupied(entry) => {
                shrimply_benchmarking::increment("Layered image source residency / Session hit");
                entry.into_mut()
            }
            Entry::Vacant(entry) => {
                shrimply_benchmarking::increment("Layered image source residency / Session miss");
                entry.insert(LayeredImageAsset::new(file.clone()))
            }
        }
    }

    pub fn retain(
        &mut self,
        _retain_video: impl FnMut(&[Uuid], Uuid, Uuid, u32) -> bool,
        _retain_vector: impl FnMut(&[Uuid], Uuid, Uuid) -> bool,
        mut retain_layered_image: impl FnMut(&Path) -> bool,
    ) {
        self.layered_image_files
            .retain(|file, _| retain_layered_image(file.path()));
    }

    pub fn clear(&mut self) {
        self.layered_image_files.clear();
    }
}

/// A stateful renderer for one visual source. Source selection belongs exclusively to
/// `create_renderer`; the compositor only retains and invokes this interface.
pub trait VisualElement {
    fn matches(&self, item: &VideoItem, canvas_size: CanvasSize) -> bool;

    fn prepare(
        &mut self,
        _request: VisualPrepareRequest<'_>,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<(), String> {
        Ok(())
    }

    fn take_source_duration(&mut self) -> Option<Time> {
        None
    }

    fn take_manim_parameters(
        &mut self,
    ) -> Option<(Vec<shrimply_project::project::ManimParameter>, bool)> {
        None
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        compositor: &mut CudaVideoCompositor,
        track_id: Uuid,
        cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String>;
}

pub fn create_renderer(
    item: &VideoItem,
    canvas_size: CanvasSize,
) -> Result<Box<dyn VisualElement>, String> {
    match &item.content {
        VideoItemContent::Media => Err("media renderer requires a per-track decoder".to_string()),
        VideoItemContent::Image | VideoItemContent::Gif => {
            Ok(Box::new(ImageDecodeSession::new(item)?))
        }
        VideoItemContent::Svg => Ok(Box::new(SvgRenderSession::new(item, canvas_size)?)),
        VideoItemContent::Pdf(_) => Ok(Box::new(PdfRenderSession::new(item)?)),
        VideoItemContent::Manim(_) => Ok(Box::new(ManimElement::new(item, canvas_size)?)),
        VideoItemContent::Blender(_) => Ok(Box::new(BlenderElement::new(item, canvas_size)?)),
        VideoItemContent::LayeredImage(_) => Ok(Box::new(LayeredImageRenderSession::new(item)?)),
        VideoItemContent::Text(_) => Ok(Box::new(TextElement::new(canvas_size))),
        VideoItemContent::Shape(_) => Ok(Box::new(ShapeElement::new(canvas_size))),
        VideoItemContent::Paint(_) => Ok(Box::new(PaintElement::new(item))),
        VideoItemContent::Background(_) => Ok(Box::new(BackgroundElement::new(canvas_size))),
        VideoItemContent::Obj(_) => Ok(Box::new(ObjElement::new(item)?)),
        VideoItemContent::Gaussian(_) => Ok(Box::new(GaussianElement::new(item)?)),
        VideoItemContent::FoldedSequence(_) => {
            Err("folded sequences are rendered by the compositor".to_string())
        }
    }
}

/// Read-only frame evaluation services available while a modifier appends its lazy operation.
pub struct VisualModifierContext<'a> {
    pub project: &'a Project,
    pub item: &'a VideoItem,
    pub position: Time,
    pub accuracy: CompositeAccuracy,
    pub require_complete_assets: bool,
    pub modifier_id: Uuid,
    pub modifier_index: usize,
    pub evaluation: &'a VisualEvaluation,
    pub expressions: &'a mut TransformExpressionCache,
    pub mask_source: Option<Rc<VisualFrame>>,
    pub analysis_cache_key: Option<String>,
}

impl<'a> VisualModifierContext<'a> {
    pub fn new(
        project: &'a Project,
        item: &'a VideoItem,
        position: Time,
        modifier_id: Uuid,
        modifier_index: usize,
        evaluation: &'a VisualEvaluation,
        expressions: &'a mut TransformExpressionCache,
    ) -> Self {
        Self {
            project,
            item,
            position,
            accuracy: CompositeAccuracy::default(),
            require_complete_assets: false,
            modifier_id,
            modifier_index,
            evaluation,
            expressions,
            mask_source: None,
            analysis_cache_key: None,
        }
    }
}
