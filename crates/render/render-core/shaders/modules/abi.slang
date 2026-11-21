static const uint COMPUTE_GROUP_SIZE = 256u;

#if defined(SHRIMPLY_PORTABLE_INDICES)
typedef uint BufferIndex;
// Preserve the native 64-bit pitch fields in uploaded layer structures without
// requiring 64-bit arithmetic. Portable allocations are bounded to UINT32_MAX.
struct BufferPitch { uint low; uint high; };
BufferIndex buffer_pitch(BufferPitch value) { return value.low; }
#else
typedef uint64_t BufferIndex;
#if defined(SHRIMPLY_VULKAN)
typedef uint64_t BufferPitch;
#else
typedef size_t BufferPitch;
#endif
BufferIndex buffer_pitch(BufferPitch value) { return BufferIndex(value); }
#endif

#if defined(SHRIMPLY_WORD_STORAGE)
// The physical-storage-buffer extension permits vector pointer bitcasts before
// SPIR-V 1.5. Emit that instruction directly: older Slang profiles otherwise
// introduce unnecessary 64-bit arithmetic for the same conversion.
uint2 pointer_words<T>(T *value) {
  return spirv_asm { result:$$uint2 = OpBitcast $value };
}
typedef uint *WordPointer;
WordPointer word_pointer(uint2 value) {
  return spirv_asm { result:$$WordPointer = OpBitcast $value };
}
#endif

bool buffer_is_null<T>(T *value) {
#if defined(SHRIMPLY_WORD_STORAGE)
  return all(pointer_words(value) == uint2(0u));
#elif defined(SHRIMPLY_PORTABLE_INDICES)
  return all(bit_cast<uint2>(value) == uint2(0u));
#else
  return value == nullptr;
#endif
}

// Shader storage can be word-backed without changing byte-oriented media layouts.
#if defined(SHRIMPLY_WORD_STORAGE)
typedef uint ByteStorage;
typedef uint SignedByteStorage;
typedef uint HalfStorage;
typedef uint EnumStorage;

uint *packed_words(uint *data, inout BufferIndex byte_index) {
  uint2 address = pointer_words(data);
  byte_index += address.x & 3u;
  address.x &= ~3u;
  return word_pointer(address);
}
uint load_byte(ByteStorage *data, BufferIndex index) {
  uint *words = packed_words(data, index);
  return (words[index / 4u] >> ((index % 4u) * 8u)) & 255u;
}
int load_signed_byte(SignedByteStorage *data, BufferIndex index) {
  return int(load_byte(data, index) << 24u) >> 24;
}
uint load_half(HalfStorage *data, BufferIndex index) {
  BufferIndex byte_index = index * 2u;
  uint *words = packed_words(data, byte_index);
  uint shift = (byte_index % 4u) * 8u;
  uint value = words[byte_index / 4u] >> shift;
  if (shift == 24u)
    value |= words[byte_index / 4u + 1u] << 8u;
  return value & 65535u;
}
void store_byte(ByteStorage *data, BufferIndex index, uint value) {
  uint *words = packed_words(data, index);
  uint shift = (index % 4u) * 8u;
  // Different invocations own distinct bytes of the same word. Atomic masks
  // preserve their neighboring bytes without requiring byte storage features.
  InterlockedAnd(words[index / 4u], ~(255u << shift));
  InterlockedOr(words[index / 4u], (value & 255u) << shift);
}
void store_half(HalfStorage *data, BufferIndex index, uint value) {
  store_byte(data, index * 2u, value);
  store_byte(data, index * 2u + 1u, value >> 8u);
}
#else
typedef uint8_t ByteStorage;
typedef int8_t SignedByteStorage;
typedef uint16_t HalfStorage;
typedef uint8_t EnumStorage;
uint load_byte(ByteStorage *data, BufferIndex index) { return data[index]; }
int load_signed_byte(SignedByteStorage *data, BufferIndex index) { return data[index]; }
uint load_half(HalfStorage *data, BufferIndex index) { return data[index]; }
void store_byte(ByteStorage *data, BufferIndex index, uint value) { data[index] = uint8_t(value); }
void store_half(HalfStorage *data, BufferIndex index, uint value) { data[index] = uint16_t(value); }
#endif

[__AttributeUsage(_AttributeTargets.Struct)]
[__AttributeUsage(_AttributeTargets.Param)]
struct RustTypeAttribute { string name; };

