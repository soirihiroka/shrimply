//! Ordered dispatch inputs for the existing Slang pixel kernels. Backends own
//! allocation and argument binding; pass order and resolved values are shared.
use crate::ColorCorrectionParams;

#[derive(Clone, Copy)]
pub struct CornerPin {
    pub inverse_homography: glam::Mat3,
    pub corners: [glam::Vec2; 4],
    pub perspective: f32,
}

#[derive(Clone, Copy)]
pub enum PixelEffect {
    CornerPin(CornerPin),
    DropShadow(crate::DropShadowParams),
    GlowBloom {
        threshold: f32,
        radius: u32,
        intensity: f32,
    },

    ChromaKey(crate::ChromaKeyParams),
    BulgePinch {
        center: glam::Vec2,
        radius_fraction: f32,
        strength: f32,
    },
    Twirl {
        center: glam::Vec2,
        radius_fraction: f32,
        angle: f32,
    },
    WaveRipple(crate::WaveRippleParams),
    DisplacementMap {
        amount: f32,
        scale: f32,
        phase: f32,
    },
    Fisheye {
        intensity: f32,
        center: glam::Vec2,
    },
    LensDistortion {
        distortion: f32,
        center: glam::Vec2,
    },
    Kaleidoscope(crate::KaleidoscopeParams),
    ChannelMixer(glam::Mat3),
    ColorizeDuotone(crate::ColorizeDuotoneParams),
    Threshold(crate::ThresholdParams),
    EdgeDetection(crate::EdgeDetectionParams),
    FilmGrain {
        amount: f32,
        size: f32,
        colored: f32,
        seed: f32,
    },
    ScanlinesCrt(crate::ScanlinesCrtParams),
    Halftone(crate::HalftoneParams),
    AlphaOutline {
        radius: u32,
        color: u32,
    },
    Erode(u32),
    Dilate(u32),
    Kuwahara {
        radius: u32,
        generalized: bool,
    },
    Invert(f32),
    PixelateMosaic {
        block_width: u32,
        block_height: u32,
    },
    Sharpen {
        amount: f32,
        radius: u32,
    },
    ChromaticAberration([f32; 4]),
    Emboss {
        direction: f32,
        depth: f32,
        amount: f32,
    },
    LumaKey {
        threshold: f32,
        softness: f32,
        invert: bool,
    },
    DirectionalBlur {
        radius: u32,
        angle: f32,
    },
    ZoomBlur(crate::ZoomBlurParams),
    RadialBlur(crate::RadialBlurParams),
    TransitionMask(TransitionMask),
    TransitionBlur(u32),
    TransitionPixelate(u32),
    Posterize(f32),
    Mirror {
        horizontal: u32,
        vertical: u32,
    },
    Vignette {
        amount: f32,
        midpoint: f32,
        softness: f32,
    },
    ColorCorrection(ColorCorrectionParams),
    GaussianBlur {
        radius_x: u32,
        radius_y: u32,
        blur_rgb: bool,
        blur_alpha: bool,
    },
}

#[derive(Clone, Copy)]
pub enum Module {
    General,
    Geometry,
    Matte,
    Blur,
}

#[derive(Clone, Copy)]
pub enum BufferSlot {
    Input,
    Output,
    Scratch,
}

pub enum Value {
    CornerPin {
        parameters: CornerPin,
        width: u32,
        height: u32,
    },
    DropShadow(crate::DropShadowParams),
    Kaleidoscope(crate::KaleidoscopeParams),
    ChannelMixer(glam::Mat3),
    Halftone(crate::HalftoneParams),
    Buffer(BufferSlot),
    U32(u32),
    U64(u64),
    F32(f32),
    Floats(Vec<f32>),
    SampledBlur {
        center: glam::Vec2,
        amount: f32,
        samples: u32,
    },
    Bool(bool),
    ColorCorrection(ColorCorrectionParams),
    TransitionMask(crate::VisualTransitionMaskParams),
}

