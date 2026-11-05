import abi;
import common;
import math;

Color color(float r, float g, float b, float a) {
  Color result = {r, g, b, a};
  return result;
}

Color color_from_rgba(uint value) {
  return color(float(value & 0xffu) / 255.0f,
               float((value >> 8) & 0xffu) / 255.0f,
               float((value >> 16) & 0xffu) / 255.0f,
               float((value >> 24) & 0xffu) / 255.0f);
}

uint color_channel(float value) {
  return uint(clamp(value, 0.0f, 1.0f) * 255.0f + 0.5f);
}

uint color_to_rgba(Color value) {
  return color_channel(value.r) | (color_channel(value.g) << 8) |
         (color_channel(value.b) << 16) | (color_channel(value.a) << 24);
}

Color color_with_alpha(Color value, float alpha) {
  value.a = alpha;
  return value;
}

Color color_lerp(Color left, Color right, float amount) {
  return color(left.r + (right.r - left.r) * amount,
               left.g + (right.g - left.g) * amount,
               left.b + (right.b - left.b) * amount,
               left.a + (right.a - left.a) * amount);
}

float rec709_luma(Color value) {
  return 0.2126f * value.r + 0.7152f * value.g + 0.0722f * value.b;
}

Color color_premultiply(Color value) {
  value.r *= value.a;
  value.g *= value.a;
  value.b *= value.a;
  return value;
}

Color color_unpremultiply(Color value) {
  if (value.a <= F32_EPSILON)
    return color(0.0f, 0.0f, 0.0f, 0.0f);
  value.r /= value.a;
  value.g /= value.a;
  value.b /= value.a;
  return value;
}

Color color_corrected(Color value, ColorCorrectionParams params) {
  float exposure = pow(2.0f, params.exposure);
  value.r *= exposure;
  value.g *= exposure;
  value.b *= exposure;
  value.r += params.temperature * 0.1f + params.tint * 0.03f;
  value.g -= params.tint * 0.08f;
  value.b += -params.temperature * 0.1f + params.tint * 0.03f;
  float contrast = max(1.0f + params.contrast, 0.0f);
  value.r =
      clamp((value.r - 0.5f) * contrast + 0.5f + params.brightness, 0.0f, 1.0f);
  value.g =
      clamp((value.g - 0.5f) * contrast + 0.5f + params.brightness, 0.0f, 1.0f);
  value.b =
      clamp((value.b - 0.5f) * contrast + 0.5f + params.brightness, 0.0f, 1.0f);
  float maximum = max(value.r, max(value.g, value.b));
  float minimum = min(value.r, min(value.g, value.b));
  float delta = maximum - minimum;
  float hue = 0.0f;
  if (delta > 0.000001f) {
    if (maximum == value.r)
      hue = (value.g - value.b) / delta / 6.0f;
    else if (maximum == value.g)
      hue = ((value.b - value.r) / delta + 2.0f) / 6.0f;
    else
      hue = ((value.r - value.g) / delta + 4.0f) / 6.0f;
  }
  hue = rem_euclid(hue + params.hue_turns, 1.0f);
  float source_saturation = maximum <= 0.000001f ? 0.0f : delta / maximum;
  float saturation =
      clamp(source_saturation * max(params.saturation, 0.0f), 0.0f, 1.0f);
  float final_value = clamp(maximum * max(params.value, 0.0f), 0.0f, 1.0f);
  float sector = hue * 6.0f;
  float x = 1.0f - abs(rem_euclid(sector * 0.5f, 1.0f) * 2.0f - 1.0f);
  float3 rgb;
  if (sector < 1.0f)
    rgb = float3(1.0f, x, 0.0f);
  else if (sector < 2.0f)
    rgb = float3(x, 1.0f, 0.0f);
  else if (sector < 3.0f)
    rgb = float3(0.0f, 1.0f, x);
  else if (sector < 4.0f)
    rgb = float3(0.0f, x, 1.0f);
  else if (sector < 5.0f)
    rgb = float3(x, 0.0f, 1.0f);
  else
    rgb = float3(1.0f, 0.0f, x);
  float offset = final_value * (1.0f - saturation);
  float scale = final_value * saturation;
  float inverse_gamma = 1.0f / max(params.gamma, 0.01f);
  value.r = pow(rgb.x * scale + offset, inverse_gamma);
  value.g = pow(rgb.y * scale + offset, inverse_gamma);
  value.b = pow(rgb.z * scale + offset, inverse_gamma);
  return value;
}

Color aces_tone_mapped(Color value) {
  float3 rgb = float3(value.r, value.g, value.b);
  rgb = (rgb * (2.51f * rgb + 0.03f)) / (rgb * (2.43f * rgb + 0.59f) + 0.14f);
  rgb = clamp(rgb, 0.0f, 1.0f);
  value.r = rgb.x;
  value.g = rgb.y;
  value.b = rgb.z;
  return value;
}

