use shrimply_evaluation::{
    TransformExpressionCache, VisualEvaluation, resolve_scalar, resolve_vec2,
};
use shrimply_project::project::{AlphaMaskShape, VisualAlphaMask};
use shrimply_render_core::ShapeAlphaMaskKind;

#[derive(Clone)]
pub struct ResolvedShapeAlphaMask {
    pub center: glam::Vec2,
    pub size: glam::Vec2,
    pub rotation_degrees: f32,
    pub feather: f32,
    pub rounding: f32,
    pub shape: ShapeAlphaMaskKind,
    pub vertices: Vec<glam::Vec2>,
    pub invert: bool,
}

pub struct Branch {
    pub canvas_to_local: glam::Mat3,
    pub affected_opacity: f32,
}

/// The alpha plane selects another stream of the same source, with identical
/// trim/speed timing. It is sampled in source coordinates before item modifiers.
pub fn video_source(
    item: &shrimply_project::project::VideoItem,
    stream: u32,
    canvas: shrimply_project::project::CanvasSize,
) -> (
    shrimply_project::project::VideoItem,
    shrimply_project::project::CanvasSize,
) {
    let mut source = item.clone();
    source.track_id = stream;
    source.alpha_mask_video = None;
    source.stabilize_video = false;
    let size = shrimply_project::project::CanvasSize {
        width: if source.source_width > 0 {
            source.source_width
        } else {
            canvas.width.max(1)
        },
        height: if source.source_height > 0 {
            source.source_height
        } else {
            canvas.height.max(1)
        },
    };
    (source, size)
}

/// Mask coordinates belong to the original raster. Bake only the modifier's
/// opacity change into its branch; the original layer opacity is applied later.
pub fn branch(
    transform: glam::Mat3,
    original_opacity: f32,
    affected_opacity: f32,
) -> Result<Branch, String> {
    Ok(Branch {
        canvas_to_local: shrimply_render_core::math::inverse_affine(transform)
            .ok_or("alpha mask transform is not invertible")?,
        affected_opacity: if original_opacity.abs() > f32::EPSILON {
            affected_opacity / original_opacity
        } else {
            1.0
        },
    })
}

pub fn resolve(
    mask: &VisualAlphaMask,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> ResolvedShapeAlphaMask {
    let size = resolve_vec2(&mask.size, evaluation, expressions).max(glam::Vec2::ZERO);
    ResolvedShapeAlphaMask {
        center: resolve_vec2(&mask.center, evaluation, expressions),
        size,
        rotation_degrees: resolve_scalar(&mask.rotation_degrees, evaluation, expressions),
        feather: resolve_scalar(&mask.feather, evaluation, expressions).clamp(0.0, 1.0),
        rounding: resolve_scalar(&mask.rounding, evaluation, expressions).clamp(0.0, 1.0),
        shape: match mask.shape {
            AlphaMaskShape::Rectangle => shrimply_render_core::ShapeAlphaMaskKind::Rectangle,
            AlphaMaskShape::Ellipse => shrimply_render_core::ShapeAlphaMaskKind::Ellipse,
            AlphaMaskShape::Polygon => shrimply_render_core::ShapeAlphaMaskKind::Polygon,
        },
        vertices: if mask.shape == AlphaMaskShape::Polygon {
            mask.vertices.iter().map(|point| *point * size).collect()
        } else {
            Vec::new()
        },
        invert: mask.invert,
    }
}
