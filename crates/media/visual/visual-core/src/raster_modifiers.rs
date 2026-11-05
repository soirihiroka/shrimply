//! Ordered raster state changes surrounding the shared pixel kernels.
use shrimply_project_document::project::{ItemAddress, Project, Time, VideoItem};
use shrimply_project_evaluation::{
    TransformExpressionCache, VisualEvaluation, resolve_color, resolve_scalar,
};
use shrimply_render_core::{VideoSampleMethod, effects::PixelEffect};
use shrimply_visual_modifiers::{ModifierEffect, RasterModifierEffect};

pub enum Operation {
    Pixel(PixelEffect),
    Dithering(Dithering),
    Sam2Mask(crate::sam2::ResolvedMask),
    TransparentFillMask(crate::transparent_fill::ResolvedMask),
    Mask(ExternalMask),
    Transform(shrimply_math_geometry::ComposedTransform2D),
    Opacity(f32),
    Sampling(VideoSampleMethod),
    TextureBounds {
        edges: [f32; 4],
        address_mode: shrimply_render_core::TextureAddressMode,
    },
    CropPercentage([f32; 4]),
    CropPixels([f32; 4]),
    RasterBoundary,
}

pub struct Dithering {
    pub pattern: shrimply_render_core::DitheringPattern,
    pub color_mode: shrimply_render_core::DitheringColorMode,
    pub levels: f32,
    pub amount: f32,
    pub palette: Vec<u32>,
}

impl Operation {
    pub fn apply_spatial(&self, state: &mut shrimply_render_core::effects::SpatialState) -> bool {
        match self {
            Self::Transform(transform) => state.transform = transform.matrix * state.transform,
            Self::Opacity(opacity) => state.parameters.opacity *= opacity,
            Self::Sampling(method) => state.parameters.sample_method = *method,
            Self::TextureBounds {
                edges,
                address_mode,
            } => {
                for (current, added) in state.texture_edges.iter_mut().zip(edges) {
                    *current += added;
                }
                state.parameters.address_mode = *address_mode;
            }
            Self::CropPercentage(crop) => {
                (state.modifier_crop, state.modifier_crop_pixels) =
                    shrimply_render_core::math::compose_fractional_crop(
                        state.modifier_crop,
                        state.modifier_crop_pixels,
                        *crop,
                    );
            }
            Self::CropPixels(crop) => {
                for (current, added) in state.modifier_crop_pixels.iter_mut().zip(crop) {
                    *current += added.max(0.0);
                }
            }
            Self::RasterBoundary => {}
            Self::Pixel(_)
            | Self::Dithering(_)
            | Self::Sam2Mask(_)
            | Self::TransparentFillMask(_)
            | Self::Mask(_) => return false,
        }
        true
    }
}

pub struct ExternalMask {
    pub source: Option<ExternalDependency>,
    pub luminance: bool,
    pub invert: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalDependency {
    pub address: ItemAddress,
    pub scope_positions: Vec<Time>,
}

impl ExternalDependency {
    pub fn position(&self) -> Time {
        *self
            .scope_positions
            .last()
            .expect("external dependency has a sequence scope position")
    }
}

pub struct Modifier {
    pub operation: Operation,
    pub alpha_mask: Option<crate::alpha_mask::ResolvedShapeAlphaMask>,
}

pub struct ModifierRequest<'a> {
    pub project: &'a Project,
    pub address: &'a ItemAddress,
    pub item: &'a VideoItem,
    pub position: Time,
    pub scope_positions: &'a [Time],
    pub modifier_index: usize,
    pub require_complete_assets: bool,
}

pub struct ChainRequest<'a> {
    pub project: &'a Project,
    pub address: &'a ItemAddress,
    pub item: &'a VideoItem,
    pub position: Time,
    pub scope_positions: &'a [Time],
    pub require_complete_assets: bool,
}

pub struct ModifierPlan {
    pub source: Vec<usize>,
    pub raster: Vec<usize>,
}

pub fn plan(item: &VideoItem) -> Result<ModifierPlan, String> {
    item.modifier_output_state()?;
    let mut source = Vec::new();
    let mut raster = Vec::new();
    for (index, modifier) in item.modifiers.iter().enumerate() {
        if !modifier.enabled {
            continue;
        }
        match &modifier.effect {
            ModifierEffect::Raster(_) => raster.push(index),
            _ => source.push(index),
        }
    }
    Ok(ModifierPlan { source, raster })
}