[RustType("glam::Vec2")] struct Vec2 {
  float x;
  float y;
};

[RustType("glam::Mat3")] struct Mat3 {
  float3 x_axis;
  float3 y_axis;
  float3 z_axis;
};

[RustType("shrimply_math_color::Color")] struct Color {
  float r;
  float g;
  float b;
  float a;
};

[RustType("shrimply_math_color::ColorCorrectionParams")] struct
    ColorCorrectionParams {
      float exposure;
      float gamma;
      float temperature;
      float tint;
      float brightness;
      float contrast;
      float hue_turns;
      float saturation;
      float value;
    };

enum LayerBlendMode : EnumStorage {
  Normal = 1,
  PassThrough = 0,
  Dissolve = 2,
  Darken = 3,
  Multiply = 4,
  ColorBurn = 5,
  LinearBurn = 6,
  DarkerColor = 7,
  Lighten = 8,
  Screen = 9,
  ColorDodge = 10,
  Add = 11,
  LighterColor = 12,
  Overlay = 13,
  SoftLight = 14,
  HardLight = 15,
  VividLight = 16,
  LinearLight = 17,
  PinLight = 18,
  HardMix = 19,
  Difference = 20,
  Exclusion = 21,
  Subtract = 22,
  Divide = 23,
  Reflect = 24,
  Hue = 25,
  Saturation = 26,
  Color = 27,
  Luminosity = 28,
};

struct AffineStabilizationParams {
  uint *input;
  uint width;
  uint height;
  Mat3 source_transform;
};

struct CornerPinParams {
  Vec2 corners[4];
  uint *input;
  uint width;
  uint height;
  Mat3 inverse_homography;
  float perspective;
};

struct LayerCompositeParams {
  uint *source;
  uint *clipping_base;
  BufferPitch source_pitch;
  BufferPitch clipping_base_pitch;
  uint width;
  float opacity;
  float clipping_base_opacity;
  uint noise_seed;
  LayerBlendMode mode;
};

enum DitheringPattern : uint {
  Bayer2x2,
  Bayer4x4,
  Bayer8x8,
};

enum DitheringColorMode : uint {
  Color,
  Grayscale,
  Palette,
};

enum MorphologyOperation : uint {
  Erode,
  Dilate,
};

struct BulgePinchParams {
  Vec2 center;
  float radius;
  float strength;
};

struct ChannelMixerParams {
  Mat3 matrix;
};

struct ChromaticAberrationParams {
  float offsets[4];
};

struct ColorizeDuotoneParams {
  Color shadow;
  Color highlight;
};

struct RadialBlurParams {
  Vec2 center;
  float angle;
  uint samples;
};

struct ZoomBlurParams {
  Vec2 center;
  float strength;
  uint samples;
};

struct DitheringParams {
  uint *palette;
  float levels;
  float amount;
  uint palette_len;
  DitheringPattern pattern;
  DitheringColorMode color_mode;
};

struct DropShadowParams {
  Vec2 offset;
  uint radius;
  uint color;
};

struct EdgeDetectionParams {
  Color edge;
  Color background;
  float amount;
};

struct KaleidoscopeParams {
  Vec2 center;
  uint segments;
  float rotation;
};

struct MaskParams {
  uint *mask;
  uint input_width;
  uint mask_width;
  uint mask_height;
  Mat3 transform;
  bool luminance;
  bool invert;
};

struct AlphaMaskParams {
  uint *mask;
  uint input_width;
  uint input_height;
  uint mask_width;
  uint mask_height;
};

enum ShapeAlphaMaskKind : uint {
  Rectangle,
  Ellipse,
  Polygon,
};

struct ShapeAlphaMaskParams {
  uint *base;
  Vec2 center;
  Vec2 size;
  Vec2 *vertices;
  uint input_width;
  Mat3 canvas_to_local;
  float local_width;
  float local_height;
  float rotation_degrees;
  float feather;
  float rounding;
  uint vertex_count;
  ShapeAlphaMaskKind shape;
  bool invert;
};

struct ScanlinesCrtParams {
  float spacing;
  float intensity;
  float curvature;
  float mask;
};

struct HalftoneParams {
  float size;
  float angle;
  float contrast;
  uint mode;
  float channel_offset;
  float channel_angle_offset;
};

struct ThresholdParams {
  Color low;
  Color high;
  float threshold;
};

