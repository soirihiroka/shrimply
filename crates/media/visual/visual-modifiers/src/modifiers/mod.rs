use hashbrown::HashSet;

use serde::{Deserialize, Serialize};
#[cfg(feature = "preview")]
use std::any::Any;
use uuid::Uuid;

#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{PreviewBuilder, PreviewProvider, PreviewTarget};

pub use shrimply_property_model::modifier_model::*;
use shrimply_property_model::timeline_value::*;

pub mod alpha_outline;
pub mod bulge_pinch;
pub mod cache;
pub mod channel_mixer;
pub mod chroma_key;
pub mod chromatic_aberration;
pub mod color_correction;
pub mod colorize_duotone;
pub mod corner_pin;
pub mod crop;
pub mod directional_blur;
pub mod displacement_map;
pub mod dithering;
pub mod drop_shadow;
pub mod edge_detection;
pub mod emboss;
pub mod erode_dilate;
pub mod film_grain;
pub mod fisheye;
pub mod gaussian_blur;
pub mod glow_bloom;
pub mod halftone;
pub mod hsv;
pub mod invert;
pub mod kaleidoscope;
pub mod kuwahara;
pub mod lens_distortion;
pub mod luma_key;
pub mod mask;
pub mod mirror;
pub mod opacity;
pub mod path_offset;
pub mod pixelate_mosaic;
pub mod posterize;
#[cfg(feature = "preview")]
mod preview;
pub mod radial_blur;
pub mod rasterize;
pub mod repeat;
pub mod sam2;
pub mod sampling;
pub mod scanlines_crt;
pub use shrimply_scene_3d_modifiers as scene_3d;
pub mod shaky_path;
pub mod sharpen;
pub mod text_mask;
pub mod texture_bounds;
pub mod threshold;
pub mod transform;
pub mod transparent_fill;
pub mod twirl;
pub mod vectorize;
pub mod vignette;
pub mod wave_ripple;
pub mod zoom_blur;