float linear_channel_to_srgb(float value) {
  return clamp(value <= 0.0031308f ? value * 12.92f
                                   : 1.055f * pow(value, 1.0f / 2.4f) - 0.055f,
               0.0f, 1.0f);
}

Color linear_to_srgb(Color value) {
  value.r = linear_channel_to_srgb(value.r);
  value.g = linear_channel_to_srgb(value.g);
  value.b = linear_channel_to_srgb(value.b);
  return value;
}

float bt2020_luma(Color value) {
  return 0.2627f * value.r + 0.6780f * value.g + 0.0593f * value.b;
}

float bt2020_ycbcr_distance(Color left, Color right) {
  float left_luma = bt2020_luma(left);
  float right_luma = bt2020_luma(right);
  float luma = left_luma - right_luma;
  float cb = (left.b - left_luma) * 0.5315f - (right.b - right_luma) * 0.5315f;
  float cr = (left.r - left_luma) * 0.6782f - (right.r - right_luma) * 0.6782f;
  float alpha = left.a - right.a;
  return sqrt(2.0f * luma * luma + cb * cb + cr * cr + alpha * alpha);
}

Color from_bt709_ycbcr(float luma, float cb, float cr, float alpha) {
  luma = max(luma - 0.0625f, 0.0f) * 1.1643835f;
  return color(clamp(luma + 1.7927411f * cr, 0.0f, 1.0f),
               clamp(luma - 0.21324861f * cb - 0.53290933f * cr, 0.0f, 1.0f),
               clamp(luma + 2.1124017f * cb, 0.0f, 1.0f), alpha);
}

// swscale supplies fixed-point YUV coefficients for the source matrix.
Color from_ycbcr(float luma, float2 chroma, int4 coefficients, bool full_range) {
  const float FIXED_POINT_SCALE = 65536.0f;
  float4 matrix = float4(coefficients) / FIXED_POINT_SCALE;
  luma = full_range ? luma : (luma - 16.0f / 255.0f) * (255.0f / 219.0f);
  chroma -= 128.0f / 255.0f;
  if (full_range)
    chroma *= 224.0f / 255.0f;
  return color(clamp(luma + matrix.x * chroma.y, 0.0f, 1.0f),
               clamp(luma - matrix.z * chroma.x - matrix.w * chroma.y, 0.0f, 1.0f),
               clamp(luma + matrix.y * chroma.x, 0.0f, 1.0f), 1.0f);
}

uint3 to_bt709_ycbcr(Color value) {
  return uint3(
      uint(clamp(16.0f + 219.0f * rec709_luma(value), 0.0f, 255.0f) + 0.5f),
      uint(clamp(128.0f + 224.0f * (-0.114572f * value.r - 0.385428f * value.g +
                                    0.5f * value.b),
                 0.0f, 255.0f) +
           0.5f),
      uint(clamp(128.0f + 224.0f * (0.5f * value.r - 0.454153f * value.g -
                                    0.045847f * value.b),
                 0.0f, 255.0f) +
           0.5f));
}

