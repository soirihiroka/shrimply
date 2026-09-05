//! Parameter evaluation shared by the CUDA and Metal modifier adapters.
use shrimply_evaluation::{
    TransformExpressionCache, VisualEvaluation, resolve_color, resolve_scalar, resolve_vec2,
};
use shrimply_render_core::{ColorCorrectionParams, effects::PixelEffect};
use shrimply_video_modifiers::{
    alpha_outline::AlphaOutlineModifier,
    bulge_pinch::BulgePinchModifier,
    channel_mixer::ChannelMixerModifier,
    chroma_key::ChromaKeyModifier,
    chromatic_aberration::ChromaticAberrationModifier,
    color_correction::ColorCorrectionModifier,
    colorize_duotone::ColorizeDuotoneModifier,
    directional_blur::DirectionalBlurModifier,
    displacement_map::DisplacementMapModifier,
    drop_shadow::DropShadowModifier,
    edge_detection::EdgeDetectionModifier,
    emboss::EmbossModifier,
    erode_dilate::{ErodeDilateModifier, ErodeDilateOperation},
    film_grain::FilmGrainModifier,
    fisheye::FisheyeModifier,
    gaussian_blur::{GaussianBlurChannels, GaussianBlurModifier},
    glow_bloom::GlowBloomModifier,
    halftone::HalftoneModifier,
    invert::InvertModifier,
    kaleidoscope::KaleidoscopeModifier,
    kuwahara::{KuwaharaModifier, KuwaharaVersion},
    lens_distortion::LensDistortionModifier,
    luma_key::LumaKeyModifier,
    mirror::MirrorModifier,
    pixelate_mosaic::PixelateMosaicModifier,
    posterize::PosterizeModifier,
    radial_blur::RadialBlurModifier,
    scanlines_crt::ScanlinesCrtModifier,
    sharpen::SharpenModifier,
    threshold::ThresholdModifier,
    twirl::TwirlModifier,
    vignette::VignetteModifier,
    wave_ripple::WaveRippleModifier,
    zoom_blur::ZoomBlurModifier,
};