#[cfg(feature = "preview")]
pub use preview::MODIFIER_PREVIEW_FACET;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualKind {
    Scene3d,
    Vector,
    Manim,
    Background,
    Raster,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModifierSource {
    Other,
    Image,
    Paint,
    Text,
    Obj,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModifierState {
    pub source: ModifierSource,
    pub kind: VisualKind,
    pub pristine: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModifierOutput {
    Preserve,
    Raster,
    Vector,
}

#[derive(Clone, Copy, Debug)]
pub struct ModifierContract {
    pub scene_3d: bool,
    pub vector: bool,
    pub manim: bool,
    pub background: bool,
    pub raster: bool,
    pub source: Option<ModifierSource>,
    pub output: ModifierOutput,
    pub requires_pristine: bool,
}

impl ModifierContract {
    pub const SCENE_3D: Self = Self {
        scene_3d: true,
        vector: false,
        manim: false,
        background: false,
        raster: false,
        source: Some(ModifierSource::Obj),
        output: ModifierOutput::Preserve,
        requires_pristine: false,
    };
    pub const VECTOR: Self = Self {
        scene_3d: false,
        vector: true,
        manim: false,
        background: false,
        raster: false,
        source: None,
        output: ModifierOutput::Preserve,
        requires_pristine: false,
    };
    pub const PAINT: Self = Self {
        source: Some(ModifierSource::Paint),
        ..Self::VECTOR
    };
    pub const TEXT: Self = Self {
        source: Some(ModifierSource::Text),
        ..Self::VECTOR
    };
    pub const RASTER: Self = Self {
        scene_3d: false,
        vector: false,
        manim: false,
        background: false,
        raster: true,
        source: None,
        output: ModifierOutput::Preserve,
        requires_pristine: false,
    };
    pub const RASTERIZE: Self = Self {
        scene_3d: true,
        vector: true,
        manim: true,
        background: true,
        raster: false,
        source: None,
        output: ModifierOutput::Raster,
        requires_pristine: false,
    };
    pub const VECTORIZE: Self = Self {
        scene_3d: false,
        vector: false,
        manim: false,
        background: false,
        raster: true,
        source: Some(ModifierSource::Image),
        output: ModifierOutput::Vector,
        requires_pristine: true,
    };

    pub fn accepts(self, state: ModifierState) -> bool {
        self.source.is_none_or(|source| source == state.source)
            && (!self.requires_pristine || state.pristine)
            && match state.kind {
                VisualKind::Scene3d => self.scene_3d,
                VisualKind::Vector => self.vector,
                VisualKind::Manim => self.manim,
                VisualKind::Background => self.background,
                VisualKind::Raster => self.raster,
            }
    }

    pub fn output(self, mut input: ModifierState) -> ModifierState {
        match self.output {
            ModifierOutput::Preserve => {}
            ModifierOutput::Raster => input.kind = VisualKind::Raster,
            ModifierOutput::Vector => input.kind = VisualKind::Vector,
        }
        input.pristine = false;
        input
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "stage", content = "effect", rename_all = "snake_case")]
pub enum ModifierEffect {
    Scene3d(Box<scene_3d::Scene3dModifierEffect>),
    Vectorize(vectorize::VectorizeModifier),
    Vector(Box<VectorModifierEffect>),
    Rasterize(rasterize::RasterizeModifier),
    Raster(Box<RasterModifierEffect>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum VectorModifierEffect {
    Transform(Box<transform::TransformModifier>),
    Repeat(Box<repeat::RepeatModifier>),
    ShakyPath(Box<shaky_path::ShakyPathModifier>),
    PathOffset(Box<path_offset::PathOffsetModifier>),
    Opacity(opacity::OpacityModifier),
    Hsv(hsv::HsvModifier),
    TextMask(text_mask::TextMaskModifier),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum RasterModifierEffect {
    Cache(cache::CacheModifier),
    Transform(Box<transform::TransformModifier>),
    TextureBounds(texture_bounds::TextureBoundsModifier),
    Sampling(sampling::SamplingModifier),
    Crop(crop::CropModifier),
    CornerPin(corner_pin::CornerPinModifier),
    Opacity(opacity::OpacityModifier),
    ChromaKey(chroma_key::ChromaKeyModifier),
    Kuwahara(kuwahara::KuwaharaModifier),
    GaussianBlur(gaussian_blur::GaussianBlurModifier),
    Fisheye(fisheye::FisheyeModifier),
    Sharpen(sharpen::SharpenModifier),
    Vignette(vignette::VignetteModifier),
    PixelateMosaic(pixelate_mosaic::PixelateMosaicModifier),
    Posterize(posterize::PosterizeModifier),
    Threshold(threshold::ThresholdModifier),
    FilmGrain(film_grain::FilmGrainModifier),
    ChromaticAberration(chromatic_aberration::ChromaticAberrationModifier),
    EdgeDetection(edge_detection::EdgeDetectionModifier),
    Emboss(emboss::EmbossModifier),
    DirectionalBlur(directional_blur::DirectionalBlurModifier),
    Dithering(dithering::DitheringModifier),
    GlowBloom(glow_bloom::GlowBloomModifier),
    Twirl(twirl::TwirlModifier),
    BulgePinch(bulge_pinch::BulgePinchModifier),
    WaveRipple(wave_ripple::WaveRippleModifier),
    Mirror(mirror::MirrorModifier),
    Kaleidoscope(kaleidoscope::KaleidoscopeModifier),
    ColorizeDuotone(colorize_duotone::ColorizeDuotoneModifier),
    Invert(invert::InvertModifier),
    ChannelMixer(Box<channel_mixer::ChannelMixerModifier>),
    AlphaOutline(alpha_outline::AlphaOutlineModifier),
    DropShadow(drop_shadow::DropShadowModifier),
    Halftone(halftone::HalftoneModifier),
    ScanlinesCrt(scanlines_crt::ScanlinesCrtModifier),
    LensDistortion(lens_distortion::LensDistortionModifier),
    DisplacementMap(displacement_map::DisplacementMapModifier),
    LumaKey(luma_key::LumaKeyModifier),
    Mask(mask::MaskModifier),
    Sam2(sam2::Sam2Modifier),
    TransparentFill(transparent_fill::TransparentFillModifier),
    RadialBlur(radial_blur::RadialBlurModifier),
    ZoomBlur(zoom_blur::ZoomBlurModifier),
    ErodeDilate(erode_dilate::ErodeDilateModifier),
    ColorCorrection(Box<color_correction::ColorCorrectionModifier>),
}

impl VectorModifierEffect {
    pub const CATALOG: &[fn() -> Self] = &[
        || Self::Transform(Default::default()),
        || Self::Opacity(Default::default()),
        || Self::Repeat(Default::default()),
        || Self::ShakyPath(Default::default()),
        || Self::PathOffset(Default::default()),
        || Self::Hsv(Default::default()),
        || Self::TextMask(Default::default()),
    ];
}

impl RasterModifierEffect {
    pub const CATALOG: &[fn() -> Self] = &[
        || Self::Cache(Default::default()),
        || Self::TextureBounds(Default::default()),
        || Self::Sampling(Default::default()),
        || Self::Crop(Default::default()),
        || Self::CornerPin(Default::default()),
        || Self::ChromaKey(Default::default()),
        || Self::Kuwahara(Default::default()),
        || Self::GaussianBlur(Default::default()),
        || Self::Fisheye(Default::default()),
        || Self::Sharpen(Default::default()),
        || Self::Vignette(Default::default()),
        || Self::PixelateMosaic(Default::default()),
        || Self::Posterize(Default::default()),
        || Self::Threshold(Default::default()),
        || Self::FilmGrain(Default::default()),
        || Self::ChromaticAberration(Default::default()),
        || Self::EdgeDetection(Default::default()),
        || Self::Emboss(Default::default()),
        || Self::DirectionalBlur(Default::default()),
        || Self::Dithering(Default::default()),
        || Self::GlowBloom(Default::default()),
        || Self::Twirl(Default::default()),
        || Self::BulgePinch(Default::default()),
        || Self::WaveRipple(Default::default()),
        || Self::Mirror(Default::default()),
        || Self::Kaleidoscope(Default::default()),
        || Self::ColorizeDuotone(Default::default()),
        || Self::Invert(Default::default()),
        || Self::ChannelMixer(Default::default()),
        || Self::AlphaOutline(Default::default()),
        || Self::DropShadow(Default::default()),
        || Self::Halftone(Default::default()),
        || Self::ScanlinesCrt(Default::default()),
        || Self::LensDistortion(Default::default()),
        || Self::DisplacementMap(Default::default()),
        || Self::LumaKey(Default::default()),
        || Self::Mask(Default::default()),
        || Self::Sam2(Default::default()),
        || Self::TransparentFill(Default::default()),
        || Self::RadialBlur(Default::default()),
        || Self::ZoomBlur(Default::default()),
        || Self::ErodeDilate(Default::default()),
        || Self::ColorCorrection(Default::default()),
    ];
}

macro_rules! impl_modifier_model_enum {
    ($name:ty, $($variant:ident),+ $(,)?) => {
        impl $name {
            fn model(&self) -> &dyn ModifierModel {
                match self { $(Self::$variant(value) => value),+ }
            }

            fn model_mut(&mut self) -> &mut dyn ModifierModel {
                match self { $(Self::$variant(value) => value),+ }
            }
        }

        impl ModifierModel for $name {
            fn display_name(&self) -> &'static str {
                self.model().display_name()
            }
            fn keywords(&self) -> &'static [&'static str] {
                self.model().keywords()
            }
            fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
                self.model_mut().ensure_ids(seen)
            }
            fn keyframe_span(&self) -> KeyframeSpan {
                self.model().keyframe_span()
            }
            fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
                self.model().number(id)
            }
            fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
                self.model_mut().number_mut(id)
            }
            fn number2(&self, id: Uuid) -> Option<&TimelineValue<glam::Vec2>> {
                self.model().number2(id)
            }
            fn number2_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<glam::Vec2>> {
                self.model_mut().number2_mut(id)
            }
            fn number3(&self, id: Uuid) -> Option<&TimelineValue<glam::Vec3>> {
                self.model().number3(id)
            }
            fn number3_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<glam::Vec3>> {
                self.model_mut().number3_mut(id)
            }
            fn color_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<shrimply_property_model::Color<u8>>> {
                self.model_mut().color_mut(id)
            }
            fn text(&self, id: Uuid) -> Option<&TimelineValue<String>> {
                self.model().text(id)
            }
            fn text_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<String>> {
                self.model_mut().text_mut(id)
            }
            fn integer(&self, id: Uuid) -> Option<&TimelineValue<u32>> {
                self.model().integer(id)
            }
            fn integer_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<u32>> {
                self.model_mut().integer_mut(id)
            }
            fn sample_method(&self, id: Uuid) -> Option<&TimelineValue<shrimply_property_model::VideoSampleMethod>> {
                self.model().sample_method(id)
            }
            fn sample_method_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<shrimply_property_model::VideoSampleMethod>> {
                self.model_mut().sample_method_mut(id)
            }
        }
    };
}

impl_modifier_model_enum!(
    VectorModifierEffect,
    Transform,
    Repeat,
    ShakyPath,
    PathOffset,
    Opacity,
    Hsv,
    TextMask,
);
impl_modifier_model_enum!(
    RasterModifierEffect,
    Cache,
    Transform,
    TextureBounds,
    Sampling,
    Crop,
    CornerPin,
    Opacity,
    ChromaKey,
    Kuwahara,
    GaussianBlur,
    Fisheye,
    Sharpen,
    Vignette,
    PixelateMosaic,
    Posterize,
    Threshold,
    FilmGrain,
    ChromaticAberration,
    EdgeDetection,
    Emboss,
    DirectionalBlur,
    Dithering,
    GlowBloom,
    Twirl,
    BulgePinch,
    WaveRipple,
    Mirror,
    Kaleidoscope,
    ColorizeDuotone,
    Invert,
    ChannelMixer,
    AlphaOutline,
    DropShadow,
    Halftone,
    ScanlinesCrt,
    LensDistortion,
    DisplacementMap,
    LumaKey,
    Mask,
    Sam2,
    TransparentFill,
    RadialBlur,
    ZoomBlur,
    ErodeDilate,
    ColorCorrection,
);
impl_modifier_model_enum!(
    ModifierEffect,
    Scene3d,
    Vectorize,
    Vector,
    Rasterize,
    Raster
);

impl ModifierEffect {
    pub fn scene_3d(effect: scene_3d::Scene3dModifierEffect) -> Self {
        Self::Scene3d(Box::new(effect))
    }

    pub fn vector(effect: VectorModifierEffect) -> Self {
        Self::Vector(Box::new(effect))
    }

    pub fn raster(effect: RasterModifierEffect) -> Self {
        Self::Raster(Box::new(effect))
    }

    pub fn catalog() -> impl Iterator<Item = Self> {
        scene_3d::Scene3dModifierEffect::CATALOG
            .iter()
            .map(|new| Self::scene_3d(new()))
            .chain(std::iter::once_with(|| Self::Vectorize(Default::default())))
            .chain(
                VectorModifierEffect::CATALOG
                    .iter()
                    .map(|new| Self::vector(new())),
            )
            .chain(std::iter::once_with(|| Self::Rasterize(Default::default())))
            .chain(
                RasterModifierEffect::CATALOG
                    .iter()
                    .map(|new| Self::raster(new())),
            )
    }

    pub fn contract(&self) -> ModifierContract {
        match self {
            Self::Scene3d(_) => ModifierContract::SCENE_3D,
            Self::Vectorize(_) => ModifierContract::VECTORIZE,
            Self::Vector(effect) => match &**effect {
                VectorModifierEffect::PathOffset(_) => ModifierContract::PAINT,
                VectorModifierEffect::TextMask(_) => ModifierContract::TEXT,
                _ => ModifierContract::VECTOR,
            },
            Self::Rasterize(_) => ModifierContract::RASTERIZE,
            Self::Raster(_) => ModifierContract::RASTER,
        }
    }

    pub fn adapted_for(&self, input: ModifierState) -> Option<Self> {
        if self.contract().accepts(input) {
            return Some(self.clone());
        }
        MODIFIER_ADAPTERS
            .iter()
            .find_map(|adapter| adapter(self, input))
    }

    #[cfg(feature = "preview")]
    pub fn preview_target_mut(&mut self, target: PreviewTarget) -> Option<&mut dyn Any> {
        if target.facet() != MODIFIER_PREVIEW_FACET {
            return None;
        }
        match self {
            Self::Vector(effect) => effect.preview_target_mut(target),
            Self::Raster(effect) => effect.preview_target_mut(target),
            Self::Scene3d(_) | Self::Vectorize(_) | Self::Rasterize(_) => None,
        }
    }
}

impl ModifierEffect {
    #[cfg(feature = "preview")]
    pub fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        match self {
            Self::Vector(effect) => effect.preview_provider(target, builder),
            Self::Raster(effect) => effect.preview_provider(target, builder),
            Self::Scene3d(_) | Self::Vectorize(_) | Self::Rasterize(_) => None,
        }
    }
}

