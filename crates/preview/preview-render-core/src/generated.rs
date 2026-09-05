use super::*;
use shrimply_math_geometry::ComposedTransform2D;
use shrimply_project::project::{CanvasSize, VideoItem};
use shrimply_video_core::generated::{
    GeneratedFrame, GeneratedVisual, TextMaskOperation, VectorOperation,
};
use shrimply_video_modifiers::ModifierEffect;

pub(super) struct PreparedVector {
    pub frame: GeneratedFrame,
    pub is_vector: bool,
    pub sample_method: shrimply_render_core::VideoSampleMethod,
    pub effects: Vec<shrimply_video_core::raster_modifiers::Operation>,
}

struct TextSource {
    text: shrimply_video_core::text::PreparedText,
    masks: Vec<TextMaskOperation>,
}

struct SvgSource {
    svg: std::sync::Arc<shrimply_video_core::svg::PreparedSvg>,
    root_size: CanvasSize,
    transition: Option<shrimply_video_core::generated::GeneratedTransition>,
}

impl GeneratedVisual for SvgSource {
    fn draw(
        &self,
        canvas: &skia_safe::Canvas,
        _evaluation: &VisualEvaluation,
        _expressions: &mut TransformExpressionCache,
        path_effect: Option<&skia_safe::PathEffect>,
    ) {
        self.svg
            .draw(canvas, self.root_size, self.transition, path_effect);
    }
}

impl GeneratedVisual for TextSource {
    fn draw(
        &self,
        canvas: &skia_safe::Canvas,
        _evaluation: &VisualEvaluation,
        _expressions: &mut TransformExpressionCache,
        path_effect: Option<&skia_safe::PathEffect>,
    ) {
        self.text.draw_with_masks(canvas, path_effect, &self.masks);
    }
}

impl Scene {
    pub(super) fn vector(
        &mut self,
        item: &VideoItem,
        evaluation: VisualEvaluation,
        native: CanvasSize,
        transform: ComposedTransform2D,
        transition: Option<shrimply_video_core::generated::GeneratedTransition>,
        svg: Option<std::sync::Arc<shrimply_video_core::svg::PreparedSvg>>,
    ) -> Result<PreparedVector, String> {
        let render_size = shrimply_video_core::generated::render_canvas(
            item,
            native,
            &evaluation,
            &mut self.expressions,
        );
        let mut operations = Vec::new();
        let mut effects = Vec::new();
        let mut is_vector = true;
        let mut sample_method = item.sample_method.value_at(evaluation.local_time());
        for modifier in item.modifiers.iter().filter(|modifier| modifier.enabled) {
            if modifier.alpha_mask.is_some() {
                return Err("Vector modifier masks are not yet connected to Metal".into());
            }
            match &modifier.effect {
                ModifierEffect::Vector(effect) if is_vector => {
                    operations.extend(shrimply_video_core::vector_modifiers::operation(
                        effect,
                        &evaluation,
                        &mut self.expressions,
                    ));
                }
                ModifierEffect::Rasterize(effect) if is_vector => {
                    is_vector = false;
                    sample_method = effect.sample_method.value_at(evaluation.local_time());
                }
                ModifierEffect::Raster(effect) if !is_vector => {
                    effects.push(
                        shrimply_video_core::raster_modifiers::operation(
                            effect,
                            &evaluation,
                            &mut self.expressions,
                            self.requested_accuracy.content_accurate(),
                        )?
                        .ok_or("Vector source's raster modifier is not yet connected to Metal")?,
                    );
                }
                _ => {
                    return Err(
                        "Unsupported generated modifier or invalid vector/raster modifier order"
                            .into(),
                    );
                }
            }
        }
        let masks = shrimply_video_core::text::take_masks(&mut operations);
        let (visual, source_offset): (Box<dyn GeneratedVisual>, _) = match item.content {
            VideoItemContent::Shape(_) => {
                if !masks.is_empty() {
                    return Err("TextMask requires a text source".into());
                }
                let shape = shrimply_video_core::shape::prepare(
                    native,
                    render_size,
                    item,
                    evaluation.clone(),
                    transition,
                    &mut self.expressions,
                );
                let offset = ComposedTransform2D {
                    matrix: glam::Mat3::from_translation(-shape.content_offset),
                };
                (Box::new(shape), offset)
            }
            VideoItemContent::Text(_) => {
                let text = shrimply_video_core::text::prepare(
                    native,
                    render_size,
                    item,
                    evaluation.clone(),
                    transition,
                    &mut self.expressions,
                );
                let offset = text.source_offset;
                (Box::new(TextSource { text, masks }), offset)
            }
            VideoItemContent::Paint(_) => {
                if !masks.is_empty() {
                    return Err("TextMask requires a text source".into());
                }
                let paint = shrimply_video_core::paint::prepare(
                    native,
                    render_size,
                    item,
                    evaluation.clone(),
                    transition,
                    &mut self.expressions,
                    self.paint_caches.entry(item.id).or_default().clone(),
                )?;
                (
                    Box::new(paint),
                    ComposedTransform2D {
                        matrix: glam::Mat3::IDENTITY,
                    },
                )
            }
            VideoItemContent::Svg => {
                if !masks.is_empty() {
                    return Err("TextMask requires a text source".into());
                }
                (
                    Box::new(SvgSource {
                        svg: svg.ok_or("SVG source was not prepared")?,
                        root_size: CanvasSize {
                            width: item.source_width.max(1),
                            height: item.source_height.max(1),
                        },
                        transition,
                    }),
                    ComposedTransform2D {
                        matrix: glam::Mat3::IDENTITY,
                    },
                )
            }
            _ => return Err("Unsupported generated vector source".into()),
        };
        operations.insert(
            0,
            VectorOperation::Transform(transform.compose(source_offset)),
        );
        Ok(PreparedVector {
            frame: GeneratedFrame {
                visual,
                evaluation,
                operations,
                render_size,
                canvas_size: native,
                drawing_strategy: item.skia_drawing_strategy,
            },
            is_vector,
            sample_method,
            effects,
        })
    }
}
