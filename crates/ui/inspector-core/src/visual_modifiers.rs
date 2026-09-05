use shrimply_core::modifier_model::ModifierModel;
use shrimply_project::project::{ItemAddress, Time, VideoItem, VisualModifier};
use shrimply_state::player_state::{self, ProjectChange};
use shrimply_video_modifiers::{
    ModifierEffect, RasterModifierEffect, VectorModifierEffect, VisualKind,
    scene_3d::Scene3dModifierEffect,
};

use crate::{
    AnalysisControlPresentation, ControlKind, InspectorControl, InspectorControlAction,
    InspectorController, InspectorRuntime, InspectorSection, InspectorTarget, LayeredState,
    NumberSpec, TextKeyframeCommits,
};

mod alpha_outline;
mod bulge_pinch;
mod cache;
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
mod ground;
mod halftone;
mod hsv;
mod invert;
mod kaleidoscope;
mod kuwahara;
mod lens_distortion;
mod luma_key;
mod mask;
mod mirror;
mod object_3d;
mod opacity;
mod path_offset;
mod pixelate_mosaic;
mod point_light;
mod posterize;
mod radial_blur;
mod rasterize;
mod repeat;
mod sam2;
mod sampling;
mod scanlines_crt;
mod shaky_path;
mod shape_3d;
mod sharpen;
mod sun_light;
mod text_3d;
mod text_mask;
mod texture_bounds;
mod threshold;
mod transform;
mod transparent_fill;
mod twirl;
mod vectorize;
mod vignette;
mod wave_ripple;
mod zoom_blur;

pub use crate::alpha_mask::AlphaMaskPresentation as VisualModifierAlphaMaskPresentation;
pub use cache::visual_cache_status;
pub use kuwahara::{
    VERSION_COMMIT as KUWAHARA_VERSION_COMMIT, version as kuwahara_version,
    version_mut as kuwahara_version_mut,
};
pub use mask::{
    MODE_COMMIT as MASK_MODE_COMMIT, mode_value as mask_mode_value,
    mode_value_mut as mask_mode_value_mut, set_mask_source, source_label as mask_source_label,
};
pub use opacity::OpacityModifierPresentation;
pub use rasterize::{
    SAMPLE_METHOD_COMMIT as RASTERIZE_SAMPLE_METHOD_COMMIT,
    sample_method as rasterize_sample_method, sample_method_mut as rasterize_sample_method_mut,
};
pub use repeat::{
    OFFSET_AXIS_COMMIT as REPEAT_OFFSET_AXIS_COMMIT, offset_axis as repeat_offset_axis,
    offset_axis_mut as repeat_offset_axis_mut,
};
pub use sam2::{
    ANALYZE_TOOLTIP as SAM2_ANALYZE_TOOLTIP, EDIT_COMMIT as SAM2_EDIT_COMMIT, sam2_analysis_control,
};
pub use sampling::{
    METHOD_COMMIT as SAMPLING_METHOD_COMMIT, method as sampling_method,
    method_mut as sampling_method_mut,
};
pub use texture_bounds::{
    ADDRESS_MODE_COMMIT as TEXTURE_BOUNDS_ADDRESS_MODE_COMMIT,
    address_mode as texture_bounds_address_mode,
    address_mode_mut as texture_bounds_address_mode_mut,
};
pub use transform::TransformModifierPresentation;
pub use transparent_fill::EDIT_COMMIT as TRANSPARENT_FILL_EDIT_COMMIT;