impl VectorModifierEffect {
    #[cfg(feature = "preview")]
    fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        match self {
            Self::Transform(effect) => effect.preview_provider(target, builder),
            Self::Repeat(effect) => effect.preview_provider(target, builder),
            Self::ShakyPath(_)
            | Self::PathOffset(_)
            | Self::Opacity(_)
            | Self::Hsv(_)
            | Self::TextMask(_) => None,
        }
    }
}

impl VectorModifierEffect {
    #[cfg(feature = "preview")]
    fn preview_target_mut(&mut self, target: PreviewTarget) -> Option<&mut dyn Any> {
        match self {
            Self::Transform(effect) if preview::is_target(target) => Some(effect.as_mut()),
            Self::Repeat(effect) if preview::is_target(target) => Some(effect.as_mut()),
            _ => None,
        }
    }
}

impl RasterModifierEffect {
    #[cfg(feature = "preview")]
    fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        match self {
            Self::Cache(_) => None,
            Self::Transform(effect) => effect.preview_provider(target, builder),
            Self::TextureBounds(effect) => effect.preview_provider(target, builder),
            Self::Crop(effect) => effect.preview_provider(target, builder),
            Self::CornerPin(effect) => effect.preview_provider(target, builder),
            Self::Fisheye(effect) => effect.preview_provider(target, builder),
            Self::Twirl(effect) => effect.preview_provider(target, builder),
            Self::BulgePinch(effect) => effect.preview_provider(target, builder),
            Self::Kaleidoscope(effect) => effect.preview_provider(target, builder),
            Self::LensDistortion(effect) => effect.preview_provider(target, builder),
            Self::RadialBlur(effect) => effect.preview_provider(target, builder),
            Self::ZoomBlur(effect) => effect.preview_provider(target, builder),
            Self::DirectionalBlur(effect) => effect.preview_provider(target, builder),
            Self::Emboss(effect) => effect.preview_provider(target, builder),
            Self::WaveRipple(effect) => effect.preview_provider(target, builder),
            Self::Mask(effect) => effect.preview_provider(target, builder),
            Self::Sam2(effect) => effect.preview_provider(target, builder),
            Self::TransparentFill(effect) => effect.preview_provider(target, builder),
            Self::ChromaticAberration(effect) => effect.preview_provider(target, builder),
            Self::PixelateMosaic(effect) => effect.preview_provider(target, builder),
            Self::Vignette(effect) => effect.preview_provider(target, builder),
            Self::ScanlinesCrt(effect) => effect.preview_provider(target, builder),
            Self::ErodeDilate(effect) => effect.preview_provider(target, builder),
            Self::DisplacementMap(effect) => effect.preview_provider(target, builder),
            _ => None,
        }
    }
}

