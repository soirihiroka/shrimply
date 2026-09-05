use super::*;

impl PixelEffect {
    pub fn passes(&self, width: u32, height: u32) -> Vec<Pass> {
        use BufferSlot::{Input, Output, Scratch};
        use Value::{Bool, Buffer, F32, U32, U64};
        let count = u64::from(width) * u64::from(height);
        match *self {
            Self::CornerPin(parameters) => vec![Pass {
                module: Module::Geometry,
                kernel: "corner_pin",
                arguments: vec![
                    (
                        "params",
                        Value::CornerPin {
                            parameters,
                            width,
                            height,
                        },
                    ),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                ],
            }],
            Self::DropShadow(params) => vec![
                Pass {
                    module: Module::General,
                    kernel: "drop_shadow_horizontal",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("width", U32(width)),
                        ("height", U32(height)),
                        ("out", Buffer(Scratch)),
                        ("out_len", U64(count)),
                        ("params", Value::DropShadow(params)),
                    ],
                },
                Pass {
                    module: Module::General,
                    kernel: "drop_shadow_vertical",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("horizontal", Buffer(Scratch)),
                        ("width", U32(width)),
                        ("height", U32(height)),
                        ("out", Buffer(Output)),
                        ("out_len", U64(count)),
                        ("params", Value::DropShadow(params)),
                    ],
                },
            ],
            Self::GlowBloom {
                threshold,
                radius,
                intensity,
            } => vec![
                Pass {
                    module: Module::Blur,
                    kernel: "glow_bloom_horizontal",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("width", U32(width)),
                        ("output", Buffer(Scratch)),
                        ("output_count", U64(count)),
                        ("threshold", F32(threshold)),
                        ("radius_u", U32(radius)),
                    ],
                },
                Pass {
                    module: Module::Blur,
                    kernel: "glow_bloom_vertical",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("horizontal", Buffer(Scratch)),
                        ("width", U32(width)),
                        ("height", U32(height)),
                        ("output", Buffer(Output)),
                        ("output_count", U64(count)),
                        ("radius_u", U32(radius)),
                        ("intensity", F32(intensity)),
                    ],
                },
            ],

            Self::ChromaKey(params) => vec![Pass {
                module: Module::Matte,
                kernel: "chroma_key",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("output", Buffer(Output)),
                    ("output_count", U64(count)),
                    (
                        "params",
                        Value::Floats(vec![
                            params.key.r,
                            params.key.g,
                            params.key.b,
                            params.key.a,
                            params.similarity,
                            params.softness,
                            params.spill,
                        ]),
                    ),
                ],
            }],

            Self::BulgePinch {
                center,
                radius_fraction,
                strength,
            } => vec![Pass {
                module: Module::Geometry,
                kernel: "bulge_pinch",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("w", U32(width)),
                    ("h", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    (
                        "params",
                        Value::Floats(vec![
                            center.x,
                            center.y,
                            radius_fraction * width.min(height) as f32,
                            strength,
                        ]),
                    ),
                ],
            }],
            Self::Twirl {
                center,
                radius_fraction,
                angle,
            } => vec![Pass {
                module: Module::Geometry,
                kernel: "twirl",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("w", U32(width)),
                    ("h", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    (
                        "params",
                        Value::Floats(vec![
                            center.x,
                            center.y,
                            radius_fraction * width.min(height) as f32,
                            angle,
                        ]),
                    ),
                ],
            }],
            Self::WaveRipple(params) => vec![Pass {
                module: Module::Geometry,
                kernel: "wave_ripple",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("w", U32(width)),
                    ("h", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    (
                        "params",
                        Value::Floats(vec![
                            params.amplitude,
                            params.wavelength,
                            params.angle,
                            params.phase,
                        ]),
                    ),
                ],
            }],
            Self::DisplacementMap {
                amount,
                scale,
                phase,
            } => vec![Pass {
                module: Module::Geometry,
                kernel: "displacement_map",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("w", U32(width)),
                    ("h", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("amount", F32(amount)),
                    ("scale", F32(scale)),
                    ("phase", F32(phase)),
                ],
            }],
            Self::Fisheye { intensity, center } => vec![Pass {
                module: Module::Geometry,
                kernel: "fisheye",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("intensity", F32(intensity)),
                    ("center_value", Value::Floats(center.to_array().to_vec())),
                ],
            }],
            Self::LensDistortion { distortion, center } => vec![Pass {
                module: Module::Geometry,
                kernel: "lens_distortion",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("w", U32(width)),
                    ("h", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("distortion", F32(distortion)),
                    ("center", Value::Floats(center.to_array().to_vec())),
                ],
            }],
            Self::Kaleidoscope(params) => vec![Pass {
                module: Module::Geometry,
                kernel: "kaleidoscope",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("w", U32(width)),
                    ("h", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("params", Value::Kaleidoscope(params)),
                ],
            }],
            Self::ChannelMixer(matrix) => vec![Pass {
                module: Module::General,
                kernel: "channel_mixer",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("params", Value::ChannelMixer(matrix)),
                ],
            }],
            Self::ColorizeDuotone(params) => vec![Pass {
                module: Module::General,
                kernel: "colorize_duotone",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    (
                        "params",
                        Value::Floats(vec![
                            params.shadow.r,
                            params.shadow.g,
                            params.shadow.b,
                            params.shadow.a,
                            params.highlight.r,
                            params.highlight.g,
                            params.highlight.b,
                            params.highlight.a,
                        ]),
                    ),
                ],
            }],
            Self::Threshold(params) => vec![Pass {
                module: Module::General,
                kernel: "threshold",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    (
                        "params",
                        Value::Floats(vec![
                            params.low.r,
                            params.low.g,
                            params.low.b,
                            params.low.a,
                            params.high.r,
                            params.high.g,
                            params.high.b,
                            params.high.a,
                            params.threshold,
                        ]),
                    ),
                ],
            }],
            Self::EdgeDetection(params) => vec![Pass {
                module: Module::General,
                kernel: "edge_detection",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    (
                        "params",
                        Value::Floats(vec![
                            params.edge.r,
                            params.edge.g,
                            params.edge.b,
                            params.edge.a,
                            params.background.r,
                            params.background.g,
                            params.background.b,
                            params.background.a,
                            params.amount,
                        ]),
                    ),
                ],
            }],
            Self::FilmGrain {
                amount,
                size,
                colored,
                seed,
            } => vec![Pass {
                module: Module::General,
                kernel: "film_grain",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("amount", F32(amount)),
                    ("size", F32(size)),
                    ("colored", F32(colored)),
                    ("seed", F32(seed)),
                ],
            }],
            Self::ScanlinesCrt(params) => vec![Pass {
                module: Module::General,
                kernel: "scanlines_crt",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    (
                        "params",
                        Value::Floats(vec![
                            params.spacing,
                            params.intensity,
                            params.curvature,
                            params.mask,
                        ]),
                    ),
                ],
            }],
            Self::Halftone(params) => vec![Pass {
                module: Module::General,
                kernel: "halftone",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("params", Value::Halftone(params)),
                ],
            }],
            Self::AlphaOutline { radius, color } => vec![
                Pass {
                    module: Module::General,
                    kernel: "alpha_outline_horizontal",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("width", U32(width)),
                        ("out", Buffer(Scratch)),
                        ("out_len", U64(count)),
                        ("radius", U32(radius)),
                    ],
                },
                Pass {
                    module: Module::General,
                    kernel: "alpha_outline_vertical",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("horizontal", Buffer(Scratch)),
                        ("width", U32(width)),
                        ("height", U32(height)),
                        ("out", Buffer(Output)),
                        ("out_len", U64(count)),
                        ("radius", U32(radius)),
                        ("outline_value", U32(color)),
                    ],
                },
            ],
            Self::Erode(radius) | Self::Dilate(radius) => vec![
                Pass {
                    module: Module::Blur,
                    kernel: if matches!(self, Self::Erode(_)) {
                        "erode_horizontal"
                    } else {
                        "dilate_horizontal"
                    },
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("width", U32(width)),
                        ("output", Buffer(Scratch)),
                        ("output_count", U64(count)),
                        ("radius", U32(radius)),
                    ],
                },
                Pass {
                    module: Module::Blur,
                    kernel: if matches!(self, Self::Erode(_)) {
                        "erode_vertical"
                    } else {
                        "dilate_vertical"
                    },
                    arguments: vec![
                        ("source", Buffer(Input)),
                        ("horizontal", Buffer(Scratch)),
                        ("width", U32(width)),
                        ("height", U32(height)),
                        ("output", Buffer(Output)),
                        ("output_count", U64(count)),
                        ("radius", U32(radius)),
                    ],
                },
            ],
            Self::Kuwahara {
                radius,
                generalized,
            } => vec![
                Pass {
                    module: Module::Blur,
                    kernel: "kuwahara_horizontal_statistics",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("width", U32(width)),
                        ("output", Buffer(Scratch)),
                        ("output_count", U64(count)),
                        ("radius", U32(radius)),
                    ],
                },
                Pass {
                    module: Module::Blur,
                    kernel: "kuwahara_vertical",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("statistics", Buffer(Scratch)),
                        ("width", U32(width)),
                        ("height", U32(height)),
                        ("output", Buffer(Output)),
                        ("output_count", U64(count)),
                        ("radius", U32(radius)),
                        ("generalized", Bool(generalized)),
                    ],
                },
            ],
            Self::PixelateMosaic {
                block_width,
                block_height,
            } => vec![Pass {
                module: Module::Geometry,
                kernel: "pixelate_mosaic",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("block_width", U32(block_width)),
                    ("block_height", U32(block_height)),
                ],
            }],
            Self::ChromaticAberration(offsets) => vec![Pass {
                module: Module::General,
                kernel: "chromatic_aberration",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("params", Value::Floats(offsets.to_vec())),
                ],
            }],
            Self::Emboss {
                direction,
                depth,
                amount,
            } => vec![Pass {
                module: Module::General,
                kernel: "emboss",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("direction", F32(direction)),
                    ("depth", F32(depth)),
                    ("amount", F32(amount)),
                ],
            }],
            Self::LumaKey {
                threshold,
                softness,
                invert,
            } => vec![Pass {
                module: Module::Matte,
                kernel: "luma_key",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("output", Buffer(Output)),
                    ("output_count", U64(count)),
                    ("threshold", F32(threshold)),
                    ("softness", F32(softness)),
                    ("invert", Bool(invert)),
                ],
            }],
            Self::DirectionalBlur { radius, angle } => vec![Pass {
                module: Module::Blur,
                kernel: "directional_blur",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("output", Buffer(Output)),
                    ("output_count", U64(count)),
                    ("radius", U32(radius)),
                    ("angle", F32(angle)),
                ],
            }],
            Self::ZoomBlur(params) => vec![Pass {
                module: Module::Blur,
                kernel: "zoom_blur",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("output", Buffer(Output)),
                    ("output_count", U64(count)),
                    (
                        "params",
                        Value::SampledBlur {
                            center: params.center,
                            amount: params.strength,
                            samples: params.samples,
                        },
                    ),
                ],
            }],
            Self::RadialBlur(params) => vec![Pass {
                module: Module::Blur,
                kernel: "radial_blur",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("output", Buffer(Output)),
                    ("output_count", U64(count)),
                    (
                        "params",
                        Value::SampledBlur {
                            center: params.center,
                            amount: params.angle,
                            samples: params.samples,
                        },
                    ),
                ],
            }],
            Self::Sharpen { amount, radius } => vec![
                Pass {
                    module: Module::Blur,
                    kernel: "sharpen_blur_horizontal",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("width", U32(width)),
                        ("output", Buffer(Output)),
                        ("output_count", U64(count)),
                        ("radius", U32(radius)),
                    ],
                },
                Pass {
                    module: Module::Blur,
                    kernel: "sharpen_blur_vertical",
                    arguments: vec![
                        ("input", Buffer(Output)),
                        ("width", U32(width)),
                        ("height", U32(height)),
                        ("output", Buffer(Scratch)),
                        ("output_count", U64(count)),
                        ("radius", U32(radius)),
                    ],
                },
                Pass {
                    module: Module::Blur,
                    kernel: "unsharp_mask",
                    arguments: vec![
                        ("original", Buffer(Input)),
                        ("blurred", Buffer(Scratch)),
                        ("output", Buffer(Output)),
                        ("output_count", U64(count)),
                        ("amount", F32(amount)),
                    ],
                },
            ],
            Self::TransitionMask(mask) => vec![Pass {
                module: Module::Matte,
                kernel: "visual_transition_mask",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("output", Buffer(Output)),
                    ("output_count", U64(count)),
                    (
                        "params",
                        Value::TransitionMask(crate::VisualTransitionMaskParams {
                            kind: mask.kind,
                            visibility: mask.visibility,
                            angle_degrees: mask.angle_degrees,
                            softness: mask.softness,
                            center: if mask.normalized_center {
                                mask.center * glam::Vec2::new(width as f32, height as f32)
                            } else {
                                mask.center
                            },
                            grain_size: mask.grain_size,
                            line_variation: mask.line_variation,
                        }),
                    ),
                ],
            }],
            Self::TransitionPixelate(block_size) => vec![Pass {
                module: Module::Matte,
                kernel: "transition_pixelate",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("output", Buffer(Output)),
                    ("output_count", U64(count)),
                    ("block_width", U32(block_size)),
                    ("block_height", U32(block_size)),
                ],
            }],
            Self::TransitionBlur(radius) => vec![
                Pass {
                    module: Module::Matte,
                    kernel: "transition_gaussian_blur_horizontal",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("width", U32(width)),
                        ("output", Buffer(Scratch)),
                        ("output_count", U64(count)),
                        ("radius_u", U32(radius)),
                    ],
                },
                Pass {
                    module: Module::Matte,
                    kernel: "transition_gaussian_blur_vertical",
                    arguments: vec![
                        ("input", Buffer(Scratch)),
                        ("width", U32(width)),
                        ("height", U32(height)),
                        ("output", Buffer(Output)),
                        ("output_count", U64(count)),
                        ("radius_u", U32(radius)),
                    ],
                },
            ],
            Self::Posterize(levels) => vec![Pass {
                module: Module::General,
                kernel: "posterize",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("levels", F32(levels)),
                ],
            }],
            Self::Mirror {
                horizontal,
                vertical,
            } => vec![Pass {
                module: Module::Geometry,
                kernel: "mirror",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("w", U32(width)),
                    ("h", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("horizontal", U32(horizontal)),
                    ("vertical", U32(vertical)),
                ],
            }],
            Self::Vignette {
                amount,
                midpoint,
                softness,
            } => vec![Pass {
                module: Module::General,
                kernel: "vignette",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("width", U32(width)),
                    ("height", U32(height)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("amount", F32(amount)),
                    ("midpoint", F32(midpoint)),
                    ("softness", F32(softness)),
                ],
            }],
            Self::Invert(amount) => vec![Pass {
                module: Module::General,
                kernel: "invert",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("amount", F32(amount)),
                ],
            }],
            Self::ColorCorrection(params) => vec![Pass {
                module: Module::General,
                kernel: "color_correction",
                arguments: vec![
                    ("input", Buffer(Input)),
                    ("out", Buffer(Output)),
                    ("out_len", U64(count)),
                    ("params", Value::ColorCorrection(params)),
                ],
            }],
            Self::GaussianBlur {
                radius_x,
                radius_y,
                blur_rgb,
                blur_alpha,
            } => vec![
                Pass {
                    module: Module::Blur,
                    kernel: "gaussian_blur_horizontal",
                    arguments: vec![
                        ("input", Buffer(Input)),
                        ("width", U32(width)),
                        ("output", Buffer(Scratch)),
                        ("output_count", U64(count)),
                        ("radius", U32(radius_x)),
                    ],
                },
                Pass {
                    module: Module::Blur,
                    kernel: "gaussian_blur_vertical",
                    arguments: vec![
                        ("input", Buffer(Scratch)),
                        ("original", Buffer(Input)),
                        ("width", U32(width)),
                        ("height", U32(height)),
                        ("output", Buffer(Output)),
                        ("output_count", U64(count)),
                        ("radius", U32(radius_y)),
                        ("blur_rgb", Bool(blur_rgb)),
                        ("blur_alpha", Bool(blur_alpha)),
                    ],
                },
            ],
        }
    }
}