pub fn pixel_effect(
    effect: &shrimply_video_modifiers::RasterModifierEffect,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> Option<PixelEffect> {
    use shrimply_video_modifiers::RasterModifierEffect;
    Some(match effect {
        RasterModifierEffect::DropShadow(modifier) => {
            drop_shadow(modifier, evaluation, expressions)
        }
        RasterModifierEffect::GlowBloom(modifier) => glow_bloom(modifier, evaluation, expressions),
        RasterModifierEffect::ChromaKey(modifier) => chroma_key(modifier, evaluation, expressions),
        RasterModifierEffect::BulgePinch(modifier) => {
            bulge_pinch(modifier, evaluation, expressions)
        }
        RasterModifierEffect::Twirl(modifier) => twirl(modifier, evaluation, expressions),
        RasterModifierEffect::WaveRipple(modifier) => {
            wave_ripple(modifier, evaluation, expressions)
        }
        RasterModifierEffect::DisplacementMap(modifier) => {
            displacement_map(modifier, evaluation, expressions)
        }
        RasterModifierEffect::Fisheye(modifier) => fisheye(modifier, evaluation, expressions),
        RasterModifierEffect::LensDistortion(modifier) => {
            lens_distortion(modifier, evaluation, expressions)
        }
        RasterModifierEffect::Kaleidoscope(modifier) => {
            kaleidoscope(modifier, evaluation, expressions)
        }
        RasterModifierEffect::ChannelMixer(modifier) => {
            channel_mixer(modifier, evaluation, expressions)
        }
        RasterModifierEffect::ColorizeDuotone(modifier) => {
            colorize_duotone(modifier, evaluation, expressions)
        }
        RasterModifierEffect::Threshold(modifier) => threshold(modifier, evaluation, expressions),
        RasterModifierEffect::EdgeDetection(modifier) => {
            edge_detection(modifier, evaluation, expressions)
        }
        RasterModifierEffect::AlphaOutline(modifier) => {
            alpha_outline(modifier, evaluation, expressions)
        }
        RasterModifierEffect::ErodeDilate(modifier) => {
            erode_dilate(modifier, evaluation, expressions)
        }
        RasterModifierEffect::FilmGrain(modifier) => film_grain(modifier, evaluation, expressions),
        RasterModifierEffect::Halftone(modifier) => halftone(modifier, evaluation, expressions),
        RasterModifierEffect::Kuwahara(modifier) => kuwahara(modifier, evaluation, expressions),
        RasterModifierEffect::ScanlinesCrt(modifier) => {
            scanlines_crt(modifier, evaluation, expressions)
        }
        RasterModifierEffect::Invert(modifier) => invert(modifier, evaluation, expressions),
        RasterModifierEffect::ColorCorrection(modifier) => {
            color_correction(modifier, evaluation, expressions)
        }
        RasterModifierEffect::GaussianBlur(modifier) => {
            gaussian_blur(modifier, evaluation, expressions)
        }
        RasterModifierEffect::Posterize(modifier) => posterize(modifier, evaluation, expressions),
        RasterModifierEffect::Mirror(modifier) => mirror(modifier),
        RasterModifierEffect::Vignette(modifier) => vignette(modifier, evaluation, expressions),
        RasterModifierEffect::PixelateMosaic(modifier) => {
            pixelate_mosaic(modifier, evaluation, expressions)
        }
        RasterModifierEffect::Sharpen(modifier) => sharpen(modifier, evaluation, expressions),
        RasterModifierEffect::ChromaticAberration(modifier) => {
            chromatic_aberration(modifier, evaluation, expressions)
        }
        RasterModifierEffect::Emboss(modifier) => emboss(modifier, evaluation, expressions),
        RasterModifierEffect::LumaKey(modifier) => luma_key(modifier, evaluation, expressions),
        RasterModifierEffect::DirectionalBlur(modifier) => {
            directional_blur(modifier, evaluation, expressions)
        }
        RasterModifierEffect::ZoomBlur(modifier) => zoom_blur(modifier, evaluation, expressions),
        RasterModifierEffect::RadialBlur(modifier) => {
            radial_blur(modifier, evaluation, expressions)
        }
        _ => return None,
    })
}

pub fn corner_pin(
    effect: &shrimply_video_modifiers::corner_pin::CornerPinModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> Result<PixelEffect, String> {
    let corners = [
        &effect.top_left,
        &effect.top_right,
        &effect.bottom_right,
        &effect.bottom_left,
    ]
    .map(|value| {
        resolve_vec2(value, evaluation, expressions).clamp(glam::Vec2::ZERO, glam::Vec2::ONE)
    });
    let perspective = resolve_scalar(&effect.perspective, evaluation, expressions).clamp(0.0, 1.0);
    let inverse_homography = if corners == shrimply_math_geometry::UNIT_QUAD {
        glam::Mat3::IDENTITY
    } else {
        shrimply_math_geometry::corner_pin_inverse(corners)
            .ok_or("corner pin destination must be a non-degenerate convex quadrilateral")?
    };
    Ok(PixelEffect::CornerPin(
        shrimply_render_core::effects::CornerPin {
            inverse_homography,
            corners,
            perspective,
        },
    ))
}

pub fn invert(
    modifier: &InvertModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::Invert(resolve_scalar(&modifier.amount, evaluation, expressions).clamp(0.0, 1.0))
}

pub fn color_correction(
    modifier: &ColorCorrectionModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let mut resolve = |value| resolve_scalar(value, evaluation, expressions);
    PixelEffect::ColorCorrection(ColorCorrectionParams {
        exposure: resolve(&modifier.exposure).clamp(-10.0, 10.0),
        gamma: resolve(&modifier.gamma).clamp(0.01, 10.0),
        temperature: resolve(&modifier.temperature).clamp(-1.0, 1.0),
        tint: resolve(&modifier.tint).clamp(-1.0, 1.0),
        brightness: resolve(&modifier.brightness).clamp(-1.0, 1.0),
        contrast: resolve(&modifier.contrast).clamp(-1.0, 1.0),
        hue_turns: resolve(&modifier.hue_degrees) / 360.0,
        saturation: resolve(&modifier.saturation).clamp(0.0, 2.0),
        value: resolve(&modifier.value).clamp(0.0, 2.0),
    })
}

pub fn gaussian_blur(
    modifier: &GaussianBlurModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let radius = resolve_vec2(&modifier.radius, evaluation, expressions)
        .clamp(glam::Vec2::ZERO, glam::Vec2::splat(100.0));
    let (blur_rgb, blur_alpha) = match modifier.channels {
        GaussianBlurChannels::Rgba => (true, true),
        GaussianBlurChannels::Rgb => (true, false),
        GaussianBlurChannels::Alpha => (false, true),
    };
    PixelEffect::GaussianBlur {
        radius_x: radius.x as u32,
        radius_y: radius.y as u32,
        blur_rgb,
        blur_alpha,
    }
}

pub fn posterize(
    modifier: &PosterizeModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::Posterize(
        resolve_scalar(&modifier.levels, evaluation, expressions).clamp(2.0, 256.0),
    )
}

pub fn mirror(modifier: &MirrorModifier) -> PixelEffect {
    PixelEffect::Mirror {
        horizontal: u32::from(modifier.horizontal),
        vertical: u32::from(modifier.vertical),
    }
}

pub fn vignette(
    modifier: &VignetteModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::Vignette {
        amount: resolve_scalar(&modifier.amount, evaluation, expressions).clamp(0.0, 1.0),
        midpoint: resolve_scalar(&modifier.midpoint, evaluation, expressions).clamp(0.0, 1.0),
        softness: resolve_scalar(&modifier.softness, evaluation, expressions).clamp(0.0, 1.0),
    }
}

pub fn pixelate_mosaic(
    modifier: &PixelateMosaicModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::PixelateMosaic {
        block_width: resolve_scalar(&modifier.block_width, evaluation, expressions)
            .clamp(1.0, 512.0) as u32,
        block_height: resolve_scalar(&modifier.block_height, evaluation, expressions)
            .clamp(1.0, 512.0) as u32,
    }
}

pub fn sharpen(
    modifier: &SharpenModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::Sharpen {
        amount: resolve_scalar(&modifier.amount, evaluation, expressions).clamp(0.0, 2.0),
        radius: resolve_scalar(&modifier.radius, evaluation, expressions).clamp(0.0, 20.0) as u32,
    }
}

pub fn chromatic_aberration(
    modifier: &ChromaticAberrationModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let mut resolve = |value| resolve_scalar(value, evaluation, expressions).clamp(-4096.0, 4096.0);
    PixelEffect::ChromaticAberration([
        resolve(&modifier.red_offset_x),
        resolve(&modifier.red_offset_y),
        resolve(&modifier.blue_offset_x),
        resolve(&modifier.blue_offset_y),
    ])
}

pub fn emboss(
    modifier: &EmbossModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::Emboss {
        direction: resolve_scalar(&modifier.direction_degrees, evaluation, expressions),
        depth: resolve_scalar(&modifier.depth, evaluation, expressions).clamp(0.0, 10.0),
        amount: resolve_scalar(&modifier.amount, evaluation, expressions).clamp(0.0, 1.0),
    }
}

pub fn luma_key(
    modifier: &LumaKeyModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::LumaKey {
        threshold: resolve_scalar(&modifier.threshold, evaluation, expressions).clamp(0.0, 1.0),
        softness: resolve_scalar(&modifier.softness, evaluation, expressions).clamp(0.0, 1.0),
        invert: modifier.invert,
    }
}

pub fn directional_blur(
    modifier: &DirectionalBlurModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::DirectionalBlur {
        radius: resolve_scalar(&modifier.radius, evaluation, expressions).clamp(0.0, 100.0) as u32,
        angle: resolve_scalar(&modifier.angle_degrees, evaluation, expressions).to_radians(),
    }
}

pub fn zoom_blur(
    modifier: &ZoomBlurModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::ZoomBlur(shrimply_render_core::ZoomBlurParams {
        center: resolve_vec2(&modifier.center, evaluation, expressions)
            .clamp(glam::Vec2::ZERO, glam::Vec2::ONE),
        strength: resolve_scalar(&modifier.strength, evaluation, expressions).clamp(-1.0, 1.0),
        samples: resolve_scalar(&modifier.samples, evaluation, expressions).clamp(1.0, 128.0)
            as u32,
    })
}

pub fn radial_blur(
    modifier: &RadialBlurModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::RadialBlur(shrimply_render_core::RadialBlurParams {
        center: resolve_vec2(&modifier.center, evaluation, expressions)
            .clamp(glam::Vec2::ZERO, glam::Vec2::ONE),
        angle: resolve_scalar(&modifier.angle_degrees, evaluation, expressions)
            .clamp(-360.0, 360.0)
            .to_radians(),
        samples: resolve_scalar(&modifier.samples, evaluation, expressions).clamp(1.0, 128.0)
            as u32,
    })
}

pub fn film_grain(
    modifier: &FilmGrainModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let mut resolve = |value| resolve_scalar(value, evaluation, expressions);
    PixelEffect::FilmGrain {
        amount: resolve(&modifier.amount).clamp(0.0, 2.0),
        size: resolve(&modifier.size).clamp(1.0, 256.0),
        colored: resolve(&modifier.colored).clamp(0.0, 1.0),
        seed: resolve(&modifier.seed),
    }
}

pub fn scanlines_crt(
    modifier: &ScanlinesCrtModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let mut resolve = |value| resolve_scalar(value, evaluation, expressions);
    PixelEffect::ScanlinesCrt(shrimply_render_core::ScanlinesCrtParams {
        spacing: resolve(&modifier.spacing).clamp(1.0, 100.0),
        intensity: resolve(&modifier.intensity).clamp(0.0, 1.0),
        curvature: resolve(&modifier.curvature).clamp(0.0, 2.0),
        mask: resolve(&modifier.mask_strength).clamp(0.0, 1.0),
    })
}

pub fn halftone(
    modifier: &HalftoneModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let mut resolve = |value| resolve_scalar(value, evaluation, expressions);
    PixelEffect::Halftone(shrimply_render_core::HalftoneParams {
        size: resolve(&modifier.size).clamp(1.0, 1024.0),
        angle: resolve(&modifier.angle_degrees),
        contrast: resolve(&modifier.contrast).clamp(0.0, 10.0),
        mode: modifier.mode.value_at(evaluation.local_time()) as u32,
        channel_offset: resolve(&modifier.rgb_distance).clamp(0.0, 1024.0),
        channel_angle_offset: resolve(&modifier.channel_angle_offset),
    })
}

pub fn alpha_outline(
    modifier: &AlphaOutlineModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::AlphaOutline {
        radius: resolve_scalar(&modifier.width, evaluation, expressions).clamp(0.0, 32.0) as u32,
        color: resolve_color(&modifier.color, evaluation, expressions).to_rgba_u32(),
    }
}

pub fn erode_dilate(
    modifier: &ErodeDilateModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let radius = resolve_scalar(&modifier.radius, evaluation, expressions).clamp(0.0, 100.0) as u32;
    match modifier.operation.value_at(evaluation.local_time()) {
        ErodeDilateOperation::Erode => PixelEffect::Erode(radius),
        ErodeDilateOperation::Dilate => PixelEffect::Dilate(radius),
    }
}

pub fn kuwahara(
    modifier: &KuwaharaModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::Kuwahara {
        radius: resolve_scalar(&modifier.radius, evaluation, expressions).clamp(0.0, 32.0) as u32,
        generalized: modifier.version.value_at(evaluation.local_time())
            == KuwaharaVersion::Generalized,
    }
}

pub fn channel_mixer(
    modifier: &ChannelMixerModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let mut resolve = |value| resolve_scalar(value, evaluation, expressions).clamp(-2.0, 2.0);
    PixelEffect::ChannelMixer(glam::Mat3::from_cols_array(&[
        resolve(&modifier.rr),
        resolve(&modifier.gr),
        resolve(&modifier.br),
        resolve(&modifier.rg),
        resolve(&modifier.gg),
        resolve(&modifier.bg),
        resolve(&modifier.rb),
        resolve(&modifier.gb),
        resolve(&modifier.bb),
    ]))
}

pub fn colorize_duotone(
    modifier: &ColorizeDuotoneModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::ColorizeDuotone(shrimply_render_core::ColorizeDuotoneParams {
        shadow: resolve_color(&modifier.shadow_color, evaluation, expressions).into(),
        highlight: resolve_color(&modifier.highlight_color, evaluation, expressions).into(),
    })
}

pub fn threshold(
    modifier: &ThresholdModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let low = resolve_color(&modifier.low_color, evaluation, expressions);
    let high = resolve_color(&modifier.high_color, evaluation, expressions);
    PixelEffect::Threshold(shrimply_render_core::ThresholdParams {
        low: low.into(),
        high: high.into(),
        threshold: resolve_scalar(&modifier.threshold, evaluation, expressions).clamp(0.0, 1.0),
    })
}

pub fn edge_detection(
    modifier: &EdgeDetectionModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let edge = resolve_color(&modifier.edge_color, evaluation, expressions);
    let background = resolve_color(&modifier.background_color, evaluation, expressions);
    PixelEffect::EdgeDetection(shrimply_render_core::EdgeDetectionParams {
        edge: edge.into(),
        background: background.into(),
        amount: resolve_scalar(&modifier.amount, evaluation, expressions).clamp(0.0, 1.0),
    })
}

pub fn bulge_pinch(
    modifier: &BulgePinchModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::BulgePinch {
        center: resolve_vec2(&modifier.center, evaluation, expressions)
            .clamp(glam::Vec2::ZERO, glam::Vec2::ONE),
        radius_fraction: resolve_scalar(&modifier.radius, evaluation, expressions).clamp(0.0, 1.0),
        strength: resolve_scalar(&modifier.strength, evaluation, expressions).clamp(-1.0, 1.0),
    }
}

pub fn twirl(
    modifier: &TwirlModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::Twirl {
        center: resolve_vec2(&modifier.center, evaluation, expressions)
            .clamp(glam::Vec2::ZERO, glam::Vec2::ONE),
        radius_fraction: resolve_scalar(&modifier.radius, evaluation, expressions).clamp(0.0, 1.0),
        angle: resolve_scalar(&modifier.angle_degrees, evaluation, expressions).to_radians(),
    }
}

pub fn wave_ripple(
    modifier: &WaveRippleModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::WaveRipple(shrimply_render_core::WaveRippleParams {
        amplitude: resolve_scalar(&modifier.amplitude, evaluation, expressions)
            .clamp(-512.0, 512.0),
        wavelength: resolve_scalar(&modifier.wavelength, evaluation, expressions)
            .clamp(1.0, 4096.0),
        angle: resolve_scalar(&modifier.angle_degrees, evaluation, expressions).to_radians(),
        phase: resolve_scalar(&modifier.phase, evaluation, expressions),
    })
}

pub fn displacement_map(
    modifier: &DisplacementMapModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::DisplacementMap {
        amount: resolve_scalar(&modifier.amount, evaluation, expressions).clamp(-512.0, 512.0),
        scale: resolve_scalar(&modifier.scale, evaluation, expressions).clamp(1.0, 4096.0),
        phase: resolve_scalar(&modifier.phase, evaluation, expressions),
    }
}

pub fn fisheye(
    modifier: &FisheyeModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let center = resolve_vec2(&modifier.center, evaluation, expressions)
        .clamp(glam::Vec2::ZERO, glam::Vec2::ONE);
    PixelEffect::Fisheye {
        intensity: resolve_scalar(&modifier.intensity, evaluation, expressions).clamp(-1.0, 1.0),
        center,
    }
}

pub fn lens_distortion(
    modifier: &LensDistortionModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let center = resolve_vec2(&modifier.center, evaluation, expressions)
        .clamp(glam::Vec2::ZERO, glam::Vec2::ONE);
    PixelEffect::LensDistortion {
        distortion: resolve_scalar(&modifier.distortion, evaluation, expressions).clamp(-2.0, 2.0),
        center,
    }
}

pub fn kaleidoscope(
    modifier: &KaleidoscopeModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::Kaleidoscope(shrimply_render_core::KaleidoscopeParams {
        center: resolve_vec2(&modifier.center, evaluation, expressions)
            .clamp(glam::Vec2::ZERO, glam::Vec2::ONE),
        segments: resolve_scalar(&modifier.segments, evaluation, expressions).clamp(2.0, 64.0)
            as u32,
        rotation: resolve_scalar(&modifier.rotation_degrees, evaluation, expressions).to_radians(),
    })
}

pub fn chroma_key(
    modifier: &ChromaKeyModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::ChromaKey(shrimply_render_core::ChromaKeyParams {
        key: resolve_color(&modifier.key_color, evaluation, expressions).into(),
        similarity: resolve_scalar(&modifier.similarity, evaluation, expressions).clamp(0.0, 1.0),
        softness: resolve_scalar(&modifier.softness, evaluation, expressions).clamp(0.0, 1.0),
        spill: resolve_scalar(&modifier.spill_suppression, evaluation, expressions).clamp(0.0, 1.0),
    })
}

pub fn drop_shadow(
    modifier: &DropShadowModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    let offset = resolve_vec2(&modifier.offset, evaluation, expressions);
    let color = resolve_color(&modifier.color, evaluation, expressions);
    PixelEffect::DropShadow(shrimply_render_core::DropShadowParams {
        offset,
        radius: resolve_scalar(&modifier.blur_radius, evaluation, expressions).clamp(0.0, 32.0)
            as u32,
        color: color.to_rgba_u32(),
    })
}

pub fn glow_bloom(
    modifier: &GlowBloomModifier,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> PixelEffect {
    PixelEffect::GlowBloom {
        threshold: resolve_scalar(&modifier.threshold, evaluation, expressions).clamp(0.0, 1.0),
        radius: resolve_scalar(&modifier.radius, evaluation, expressions).clamp(0.0, 32.0) as u32,
        intensity: resolve_scalar(&modifier.intensity, evaluation, expressions).clamp(0.0, 5.0),
    }
}