impl Value {
    /// Encode declaration-order CUDA arguments. Metal reuses matching scalar and
    /// vector layouts; matrix-containing blocks use named backend reflection.
    pub fn bytes(&self, buffer_address: impl FnOnce(BufferSlot) -> u64) -> Vec<u8> {
        match *self {
            Self::CornerPin {
                parameters,
                width,
                height,
            } => {
                let mut bytes: Vec<_> = parameters
                    .corners
                    .into_iter()
                    .flat_map(|corner| corner.to_array())
                    .flat_map(f32::to_ne_bytes)
                    .collect();
                bytes.extend(buffer_address(BufferSlot::Input).to_ne_bytes());
                bytes.extend(width.to_ne_bytes());
                bytes.extend(height.to_ne_bytes());
                bytes.extend(
                    parameters
                        .inverse_homography
                        .to_cols_array()
                        .into_iter()
                        .flat_map(f32::to_ne_bytes),
                );
                bytes.extend(parameters.perspective.to_ne_bytes());
                bytes
            }
            Self::DropShadow(params) => {
                let mut bytes: Vec<_> = params
                    .offset
                    .to_array()
                    .into_iter()
                    .flat_map(f32::to_ne_bytes)
                    .collect();
                bytes.extend(params.radius.to_ne_bytes());
                bytes.extend(params.color.to_ne_bytes());
                bytes
            }

            Self::Kaleidoscope(params) => {
                let mut bytes: Vec<_> = params
                    .center
                    .to_array()
                    .into_iter()
                    .flat_map(f32::to_ne_bytes)
                    .collect();
                bytes.extend(params.segments.to_ne_bytes());
                bytes.extend(params.rotation.to_ne_bytes());
                bytes
            }
            Self::ChannelMixer(matrix) => matrix
                .to_cols_array()
                .into_iter()
                .flat_map(f32::to_ne_bytes)
                .collect(),
            Self::Halftone(params) => {
                let mut bytes: Vec<_> = [params.size, params.angle, params.contrast]
                    .into_iter()
                    .flat_map(f32::to_ne_bytes)
                    .collect();
                bytes.extend(params.mode.to_ne_bytes());
                bytes.extend(params.channel_offset.to_ne_bytes());
                bytes.extend(params.channel_angle_offset.to_ne_bytes());
                bytes
            }
            Self::Buffer(slot) => buffer_address(slot).to_ne_bytes().to_vec(),
            Self::U32(value) => value.to_ne_bytes().to_vec(),
            Self::U64(value) => value.to_ne_bytes().to_vec(),
            Self::F32(value) => value.to_ne_bytes().to_vec(),
            Self::Floats(ref values) => values
                .iter()
                .flat_map(|value| value.to_ne_bytes())
                .collect(),
            Self::SampledBlur {
                center,
                amount,
                samples,
            } => {
                let mut bytes: Vec<_> = [center.x, center.y, amount]
                    .into_iter()
                    .flat_map(f32::to_ne_bytes)
                    .collect();
                bytes.extend(samples.to_ne_bytes());
                bytes
            }
            Self::Bool(value) => vec![u8::from(value)],
            Self::TransitionMask(params) => {
                let mut bytes = (params.kind as u32).to_ne_bytes().to_vec();
                for value in [
                    params.visibility,
                    params.angle_degrees,
                    params.softness,
                    params.center.x,
                    params.center.y,
                ] {
                    bytes.extend(value.to_ne_bytes());
                }
                bytes.extend(params.grain_size.to_ne_bytes());
                bytes.extend(params.line_variation.to_ne_bytes());
                bytes
            }
            Self::ColorCorrection(params) => [
                params.exposure,
                params.gamma,
                params.temperature,
                params.tint,
                params.brightness,
                params.contrast,
                params.hue_turns,
                params.saturation,
                params.value,
            ]
            .into_iter()
            .flat_map(f32::to_ne_bytes)
            .collect(),
        }
    }
}

