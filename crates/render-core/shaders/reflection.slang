import abi;

[shader("compute")] void reflect_abi(
    uniform AffineStabilizationParams affine_stabilization,
    uniform CornerPinParams corner_pin,
    uniform LayerCompositeParams layer_composite,
    uniform BulgePinchParams bulge_pinch,
    uniform ChannelMixerParams channel_mixer,
    uniform ChromaticAberrationParams chromatic_aberration,
    uniform ColorCorrectionParams color_correction,
    uniform ColorizeDuotoneParams colorize_duotone,
    uniform RadialBlurParams radial_blur, uniform ZoomBlurParams zoom_blur,
    uniform DitheringParams dithering, uniform DropShadowParams drop_shadow,
    uniform EdgeDetectionParams edge_detection,
    uniform KaleidoscopeParams kaleidoscope, uniform MaskParams mask,
    uniform AlphaMaskParams alpha_mask,
    uniform ShapeAlphaMaskParams shape_alpha_mask,
    uniform ScanlinesCrtParams scanlines_crt, uniform HalftoneParams halftone,
    uniform ThresholdParams threshold, uniform TwirlParams twirl,
    uniform WaveRippleParams wave_ripple, uniform Nv12LayerParams nv12_layer,
    uniform ChromaKeyParams chroma_key,
    uniform VisualTransitionMaskParams visual_transition,
    uniform Sam2ProxyParams sam2_proxy, uniform Sam2MaskParams sam2_mask,
    uniform TransparentFillMaskParams transparent_fill,
    [RustType("shrimply_math_color::LayerBlendMode")]
    uniform LayerBlendMode layer_blend_mode,
    uniform ImageDescriptor image, uniform ConvolutionTerm convolution_term,
    uniform ConvolutionParams convolution, uniform AlphaParams alpha) {}