float blend_luminosity(Color c) {
  return 0.3f * c.r + 0.59f * c.g + 0.11f * c.b;
}
float blend_saturation(Color c) {
  return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
}
Color blend_clip(Color c) {
  float l = blend_luminosity(c), lo = min(c.r, min(c.g, c.b)),
        hi = max(c.r, max(c.g, c.b));
  if (lo < 0.0f) {
    c.r = l + (c.r - l) * l / (l - lo);
    c.g = l + (c.g - l) * l / (l - lo);
    c.b = l + (c.b - l) * l / (l - lo);
  }
  if (hi > 1.0f) {
    c.r = l + (c.r - l) * (1.0f - l) / (hi - l);
    c.g = l + (c.g - l) * (1.0f - l) / (hi - l);
    c.b = l + (c.b - l) * (1.0f - l) / (hi - l);
  }
  c.r = clamp(c.r, 0.0f, 1.0f);
  c.g = clamp(c.g, 0.0f, 1.0f);
  c.b = clamp(c.b, 0.0f, 1.0f);
  return c;
}
Color blend_with_luminosity(Color c, float luminosity) {
  float delta = luminosity - blend_luminosity(c);
  c.r += delta;
  c.g += delta;
  c.b += delta;
  return blend_clip(c);
}
Color blend_with_saturation(Color c, float saturation) {
  float channels[3] = {c.r, c.g, c.b};
  uint lo = channels[0] <= channels[1] && channels[0] <= channels[2]
                ? 0u
                : (channels[1] <= channels[2] ? 1u : 2u),
       hi = channels[0] >= channels[1] && channels[0] >= channels[2]
                ? 0u
                : (channels[1] >= channels[2] ? 1u : 2u);
  if (lo == hi)
    return color(0.0f, 0.0f, 0.0f, c.a);
  uint middle = 3u - lo - hi;
  float result[3] = {0.0f, 0.0f, 0.0f};
  if (channels[hi] > channels[lo]) {
    result[middle] = (channels[middle] - channels[lo]) * saturation /
                     (channels[hi] - channels[lo]);
    result[hi] = saturation;
  }
  return color(result[0], result[1], result[2], c.a);
}
float color_burn(float s, float d) {
  return s <= 0.0f ? 0.0f : 1.0f - min((1.0f - d) / s, 1.0f);
}
float color_dodge(float s, float d) {
  return s >= 1.0f ? 1.0f : min(d / (1.0f - s), 1.0f);
}
float hard_light(float s, float d) {
  return s <= 0.5f ? 2.0f * s * d : 1.0f - 2.0f * (1.0f - s) * (1.0f - d);
}
float vivid_light(float s, float d) {
  return s <= 0.5f ? color_burn(s * 2.0f, d)
                   : color_dodge((s - 0.5f) * 2.0f, d);
}
float blend_channel(float s, float d, LayerBlendMode mode) {
  if (mode == LayerBlendMode::PassThrough || mode == LayerBlendMode::Normal ||
      mode == LayerBlendMode::Dissolve)
    return s;
  if (mode == LayerBlendMode::Darken)
    return min(s, d);
  if (mode == LayerBlendMode::Multiply)
    return s * d;
  if (mode == LayerBlendMode::ColorBurn)
    return color_burn(s, d);
  if (mode == LayerBlendMode::LinearBurn)
    return max(s + d - 1.0f, 0.0f);
  if (mode == LayerBlendMode::Lighten)
    return max(s, d);
  if (mode == LayerBlendMode::Screen)
    return s + d - s * d;
  if (mode == LayerBlendMode::ColorDodge)
    return color_dodge(s, d);
  if (mode == LayerBlendMode::Add)
    return min(s + d, 1.0f);
  if (mode == LayerBlendMode::Overlay)
    return hard_light(d, s);
  if (mode == LayerBlendMode::SoftLight) {
    if (s <= 0.5f)
      return d - (1.0f - 2.0f * s) * d * (1.0f - d);
    float curve = d <= 0.25f ? ((16.0f * d - 12.0f) * d + 4.0f) * d : sqrt(d);
    return d + (2.0f * s - 1.0f) * (curve - d);
  }
  if (mode == LayerBlendMode::HardLight)
    return hard_light(s, d);
  if (mode == LayerBlendMode::VividLight)
    return vivid_light(s, d);
  if (mode == LayerBlendMode::LinearLight)
    return clamp(d + 2.0f * s - 1.0f, 0.0f, 1.0f);
  if (mode == LayerBlendMode::PinLight)
    return s <= 0.5f ? min(d, s * 2.0f) : max(d, 2.0f * s - 1.0f);
  if (mode == LayerBlendMode::HardMix)
    return vivid_light(s, d) >= 0.5f ? 1.0f : 0.0f;
  if (mode == LayerBlendMode::Difference)
    return abs(d - s);
  if (mode == LayerBlendMode::Exclusion)
    return d + s - 2.0f * d * s;
  if (mode == LayerBlendMode::Subtract)
    return max(d - s, 0.0f);
  if (mode == LayerBlendMode::Divide)
    return s <= 0.0f ? 1.0f : min(d / s, 1.0f);
  if (mode == LayerBlendMode::Reflect)
    return s >= 1.0f ? 1.0f : min(d * d / (1.0f - s), 1.0f);
  return s;
}
Color blend_rgb(Color source, Color destination, LayerBlendMode mode,
                bool clamp_rgb) {
  Color result;
  if (mode == LayerBlendMode::Hue)
    result = blend_with_luminosity(
        blend_with_saturation(source, blend_saturation(destination)),
        blend_luminosity(destination));
  else if (mode == LayerBlendMode::Saturation)
    result = blend_with_luminosity(
        blend_with_saturation(destination, blend_saturation(source)),
        blend_luminosity(destination));
  else if (mode == LayerBlendMode::Color)
    result = blend_with_luminosity(source, blend_luminosity(destination));
  else if (mode == LayerBlendMode::Luminosity)
    result = blend_with_luminosity(destination, blend_luminosity(source));
  else if (mode == LayerBlendMode::DarkerColor)
    result = blend_luminosity(source) < blend_luminosity(destination)
                 ? source
                 : destination;
  else if (mode == LayerBlendMode::LighterColor)
    result = blend_luminosity(source) > blend_luminosity(destination)
                 ? source
                 : destination;
  else
    result = color(blend_channel(source.r, destination.r, mode),
                   blend_channel(source.g, destination.g, mode),
                   blend_channel(source.b, destination.b, mode), source.a);
  if (clamp_rgb) {
    result.r = clamp(result.r, 0.0f, 1.0f);
    result.g = clamp(result.g, 0.0f, 1.0f);
    result.b = clamp(result.b, 0.0f, 1.0f);
  }
  return result;
}
Color blend_over(Color source, Color destination, LayerBlendMode mode,
                 float source_alpha, bool clamp_rgb) {
  source_alpha = clamp(source_alpha, 0.0f, 1.0f);
  float output_alpha = source_alpha + destination.a * (1.0f - source_alpha);
  if (output_alpha <= F32_EPSILON)
    return color(0.0f, 0.0f, 0.0f, 0.0f);
  Color blended = blend_rgb(source, destination, mode, clamp_rgb);
  float3 value =
      ((1.0f - destination.a) * source_alpha *
           float3(source.r, source.g, source.b) +
       (1.0f - source_alpha) * destination.a *
           float3(destination.r, destination.g, destination.b) +
       source_alpha * destination.a * float3(blended.r, blended.g, blended.b)) /
      output_alpha;
  return color(value.x, value.y, value.z, output_alpha);
}