struct TwirlParams {
  Vec2 center;
  float radius;
  float angle;
};

struct WaveRippleParams {
  float amplitude;
  float wavelength;
  float angle;
  float phase;
};

enum VideoSampleMethod : EnumStorage {
  Bicubic = 2,
  Nearest = 0,
  Bilinear = 1,
  Mitchell = 3,
  Lanczos2 = 4,
  Fsr1Easu = 5,
  NvidiaImageScaling = 6,
  Anime4k = 7,
  Anime4kSrgan = 8,
  Xbrz = 9,
  Lanczos3 = 10,
};

enum TextureAddressMode : EnumStorage {
  Transparent,
  ClampToEdge,
  Repeat,
  MirrorRepeat,
  BlurredMirror,
  Stochastic,
};

enum LayerKind : EnumStorage {
  Nv12,
  Rgba,
  RgbaOpaque,
};

struct Nv12LayerParams {
  float crop[4];
  float padding[4];
  ByteStorage *y_plane;
  ByteStorage *uv_plane;
  uint *rgba;
  BufferPitch y_pitch;
  BufferPitch uv_pitch;
  BufferPitch rgba_pitch;
  uint source_width;
  uint source_height;
  uint canvas_width;
  Mat3 inverse;
  uint motion_transform_offset;
  uint motion_transform_count;
  uint motion_sample_count;
  float opacity;
#if defined(SHRIMPLY_WORD_STORAGE)
  uint blend_mode;
#else
  LayerBlendMode blend_mode;
  VideoSampleMethod sample_method;
  TextureAddressMode address_mode;
  LayerKind kind;
#endif

};

LayerBlendMode layer_blend_mode(Nv12LayerParams value) {
#if defined(SHRIMPLY_WORD_STORAGE)
  return LayerBlendMode(value.blend_mode & 255u);
#else
  return value.blend_mode;
#endif
}
VideoSampleMethod layer_sample_method(Nv12LayerParams value) {
#if defined(SHRIMPLY_WORD_STORAGE)
  return VideoSampleMethod((value.blend_mode >> 8u) & 255u);
#else
  return value.sample_method;
#endif
}
TextureAddressMode layer_address_mode(Nv12LayerParams value) {
#if defined(SHRIMPLY_WORD_STORAGE)
  return TextureAddressMode((value.blend_mode >> 16u) & 255u);
#else
  return value.address_mode;
#endif
}
LayerKind layer_kind(Nv12LayerParams value) {
#if defined(SHRIMPLY_WORD_STORAGE)
  return LayerKind(value.blend_mode >> 24u);
#else
  return value.kind;
#endif
}
LayerBlendMode composite_blend_mode(LayerCompositeParams value) {
#if defined(SHRIMPLY_WORD_STORAGE)
  return LayerBlendMode(uint(value.mode) & 255u);
#else
  return value.mode;
#endif
}

struct ChromaKeyParams {
  Color key;
  float similarity;
  float softness;
  float spill;
};

enum VisualTransitionMaskKind : uint {
  Wipe,
  Iris,
  Dissolve,
  ClockWipe,
  TriangularFold,
  StreakWipe,
};

struct VisualTransitionMaskParams {
  VisualTransitionMaskKind kind;
  float visibility;
  float angle_degrees;
  float softness;
  Vec2 center;
  uint grain_size;
  float line_variation;
};

struct Sam2ProxyParams {
  uint input_width;
  uint input_height;
  uint model_size;
};

struct Sam2MaskParams {
  uint output_width;
  uint output_height;
  uint mask_size;
  float threshold;
  float softness;
  float quantization_scale;
  bool invert;
};

struct TransparentFillMaskParams {
  uint width;
  uint height;
  uint stride;
};

struct ImageDescriptor {
  float4 *pixels;
  uint width;
  uint height;
};

struct ConvolutionTerm {
  float weights[16];
  float offset_x;
  float offset_y;
  uint input;
  uint activation;
};

struct ConvolutionParams {
  ImageDescriptor images[16];
  float bias[4];
  ImageDescriptor residual;
  float result_scale;
  float residual_scale;
  uint width;
  uint height;
};

struct AlphaParams {
  float4 *source;
  ByteStorage *alpha_source;
  BufferPitch alpha_pitch;
  uint alpha_width;
  uint alpha_height;
  uint width;
  uint height;
};