const TEXT_3D_KEYFRAME_COMMITS: TextKeyframeCommits = TextKeyframeCommits {
    toggle: "3d-text-keyframes",
    add: "add-3d-text-keyframe",
    delete: "delete-3d-text-keyframe",
    move_keyframe: "move-3d-text-keyframe",
    paste: "paste-3d-text-keyframes",
    interpolation: "3d-text-interpolation",
    text_interpolation: "3d-text-change-interpolation",
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualModifierChoice {
    pub key: String,
    pub label: &'static str,
    pub search_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisualModifierPresentation {
    pub id: uuid::Uuid,
    pub index: usize,
    pub title: &'static str,
    pub enabled: bool,
    pub default_effect: serde_json::Value,
    pub can_move_up: bool,
    pub can_move_down: bool,
    pub can_remove: bool,
    pub body: Option<VisualModifierBodyPresentation>,
    pub alpha_mask: Option<VisualModifierAlphaMaskPresentation>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VisualModifierBodyPresentation {
    AlphaOutline(InspectorSection),
    BulgePinch(InspectorSection),
    Cache(InspectorSection),
    ChannelMixer(InspectorSection),
    ChromaKey(InspectorSection),
    ChromaticAberration(InspectorSection),
    ColorCorrection(InspectorSection),
    ColorizeDuotone(InspectorSection),
    CornerPin(InspectorSection),
    Crop(InspectorSection),
    DirectionalBlur(InspectorSection),
    DisplacementMap(InspectorSection),
    Dithering(InspectorSection),
    DropShadow(InspectorSection),
    EdgeDetection(InspectorSection),
    Emboss(InspectorSection),
    ErodeDilate(InspectorSection),
    FilmGrain(InspectorSection),
    Fisheye(InspectorSection),
    GaussianBlur(InspectorSection),
    GlowBloom(InspectorSection),
    Ground(InspectorSection),
    Halftone(InspectorSection),
    Hsv(InspectorSection),
    Invert(InspectorSection),
    Kaleidoscope(InspectorSection),
    Kuwahara(InspectorSection),
    LensDistortion(InspectorSection),
    LumaKey(InspectorSection),
    Mask(InspectorSection),
    Mirror(InspectorSection),
    Object3d(InspectorSection),
    PointLight(InspectorSection),
    Opacity(Box<OpacityModifierPresentation>),
    PathOffset(InspectorSection),
    PixelateMosaic(InspectorSection),
    Posterize(InspectorSection),
    RadialBlur(InspectorSection),
    Rasterize(InspectorSection),
    Sam2(InspectorSection),
    Repeat(InspectorSection),
    Sampling(InspectorSection),
    ScanlinesCrt(InspectorSection),
    Shape3d(InspectorSection),
    ShakyPath(InspectorSection),
    Sharpen(InspectorSection),
    SunLight(InspectorSection),
    TextMask(InspectorSection),
    Text3d(InspectorSection),
    TextureBounds(InspectorSection),
    Threshold(InspectorSection),
    TransparentFill(InspectorSection),
    Transform(Box<TransformModifierPresentation>),
    Twirl(InspectorSection),
    Vectorize(InspectorSection),
    Vignette(InspectorSection),
    WaveRipple(InspectorSection),
    ZoomBlur(InspectorSection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualModifierChainAction {
    MoveUp,
    MoveDown,
    Remove,
}

pub fn visual_modifier_presentations(
    project: &shrimply_project::project::Project,
    address: &ItemAddress,
    item: &VideoItem,
    runtime: InspectorRuntime,
) -> Vec<VisualModifierPresentation> {
    item.modifiers
        .iter()
        .enumerate()
        .map(|(index, modifier)| VisualModifierPresentation {
            id: modifier.id,
            index,
            title: modifier.effect.display_name(),
            enabled: modifier.enabled,
            default_effect: serde_json::to_value(default_visual_modifier_effect(&modifier.effect))
                .expect("visual modifier effect must serialize"),
            can_move_up: visual_modifier_action_valid(
                item,
                modifier.id,
                VisualModifierChainAction::MoveUp,
            ),
            can_move_down: visual_modifier_action_valid(
                item,
                modifier.id,
                VisualModifierChainAction::MoveDown,
            ),
            can_remove: visual_modifier_action_valid(
                item,
                modifier.id,
                VisualModifierChainAction::Remove,
            ),
            body: match &modifier.effect {
                ModifierEffect::Vector(effect) => match &**effect {
                    VectorModifierEffect::Transform(value) => {
                        Some(VisualModifierBodyPresentation::Transform(Box::new(
                            transform::presentation(value, index, runtime),
                        )))
                    }
                    VectorModifierEffect::Opacity(value) => {
                        Some(VisualModifierBodyPresentation::Opacity(Box::new(
                            opacity::presentation(value, index, runtime),
                        )))
                    }
                    VectorModifierEffect::PathOffset(value) => {
                        Some(VisualModifierBodyPresentation::PathOffset(
                            path_offset::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    VectorModifierEffect::Repeat(value) => {
                        Some(VisualModifierBodyPresentation::Repeat(
                            repeat::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    VectorModifierEffect::ShakyPath(value) => {
                        Some(VisualModifierBodyPresentation::ShakyPath(
                            shaky_path::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    VectorModifierEffect::Hsv(value) => Some(VisualModifierBodyPresentation::Hsv(
                        hsv::presentation(value, index, runtime),
                    )),
                    VectorModifierEffect::TextMask(value) => {
                        Some(VisualModifierBodyPresentation::TextMask(
                            text_mask::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                },
                ModifierEffect::Vectorize(value) => {
                    Some(VisualModifierBodyPresentation::Vectorize(
                        vectorize::presentation(value, index, modifier.id, runtime),
                    ))
                }
                ModifierEffect::Rasterize(value) => {
                    Some(VisualModifierBodyPresentation::Rasterize(
                        rasterize::presentation(value, index, modifier.id, runtime),
                    ))
                }
                ModifierEffect::Raster(effect) => match &**effect {
                    RasterModifierEffect::AlphaOutline(value) => {
                        Some(VisualModifierBodyPresentation::AlphaOutline(
                            alpha_outline::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::BulgePinch(value) => {
                        Some(VisualModifierBodyPresentation::BulgePinch(
                            bulge_pinch::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::Cache(value) => {
                        Some(VisualModifierBodyPresentation::Cache(cache::presentation(
                            value,
                            index,
                            modifier.id,
                            runtime,
                        )))
                    }
                    RasterModifierEffect::ChannelMixer(value) => {
                        Some(VisualModifierBodyPresentation::ChannelMixer(
                            channel_mixer::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::ChromaKey(value) => {
                        Some(VisualModifierBodyPresentation::ChromaKey(
                            chroma_key::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::ChromaticAberration(value) => {
                        Some(VisualModifierBodyPresentation::ChromaticAberration(
                            chromatic_aberration::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::ColorCorrection(value) => {
                        Some(VisualModifierBodyPresentation::ColorCorrection(
                            color_correction::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::ColorizeDuotone(value) => {
                        Some(VisualModifierBodyPresentation::ColorizeDuotone(
                            colorize_duotone::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::CornerPin(value) => {
                        Some(VisualModifierBodyPresentation::CornerPin(
                            corner_pin::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    RasterModifierEffect::Crop(value) => {
                        Some(VisualModifierBodyPresentation::Crop(crop::presentation(
                            value,
                            index,
                            modifier.id,
                            runtime,
                        )))
                    }
                    RasterModifierEffect::DirectionalBlur(value) => {
                        Some(VisualModifierBodyPresentation::DirectionalBlur(
                            directional_blur::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::DisplacementMap(value) => {
                        Some(VisualModifierBodyPresentation::DisplacementMap(
                            displacement_map::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::Dithering(value) => {
                        Some(VisualModifierBodyPresentation::Dithering(
                            dithering::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    RasterModifierEffect::DropShadow(value) => {
                        Some(VisualModifierBodyPresentation::DropShadow(
                            drop_shadow::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::EdgeDetection(value) => {
                        Some(VisualModifierBodyPresentation::EdgeDetection(
                            edge_detection::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::Emboss(value) => {
                        Some(VisualModifierBodyPresentation::Emboss(
                            emboss::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::ErodeDilate(value) => {
                        Some(VisualModifierBodyPresentation::ErodeDilate(
                            erode_dilate::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::FilmGrain(value) => {
                        Some(VisualModifierBodyPresentation::FilmGrain(
                            film_grain::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::Fisheye(value) => {
                        Some(VisualModifierBodyPresentation::Fisheye(
                            fisheye::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::GaussianBlur(value) => {
                        Some(VisualModifierBodyPresentation::GaussianBlur(
                            gaussian_blur::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::GlowBloom(value) => {
                        Some(VisualModifierBodyPresentation::GlowBloom(
                            glow_bloom::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::Halftone(value) => {
                        Some(VisualModifierBodyPresentation::Halftone(
                            halftone::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    RasterModifierEffect::Invert(value) => {
                        Some(VisualModifierBodyPresentation::Invert(
                            invert::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::Kaleidoscope(value) => {
                        Some(VisualModifierBodyPresentation::Kaleidoscope(
                            kaleidoscope::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    RasterModifierEffect::Kuwahara(value) => {
                        Some(VisualModifierBodyPresentation::Kuwahara(
                            kuwahara::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    RasterModifierEffect::LensDistortion(value) => {
                        Some(VisualModifierBodyPresentation::LensDistortion(
                            lens_distortion::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::LumaKey(value) => {
                        Some(VisualModifierBodyPresentation::LumaKey(
                            luma_key::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    RasterModifierEffect::Mask(value) => {
                        Some(VisualModifierBodyPresentation::Mask(mask::presentation(
                            project,
                            address,
                            value,
                            index,
                            modifier.id,
                            runtime,
                        )))
                    }
                    RasterModifierEffect::Mirror(value) => {
                        Some(VisualModifierBodyPresentation::Mirror(
                            mirror::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    RasterModifierEffect::Transform(value) => {
                        Some(VisualModifierBodyPresentation::Transform(Box::new(
                            transform::presentation(value, index, runtime),
                        )))
                    }
                    RasterModifierEffect::Opacity(value) => {
                        Some(VisualModifierBodyPresentation::Opacity(Box::new(
                            opacity::presentation(value, index, runtime),
                        )))
                    }
                    RasterModifierEffect::PixelateMosaic(value) => {
                        Some(VisualModifierBodyPresentation::PixelateMosaic(
                            pixelate_mosaic::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::Posterize(value) => {
                        Some(VisualModifierBodyPresentation::Posterize(
                            posterize::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::RadialBlur(value) => {
                        Some(VisualModifierBodyPresentation::RadialBlur(
                            radial_blur::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::Sampling(value) => {
                        Some(VisualModifierBodyPresentation::Sampling(
                            sampling::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    RasterModifierEffect::ScanlinesCrt(value) => {
                        Some(VisualModifierBodyPresentation::ScanlinesCrt(
                            scanlines_crt::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    RasterModifierEffect::Sharpen(value) => {
                        Some(VisualModifierBodyPresentation::Sharpen(
                            sharpen::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::Sam2(value) => {
                        Some(VisualModifierBodyPresentation::Sam2(sam2::presentation(
                            value,
                            index,
                            modifier.id,
                            runtime,
                        )))
                    }
                    RasterModifierEffect::Threshold(value) => {
                        Some(VisualModifierBodyPresentation::Threshold(
                            threshold::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::TransparentFill(value) => {
                        Some(VisualModifierBodyPresentation::TransparentFill(
                            transparent_fill::presentation(
                                value,
                                index,
                                modifier.id,
                                shrimply_video_cuda::transparent_fill_analysis::status(
                                    project,
                                    address,
                                    modifier.id,
                                ),
                                runtime,
                            ),
                        ))
                    }
                    RasterModifierEffect::TextureBounds(value) => {
                        Some(VisualModifierBodyPresentation::TextureBounds(
                            texture_bounds::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    RasterModifierEffect::Twirl(value) => {
                        Some(VisualModifierBodyPresentation::Twirl(twirl::presentation(
                            value, index, runtime,
                        )))
                    }
                    RasterModifierEffect::Vignette(value) => {
                        Some(VisualModifierBodyPresentation::Vignette(
                            vignette::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::WaveRipple(value) => {
                        Some(VisualModifierBodyPresentation::WaveRipple(
                            wave_ripple::presentation(value, index, runtime),
                        ))
                    }
                    RasterModifierEffect::ZoomBlur(value) => {
                        Some(VisualModifierBodyPresentation::ZoomBlur(
                            zoom_blur::presentation(value, index, runtime),
                        ))
                    }
                },
                ModifierEffect::Scene3d(effect) => match &**effect {
                    Scene3dModifierEffect::Object(value) => {
                        Some(VisualModifierBodyPresentation::Object3d(
                            object_3d::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    Scene3dModifierEffect::Ground(value) => {
                        Some(VisualModifierBodyPresentation::Ground(
                            ground::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    Scene3dModifierEffect::PointLight(value) => {
                        Some(VisualModifierBodyPresentation::PointLight(
                            point_light::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    Scene3dModifierEffect::Shape(value) => {
                        Some(VisualModifierBodyPresentation::Shape3d(
                            shape_3d::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    Scene3dModifierEffect::Text(value) => {
                        Some(VisualModifierBodyPresentation::Text3d(
                            text_3d::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                    Scene3dModifierEffect::SunLight(value) => {
                        Some(VisualModifierBodyPresentation::SunLight(
                            sun_light::presentation(value, index, modifier.id, runtime),
                        ))
                    }
                },
            },
            alpha_mask: matches!(
                modifier.effect,
                ModifierEffect::Raster(ref effect)
                    if !matches!(&**effect, RasterModifierEffect::Cache(_))
            )
            .then(|| {
                crate::alpha_mask::presentation(
                    modifier.alpha_mask.as_ref(),
                    &format!("/modifiers/{index}/alpha_mask"),
                    Some(modifier.id),
                    crate::alpha_mask::preview_focus(
                        item.id,
                        shrimply_project::project::VisualAlphaMaskTarget::Modifier(modifier.id),
                        true,
                    ),
                    runtime,
                )
            }),
        })
        .collect()
}

pub(crate) fn visual_modifier_color<'a>(
    item: &'a VideoItem,
    path: &str,
    timeline_id: uuid::Uuid,
) -> Option<&'a shrimply_core::timeline_value::TimelineValue<shrimply_core::Color<u8>>> {
    let (modifier, field) = visual_modifier_at_path(item, path)?;
    match &modifier.effect {
        ModifierEffect::Scene3d(effect) => match &**effect {
            Scene3dModifierEffect::Object(value) => object_3d::color(value, field, timeline_id),
            Scene3dModifierEffect::PointLight(value) => {
                point_light::color(value, field, timeline_id)
            }
            Scene3dModifierEffect::Shape(value) => shape_3d::color(value, field, timeline_id),
            Scene3dModifierEffect::Text(value) => text_3d::color(value, field, timeline_id),
            Scene3dModifierEffect::SunLight(value) => sun_light::color(value, field, timeline_id),
            _ => None,
        },
        ModifierEffect::Raster(effect) => {
            let timeline = match (&**effect, field) {
                (RasterModifierEffect::AlphaOutline(value), "effect/effect/config/color") => {
                    &value.color
                }
                (RasterModifierEffect::ChromaKey(value), "effect/effect/config/key_color") => {
                    &value.key_color
                }
                (RasterModifierEffect::EdgeDetection(value), "effect/effect/config/edge_color") => {
                    &value.edge_color
                }
                (
                    RasterModifierEffect::EdgeDetection(value),
                    "effect/effect/config/background_color",
                ) => &value.background_color,
                (
                    RasterModifierEffect::ColorizeDuotone(value),
                    "effect/effect/config/shadow_color",
                ) => &value.shadow_color,
                (
                    RasterModifierEffect::ColorizeDuotone(value),
                    "effect/effect/config/highlight_color",
                ) => &value.highlight_color,
                (RasterModifierEffect::DropShadow(value), "effect/effect/config/color") => {
                    &value.color
                }
                (RasterModifierEffect::Dithering(value), field) => {
                    return dithering::palette_color(value, field, timeline_id);
                }
                (RasterModifierEffect::Threshold(value), "effect/effect/config/low_color") => {
                    &value.low_color
                }
                (RasterModifierEffect::Threshold(value), "effect/effect/config/high_color") => {
                    &value.high_color
                }
                _ => return None,
            };
            (timeline.id == timeline_id).then_some(timeline)
        }
        _ => None,
    }
}

pub(crate) fn visual_modifier_color_by_id(
    item: &VideoItem,
    timeline_id: uuid::Uuid,
) -> Option<&shrimply_core::timeline_value::TimelineValue<shrimply_core::Color<u8>>> {
    item.modifiers
        .iter()
        .find_map(|modifier| match &modifier.effect {
            ModifierEffect::Scene3d(effect) => match &**effect {
                Scene3dModifierEffect::Object(value) => {
                    Some(&value.material.base_color).filter(|value| value.id == timeline_id)
                }
                Scene3dModifierEffect::PointLight(value) => {
                    Some(&value.color).filter(|value| value.id == timeline_id)
                }
                Scene3dModifierEffect::Shape(value) => {
                    Some(&value.material.base_color).filter(|value| value.id == timeline_id)
                }
                Scene3dModifierEffect::Text(value) => {
                    Some(&value.material.base_color).filter(|value| value.id == timeline_id)
                }
                Scene3dModifierEffect::SunLight(value) => {
                    Some(&value.color).filter(|value| value.id == timeline_id)
                }
                _ => None,
            },
            ModifierEffect::Raster(effect) => match &**effect {
                RasterModifierEffect::AlphaOutline(value) => {
                    Some(&value.color).filter(|value| value.id == timeline_id)
                }
                RasterModifierEffect::ChromaKey(value) => {
                    Some(&value.key_color).filter(|value| value.id == timeline_id)
                }
                RasterModifierEffect::ColorizeDuotone(value) => {
                    [&value.shadow_color, &value.highlight_color]
                        .into_iter()
                        .find(|value| value.id == timeline_id)
                }
                RasterModifierEffect::Dithering(value) => {
                    value.palette.iter().find(|value| value.id == timeline_id)
                }
                RasterModifierEffect::DropShadow(value) => {
                    Some(&value.color).filter(|value| value.id == timeline_id)
                }
                RasterModifierEffect::EdgeDetection(value) => {
                    [&value.edge_color, &value.background_color]
                        .into_iter()
                        .find(|value| value.id == timeline_id)
                }
                RasterModifierEffect::Threshold(value) => [&value.low_color, &value.high_color]
                    .into_iter()
                    .find(|value| value.id == timeline_id),
                _ => None,
            },
            _ => None,
        })
}

pub(crate) fn visual_modifier_number<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<&'a shrimply_core::timeline_value::TimelineValue<f32>> {
    let (modifier, field) = visual_modifier_at_path(item, path)?;
    match &modifier.effect {
        ModifierEffect::Vector(effect) => match &**effect {
            VectorModifierEffect::PathOffset(value) => path_offset::number(value, field, id),
            VectorModifierEffect::Repeat(value) => repeat::number(value, field, id),
            VectorModifierEffect::ShakyPath(value) => shaky_path::number(value, field, id),
            _ => modifier.number(id),
        },
        ModifierEffect::Raster(effect) => match &**effect {
            RasterModifierEffect::ScanlinesCrt(value) => scanlines_crt::number(value, field, id),
            RasterModifierEffect::TextureBounds(value) => texture_bounds::number(value, field, id),
            RasterModifierEffect::TransparentFill(value) => {
                transparent_fill::number(value, field, id)
            }
            _ => modifier.number(id),
        },
        ModifierEffect::Scene3d(effect) => match &**effect {
            Scene3dModifierEffect::Shape(value) => shape_3d::number(value, field, id),
            Scene3dModifierEffect::Text(value) => text_3d::number(value, field, id),
            Scene3dModifierEffect::SunLight(value) => sun_light::number(value, field, id),
            _ => modifier.number(id),
        },
        _ => modifier.number(id),
    }
}

pub(crate) fn visual_modifier_matches(item: &VideoItem, path: &str, id: uuid::Uuid) -> bool {
    visual_modifier_at_path(item, path).is_some_and(|(modifier, _)| modifier.id == id)
}

pub(crate) fn visual_modifier_vector2<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<&'a shrimply_core::timeline_value::TimelineValue<glam::Vec2>> {
    let (modifier, field) = visual_modifier_at_path(item, path)?;
    match &modifier.effect {
        ModifierEffect::Vector(effect) => match &**effect {
            VectorModifierEffect::Repeat(value) => repeat::vector2(value, field, id),
            _ => modifier.number2(id),
        },
        ModifierEffect::Raster(effect) => match &**effect {
            RasterModifierEffect::TransparentFill(value) => {
                transparent_fill::vector2(value, field, id)
            }
            _ => modifier.number2(id),
        },
        _ => modifier.number2(id),
    }
}

pub(crate) fn visual_modifier_vector3<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<&'a shrimply_core::timeline_value::TimelineValue<glam::Vec3>> {
    let (modifier, field) = visual_modifier_at_path(item, path)?;
    match &modifier.effect {
        ModifierEffect::Scene3d(effect) => match &**effect {
            Scene3dModifierEffect::Shape(value) => shape_3d::vector3(value, field, id),
            Scene3dModifierEffect::Text(value) => text_3d::vector3(value, field, id),
            Scene3dModifierEffect::SunLight(value) => sun_light::vector3(value, field, id),
            _ => modifier.effect.number3(id),
        },
        _ => modifier.effect.number3(id),
    }
}

pub(crate) fn set_visual_modifier_field(
    item: &mut VideoItem,
    path: &str,
    text: &str,
) -> Option<Result<bool, String>> {
    text_3d::set_field(item, path, text)
        .or_else(|| shape_3d::set_field(item, path, text))
        .or_else(|| vectorize::set_field(item, path, text))
}

pub(crate) fn visual_modifier_text<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<&'a shrimply_core::timeline_value::TimelineValue<String>> {
    let (modifier, field) = visual_modifier_at_path(item, path)?;
    let ModifierEffect::Scene3d(effect) = &modifier.effect else {
        return None;
    };
    let Scene3dModifierEffect::Text(value) = &**effect else {
        return None;
    };
    text_3d::text(value, field, id)
}

pub(crate) fn erode_dilate_operation<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<
    &'a shrimply_core::timeline_value::TimelineValue<
        shrimply_video_modifiers::erode_dilate::ErodeDilateOperation,
    >,
> {
    let (modifier, field) = visual_modifier_at_path(item, path)?;
    if field != "effect/effect/config/operation" {
        return None;
    }
    let ModifierEffect::Raster(effect) = &modifier.effect else {
        return None;
    };
    let RasterModifierEffect::ErodeDilate(value) = &**effect else {
        return None;
    };
    (value.operation.id == id).then_some(&value.operation)
}

pub(crate) fn halftone_mode<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<
    &'a shrimply_core::timeline_value::TimelineValue<
        shrimply_video_modifiers::halftone::HalftoneMode,
    >,
> {
    halftone::mode(item, path, id)
}

pub(crate) fn kuwahara_version_timeline<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<
    &'a shrimply_core::timeline_value::TimelineValue<
        shrimply_video_modifiers::kuwahara::KuwaharaVersion,
    >,
> {
    kuwahara::version_at_path(item, path, id)
}

pub(crate) fn rasterize_sample_method_timeline<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<&'a shrimply_core::timeline_value::TimelineValue<shrimply_core::VideoSampleMethod>> {
    rasterize::sample_method_at_path(item, path, id)
}

pub(crate) fn repeat_offset_axis_timeline<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<
    &'a shrimply_core::timeline_value::TimelineValue<
        shrimply_video_modifiers::repeat::RepeatOffsetAxis,
    >,
> {
    repeat::offset_axis_at_path(item, path, id)
}

pub(crate) fn sampling_method_timeline<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<&'a shrimply_core::timeline_value::TimelineValue<shrimply_core::VideoSampleMethod>> {
    sampling::method_at_path(item, path, id)
}

pub(crate) fn texture_bounds_address_mode_timeline<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<&'a shrimply_core::timeline_value::TimelineValue<shrimply_core::TextureAddressMode>> {
    texture_bounds::address_mode_at_path(item, path, id)
}

pub(crate) fn is_sampling_method(item: &VideoItem, path: &str) -> bool {
    sampling::is_method(item, path)
}

pub(crate) fn dithering_pattern<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<
    &'a shrimply_core::timeline_value::TimelineValue<
        shrimply_video_modifiers::dithering::DitheringPattern,
    >,
> {
    dithering::pattern(item, path, id)
}

pub(crate) fn dithering_color_mode<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<
    &'a shrimply_core::timeline_value::TimelineValue<
        shrimply_video_modifiers::dithering::DitheringColorMode,
    >,
> {
    dithering::color_mode(item, path, id)
}

pub(crate) fn mask_mode<'a>(
    item: &'a VideoItem,
    path: &str,
    id: uuid::Uuid,
) -> Option<
    &'a shrimply_core::timeline_value::TimelineValue<shrimply_video_modifiers::mask::MaskMode>,
> {
    mask::mode(item, path, id)
}

pub(crate) fn is_mask_mode(item: &VideoItem, path: &str) -> bool {
    mask::is_mode(item, path)
}

pub(crate) fn visual_modifier_at_path<'a, 'b>(
    item: &'a VideoItem,
    path: &'b str,
) -> Option<(&'a VisualModifier, &'b str)> {
    let (index, field) = path.strip_prefix("/modifiers/")?.split_once('/')?;
    Some((item.modifiers.get(index.parse::<usize>().ok()?)?, field))
}

pub fn default_visual_modifier_effect(effect: &ModifierEffect) -> ModifierEffect {
    if let ModifierEffect::Raster(effect) = effect {
        match &**effect {
            RasterModifierEffect::Transform(_) => {
                return ModifierEffect::raster(RasterModifierEffect::Transform(Default::default()));
            }
            RasterModifierEffect::Opacity(_) => {
                return ModifierEffect::raster(RasterModifierEffect::Opacity(Default::default()));
            }
            _ => {}
        }
    }
    let key = visual_modifier_key(effect);
    ModifierEffect::catalog()
        .find(|candidate| visual_modifier_key(candidate) == key)
        .expect("every visual modifier effect must have a catalog default")
}

pub(super) fn modifier_scalar_control(
    path: String,
    label: impl Into<String>,
    timeline: &shrimply_core::timeline_value::TimelineValue<f32>,
    runtime: InspectorRuntime,
    number: NumberSpec,
    rotating: bool,
) -> InspectorControl {
    let value = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
    let control = InspectorControl::new(ControlKind::LayeredNumber, path.clone(), label)
        .value(value.to_string())
        .number(number)
        .width_characters(8)
        .layered(path, LayeredState::from(timeline))
        .timeline(
            timeline.id,
            crate::transform::scalar_graph(timeline, value, runtime),
        )
        .live_commit("visual-modifier-value");
    if rotating {
        control.rotating_icon("rotation.svg", 0.0)
    } else {
        control
    }
}

pub(super) fn modifier_vector2_control(
    path: String,
    label: impl Into<String>,
    timeline: &shrimply_core::timeline_value::TimelineValue<glam::Vec2>,
    runtime: InspectorRuntime,
    number: NumberSpec,
    lock: bool,
) -> InspectorControl {
    let value = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
    let control = InspectorControl::new(ControlKind::LayeredVector2, path.clone(), label)
        .components(vec![value.x.to_string(), value.y.to_string()])
        .number(number)
        .width_characters(7)
        .prefixes(["X", "Y"])
        .layered(path, LayeredState::from(timeline))
        .timeline(
            timeline.id,
            crate::transform::vector_speed_graph(timeline, runtime),
        )
        .live_commit("visual-modifier-vector");
    if lock { control.lock() } else { control }
}

pub(super) fn modifier_vector3_control(
    path: String,
    label: impl Into<String>,
    timeline: &shrimply_core::timeline_value::TimelineValue<glam::Vec3>,
    runtime: InspectorRuntime,
    number: NumberSpec,
    lock: bool,
    rotating: bool,
) -> InspectorControl {
    let value = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
    let control = InspectorControl::new(ControlKind::LayeredVector3, path.clone(), label)
        .components(vec![
            value.x.to_string(),
            value.y.to_string(),
            value.z.to_string(),
        ])
        .number(number)
        .width_characters(5)
        .prefixes(["X", "Y", "Z"])
        .layered(path, LayeredState::from(timeline))
        .timeline(timeline.id, vector3_speed_graph(timeline, runtime))
        .live_commit("visual-modifier-vector");
    let control = if lock { control.lock() } else { control };
    if rotating {
        control.rotating_icon("rotation.svg", 0.0)
    } else {
        control
    }
}

pub(super) fn modifier_color_control(
    path: String,
    label: impl Into<String>,
    timeline: &shrimply_core::timeline_value::TimelineValue<shrimply_core::Color<u8>>,
    runtime: InspectorRuntime,
) -> InspectorControl {
    let value = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
    InspectorControl::new(ControlKind::LayeredColor, path.clone(), label)
        .components(vec![
            value.r.to_string(),
            value.g.to_string(),
            value.b.to_string(),
            value.a.to_string(),
        ])
        .layered(path, LayeredState::from(timeline))
        .timeline(
            timeline.id,
            crate::timeline_color::speed_graph(timeline, runtime),
        )
        .live_commit("visual-modifier-color")
}

pub(super) fn modifier_boolean_control(
    path: String,
    label: impl Into<String>,
    value: bool,
    commit: &'static str,
) -> InspectorControl {
    InspectorControl::new(ControlKind::Boolean, path, label)
        .value(value.to_string())
        .immediate_commit(commit)
}

pub(super) fn modifier_analysis_control(
    path: String,
    status: AnalysisControlPresentation,
    action: InspectorControlAction,
) -> InspectorControl {
    let mut control = InspectorControl::new(ControlKind::Analysis, path, "")
        .value(status.label.clone())
        .components(vec![
            status.progress.to_string(),
            u8::from(status.running).to_string(),
            u8::from(status.cancelling).to_string(),
            u8::from(status.suggested).to_string(),
        ])
        .sensitive(status.sensitive)
        .tooltip(status.tooltip.as_str())
        .busy(status.running || status.cancelling)
        .action(action);
    control.analysis = Some(status);
    control
}

pub(super) fn modifier_text_control(
    path: String,
    label: impl Into<String>,
    timeline: &shrimply_core::timeline_value::TimelineValue<String>,
    runtime: InspectorRuntime,
) -> InspectorControl {
    let value = crate::timeline_text::value_at(timeline, runtime.local_time.unwrap_or(Time::ZERO));
    InspectorControl::new(ControlKind::LayeredText, path.clone(), label)
        .value(value)
        .layered(path, LayeredState::from(timeline))
        .timeline(
            timeline.id,
            crate::timeline_text::speed_graph(timeline, runtime),
        )
        .live_commit("edit-3d-text")
        .timeline_commits(TEXT_3D_KEYFRAME_COMMITS.toggle, "3d-text-expression")
        .text_keyframe_commits(TEXT_3D_KEYFRAME_COMMITS)
}

pub(crate) fn vector3_speed_graph(
    timeline: &shrimply_core::timeline_value::TimelineValue<glam::Vec3>,
    runtime: InspectorRuntime,
) -> Option<crate::ScalarGraph> {
    crate::timeline_value::vector::scalar_speed_graph(timeline, runtime)
}

fn enum_text(value: impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .expect("visual modifier enum must serialize")
        .as_str()
        .expect("visual modifier enum must serialize as text")
        .to_string()
}

pub fn visual_modifier_action_valid(
    item: &VideoItem,
    id: uuid::Uuid,
    action: VisualModifierChainAction,
) -> bool {
    edited_visual_modifier_chain(item, id, action).is_some()
}

pub fn visual_modifier_catalog(item: &VideoItem) -> Vec<VisualModifierChoice> {
    let Ok(state) = item.modifier_output_state() else {
        return Vec::new();
    };
    ModifierEffect::catalog()
        .filter_map(|effect| {
            let key = visual_modifier_key(&effect);
            let effect = effect.adapted_for(state)?;
            Some(VisualModifierChoice {
                key,
                label: effect.display_name(),
                search_text: std::iter::once(effect.display_name())
                    .chain(effect.keywords().iter().copied())
                    .collect::<Vec<_>>()
                    .join(" "),
            })
        })
        .collect()
}

impl InspectorController {
    pub fn text_3d_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "3D text modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "3D text modifier is no longer available".to_string())?;
        let ModifierEffect::Scene3d(effect) = &modifier.effect else {
            return Err("3D text modifier is no longer available".to_string());
        };
        let Scene3dModifierEffect::Text(value) = &**effect else {
            return Err("3D text modifier is no longer available".to_string());
        };
        Ok(text_3d::presentation(value, index, modifier_id, runtime))
    }

    pub fn sun_light_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "Sun Light modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "Sun Light modifier is no longer available".to_string())?;
        let ModifierEffect::Scene3d(effect) = &modifier.effect else {
            return Err("Sun Light modifier is no longer available".to_string());
        };
        let Scene3dModifierEffect::SunLight(value) = &**effect else {
            return Err("Sun Light modifier is no longer available".to_string());
        };
        Ok(sun_light::presentation(value, index, modifier_id, runtime))
    }

    pub fn shape_3d_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "3D shape modifier is no longer available".to_string())?;
        let ModifierEffect::Scene3d(effect) = &modifier.effect else {
            return Err("3D shape modifier is no longer available".to_string());
        };
        let Scene3dModifierEffect::Shape(value) = &**effect else {
            return Err("3D shape modifier is no longer available".to_string());
        };
        Ok(shape_3d::presentation(value, index, modifier_id, runtime))
    }

    pub fn repeat_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "Repeat modifier is no longer available".to_string())?;
        let ModifierEffect::Vector(effect) = &modifier.effect else {
            return Err("Repeat modifier is no longer available".to_string());
        };
        let VectorModifierEffect::Repeat(value) = &**effect else {
            return Err("Repeat modifier is no longer available".to_string());
        };
        Ok(repeat::presentation(value, index, modifier_id, runtime))
    }

    pub fn sampling_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "Sampling modifier is no longer available".to_string())?;
        let ModifierEffect::Raster(effect) = &modifier.effect else {
            return Err("Sampling modifier is no longer available".to_string());
        };
        let RasterModifierEffect::Sampling(value) = &**effect else {
            return Err("Sampling modifier is no longer available".to_string());
        };
        Ok(sampling::presentation(value, index, modifier_id, runtime))
    }

    pub fn texture_bounds_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "Texture bounds modifier is no longer available".to_string())?;
        let ModifierEffect::Raster(effect) = &modifier.effect else {
            return Err("Texture bounds modifier is no longer available".to_string());
        };
        let RasterModifierEffect::TextureBounds(value) = &**effect else {
            return Err("Texture bounds modifier is no longer available".to_string());
        };
        Ok(texture_bounds::presentation(
            value,
            index,
            modifier_id,
            runtime,
        ))
    }

    pub fn scanlines_crt_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "Scanlines/CRT modifier is no longer available".to_string())?;
        let ModifierEffect::Raster(effect) = &modifier.effect else {
            return Err("Scanlines/CRT modifier is no longer available".to_string());
        };
        let RasterModifierEffect::ScanlinesCrt(value) = &**effect else {
            return Err("Scanlines/CRT modifier is no longer available".to_string());
        };
        Ok(scanlines_crt::presentation(
            value,
            index,
            modifier_id,
            runtime,
        ))
    }

    pub fn path_offset_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "Path offset modifier is no longer available".to_string())?;
        let ModifierEffect::Vector(effect) = &modifier.effect else {
            return Err("Path offset modifier is no longer available".to_string());
        };
        let VectorModifierEffect::PathOffset(value) = &**effect else {
            return Err("Path offset modifier is no longer available".to_string());
        };
        Ok(path_offset::presentation(
            value,
            index,
            modifier_id,
            runtime,
        ))
    }

    pub fn shaky_path_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "Shaky path modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "Shaky path modifier is no longer available".to_string())?;
        let ModifierEffect::Vector(effect) = &modifier.effect else {
            return Err("Shaky path modifier is no longer available".to_string());
        };
        let VectorModifierEffect::ShakyPath(value) = &**effect else {
            return Err("Shaky path modifier is no longer available".to_string());
        };
        Ok(shaky_path::presentation(value, index, modifier_id, runtime))
    }

    pub fn rasterize_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "Rasterize modifier is no longer available".to_string())?;
        let ModifierEffect::Rasterize(value) = &modifier.effect else {
            return Err("Rasterize modifier is no longer available".to_string());
        };
        Ok(rasterize::presentation(value, index, modifier_id, runtime))
    }

    pub fn kuwahara_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let project = self.project.borrow();
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "Kuwahara modifier is no longer available".to_string())?;
        let ModifierEffect::Raster(effect) = &modifier.effect else {
            return Err("Kuwahara modifier is no longer available".to_string());
        };
        let RasterModifierEffect::Kuwahara(value) = &**effect else {
            return Err("Kuwahara modifier is no longer available".to_string());
        };
        Ok(kuwahara::presentation(value, index, modifier_id, runtime))
    }

    pub fn set_visual_modifier_enabled(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        enabled: bool,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let index = item
            .modifiers
            .iter()
            .position(|modifier| modifier.id == id)
            .ok_or_else(|| "visual modifier is no longer available".to_string())?;
        if item.modifiers[index].enabled == enabled {
            return Ok(());
        }
        item.modifiers = visual_modifier_enabled_chain(item, id, enabled)
            .ok_or_else(|| "visual modifier cannot be toggled in this chain".to_string())?;
        shrimply_project::project::commit_edit(&project, "toggle-visual-modifier");
        drop(project);
        refresh(&self.player_state);
        Ok(())
    }

    pub fn reset_visual_modifier(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        effect: serde_json::Value,
    ) -> Result<(), String> {
        let effect = serde_json::from_value(effect)
            .map_err(|error| format!("invalid visual modifier: {error}"))?;
        self.reset_visual_modifier_effect(target, id, effect)
    }

    pub fn reset_visual_modifier_effect(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        effect: ModifierEffect,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let index = item
            .modifiers
            .iter()
            .position(|modifier| modifier.id == id)
            .ok_or_else(|| "visual modifier is no longer available".to_string())?;
        let mut modifiers = item.modifiers.clone();
        modifiers[index].effect = effect;
        modifiers[index].alpha_mask = None;
        if !modifier_chain_is_valid(item, &modifiers) {
            return Err("visual modifier reset would invalidate the chain".to_string());
        }
        item.modifiers = modifiers;
        shrimply_project::project::commit_edit(&project, "reset-visual-modifier");
        drop(project);
        refresh(&self.player_state);
        Ok(())
    }

    pub fn copy_visual_modifier(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        clipboard: &shrimply_property_transfer::SharedClipboard,
    ) -> Result<String, String> {
        let project = self.project.borrow();
        let modifier = project
            .video_item(video_address(target)?)
            .and_then(|item| item.modifiers.iter().find(|modifier| modifier.id == id))
            .ok_or_else(|| "visual modifier is no longer available".to_string())?;
        let title = modifier.effect.display_name().to_string();
        clipboard.borrow_mut().copy_visual_modifier(modifier);
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            ProjectChange {
                inspector: true,
                ..Default::default()
            },
        );
        Ok(title)
    }

    pub fn move_visual_modifier(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        offset: isize,
    ) -> Result<(), String> {
        let action = match offset {
            -1 => VisualModifierChainAction::MoveUp,
            1 => VisualModifierChainAction::MoveDown,
            _ => return Err("visual modifier move must be one position".to_string()),
        };
        self.edit_visual_modifier_chain(target, id, action)
    }

    pub fn remove_visual_modifier(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
    ) -> Result<(), String> {
        let project = self.project.borrow();
        let item = project
            .video_item(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        if !visual_modifier_action_valid(item, id, VisualModifierChainAction::Remove) {
            return Err("visual modifier removal would invalidate the chain".to_string());
        }
        let cached = item
            .modifiers
            .iter()
            .find(|modifier| modifier.id == id)
            .is_some_and(|modifier| {
                matches!(
                    modifier.effect,
                    ModifierEffect::Raster(ref effect)
                        if matches!(&**effect, RasterModifierEffect::Cache(_))
                )
            });
        drop(project);
        if cached {
            shrimply_video_cuda::modifier_cache::invalidate(id)?;
        }
        self.edit_visual_modifier_chain(target, id, VisualModifierChainAction::Remove)
    }

    fn edit_visual_modifier_chain(
        &self,
        target: &InspectorTarget,
        id: uuid::Uuid,
        action: VisualModifierChainAction,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(video_address(target)?)
            .ok_or_else(|| "visual modifier item is no longer available".to_string())?;
        let modifiers = edited_visual_modifier_chain(item, id, action)
            .ok_or_else(|| "visual modifier action would invalidate the chain".to_string())?;
        item.modifiers = modifiers;
        shrimply_project::project::commit_edit(
            &project,
            if action == VisualModifierChainAction::Remove {
                "remove-visual-modifier"
            } else {
                "move-visual-modifier"
            },
        );
        drop(project);
        refresh(&self.player_state);
        Ok(())
    }

    pub fn add_visual_modifier(
        &self,
        target: &InspectorTarget,
        key: &str,
    ) -> Result<uuid::Uuid, String> {
        let address = video_address(target)?;
        let position = player_state::snapshot(&self.player_state).position;
        let revision = player_state::snapshot(&self.player_state).revision;
        let mut project = self.project.borrow_mut();
        let audio = self
            .audio_sampler
            .borrow_mut()
            .sample(&project, position, revision);
        let item = project
            .video_item(address)
            .ok_or_else(|| "video item is no longer available".to_string())?;
        let state = item.modifier_output_state()?;
        let effect = ModifierEffect::catalog()
            .find(|effect| visual_modifier_key(effect) == key)
            .and_then(|effect| effect.adapted_for(state))
            .ok_or_else(|| format!("visual modifier is not available: {key}"))?;
        let effect = configured_effect(&project, item, position, &audio, effect);
        let modifier = VisualModifier::new(effect);
        let id = modifier.id;
        let item = project
            .video_item_mut(address)
            .expect("validated video item must remain available");
        let mut modifiers = item.modifiers.clone();
        modifiers.push(modifier);
        if !modifier_chain_is_valid(item, &modifiers) {
            return Err("visual modifier is not valid in this chain".to_string());
        }
        item.modifiers = modifiers;
        shrimply_project::project::commit_edit(&project, "add-visual-modifier");
        drop(project);
        refresh(&self.player_state);
        Ok(id)
    }

    pub fn can_paste_visual_modifiers(
        &self,
        target: &InspectorTarget,
        clipboard: &shrimply_property_transfer::SharedClipboard,
    ) -> bool {
        let Ok(address) = video_address(target) else {
            return false;
        };
        clipboard
            .borrow()
            .can_append_modifiers(&self.project.borrow(), std::slice::from_ref(address))
    }

    pub fn paste_visual_modifiers(
        &self,
        target: &InspectorTarget,
        clipboard: &shrimply_property_transfer::SharedClipboard,
    ) -> Result<usize, String> {
        let address = video_address(target)?.clone();
        let mut project = self.project.borrow_mut();
        let result = clipboard
            .borrow()
            .append_modifiers(&mut project, std::slice::from_ref(&address));
        if !result.changed {
            return Ok(0);
        }
        shrimply_project::project::commit_edit(&project, "paste-item-modifiers");
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            ProjectChange {
                video: result.video,
                inspector: true,
                ..Default::default()
            },
        );
        Ok(result.modifiers_added)
    }
}

fn configured_effect(
    project: &shrimply_project::project::Project,
    item: &VideoItem,
    position: Time,
    audio: &shrimply_evaluation::FrameAudioAnalysis,
    mut effect: ModifierEffect,
) -> ModifierEffect {
    let canvas = project.canvas_size;
    let canvas_size = glam::Vec2::new(canvas.width.max(1) as f32, canvas.height.max(1) as f32);
    let fallback = canvas_size * 0.5;
    let center = shrimply_evaluation::resolve_item_transform_with_audio(
        project,
        item,
        position,
        audio,
        &mut Default::default(),
    )
    .position;
    let center = if center.is_finite() { center } else { fallback };
    match &mut effect {
        ModifierEffect::Vector(effect) => {
            if let VectorModifierEffect::Transform(transform) = &mut **effect {
                **transform =
                    shrimply_video_modifiers::transform::TransformModifier::centered_at(center);
            }
        }
        ModifierEffect::Rasterize(rasterize) => {
            *rasterize = shrimply_video_modifiers::rasterize::RasterizeModifier::new(canvas_size);
        }
        ModifierEffect::Raster(effect) => {
            if let RasterModifierEffect::Transform(transform) = &mut **effect {
                **transform =
                    shrimply_video_modifiers::transform::TransformModifier::centered_at(center);
            }
        }
        ModifierEffect::Scene3d(_) | ModifierEffect::Vectorize(_) => {}
    }
    effect
}

fn visual_modifier_key(effect: &ModifierEffect) -> String {
    let value = serde_json::to_value(effect).expect("visual modifier catalog must serialize");
    let stage = value
        .get("stage")
        .and_then(serde_json::Value::as_str)
        .expect("visual modifier stage must serialize as text");
    value
        .get("effect")
        .and_then(|effect| effect.get("kind"))
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| stage.to_string(), |kind| format!("{stage}:{kind}"))
}

fn video_address(target: &InspectorTarget) -> Result<&ItemAddress, String> {
    match target {
        InspectorTarget::Item(address @ ItemAddress::Video { .. }) => Ok(address),
        _ => Err("inspector target is not a video item".to_string()),
    }
}

pub fn edited_visual_modifier_chain(
    item: &VideoItem,
    id: uuid::Uuid,
    action: VisualModifierChainAction,
) -> Option<Vec<VisualModifier>> {
    let mut modifiers = item.modifiers.clone();
    let index = modifiers.iter().position(|modifier| modifier.id == id)?;
    match action {
        VisualModifierChainAction::MoveUp if index > 0 => modifiers.swap(index, index - 1),
        VisualModifierChainAction::MoveDown if index + 1 < modifiers.len() => {
            modifiers.swap(index, index + 1);
        }
        VisualModifierChainAction::Remove => {
            modifiers.remove(index);
        }
        _ => return None,
    }
    modifier_chain_is_valid(item, &modifiers).then_some(modifiers)
}

pub fn visual_modifier_enabled_chain(
    item: &VideoItem,
    id: uuid::Uuid,
    enabled: bool,
) -> Option<Vec<VisualModifier>> {
    let mut modifiers = item.modifiers.clone();
    modifiers
        .iter_mut()
        .find(|modifier| modifier.id == id)?
        .enabled = enabled;
    modifier_chain_is_valid(item, &modifiers).then_some(modifiers)
}

pub fn modifier_chain_is_valid(item: &VideoItem, modifiers: &[VisualModifier]) -> bool {
    let Ok(state) = item.modifier_output_state_for(modifiers) else {
        return false;
    };
    !item
        .compositing
        .alpha_mask
        .as_ref()
        .is_some_and(|mask| mask.enabled)
        || state.kind == VisualKind::Raster
}

fn refresh(state: &shrimply_state::player_state::SharedPlayerState) {
    player_state::refresh_project(
        state,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
}