float3 mat3_mul(Mat3 matrix, float3 value) {
  return matrix.x_axis * value.x +
         (matrix.y_axis * value.y + matrix.z_axis * value.z);
}

float2 vec2_value(Vec2 value) { return float2(value.x, value.y); }

float2 transform_point2(Mat3 transform, float2 point) {
  return float2(transform.x_axis.x * point.x +
                    (transform.y_axis.x * point.y + transform.z_axis.x),
                transform.x_axis.y * point.x +
                    (transform.y_axis.y * point.y + transform.z_axis.y));
}

bool projective_point(Mat3 transform, float2 point, out float2 result) {
  result = float2(0.0f);
  float denominator = transform.x_axis.z * point.x +
                      (transform.y_axis.z * point.y + transform.z_axis.z);
  if (abs(denominator) <= F32_EPSILON)
    return false;
  result = float2((transform.x_axis.x * point.x +
                   (transform.y_axis.x * point.y + transform.z_axis.x)) /
                      denominator,
                  (transform.x_axis.y * point.x +
                   (transform.y_axis.y * point.y + transform.z_axis.y)) /
                      denominator);
  return true;
}

float perp_dot(float2 left, float2 right) {
  return left.x * right.y - left.y * right.x;
}

bool inverse_bilinear_quad(Vec2 corners[4], float2 point, float2 initial,
                           out float2 result) {
  result = float2(0.0f);
  float2 top_left = vec2_value(corners[0]);
  float2 top_right = vec2_value(corners[1]);
  float2 bottom_right = vec2_value(corners[2]);
  float2 bottom_left = vec2_value(corners[3]);
  float2 horizontal = top_right - top_left;
  float2 vertical = bottom_left - top_left;
  float2 diagonal = top_left - top_right + bottom_right - bottom_left;
  float x = initial.x;
  float y = initial.y;
  for (uint iteration = 0; iteration < 6u; iteration++) {
    float2 error =
        top_left + horizontal * x + vertical * y + diagonal * x * y - point;
    float2 derivative_x = horizontal + diagonal * y;
    float2 derivative_y = vertical + diagonal * x;
    float determinant = perp_dot(derivative_x, derivative_y);
    if (abs(determinant) <= 0.000001f)
      return false;
    x -= perp_dot(error, derivative_y) / determinant;
    y -= perp_dot(derivative_x, error) / determinant;
  }
  result = float2(x, y);
  return isfinite(x) && isfinite(y);
}

uint sample_bilinear_rgba(uint *input, uint width, uint height, float x,
                          float y) {
  uint x0 = uint(floor(x));
  uint y0 = uint(floor(y));
  uint x1 = min(x0 + 1u, width > 0u ? width - 1u : 0u);
  uint y1 = min(y0 + 1u, height > 0u ? height - 1u : 0u);
  float horizontal = x - float(x0);
  float vertical = y - float(y0);
  Color top_left = color_premultiply(color_from_rgba(input[y0 * width + x0]));
  Color top_right = color_premultiply(color_from_rgba(input[y0 * width + x1]));
  Color bottom_left =
      color_premultiply(color_from_rgba(input[y1 * width + x0]));
  Color bottom_right =
      color_premultiply(color_from_rgba(input[y1 * width + x1]));
  return color_to_rgba(color_unpremultiply(
      color_lerp(color_lerp(top_left, top_right, horizontal),
                 color_lerp(bottom_left, bottom_right, horizontal), vertical)));
}