/// Resolves the raster portion of a modifier chain after its source renderer has run.
/// Source renderers consume their native stages, such as OBJ scene entries, before
/// producing the RGBA input for this chain.
pub fn after_source(
    request: ChainRequest<'_>,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
    content_accurate: bool,
) -> Result<Vec<Modifier>, String> {
    let mut resolved = Vec::new();
    for modifier_index in plan(request.item)?.raster {
        if let Some(modifier) = modifier(
            ModifierRequest {
                project: request.project,
                address: request.address,
                item: request.item,
                position: request.position,
                scope_positions: request.scope_positions,
                modifier_index,
                require_complete_assets: request.require_complete_assets,
            },
            evaluation,
            expressions,
            content_accurate,
        )? {
            resolved.push(modifier);
        }
    }
    Ok(resolved)
}

pub fn external_dependencies(request: ChainRequest<'_>) -> Result<Vec<ExternalDependency>, String> {
    let mut dependencies = Vec::new();
    for index in plan(request.item)?.raster {
        let modifier = &request.item.modifiers[index];
        let ModifierEffect::Raster(effect) = &modifier.effect else {
            continue;
        };
        let RasterModifierEffect::Mask(mask) = &**effect else {
            continue;
        };
        let Some(source) = external_mask_source(
            request.project,
            request.address,
            request.position,
            request.scope_positions,
            mask.item_id,
        ) else {
            continue;
        };
        if !dependencies.contains(&source) {
            dependencies.push(source);
        }
    }
    Ok(dependencies)
}

fn external_mask_source(
    project: &Project,
    address: &ItemAddress,
    position: Time,
    scope_positions: &[Time],
    item_id: Option<uuid::Uuid>,
) -> Option<ExternalDependency> {
    let item_id = item_id?;
    let tracks = project.video_tracks_for_path(address.sequence_path())?;
    let active =
        crate::sequence::active_tracks(tracks, position, Some(std::slice::from_ref(&item_id)))
            .into_iter()
            .find(|active| active.item.id == item_id)?;
    Some(ExternalDependency {
        address: ItemAddress::Video {
            sequence_path: address.sequence_path().to_vec(),
            track_id: active.track_id,
            item_id,
        },
        scope_positions: scope_positions.to_vec(),
    })
}

pub fn modifier(
    request: ModifierRequest<'_>,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
    content_accurate: bool,
) -> Result<Option<Modifier>, String> {
    let modifier = request
        .item
        .modifiers
        .get(request.modifier_index)
        .ok_or("visual modifier no longer exists")?;
    let effect = match &modifier.effect {
        ModifierEffect::Raster(effect) => effect,
        ModifierEffect::Rasterize(_) => {
            return Ok(Some(Modifier {
                operation: Operation::RasterBoundary,
                alpha_mask: None,
            }));
        }
        _ => return Ok(None),
    };
    let alpha_mask = modifier
        .alpha_mask
        .as_ref()
        .filter(|mask| mask.enabled)
        .map(|mask| crate::alpha_mask::resolve(mask, evaluation, expressions));
    let operation = match &**effect {
        RasterModifierEffect::Sam2(_) => crate::sam2::resolve(
            request.project,
            request.address,
            request.item,
            request.position,
            modifier.id,
            request.modifier_index,
            request.require_complete_assets,
        )?
        .map(Operation::Sam2Mask),
        RasterModifierEffect::TransparentFill(_) => crate::transparent_fill::resolve(
            request.project,
            request.address,
            request.item,
            modifier.id,
            request.modifier_index,
            request.position,
            request.require_complete_assets,
        )?
        .map(Operation::TransparentFillMask),
        RasterModifierEffect::Mask(mask) => Some(Operation::Mask(ExternalMask {
            source: external_mask_source(
                request.project,
                request.address,
                request.position,
                request.scope_positions,
                mask.item_id,
            ),
            luminance: mask.mode.value_at(evaluation.local_time())
                == shrimply_visual_modifiers::mask::MaskMode::Luminance,
            invert: mask.invert,
        })),
        _ => operation(effect, evaluation, expressions, content_accurate)?,
    };
    Ok(operation.map(|operation| Modifier {
        operation,
        alpha_mask,
    }))
}

pub fn sampling(
    effect: &shrimply_visual_modifiers::sampling::SamplingModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
    content_accurate: bool,
) -> VideoSampleMethod {
    crate::generated::sampling(
        shrimply_project_evaluation::resolve(&effect.method, evaluation, expressions),
        content_accurate,
    )
}

