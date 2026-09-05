mod alpha_outline;
mod bulge_pinch;
mod channel_mixer;
mod chroma_key;
mod chromatic_aberration;
mod color_correction;
mod colorize_duotone;
mod corner_pin;
mod crop;
mod directional_blur;
mod displacement_map;
mod dithering;
mod drop_shadow;
mod edge_detection;
mod emboss;
mod erode_dilate;
mod film_grain;
mod fisheye;
mod gaussian_blur;
mod glow_bloom;
mod halftone;
mod invert;
mod kaleidoscope;
mod kuwahara;
mod lens_distortion;
mod luma_key;
mod mask;
mod mirror;
mod opacity;
mod pixelate_mosaic;
mod posterize;
mod radial_blur;
pub(crate) mod sam2;
mod sampling;
mod scanlines_crt;
mod shared;
mod sharpen;
mod stabilization_warp;
mod texture_bounds;
mod threshold;
mod transform;
pub(crate) mod transparent_fill;
mod twirl;
mod vignette;
mod wave_ripple;
mod zoom_blur;

use crate::layer::{RasterVisual, Visual};
use crate::visual_source::VisualModifierContext;
use shrimply_project::project::VideoSampleMethod;
use shrimply_video_modifiers::{ModifierEffect, RasterModifierEffect};

pub(crate) fn stabilization_warp(
    source_transform: glam::Mat3,
) -> Box<dyn crate::layer::PreservingRasterModifier> {
    Box::new(stabilization_warp::Source { source_transform })
}

pub(crate) trait RasterModifierRuntime {
    fn apply_raster(
        &self,
        input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String>;
}

pub(crate) fn apply(
    effect: &ModifierEffect,
    input: Visual,
    context: &mut VisualModifierContext<'_>,
) -> Result<Visual, String> {
    match effect {
        ModifierEffect::Scene3d(_) => Ok(input),
        // Still-image sources perform the expensive trace before the modifier chain so the
        // remaining vector modifiers never see an intermediate raster surface.
        ModifierEffect::Vectorize(_) => {
            let Visual::Vector(_) = input else {
                unreachable!("validated Vectorize modifier received untraced raster input")
            };
            Ok(input)
        }
        ModifierEffect::Vector(effect) => {
            let Visual::Vector(mut input) = input else {
                unreachable!("validated vector modifier chain received raster input")
            };
            if let Some(operation) = shrimply_video_core::vector_modifiers::operation(
                effect,
                context.evaluation,
                context.expressions,
            ) {
                input.push(operation);
            }
            Ok(Visual::Vector(input))
        }
        ModifierEffect::Rasterize(effect) => {
            let configured = effect
                .sample_method
                .value_at(context.evaluation.local_time());
            let sample_method = shrimply_video_core::generated::sampling(
                configured,
                context.accuracy.content_accurate(),
            );
            Ok(Visual::Raster(match input {
                Visual::Vector(input) => {
                    input.rasterize(context.item.skia_drawing_strategy, sample_method)
                }
                // Scene3D sources are already canvas-sized CUDA RGBA. Rasterize is their
                // semantic kind boundary and intentionally performs no copy.
                Visual::Raster(input) => input,
            }))
        }
        ModifierEffect::Raster(effect) => {
            let Visual::Raster(input) = input else {
                unreachable!("validated raster modifier chain received vector input")
            };
            apply_raster(effect, input, context).map(Visual::Raster)
        }
    }
}

fn apply_raster(
    effect: &RasterModifierEffect,
    input: RasterVisual,
    context: &mut VisualModifierContext<'_>,
) -> Result<RasterVisual, String> {
    raster_runtime(effect).apply_raster(input, context)
}

fn raster_runtime(effect: &RasterModifierEffect) -> &dyn RasterModifierRuntime {
    match effect {
        RasterModifierEffect::Cache(effect) => effect,
        RasterModifierEffect::Transform(effect) => &**effect,
        RasterModifierEffect::TextureBounds(effect) => effect,
        RasterModifierEffect::Sampling(effect) => effect,
        RasterModifierEffect::Crop(effect) => effect,
        RasterModifierEffect::CornerPin(effect) => effect,
        RasterModifierEffect::Opacity(effect) => effect,
        RasterModifierEffect::ChromaKey(effect) => effect,
        RasterModifierEffect::Kuwahara(effect) => effect,
        RasterModifierEffect::GaussianBlur(effect) => effect,
        RasterModifierEffect::Fisheye(effect) => effect,
        RasterModifierEffect::Sharpen(effect) => effect,
        RasterModifierEffect::Vignette(effect) => effect,
        RasterModifierEffect::PixelateMosaic(effect) => effect,
        RasterModifierEffect::Posterize(effect) => effect,
        RasterModifierEffect::Threshold(effect) => effect,
        RasterModifierEffect::FilmGrain(effect) => effect,
        RasterModifierEffect::ChromaticAberration(effect) => effect,
        RasterModifierEffect::EdgeDetection(effect) => effect,
        RasterModifierEffect::Emboss(effect) => effect,
        RasterModifierEffect::DirectionalBlur(effect) => effect,
        RasterModifierEffect::Dithering(effect) => effect,
        RasterModifierEffect::GlowBloom(effect) => effect,
        RasterModifierEffect::Twirl(effect) => effect,
        RasterModifierEffect::BulgePinch(effect) => effect,
        RasterModifierEffect::WaveRipple(effect) => effect,
        RasterModifierEffect::Mirror(effect) => effect,
        RasterModifierEffect::Kaleidoscope(effect) => effect,
        RasterModifierEffect::ColorizeDuotone(effect) => effect,
        RasterModifierEffect::Invert(effect) => effect,
        RasterModifierEffect::ChannelMixer(effect) => &**effect,
        RasterModifierEffect::AlphaOutline(effect) => effect,
        RasterModifierEffect::DropShadow(effect) => effect,
        RasterModifierEffect::Halftone(effect) => effect,
        RasterModifierEffect::ScanlinesCrt(effect) => effect,
        RasterModifierEffect::LensDistortion(effect) => effect,
        RasterModifierEffect::DisplacementMap(effect) => effect,
        RasterModifierEffect::LumaKey(effect) => effect,
        RasterModifierEffect::Mask(effect) => effect,
        RasterModifierEffect::Sam2(effect) => effect,
        RasterModifierEffect::TransparentFill(effect) => effect,
        RasterModifierEffect::RadialBlur(effect) => effect,
        RasterModifierEffect::ZoomBlur(effect) => effect,
        RasterModifierEffect::ErodeDilate(effect) => effect,
        RasterModifierEffect::ColorCorrection(effect) => &**effect,
    }
}

impl RasterModifierRuntime for shrimply_video_modifiers::cache::CacheModifier {
    fn apply_raster(
        &self,
        input: RasterVisual,
        _context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        Ok(input)
    }
}
