//! Shared vector drawing boundary; GPU backends supply the destination Skia canvas.
use shrimply_evaluation::{TransformEvaluation, TransformExpressionCache};
use shrimply_math_geometry::ComposedTransform2D;
use shrimply_preview_skia::{CanvasOperation, draw_with_operations};
use shrimply_project::project::{CanvasSize, SkiaDrawingStrategy};
use shrimply_video_modifiers::text_mask::{TextMaskDirection, TextMaskPartialMode};
use skia_safe::{Canvas, Color, PictureRecorder};
use std::rc::Rc;

pub fn sampling(
    configured: shrimply_render_core::VideoSampleMethod,
    content_accurate: bool,
) -> shrimply_render_core::VideoSampleMethod {
    use shrimply_render_core::VideoSampleMethod;
    if content_accurate || configured == VideoSampleMethod::Nearest {
        configured
    } else {
        VideoSampleMethod::Bilinear
    }
}

pub trait GeneratedVisual {
    fn take_error(&self) -> Option<String> {
        None
    }

    fn draw(
        &self,
        canvas: &Canvas,
        evaluation: &TransformEvaluation,
        expressions: &mut TransformExpressionCache,
        path_effect: Option<&skia_safe::PathEffect>,
    );
}

pub struct GeneratedFrame {
    pub visual: Box<dyn GeneratedVisual>,
    pub evaluation: shrimply_evaluation::VisualEvaluation,
    pub operations: Vec<VectorOperation>,
    pub render_size: CanvasSize,
    pub canvas_size: CanvasSize,
    pub drawing_strategy: SkiaDrawingStrategy,
}

impl GeneratedFrame {
    pub fn draw(
        &self,
        canvas: &Canvas,
        expressions: &mut TransformExpressionCache,
    ) -> Result<(), String> {
        self.visual.take_error();
        draw_visual(
            canvas,
            (self.render_size, self.canvas_size),
            &*self.visual,
            &self.operations,
            self.drawing_strategy,
            &self.evaluation,
            expressions,
        );
        self.visual.take_error().map_or(Ok(()), Err)
    }
}

#[derive(Clone)]
pub enum VectorOperation {
    Transform(ComposedTransform2D),
    MotionBlur(Rc<[ComposedTransform2D]>),
    Opacity(f32),
    Hsv {
        hue_turns: f32,
        saturation: f32,
        value: f32,
    },
    Repeat {
        copies_x: u32,
        copies_y: u32,
        step: glam::Vec2,
        row_offset: glam::Vec2,
    },
    ShakyPath {
        amplitude: f32,
        step_size: f32,
        seed: u32,
    },
    TextMask(TextMaskOperation),
}

#[derive(Clone, Copy)]
pub struct TextMaskOperation {
    pub amount: f32,
    pub partial_mode: TextMaskPartialMode,
    pub direction: TextMaskDirection,
}

#[derive(Clone, Copy)]
pub struct GeneratedTransition {
    pub kind: shrimply_project::project::VisualTransitionKind,
    pub side: shrimply_project::project::TransitionSide,
    pub progress: f32,
    pub interpolation: shrimply_project::project::Interpolation,
    pub ordering: shrimply_project::project::WriteOrdering,
    pub drawing_stroke_overlap: f32,
    pub drawing_stroke_length_weight: f32,
    pub drawing_fill_mode: shrimply_project::project::DrawingFillMode,
    pub morph_unit: shrimply_project::project::MorphUnit,
    pub effect_amount: f32,
    pub effect_detail: f32,
    pub effect_angle_degrees: f32,
    pub effect_fade: bool,
    pub effect_seed: u32,
}

pub fn draw_visual(
    canvas: &Canvas,
    canvas_sizes: (CanvasSize, CanvasSize),
    visual: &dyn GeneratedVisual,
    operations: &[VectorOperation],
    drawing_strategy: SkiaDrawingStrategy,
    eval: &TransformEvaluation,
    expression_cache: &mut TransformExpressionCache,
) {
    let (render_size, canvas_size) = canvas_sizes;
    // This project-sized surface is the vector rasterization boundary. All queued affine and
    // transition operations must be applied below while the source is still Skia geometry; the
    // returned texture must not be transformed as a substitute for this step.
    canvas.clear(Color::TRANSPARENT);
    canvas.save();
    canvas.scale((
        render_size.width.max(1) as f32 / canvas_size.width.max(1) as f32,
        render_size.height.max(1) as f32 / canvas_size.height.max(1) as f32,
    ));
    let path_effect = shaky_path_effect(operations);
    let canvas_operations = canvas_operations(operations);
    match drawing_strategy {
        SkiaDrawingStrategy::Immediate => {
            draw_with_operations(canvas, &canvas_operations, |canvas| {
                visual.draw(canvas, eval, expression_cache, path_effect.as_ref());
            });
        }
        SkiaDrawingStrategy::Picture => draw_picture(
            canvas,
            canvas_size.width.max(1),
            canvas_size.height.max(1),
            visual,
            &canvas_operations,
            eval,
            expression_cache,
            path_effect.as_ref(),
        ),
    }
    canvas.restore();
}