impl RasterModifierEffect {
    #[cfg(feature = "preview")]
    fn preview_target_mut(&mut self, target: PreviewTarget) -> Option<&mut dyn Any> {
        match self {
            Self::Transform(effect) if preview::is_target(target) => Some(effect.as_mut()),
            Self::TextureBounds(effect) if preview::is_target(target) => Some(effect),
            Self::Crop(effect) if preview::is_target(target) => Some(effect),
            Self::CornerPin(effect) if preview::is_target(target) => Some(effect),
            Self::Fisheye(effect) if preview::is_target(target) => Some(effect),
            Self::Twirl(effect) if preview::is_target(target) => Some(effect),
            Self::BulgePinch(effect) if preview::is_target(target) => Some(effect),
            Self::Kaleidoscope(effect) if preview::is_target(target) => Some(effect),
            Self::LensDistortion(effect) if preview::is_target(target) => Some(effect),
            Self::RadialBlur(effect) if preview::is_target(target) => Some(effect),
            Self::ZoomBlur(effect) if preview::is_target(target) => Some(effect),
            Self::DirectionalBlur(effect) if preview::is_target(target) => Some(effect),
            Self::Emboss(effect) if preview::is_target(target) => Some(effect),
            Self::WaveRipple(effect) if preview::is_target(target) => Some(effect),
            Self::Sam2(effect) if preview::is_target(target) => Some(effect),
            Self::TransparentFill(effect) if preview::is_target(target) => Some(effect),
            Self::ChromaticAberration(effect) if preview::is_target(target) => Some(effect),
            Self::PixelateMosaic(effect) if preview::is_target(target) => Some(effect),
            Self::Vignette(effect) if preview::is_target(target) => Some(effect),
            Self::ScanlinesCrt(effect) if preview::is_target(target) => Some(effect),
            Self::ErodeDilate(effect) if preview::is_target(target) => Some(effect),
            Self::DisplacementMap(effect) if preview::is_target(target) => Some(effect),
            _ => None,
        }
    }
}