pub struct Pass {
    pub module: Module,
    pub kernel: &'static str,
    /// CUDA follows declaration order; Metal looks up these names in reflection.
    pub arguments: Vec<(&'static str, Value)>,
}

pub fn needs_canvas_materialization(
    source_size: (u32, u32),
    canvas_size: (u32, u32),
    spatial_identity: bool,
) -> bool {
    !spatial_identity || source_size != canvas_size
}

impl PixelEffect {
    pub fn name(&self) -> &'static str {
        match self {
            Self::CornerPin(_) => "Corner pin",
            Self::DropShadow(_) => "Drop shadow",
            Self::GlowBloom { .. } => "Glow / bloom",
            Self::ChromaKey(_) => "Chroma key",
            Self::BulgePinch { .. } => "Bulge / pinch",
            Self::Twirl { .. } => "Twirl",
            Self::WaveRipple(_) => "Wave / ripple",
            Self::DisplacementMap { .. } => "Displacement map",
            Self::Fisheye { .. } => "Fisheye",
            Self::LensDistortion { .. } => "Lens distortion",
            Self::Kaleidoscope(_) => "Kaleidoscope",
            Self::ChannelMixer(_) => "Channel mixer",
            Self::ColorizeDuotone(_) => "Colorize / duotone",
            Self::Threshold(_) => "Threshold",
            Self::EdgeDetection(_) => "Edge detection",
            Self::FilmGrain { .. } => "Film grain",
            Self::ScanlinesCrt(_) => "Scanlines / CRT",
            Self::Halftone(_) => "Halftone",
            Self::AlphaOutline { .. } => "Alpha outline",
            Self::Erode(_) | Self::Dilate(_) => "Erode / dilate",
            Self::Kuwahara { .. } => "Kuwahara",
            Self::Invert(_) => "Invert",
            Self::PixelateMosaic { .. } => "Pixelate / mosaic",
            Self::Sharpen { .. } => "Sharpen",
            Self::ChromaticAberration(_) => "Chromatic aberration",
            Self::Emboss { .. } => "Emboss",
            Self::LumaKey { .. } => "Luma key",
            Self::DirectionalBlur { .. } => "Directional blur",
            Self::ZoomBlur(_) => "Zoom blur",
            Self::RadialBlur(_) => "Radial blur",
            Self::TransitionMask(_) => "Visual transition mask",
            Self::TransitionBlur(_) => "Transition blur",
            Self::TransitionPixelate(_) => "Transition pixelate",
            Self::Posterize(_) => "Posterize",
            Self::Mirror { .. } => "Mirror",
            Self::Vignette { .. } => "Vignette",
            Self::ColorCorrection(_) => "Color correction",
            Self::GaussianBlur { .. } => "Gaussian blur",
        }
    }

    pub fn is_identity(&self) -> bool {
        if let Self::CornerPin(parameters) = self {
            return parameters.corners == shrimply_math_geometry::UNIT_QUAD;
        }
        matches!(
            self,
            Self::Erode(0) | Self::Dilate(0) | Self::Kuwahara { radius: 0, .. }
        )
    }

    pub fn scratch_words_per_pixel(&self) -> usize {
        // Kuwahara stores left/right RGB sums and squared luminance in the float[8]
        // statistics buffer; bloom stores a four-float premultiplied color.
        const KUWAHARA_STATISTICS_COMPONENTS: usize = 8;
        const COLOR_COMPONENTS: usize = 4;
        match self {
            Self::Kuwahara { .. } => KUWAHARA_STATISTICS_COMPONENTS,
            Self::GlowBloom { .. } => COLOR_COMPONENTS,
            Self::DropShadow(_)
            | Self::GaussianBlur { .. }
            | Self::TransitionBlur(_)
            | Self::Sharpen { .. }
            | Self::AlphaOutline { .. }
            | Self::Erode(_)
            | Self::Dilate(_) => 1,
            _ => 0,
        }
    }
}

/// Pixel effects operate on a canvas-sized image before layer opacity and blend
/// mode. Preserve those final compositing values while baking spatial state once.
pub fn materialization(
    mut source: crate::Nv12LayerParams,
    width: u32,
    height: u32,
) -> (crate::Nv12LayerParams, crate::Nv12LayerParams) {
    let mut baked = source;
    source.opacity = 1.0;
    source.blend_mode = crate::LayerBlendMode::Normal;
    baked.source_width = width;
    baked.source_height = height;
    baked.rgba_pitch = width as usize * size_of::<u32>();
    baked.inverse = crate::math::Mat3::IDENTITY;
    baked.crop = [0.0; 4];
    baked.padding = [0.0; 4];
    baked.address_mode = crate::TextureAddressMode::Transparent;
    baked.kind = crate::LayerKind::Rgba;
    baked.motion_transform_offset = 0;
    baked.motion_transform_count = 0;
    baked.motion_sample_count = 0;
    (source, baked)
}

#[derive(Clone, Copy)]
pub struct TransitionMask {
    pub kind: crate::VisualTransitionMaskKind,
    pub visibility: f32,
    pub angle_degrees: f32,
    pub softness: f32,
    pub center: glam::Vec2,
    pub normalized_center: bool,
    pub grain_size: u32,
    pub line_variation: f32,
}

mod passes;