pub fn operation(
    effect: &RasterModifierEffect,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
    content_accurate: bool,
) -> Result<Option<Operation>, String> {
    Ok(Some(match effect {
        RasterModifierEffect::Cache(_) => Operation::RasterBoundary,
        RasterModifierEffect::CornerPin(effect) => Operation::Pixel(crate::modifiers::corner_pin(
            effect,
            evaluation,
            expressions,
        )?),
        RasterModifierEffect::Transform(effect) => Operation::Transform(
            crate::vector_modifiers::transform(effect, evaluation, expressions).composed(),
        ),
        RasterModifierEffect::Opacity(effect) => Operation::Opacity(
            crate::vector_modifiers::opacity(effect, evaluation, expressions),
        ),
        RasterModifierEffect::Sampling(effect) => {
            Operation::Sampling(sampling(effect, evaluation, expressions, content_accurate))
        }
        RasterModifierEffect::TextureBounds(effect) => {
            let edges = [
                &effect.edges.top,
                &effect.edges.right,
                &effect.edges.bottom,
                &effect.edges.left,
            ]
            .map(|value| resolve_scalar(value, evaluation, expressions));
            let address_mode = match effect.address_mode.value_at(evaluation.local_time()) {
                shrimply_property_model::TextureAddressMode::Transparent => {
                    shrimply_render_core::TextureAddressMode::Transparent
                }
                shrimply_property_model::TextureAddressMode::ClampToEdge => {
                    shrimply_render_core::TextureAddressMode::ClampToEdge
                }
                shrimply_property_model::TextureAddressMode::Repeat => {
                    shrimply_render_core::TextureAddressMode::Repeat
                }
                shrimply_property_model::TextureAddressMode::MirrorRepeat => {
                    shrimply_render_core::TextureAddressMode::MirrorRepeat
                }
                shrimply_property_model::TextureAddressMode::BlurredMirror => {
                    shrimply_render_core::TextureAddressMode::BlurredMirror
                }
                shrimply_property_model::TextureAddressMode::Stochastic => {
                    shrimply_render_core::TextureAddressMode::Stochastic
                }
            };
            Operation::TextureBounds {
                edges,
                address_mode,
            }
        }
        RasterModifierEffect::Crop(effect) => {
            let (pixels, edges) = match effect {
                shrimply_visual_modifiers::crop::CropModifier::Percentage(edges) => (false, edges),
                shrimply_visual_modifiers::crop::CropModifier::Pixels(edges) => (true, edges),
            };
            let edges = [&edges.top, &edges.right, &edges.bottom, &edges.left]
                .map(|value| resolve_scalar(value, evaluation, expressions));
            if pixels {
                Operation::CropPixels(edges)
            } else {
                Operation::CropPercentage(edges.map(|value| (value / 100.0).clamp(0.0, 0.999_99)))
            }
        }
        RasterModifierEffect::Dithering(effect) => {
            use shrimply_visual_modifiers::dithering::{DitheringColorMode, DitheringPattern};
            Operation::Dithering(Dithering {
                pattern: match effect.pattern.value_at(evaluation.local_time()) {
                    DitheringPattern::Bayer2x2 => shrimply_render_core::DitheringPattern::Bayer2x2,
                    DitheringPattern::Bayer4x4 => shrimply_render_core::DitheringPattern::Bayer4x4,
                    DitheringPattern::Bayer8x8 => shrimply_render_core::DitheringPattern::Bayer8x8,
                },
                color_mode: match effect.color_mode.value_at(evaluation.local_time()) {
                    DitheringColorMode::Color => shrimply_render_core::DitheringColorMode::Color,
                    DitheringColorMode::Grayscale => {
                        shrimply_render_core::DitheringColorMode::Grayscale
                    }
                    DitheringColorMode::Palette => {
                        shrimply_render_core::DitheringColorMode::Palette
                    }
                },
                levels: resolve_scalar(&effect.levels, evaluation, expressions).clamp(2.0, 256.0),
                amount: resolve_scalar(&effect.amount, evaluation, expressions).clamp(0.0, 1.0),
                palette: effect
                    .palette
                    .iter()
                    .map(|color| resolve_color(color, evaluation, expressions).to_rgba_u32())
                    .collect(),
            })
        }
        _ => {
            return Ok(
                crate::modifiers::pixel_effect(effect, evaluation, expressions)
                    .map(Operation::Pixel),
            );
        }
    }))
}