type ModifierAdapter = fn(&ModifierEffect, ModifierState) -> Option<ModifierEffect>;

const MODIFIER_ADAPTERS: &[ModifierAdapter] = &[adapt_transform, adapt_opacity];

fn adapt_transform(effect: &ModifierEffect, input: ModifierState) -> Option<ModifierEffect> {
    let value = match effect {
        ModifierEffect::Vector(effect) => match &**effect {
            VectorModifierEffect::Transform(value) => value.as_ref(),
            _ => return None,
        },
        ModifierEffect::Raster(effect) => match &**effect {
            RasterModifierEffect::Transform(value) => value.as_ref(),
            _ => return None,
        },
        ModifierEffect::Scene3d(_)
        | ModifierEffect::Vectorize(_)
        | ModifierEffect::Rasterize(_) => return None,
    };
    match input.kind {
        VisualKind::Vector => Some(ModifierEffect::vector(VectorModifierEffect::Transform(
            Box::new(value.clone()),
        ))),
        VisualKind::Raster => Some(ModifierEffect::raster(RasterModifierEffect::Transform(
            Box::new(value.clone()),
        ))),
        VisualKind::Scene3d | VisualKind::Manim | VisualKind::Background => None,
    }
}

fn adapt_opacity(effect: &ModifierEffect, input: ModifierState) -> Option<ModifierEffect> {
    let value = match effect {
        ModifierEffect::Vector(effect) => match &**effect {
            VectorModifierEffect::Opacity(value) => value,
            _ => return None,
        },
        ModifierEffect::Raster(effect) => match &**effect {
            RasterModifierEffect::Opacity(value) => value,
            _ => return None,
        },
        ModifierEffect::Scene3d(_)
        | ModifierEffect::Vectorize(_)
        | ModifierEffect::Rasterize(_) => return None,
    };
    match input.kind {
        VisualKind::Vector => Some(ModifierEffect::vector(VectorModifierEffect::Opacity(
            value.clone(),
        ))),
        VisualKind::Raster => Some(ModifierEffect::raster(RasterModifierEffect::Opacity(
            value.clone(),
        ))),
        VisualKind::Scene3d | VisualKind::Manim | VisualKind::Background => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ModifierLifecycle {
    state: ModifierState,
}

impl ModifierLifecycle {
    pub fn new(state: ModifierState) -> Self {
        Self { state }
    }

    pub fn state(self) -> ModifierState {
        self.state
    }

    pub fn apply(
        &mut self,
        index: usize,
        enabled: bool,
        effect: &ModifierEffect,
    ) -> Result<(), String> {
        if !enabled {
            return Ok(());
        }
        let contract = effect.contract();
        if !contract.accepts(self.state) {
            if contract.requires_pristine {
                return Err(format!(
                    "modifier {} requires Vectorize to be the first enabled modifier on an image",
                    index + 1,
                ));
            }
            return Err(format!(
                "modifier {} requires {} input after {:?}",
                index + 1,
                if contract.source == Some(ModifierSource::Paint) {
                    "Paint"
                } else if contract.source == Some(ModifierSource::Text) {
                    "text"
                } else if contract.source == Some(ModifierSource::Obj) {
                    "OBJ scene"
                } else if contract.scene_3d && contract.vector && contract.manim {
                    "scene 3D, vector, or Manim"
                } else if contract.scene_3d {
                    "scene 3D"
                } else if contract.vector {
                    "vector"
                } else if contract.manim {
                    "Manim"
                } else {
                    "raster"
                },
                self.state.kind,
            ));
        }
        self.state = contract.output(self.state);
        Ok(())
    }
}

pub fn chain_output<'a>(
    initial: ModifierState,
    modifiers: impl IntoIterator<Item = (bool, &'a ModifierEffect)>,
) -> Result<ModifierState, String> {
    let mut lifecycle = ModifierLifecycle::new(initial);
    for (index, (enabled, effect)) in modifiers.into_iter().enumerate() {
        lifecycle.apply(index, enabled, effect)?;
    }
    Ok(lifecycle.state())
}
