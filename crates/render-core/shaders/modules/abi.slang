static const uint COMPUTE_GROUP_SIZE = 256u;

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

enum LayerBlendMode : uint8_t {
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
  size_t source_pitch;
  size_t clipping_base_pitch;
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

enum VideoSampleMethod : uint8_t {
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

enum TextureAddressMode : uint8_t {
  Transparent,
  ClampToEdge,
  Repeat,
  MirrorRepeat,
  BlurredMirror,
  Stochastic,
};

enum LayerKind : uint8_t {
  Nv12,
  Rgba,
};

struct Nv12LayerParams {
  float crop[4];
  float padding[4];
  uint8_t *y_plane;
  uint8_t *uv_plane;
  uint *rgba;
  size_t y_pitch;
  size_t uv_pitch;
  size_t rgba_pitch;
  uint source_width;
  uint source_height;
  uint canvas_width;
  Mat3 inverse;
  uint motion_transform_offset;
  uint motion_transform_count;
  uint motion_sample_count;
  float opacity;
  LayerBlendMode blend_mode;
  VideoSampleMethod sample_method;
  TextureAddressMode address_mode;
  LayerKind kind;
};

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
  uint8_t *alpha_source;
  size_t alpha_pitch;
  uint alpha_width;
  uint alpha_height;
  uint width;
  uint height;
};