fn shaky_path_effect(operations: &[VectorOperation]) -> Option<skia_safe::PathEffect> {
    operations
        .iter()
        .filter_map(|operation| match operation {
            VectorOperation::ShakyPath {
                amplitude,
                step_size,
                seed,
            } => skia_safe::PathEffect::discrete(*step_size, *amplitude, *seed),
            _ => None,
        })
        .reduce(|previous, next| skia_safe::PathEffect::compose(next, previous))
}

fn canvas_operations(operations: &[VectorOperation]) -> Vec<CanvasOperation> {
    operations
        .iter()
        .filter_map(|operation| match operation {
            VectorOperation::Transform(transform) => Some(CanvasOperation::Transform(*transform)),
            VectorOperation::MotionBlur(transforms) => {
                Some(CanvasOperation::MotionBlur(Rc::clone(transforms)))
            }
            VectorOperation::Opacity(opacity) => Some(CanvasOperation::Opacity(*opacity)),
            VectorOperation::Hsv {
                hue_turns,
                saturation,
                value,
            } => Some(CanvasOperation::Hsv {
                hue_turns: *hue_turns,
                saturation: *saturation,
                value: *value,
            }),
            VectorOperation::Repeat {
                copies_x,
                copies_y,
                step,
                row_offset,
            } => Some(CanvasOperation::Repeat {
                copies_x: *copies_x,
                copies_y: *copies_y,
                step: *step,
                row_offset: *row_offset,
            }),
            VectorOperation::ShakyPath { .. } => None,
            VectorOperation::TextMask(_) => {
                unreachable!("text mask operation escaped the text renderer")
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn draw_picture(
    canvas: &Canvas,
    width: u32,
    height: u32,
    visual: &dyn GeneratedVisual,
    operations: &[CanvasOperation],
    eval: &TransformEvaluation,
    expression_cache: &mut TransformExpressionCache,
    path_effect: Option<&skia_safe::PathEffect>,
) {
    let bounds = skia_safe::Rect::from_xywh(0.0, 0.0, width as f32, height as f32);
    let mut recorder = PictureRecorder::new();
    visual.draw(
        recorder.begin_recording(bounds, false),
        eval,
        expression_cache,
        path_effect,
    );
    let picture = recorder
        .finish_recording_as_picture(Some(&bounds))
        .expect("vector recording should contain a canvas");
    draw_with_operations(canvas, operations, |canvas| {
        canvas.draw_picture(&picture, None, None);
    });
}

use shrimply_project::project::{Time, TransitionSide, VideoItem, VisualTransitionKind};

pub fn transition(item: &VideoItem, position: Time, scene_3d: bool) -> Option<GeneratedTransition> {
    (!scene_3d)
        .then(|| crate::transition::active_visual_transition(item, position))
        .flatten()
        .and_then(|(side, transition, _, progress)| {
            matches!(
                transition.kind,
                VisualTransitionKind::Morph
                    | VisualTransitionKind::Write
                    | VisualTransitionKind::Drawing
                    | VisualTransitionKind::Create
                    | VisualTransitionKind::FacetAssembly
                    | VisualTransitionKind::Coalesce
                    | VisualTransitionKind::ContourCurrent
                    | VisualTransitionKind::SoftRefraction
                    | VisualTransitionKind::MorphologicalResolve
                    | VisualTransitionKind::LivingFill
                    | VisualTransitionKind::Diffusion
                    | VisualTransitionKind::ReverseDiffusion
            )
            .then_some(GeneratedTransition {
                kind: transition.kind,
                side,
                progress,
                interpolation: transition.interpolation,
                ordering: transition.write_ordering,
                drawing_stroke_overlap: transition.drawing_stroke_overlap,
                drawing_stroke_length_weight: transition.drawing_stroke_length_weight,
                drawing_fill_mode: transition.drawing_fill_mode,
                morph_unit: transition.morph_unit,
                effect_amount: transition.effect_amount,
                effect_detail: transition.effect_detail,
                effect_angle_degrees: transition.effect_angle_degrees,
                effect_fade: transition.effect_fade,
                effect_seed: if transition.effect_evolve_seed {
                    let start = match side {
                        TransitionSide::Intro => item.start,
                        TransitionSide::Outro => item.end.saturating_sub(transition.duration),
                    };
                    shrimply_math_media::seed_at_frequency(
                        position.saturating_sub(start),
                        transition.effect_seed_frequency,
                    )
                } else {
                    19
                },
            })
        })
}

pub fn render_canvas(
    item: &VideoItem,
    native: CanvasSize,
    evaluation: &shrimply_evaluation::VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> CanvasSize {
    const MAX_RENDER_DIMENSION: f32 = 16_384.0;
    item.modifiers
        .iter()
        .filter(|modifier| modifier.enabled)
        .find_map(|modifier| match &modifier.effect {
            shrimply_video_modifiers::ModifierEffect::Rasterize(effect) => Some(effect),
            _ => None,
        })
        .map(|effect| {
            let size = shrimply_evaluation::resolve_vec2(effect.size(), evaluation, expressions);
            shrimply_project::project::CanvasSize {
                width: if size.x > 0.0 {
                    size.x.round().clamp(1.0, MAX_RENDER_DIMENSION) as u32
                } else {
                    native.width
                },
                height: if size.y > 0.0 {
                    size.y.round().clamp(1.0, MAX_RENDER_DIMENSION) as u32
                } else {
                    native.height
                },
            }
        })
        .unwrap_or_else(|| item.rendered_canvas_size(native))
}
