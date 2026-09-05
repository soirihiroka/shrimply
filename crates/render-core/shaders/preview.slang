import abi;
import color;
import common;
import math;
import nis_coefficients;

int repeat_index(int value, int start, int length) {
  length = max(length, 1);
  int offset = (value - start) % length;
  return start + (offset < 0 ? offset + length : offset);
}
int mirror_index(int value, int start, int length) {
  length = max(length, 1);
  int period = length * 2, offset = repeat_index(value - start, 0, period);
  return start + (offset < length ? offset : period - offset - 1);
}
bool address_index(int index, uint size, float crop_start, float crop_end,
                   TextureAddressMode mode, out uint result) {
  result = 0u;
  int last = int(max(size, 1u)) - 1,
      first = clamp(int(ceil_f32(float(size) * crop_start - 0.5f)), 0, last);
  last = clamp(int(floor_f32(float(size) * (1.0f - crop_end) - 0.5f)), first,
               last);
  if (mode == TextureAddressMode::Transparent) {
    if (index < first || index > last)
      return false;
  } else if (mode == TextureAddressMode::ClampToEdge)
    index = clamp(index, first, last);
  else if (mode == TextureAddressMode::Repeat)
    index = repeat_index(index, first, last - first + 1);
  else
    index = mirror_index(index, first, last - first + 1);
  result = uint(index);
  return true;
}
float load_luma(Nv12LayerParams p, int x, int y) {
  uint sx, sy;
  if (!address_index(x, p.source_width, p.crop[3], p.crop[1], p.address_mode,
                     sx) ||
      !address_index(y, p.source_height, p.crop[0], p.crop[2], p.address_mode,
                     sy))
    return 0.0f;
  return float(((uint8_t *)p.y_plane)[uint64_t(sy) * p.y_pitch + sx]) / 255.0f;
}
float load_chroma(Nv12LayerParams p, int x, int y, uint component) {
  uint sx, sy, cw = max((p.source_width + 1u) / 2u, 1u),
               ch = max((p.source_height + 1u) / 2u, 1u);
  if (!address_index(x, cw, p.crop[3], p.crop[1], p.address_mode, sx) ||
      !address_index(y, ch, p.crop[0], p.crop[2], p.address_mode, sy))
    return 0.5f;
  return float(((uint8_t *)p.uv_plane)[uint64_t(sy) * p.uv_pitch +
                                       uint64_t(sx) * 2u + component]) /
         255.0f;
}
Color load_rgba(Nv12LayerParams p, int x, int y) {
  uint sx, sy;
  if (!address_index(x, p.source_width, p.crop[3], p.crop[1], p.address_mode,
                     sx) ||
      !address_index(y, p.source_height, p.crop[0], p.crop[2], p.address_mode,
                     sy))
    return color(0.0f, 0.0f, 0.0f, 0.0f);
  return color_premultiply(color_from_rgba(
      ((uint *)p.rgba)[uint64_t(sy) * (p.rgba_pitch / 4u) + sx]));
}
float filter_weight(float value, VideoSampleMethod method) {
  float x = abs(value);
  if (method == VideoSampleMethod::Mitchell) {
    if (x <= 1.0f)
      return 1.1666666f * x * x * x - 2.0f * x * x + 0.8888889f;
    if (x < 2.0f)
      return -0.3888889f * x * x * x + 2.0f * x * x - 3.3333333f * x +
             1.7777778f;
    return 0.0f;
  }
  if (method == VideoSampleMethod::Lanczos2 ||
      method == VideoSampleMethod::Lanczos3) {
    float radius = method == VideoSampleMethod::Lanczos2 ? 2.0f : 3.0f;
    if (x < 0.000001f)
      return 1.0f;
    if (x >= radius)
      return 0.0f;
    float a = PI * x, b = a / radius;
    return sin_f32(a) / a * sin_f32(b) / b;
  }
  if (x <= 1.0f)
    return 1.5f * x * x * x - 2.5f * x * x + 1.0f;
  if (x < 2.0f)
    return -0.5f * x * x * x + 2.5f * x * x - 4.0f * x + 2.0f;
  return 0.0f;
}
int sample_radius(VideoSampleMethod method) {
  return method == VideoSampleMethod::Lanczos3 ? 3 : 2;
}
float sample_scalar_weighted(Nv12LayerParams p, float x, float y,
                             VideoSampleMethod method, bool chroma,
                             uint component) {
  int radius = sample_radius(method), bx = int(floor_f32(x)),
      by = int(floor_f32(y));
  float value = 0.0f, total = 0.0f;
  for (int row = 1 - radius; row <= radius; ++row) {
    float wy = filter_weight(y - float(by + row), method);
    for (int column = 1 - radius; column <= radius; ++column) {
      float weight = filter_weight(x - float(bx + column), method) * wy;
      value += (chroma ? load_chroma(p, bx + column, by + row, component)
                       : load_luma(p, bx + column, by + row)) *
               weight;
      total += weight;
    }
  }
  return abs(total) > 0.000001f ? clamp(value / total, 0.0f, 1.0f) : 0.0f;
}
float sample_scalar_bilinear(Nv12LayerParams p, float x, float y, bool chroma,
                             uint component) {
  int x0 = int(floor_f32(x)), y0 = int(floor_f32(y));
  float tx = x - float(x0), ty = y - float(y0);
  float a = chroma ? load_chroma(p, x0, y0, component) : load_luma(p, x0, y0),
        b = chroma ? load_chroma(p, x0 + 1, y0, component)
                   : load_luma(p, x0 + 1, y0),
        c = chroma ? load_chroma(p, x0, y0 + 1, component)
                   : load_luma(p, x0, y0 + 1),
        d = chroma ? load_chroma(p, x0 + 1, y0 + 1, component)
                   : load_luma(p, x0 + 1, y0 + 1);
  return lerp_f32(lerp_f32(a, b, tx), lerp_f32(c, d, tx), ty);
}
Color sample_rgba_bilinear_p(Nv12LayerParams p, float x, float y) {
  int x0 = int(floor_f32(x)), y0 = int(floor_f32(y));
  float tx = x - float(x0), ty = y - float(y0);
  return color_lerp(
      color_lerp(load_rgba(p, x0, y0), load_rgba(p, x0 + 1, y0), tx),
      color_lerp(load_rgba(p, x0, y0 + 1), load_rgba(p, x0 + 1, y0 + 1), tx),
      ty);
}
Color sample_rgba_weighted_p(Nv12LayerParams p, float x, float y,
                             VideoSampleMethod method) {
  int radius = sample_radius(method), bx = int(floor_f32(x)),
      by = int(floor_f32(y));
  float4 value = float4(0.0f);
  float total = 0.0f;
  for (int row = 1 - radius; row <= radius; ++row) {
    float wy = filter_weight(y - float(by + row), method);
    for (int column = 1 - radius; column <= radius; ++column) {
      float weight = filter_weight(x - float(bx + column), method) * wy;
      Color c = load_rgba(p, bx + column, by + row);
      value += float4(c.r, c.g, c.b, c.a) * weight;
      total += weight;
    }
  }
  if (abs(total) <= 0.000001f)
    return color(0.0f, 0.0f, 0.0f, 0.0f);
  return color(value.x / total, value.y / total, value.z / total,
               value.w / total);
}
float valid_at(Nv12LayerParams p, int x, int y) {
  uint value;
  return address_index(x, p.source_width, p.crop[3], p.crop[1], p.address_mode,
                       value) &&
                 address_index(y, p.source_height, p.crop[0], p.crop[2],
                               p.address_mode, value)
             ? 1.0f
             : 0.0f;
}
float coverage_weighted(Nv12LayerParams p, float x, float y,
                        VideoSampleMethod method) {
  int radius = sample_radius(method), bx = int(floor_f32(x)),
      by = int(floor_f32(y));
  float value = 0.0f, total = 0.0f;
  for (int row = 1 - radius; row <= radius; ++row) {
    float wy = filter_weight(y - float(by + row), method);
    for (int column = 1 - radius; column <= radius; ++column) {
      float weight = filter_weight(x - float(bx + column), method) * wy;
      value += valid_at(p, bx + column, by + row) * weight;
      total += weight;
    }
  }
  return abs(total) > 0.000001f ? clamp(value / total, 0.0f, 1.0f) : 0.0f;
}
float coverage(Nv12LayerParams p, float x, float y) {
  if (p.address_mode != TextureAddressMode::Transparent)
    return 1.0f;
  if (p.sample_method == VideoSampleMethod::Nearest ||
      p.sample_method == VideoSampleMethod::Xbrz)
    return valid_at(p, int(floor_f32(x + 0.5f)), int(floor_f32(y + 0.5f)));
  if (p.sample_method == VideoSampleMethod::Bilinear ||
      p.sample_method == VideoSampleMethod::Fsr1Easu ||
      p.sample_method == VideoSampleMethod::NvidiaImageScaling ||
      p.sample_method == VideoSampleMethod::Anime4k ||
      p.sample_method == VideoSampleMethod::Anime4kSrgan) {
    int x0 = int(floor_f32(x)), y0 = int(floor_f32(y));
    float tx = x - float(x0), ty = y - float(y0);
    return clamp(
        lerp_f32(
            lerp_f32(valid_at(p, x0, y0), valid_at(p, x0 + 1, y0), tx),
            lerp_f32(valid_at(p, x0, y0 + 1), valid_at(p, x0 + 1, y0 + 1), tx),
            ty),
        0.0f, 1.0f);
  }
  return coverage_weighted(p, x, y, p.sample_method);
}
float4 crop_rect(Nv12LayerParams p) {
  float left = float(p.source_width) * p.crop[3],
        top = float(p.source_height) * p.crop[0];
  return float4(
      left, top,
      max(float(p.source_width) * (1.0f - p.crop[1] - p.crop[3]), 1.0f),
      max(float(p.source_height) * (1.0f - p.crop[0] - p.crop[2]), 1.0f));
}
float outside_distance(Nv12LayerParams p, float x, float y) {
  float4 crop = crop_rect(p);
  float dx = max(max(crop.x - x, x - (crop.x + crop.z)), 0.0f),
        dy = max(max(crop.y - y, y - (crop.y + crop.w)), 0.0f);
  return length(float2(dx, dy));
}

bool magnifying(Nv12LayerParams p) {
  float x = length(float2(p.inverse.x_axis.x, p.inverse.y_axis.x));
  float y = length(float2(p.inverse.x_axis.y, p.inverse.y_axis.y));
  return x > 0.0f && x <= 1.0f && y > 0.0f && y <= 1.0f;
}

Color fsr_load(Nv12LayerParams p, int x, int y, bool nv12) {
  if (!nv12)
    return load_rgba(p, x, y);
  float cx = (float(x) - 0.5f) * 0.5f, cy = (float(y) - 0.5f) * 0.5f;
  return from_bt709_ycbcr(
      load_luma(p, x, y), sample_scalar_bilinear(p, cx, cy, true, 0u) - 0.5f,
      sample_scalar_bilinear(p, cx, cy, true, 1u) - 0.5f, 1.0f);
}

void fsr_accumulate_direction(inout float2 direction, inout float edge_length,
                              float weight, float p[5]) {
  float left = p[2] - p[1], right = p[3] - p[2],
        horizontal_range = max(abs(left), abs(right)), horizontal = p[3] - p[1];
  direction.x += horizontal * weight;
  float horizontal_length = horizontal_range > 0.0f
                                ? min(abs(horizontal) / horizontal_range, 1.0f)
                                : 0.0f;
  edge_length += horizontal_length * horizontal_length * weight;
  float top = p[2] - p[0], bottom = p[4] - p[2],
        vertical_range = max(abs(top), abs(bottom)), vertical = p[4] - p[0];
  direction.y += vertical * weight;
  float vertical_length =
      vertical_range > 0.0f ? min(abs(vertical) / vertical_range, 1.0f) : 0.0f;
  edge_length += vertical_length * vertical_length * weight;
}

void fsr_tap(inout float3 value, inout float weight_sum, float2 offset,
             float2 direction, float2 anisotropy, float lobe, float clip,
             Color sample) {
  float rotated_x = dot(offset, direction),
        rotated_y = direction.x * offset.y - direction.y * offset.x;
  float2 distance_vector =
      float2(rotated_x * anisotropy.x, rotated_y * anisotropy.y);
  float distance_value = min(dot(distance_vector, distance_vector), clip);
  float base = 0.4f * distance_value - 1.0f,
        window = lobe * distance_value - 1.0f;
  base *= base;
  window *= window;
  base = 1.5625f * base - 0.5625f;
  float weight = base * window;
  value += float3(sample.r, sample.g, sample.b) * weight;
  weight_sum += weight;
}

Color fsr_easu(Nv12LayerParams p, float x, float y, bool nv12) {
  int base_x = int(floor_f32(x)), base_y = int(floor_f32(y));
  float2 point = float2(x - float(base_x), y - float(base_y));
  int2 offsets[12] = {int2(0, -1), int2(1, -1), int2(-1, 0), int2(0, 0),
                      int2(1, 0),  int2(2, 0),  int2(-1, 1), int2(0, 1),
                      int2(1, 1),  int2(2, 1),  int2(0, 2),  int2(1, 2)};
  Color pixels[12];
  float luma[12];
  for (uint i = 0u; i < 12u; ++i) {
    pixels[i] = fsr_load(p, base_x + offsets[i].x, base_y + offsets[i].y, nv12);
    luma[i] = 0.5f * pixels[i].r + pixels[i].g + 0.5f * pixels[i].b;
  }
  float2 direction = float2(0.0f);
  float edge_length = 0.0f;
  float values[5];
  values = {luma[0], luma[2], luma[3], luma[4], luma[7]};
  fsr_accumulate_direction(direction, edge_length,
                           (1.0f - point.x) * (1.0f - point.y), values);
  values = {luma[1], luma[3], luma[4], luma[5], luma[8]};
  fsr_accumulate_direction(direction, edge_length, point.x * (1.0f - point.y),
                           values);
  values = {luma[3], luma[6], luma[7], luma[8], luma[10]};
  fsr_accumulate_direction(direction, edge_length, (1.0f - point.x) * point.y,
                           values);
  values = {luma[4], luma[7], luma[8], luma[9], luma[11]};
  fsr_accumulate_direction(direction, edge_length, point.x * point.y, values);
  float magnitude = length(direction);
  direction = magnitude * magnitude < 1.0f / 32768.0f ? float2(1.0f, 0.0f)
                                                      : direction / magnitude;
  edge_length *= 0.5f;
  edge_length *= edge_length;
  float stretch = 1.0f / max(abs(direction.x), abs(direction.y));
  float2 anisotropy =
      float2(1.0f + (stretch - 1.0f) * edge_length, 1.0f - 0.5f * edge_length);
  float lobe = 0.5f + ((0.25f - 0.04f) - 0.5f) * edge_length,
        clip = 1.0f / lobe;
  float3 minimum = min(min(float3(pixels[3].r, pixels[3].g, pixels[3].b),
                           float3(pixels[4].r, pixels[4].g, pixels[4].b)),
                       min(float3(pixels[7].r, pixels[7].g, pixels[7].b),
                           float3(pixels[8].r, pixels[8].g, pixels[8].b)));
  float3 maximum = max(max(float3(pixels[3].r, pixels[3].g, pixels[3].b),
                           float3(pixels[4].r, pixels[4].g, pixels[4].b)),
                       max(float3(pixels[7].r, pixels[7].g, pixels[7].b),
                           float3(pixels[8].r, pixels[8].g, pixels[8].b)));
  float3 value = float3(0.0f);
  float weight_sum = 0.0f;
  for (uint i = 0u; i < 12u; ++i)
    fsr_tap(value, weight_sum, float2(offsets[i]) - point, direction,
            anisotropy, lobe, clip, pixels[i]);
  value = clamp(value / weight_sum, minimum, maximum);
  return color(value.x, value.y, value.z, 1.0f);
}

float4 finish_nv12(Nv12LayerParams p, float4 sample) {
  if (p.address_mode == TextureAddressMode::Transparent && sample.w > 0.000001f)
    sample.xyz /= sample.w;
  return sample;
}

float4 fsr_nv12(Nv12LayerParams p, float x, float y, float alpha) {
  if (!magnifying(p))
    return finish_nv12(
        p, float4(sample_scalar_weighted(p, x, y, VideoSampleMethod::Lanczos3,
                                         false, 0u),
                  sample_scalar_bilinear(p, (x - 0.5f) * 0.5f,
                                         (y - 0.5f) * 0.5f, true, 0u) -
                      0.5f,
                  sample_scalar_bilinear(p, (x - 0.5f) * 0.5f,
                                         (y - 0.5f) * 0.5f, true, 1u) -
                      0.5f,
                  alpha));
  Color rgb = fsr_easu(p, x, y, true);
  float full_luma = rec709_luma(rgb);
  return finish_nv12(p, float4(full_luma / 1.1643835f + 0.0625f,
                               (rgb.b - full_luma) / 2.1124017f,
                               (rgb.r - full_luma) / 1.7927411f, alpha));
}

float4 nis_edge_map(float p[6][6], uint row, uint column) {
  float g0 =
      abs(p[row][column] + p[row][column + 1] + p[row][column + 2] -
          p[row + 2][column] - p[row + 2][column + 1] - p[row + 2][column + 2]);
  float g45 = abs(p[row + 1][column] + p[row][column] + p[row][column + 1] -
                  p[row + 2][column + 1] - p[row + 2][column + 2] -
                  p[row + 1][column + 2]);
  float g90 =
      abs(p[row][column] + p[row + 1][column] + p[row + 2][column] -
          p[row][column + 2] - p[row + 1][column + 2] - p[row + 2][column + 2]);
  float g135 =
      abs(p[row + 1][column] + p[row + 2][column] + p[row + 2][column + 1] -
          p[row][column + 1] - p[row][column + 2] - p[row + 1][column + 2]);
  float max_axis = max(g0, g90), min_axis = min(g0, g90),
        max_diagonal = max(g45, g135), min_diagonal = min(g45, g135);
  if (max_axis + max_diagonal == 0.0f)
    return float4(0.0f);
  float axis_share = min(max_axis / (max_axis + max_diagonal), 1.0f),
        diagonal_share = 1.0f - axis_share;
  bool axis = max_axis > min_axis * 2.2011719f && max_axis > 0.0625f &&
              max_axis > min_diagonal;
  bool diagonal = max_diagonal > min_diagonal * 2.2011719f &&
                  max_diagonal > 0.0625f && max_diagonal > min_axis;
  float axis_weight = axis && diagonal ? axis_share : 1.0f,
        diagonal_weight = axis && diagonal ? diagonal_share : 1.0f;
  return float4(axis && g0 >= g90 ? axis_weight : 0.0f,
                axis && g90 > g0 ? axis_weight : 0.0f,
                diagonal && g45 >= g135 ? diagonal_weight : 0.0f,
                diagonal && g135 > g45 ? diagonal_weight : 0.0f);
}

uint nis_phase(float phase) { return min(uint(phase * 64.0f), 63u); }
float nis_local_tone(float p[6], uint phase) {
  bool selector = phase <= 32u;
  float first_min = min(min(p[1], p[2]), selector ? p[0] : p[3]),
        first_max = max(max(p[1], p[2]), selector ? p[0] : p[3]);
  float second_min = min(min(p[3], p[4]), selector ? p[2] : p[5]),
        second_max = max(max(p[3], p[4]), selector ? p[2] : p[5]);
  float first_contrast = first_max - first_min,
        second_contrast = second_max - second_min;
  return 1.0f -
         clamp((max(first_contrast, second_contrast) /
                    (min(first_contrast, second_contrast) + 1.0f / 255.0f) -
                2.0f) *
                   0.125f,
               0.0f, 1.0f);
}
float nis_evaluate(float p[6], uint phase) {
  float value = 0.0f, unsharp = 0.0f;
  for (uint i = 0u; i < 6u; ++i) {
    value += NIS_SCALER[phase][i] * p[i];
    unsharp += NIS_USM[phase][i] * p[i];
  }
  float scale = 1.0f - clamp((value - 0.45f) * (1.0f / 0.45f), 0.0f, 1.0f);
  unsharp *= scale * 1.2f + 0.4f;
  float limit = (scale * 0.36f + 0.14f) * value;
  unsharp = clamp(unsharp, -limit, limit) * nis_local_tone(p, phase);
  return value + unsharp;
}
float4 nis_lerp4(float4 left, float4 right, float amount) {
  return float4(
      lerp_f32(left.x, right.x, amount), lerp_f32(left.y, right.y, amount),
      lerp_f32(left.z, right.z, amount), lerp_f32(left.w, right.w, amount));
}
float nis_directional(float p[6][6], float fx, float fy, uint phase_x,
                      uint phase_y, float4 weights) {
  float result = 0.0f, values[6];
  if (weights.x > 0.0f) {
    for (uint i = 0u; i < 6u; ++i)
      values[i] = lerp_f32(p[i][2], p[i][3], fx);
    result += nis_evaluate(values, phase_y) * weights.x;
  }
  if (weights.y > 0.0f) {
    for (uint i = 0u; i < 6u; ++i)
      values[i] = lerp_f32(p[2][i], p[3][i], fy);
    result += nis_evaluate(values, phase_x) * weights.y;
  }
  if (weights.z > 0.0f) {
    float blend = 0.5f + 0.5f * (fx - fy), temporary[7];
    temporary[1] = lerp_f32(p[2][1], p[1][2], blend);
    temporary[3] = lerp_f32(p[3][2], p[2][3], blend);
    temporary[5] = lerp_f32(p[4][3], p[3][4], blend);
    blend -= 0.5f;
    float4 diagonal = blend >= 0.0f
                          ? float4(p[0][2], p[1][3], p[2][4], p[3][5])
                          : float4(p[2][0], p[3][1], p[4][2], p[5][3]);
    temporary[0] = lerp_f32(p[1][1], diagonal.x, abs(blend));
    temporary[2] = lerp_f32(p[2][2], diagonal.y, abs(blend));
    temporary[4] = lerp_f32(p[3][3], diagonal.z, abs(blend));
    temporary[6] = lerp_f32(p[4][4], diagonal.w, abs(blend));
    float phase = fx + fy;
    uint offset = phase >= 1.0f ? 1u : 0u;
    if (offset != 0u)
      phase -= 1.0f;
    for (uint i = 0u; i < 6u; ++i)
      values[i] = temporary[offset + i];
    result += nis_evaluate(values, nis_phase(phase)) * weights.z;
  }
  if (weights.w > 0.0f) {
    float blend = 0.5f * (fx + fy), temporary[7];
    temporary[1] = lerp_f32(p[3][1], p[4][2], blend);
    temporary[3] = lerp_f32(p[2][2], p[3][3], blend);
    temporary[5] = lerp_f32(p[1][3], p[2][4], blend);
    blend -= 0.5f;
    float4 diagonal = blend >= 0.0f
                          ? float4(p[5][2], p[4][3], p[3][4], p[2][5])
                          : float4(p[3][0], p[2][1], p[1][2], p[0][3]);
    temporary[0] = lerp_f32(p[4][1], diagonal.x, abs(blend));
    temporary[2] = lerp_f32(p[3][2], diagonal.y, abs(blend));
    temporary[4] = lerp_f32(p[2][3], diagonal.z, abs(blend));
    temporary[6] = lerp_f32(p[1][4], diagonal.w, abs(blend));
    float phase = 1.0f + fx - fy;
    uint offset = phase >= 1.0f ? 1u : 0u;
    if (offset != 0u)
      phase -= 1.0f;
    for (uint i = 0u; i < 6u; ++i)
      values[i] = temporary[offset + i];
    result += nis_evaluate(values, nis_phase(phase)) * weights.w;
  }
  return result;
}
float nis_filtered_luma(Nv12LayerParams params, float x, float y, bool rgba) {
  int base_x = int(floor_f32(x)), base_y = int(floor_f32(y));
  float fx = x - float(base_x), fy = y - float(base_y);
  uint phase_x = uint(fx * 64.0f), phase_y = uint(fy * 64.0f);
  float pixels[6][6];
  for (uint row = 0u; row < 6u; ++row)
    for (uint column = 0u; column < 6u; ++column) {
      if (rgba)
        pixels[row][column] = rec709_luma(
            load_rgba(params, base_x + int(column) - 2, base_y + int(row) - 2));
      else
        pixels[row][column] =
            load_luma(params, base_x + int(column) - 2, base_y + int(row) - 2);
    }
  float4 e00 = nis_edge_map(pixels, 0u, 0u), e01 = nis_edge_map(pixels, 0u, 1u),
         e10 = nis_edge_map(pixels, 1u, 0u), e11 = nis_edge_map(pixels, 1u, 1u);
  float4 weights =
      nis_lerp4(nis_lerp4(e00, e01, fx), nis_lerp4(e10, e11, fx), fy);
  float base_weight = 1.0f - weights.x - weights.y - weights.z - weights.w;
  float normal = 0.0f;
  for (uint column = 0u; column < 6u; ++column) {
    float vertical = 0.0f;
    for (uint row = 0u; row < 6u; ++row)
      vertical += pixels[row][column] * NIS_SCALER[phase_y][row];
    normal += vertical * NIS_SCALER[phase_x][column];
  }
  return normal * base_weight +
         nis_directional(pixels, fx, fy, phase_x, phase_y, weights);
}

Color nis_rgba(Nv12LayerParams p, float x, float y) {
  Color sample = sample_rgba_bilinear_p(p, x, y);
  if (!magnifying(p))
    return sample;
  float correction = nis_filtered_luma(p, x, y, true) - rec709_luma(sample);
  return color(clamp(sample.r + correction, 0.0f, 1.0f),
               clamp(sample.g + correction, 0.0f, 1.0f),
               clamp(sample.b + correction, 0.0f, 1.0f), sample.a);
}
float nis_luma(Nv12LayerParams p, float x, float y) {
  if (!magnifying(p))
    return sample_scalar_weighted(p, x, y, VideoSampleMethod::Lanczos3, false,
                                  0u);
  float original = sample_scalar_bilinear(p, x, y, false, 0u);
  return original + (nis_filtered_luma(p, x, y, false) - original) / 1.1643835f;
}

Color xbrz_load(Nv12LayerParams p, int x, int y, bool luma_only) {
  if (!luma_only)
    return load_rgba(p, x, y);
  float value = load_luma(p, x, y);
  return color(value, value, value, 1.0f);
}
bool xbrz_same(Color a, Color b) {
  return bt2020_ycbcr_distance(a, b) < 1.0f / 65536.0f;
}
bool xbrz_similar(Color a, Color b) {
  return bt2020_ycbcr_distance(a, b) < 30.0f / 255.0f;
}
float2 xbrz_scale(Nv12LayerParams p) {
  float x = length(float2(p.inverse.x_axis.x, p.inverse.y_axis.x)),
        y = length(float2(p.inverse.x_axis.y, p.inverse.y_axis.y));
  return 1.0f / max(float2(x, y), float2(0.000001f));
}
float xbrz_fill(float2 pixel_offset, float2 origin, float2 direction,
                float2 scale) {
  float2 offset = pixel_offset - origin;
  float along = dot(offset, direction) / dot(direction, direction);
  float2 distance = (offset - direction * along) * scale;
  float side = sign(direction.x * offset.y - direction.y * offset.x);
  return smoothstep_exact(-0.70710677f, 0.70710677f, side * length(distance));
}
Color xbrz_sample(Nv12LayerParams p, float x, float y, bool luma_only) {
  float2 scale = xbrz_scale(p);
  if (scale.x < 1.0f || scale.y < 1.0f) {
    if (luma_only) {
      float value = sample_scalar_weighted(p, x, y, VideoSampleMethod::Lanczos3,
                                           false, 0u);
      return color(value, value, value, 1.0f);
    }
    return sample_rgba_bilinear_p(p, x, y);
  }
  int base_x = int(floor_f32(x + 0.5f)), base_y = int(floor_f32(y + 0.5f));
  float2 pixel_offset = float2(x - float(base_x), y - float(base_y));
  uint blends[4] = {0u, 0u, 0u, 0u};
  for (uint corner = 0u; corner < 4u; ++corner) {
    int sx = (corner & 1u) == 0u ? -1 : 1, sy = (corner & 2u) == 0u ? -1 : 1;
    Color center = xbrz_load(p, base_x, base_y, luma_only),
          right = xbrz_load(p, base_x + sx, base_y, luma_only),
          bottom = xbrz_load(p, base_x, base_y + sy, luma_only),
          diagonal = xbrz_load(p, base_x + sx, base_y + sy, luma_only);
    if ((xbrz_same(center, right) && xbrz_same(bottom, diagonal)) ||
        (xbrz_same(center, bottom) && xbrz_same(right, diagonal)) ||
        xbrz_same(center, right) || xbrz_same(center, bottom))
      continue;
    float downward =
        4.0f * bt2020_ycbcr_distance(center, diagonal) +
        bt2020_ycbcr_distance(
            right, xbrz_load(p, base_x + 2 * sx, base_y + sy, luma_only)) +
        bt2020_ycbcr_distance(
            bottom, xbrz_load(p, base_x + sx, base_y + 2 * sy, luma_only)) +
        bt2020_ycbcr_distance(xbrz_load(p, base_x, base_y - sy, luma_only),
                              right) +
        bt2020_ycbcr_distance(xbrz_load(p, base_x - sx, base_y, luma_only),
                              bottom);
    float upward =
        4.0f * bt2020_ycbcr_distance(bottom, right) +
        bt2020_ycbcr_distance(
            diagonal, xbrz_load(p, base_x + 2 * sx, base_y, luma_only)) +
        bt2020_ycbcr_distance(xbrz_load(p, base_x, base_y + 2 * sy, luma_only),
                              diagonal) +
        bt2020_ycbcr_distance(
            center, xbrz_load(p, base_x + sx, base_y - sy, luma_only)) +
        bt2020_ycbcr_distance(xbrz_load(p, base_x - sx, base_y + sy, luma_only),
                              center);
    if (upward < downward)
      blends[corner] = 3.6f * upward < downward ? 2u : 1u;
  }
  Color value = xbrz_load(p, base_x, base_y, luma_only);
  for (uint corner = 0u; corner < 4u; ++corner) {
    if (blends[corner] == 0u)
      continue;
    int sx = (corner & 1u) == 0u ? -1 : 1, sy = (corner & 2u) == 0u ? -1 : 1;
    Color center = xbrz_load(p, base_x, base_y, luma_only),
          right = xbrz_load(p, base_x + sx, base_y, luma_only),
          bottom = xbrz_load(p, base_x, base_y + sy, luma_only);
    float shallow = bt2020_ycbcr_distance(
              right, xbrz_load(p, base_x - sx, base_y + sy, luma_only)),
          steep = bt2020_ycbcr_distance(
              bottom, xbrz_load(p, base_x + sx, base_y - sy, luma_only));
    bool vertical_blend =
             blends[corner ^ 2u] != 0u &&
             !xbrz_similar(center,
                           xbrz_load(p, base_x - sx, base_y + sy, luma_only)),
         horizontal_blend =
             blends[corner ^ 1u] != 0u &&
             !xbrz_similar(center,
                           xbrz_load(p, base_x + sx, base_y - sy, luma_only));
    bool is_corner =
        !xbrz_similar(center,
                      xbrz_load(p, base_x + sx, base_y + sy, luma_only)) &&
        xbrz_similar(xbrz_load(p, base_x + sx, base_y - sy, luma_only),
                     right) &&
        xbrz_similar(right,
                     xbrz_load(p, base_x + sx, base_y + sy, luma_only)) &&
        xbrz_similar(xbrz_load(p, base_x + sx, base_y + sy, luma_only),
                     bottom) &&
        xbrz_similar(bottom, xbrz_load(p, base_x - sx, base_y + sy, luma_only));
    bool diagonal = blends[corner] == 2u ||
                    (!vertical_blend && !horizontal_blend && !is_corner);
    float2 origin = float2(0.0f, 0.70710677f), direction = float2(1.0f, -1.0f);
    if (diagonal) {
      float is_shallow =
          2.2f * shallow <= steep &&
                  !xbrz_same(center, xbrz_load(p, base_x - sx, base_y + sy,
                                               luma_only)) &&
                  !xbrz_same(xbrz_load(p, base_x - sx, base_y, luma_only),
                             xbrz_load(p, base_x - sx, base_y + sy, luma_only))
              ? 1.0f
              : 0.0f;
      float is_steep =
          2.2f * steep <= shallow &&
                  !xbrz_same(center, xbrz_load(p, base_x + sx, base_y - sy,
                                               luma_only)) &&
                  !xbrz_same(xbrz_load(p, base_x + sx, base_y - sy, luma_only),
                             xbrz_load(p, base_x, base_y - sy, luma_only))
              ? 1.0f
              : 0.0f;
      origin = float2(0.0f, 0.5f * (1.0f - 0.5f * is_shallow));
      direction = float2(1.0f + is_shallow, -1.0f - is_steep);
    }
    Color fill = bt2020_ycbcr_distance(center, right) <
                         bt2020_ycbcr_distance(center, bottom)
                     ? right
                     : bottom;
    value = color_lerp(value, fill,
                       xbrz_fill(pixel_offset * float2(float(sx), float(sy)),
                                 origin, direction, scale));
  }
  return value;
}

void stochastic_samples(Nv12LayerParams p, float x, float y,
                        out float3 samples[3]) {
  float4 crop = crop_rect(p);
  float u = (x - crop.x) / crop.z, v = (y - crop.y) / crop.w,
        lattice_x = u - v * 0.57735026f, lattice_y = v * 1.1547005f;
  int cell_x = int(floor_f32(lattice_x)), cell_y = int(floor_f32(lattice_y));
  float fx = lattice_x - float(cell_x), fy = lattice_y - float(cell_y);
  int2 vertices[3];
  float weights[3];
  if (fx + fy < 1.0f) {
    vertices = {int2(cell_x, cell_y), int2(cell_x + 1, cell_y),
                int2(cell_x, cell_y + 1)};
    weights = {1.0f - fx - fy, fx, fy};
  } else {
    vertices = {int2(cell_x + 1, cell_y + 1), int2(cell_x, cell_y + 1),
                int2(cell_x + 1, cell_y)};
    weights = {fx + fy - 1.0f, 1.0f - fx, 1.0f - fy};
  }
  for (uint i = 0u; i < 3u; ++i)
    samples[i] = float3(
        x + (hash_unit(vertices[i].x, vertices[i].y, 0u) - 0.5f) * crop.z,
        y + (hash_unit(vertices[i].x, vertices[i].y, 1u) - 0.5f) * crop.w,
        weights[i]);
}
float stochastic_amount(Nv12LayerParams p, float distance_value) {
  float4 crop = crop_rect(p);
  return smoothstep_exact(0.0f, clamp(min(crop.z, crop.w) * 0.05f, 8.0f, 32.0f),
                          distance_value);
}

Color sample_rgba_plain(Nv12LayerParams p, float source_x, float source_y) {
  float x = source_x - 0.5f, y = source_y - 0.5f;
  if (p.sample_method == VideoSampleMethod::Nearest)
    return load_rgba(p, int(floor_f32(x + 0.5f)), int(floor_f32(y + 0.5f)));
  if (p.sample_method == VideoSampleMethod::Fsr1Easu) {
    Color bilinear = sample_rgba_bilinear_p(p, x, y);
    return magnifying(p)
               ? color_with_alpha(fsr_easu(p, x, y, false), bilinear.a)
               : bilinear;
  }
  if (p.sample_method == VideoSampleMethod::NvidiaImageScaling)
    return nis_rgba(p, x, y);
  if (p.sample_method == VideoSampleMethod::Xbrz)
    return xbrz_sample(p, x, y, false);
  if (p.sample_method == VideoSampleMethod::Bicubic ||
      p.sample_method == VideoSampleMethod::Mitchell ||
      p.sample_method == VideoSampleMethod::Lanczos2 ||
      p.sample_method == VideoSampleMethod::Lanczos3)
    return sample_rgba_weighted_p(p, x, y, p.sample_method);
  return sample_rgba_bilinear_p(p, x, y);
}
Color sample_rgba_layer(Nv12LayerParams p, float x, float y) {
  Color sample;
  float distance_value = outside_distance(p, x, y);
  if (p.address_mode == TextureAddressMode::BlurredMirror &&
      distance_value > 0.0f) {
    float offset = min(distance_value * 0.2f, 12.0f) * 0.70710677f;
    Color center = sample_rgba_plain(p, x, y),
          a = sample_rgba_plain(p, x - offset, y - offset),
          b = sample_rgba_plain(p, x + offset, y - offset),
          c = sample_rgba_plain(p, x - offset, y + offset),
          d = sample_rgba_plain(p, x + offset, y + offset);
    sample = color(center.r * 0.4f + (a.r + b.r + c.r + d.r) * 0.15f,
                   center.g * 0.4f + (a.g + b.g + c.g + d.g) * 0.15f,
                   center.b * 0.4f + (a.b + b.b + c.b + d.b) * 0.15f,
                   center.a * 0.4f + (a.a + b.a + c.a + d.a) * 0.15f);
  } else if (p.address_mode == TextureAddressMode::Stochastic &&
             distance_value > 0.0f) {
    Color base = sample_rgba_plain(p, x, y);
    float3 points[3];
    stochastic_samples(p, x, y, points);
    Color a = sample_rgba_plain(p, points[0].x, points[0].y),
          b = sample_rgba_plain(p, points[1].x, points[1].y),
          c = sample_rgba_plain(p, points[2].x, points[2].y);
    Color synthesized =
        color(a.r * points[0].z + b.r * points[1].z + c.r * points[2].z,
              a.g * points[0].z + b.g * points[1].z + c.g * points[2].z,
              a.b * points[0].z + b.b * points[1].z + c.b * points[2].z,
              a.a * points[0].z + b.a * points[1].z + c.a * points[2].z);
    sample =
        color_lerp(base, synthesized, stochastic_amount(p, distance_value));
  } else
    sample = sample_rgba_plain(p, x, y);
  float alpha = clamp(sample.a, 0.0f, 1.0f);
  return color_unpremultiply(color(clamp(sample.r, 0.0f, alpha),
                                   clamp(sample.g, 0.0f, alpha),
                                   clamp(sample.b, 0.0f, alpha), alpha));
}
float4 sample_nv12_plain_values(Nv12LayerParams p, float source_x,
                                float source_y) {
  float x = source_x - 0.5f, y = source_y - 0.5f, cx = (x - 0.5f) * 0.5f,
        cy = (y - 0.5f) * 0.5f, alpha = coverage(p, x, y), luma, cb, cr;
  if (p.sample_method == VideoSampleMethod::Fsr1Easu)
    return fsr_nv12(p, x, y, alpha);
  if (p.sample_method == VideoSampleMethod::Nearest) {
    luma = load_luma(p, int(floor_f32(x + 0.5f)), int(floor_f32(y + 0.5f)));
    cb = load_chroma(p, int(floor_f32(cx + 0.5f)), int(floor_f32(cy + 0.5f)),
                     0u) -
         0.5f;
    cr = load_chroma(p, int(floor_f32(cx + 0.5f)), int(floor_f32(cy + 0.5f)),
                     1u) -
         0.5f;
  } else if (p.sample_method == VideoSampleMethod::Bicubic ||
             p.sample_method == VideoSampleMethod::Mitchell ||
             p.sample_method == VideoSampleMethod::Lanczos2 ||
             p.sample_method == VideoSampleMethod::Lanczos3) {
    luma = sample_scalar_weighted(p, x, y, p.sample_method, false, 0u);
    cb = sample_scalar_weighted(p, cx, cy, p.sample_method, true, 0u) - 0.5f;
    cr = sample_scalar_weighted(p, cx, cy, p.sample_method, true, 1u) - 0.5f;
  } else {
    luma = p.sample_method == VideoSampleMethod::NvidiaImageScaling
               ? nis_luma(p, x, y)
               : (p.sample_method == VideoSampleMethod::Xbrz
                      ? xbrz_sample(p, x, y, true).r
                      : sample_scalar_bilinear(p, x, y, false, 0u));
    cb = sample_scalar_bilinear(p, cx, cy, true, 0u) - 0.5f;
    cr = sample_scalar_bilinear(p, cx, cy, true, 1u) - 0.5f;
  }
  return finish_nv12(p, float4(luma, cb, cr, alpha));
}
Color sample_nv12_layer(Nv12LayerParams p, float x, float y) {
  float distance_value = outside_distance(p, x, y);
  float4 sample;
  if (p.address_mode == TextureAddressMode::BlurredMirror &&
      distance_value > 0.0f) {
    float offset = min(distance_value * 0.2f, 12.0f) * 0.70710677f;
    float4 center = sample_nv12_plain_values(p, x, y),
           a = sample_nv12_plain_values(p, x - offset, y - offset),
           b = sample_nv12_plain_values(p, x + offset, y - offset),
           c = sample_nv12_plain_values(p, x - offset, y + offset),
           d = sample_nv12_plain_values(p, x + offset, y + offset);
    sample = center * 0.4f + (a + b + c + d) * 0.15f;
  } else if (p.address_mode == TextureAddressMode::Stochastic &&
             distance_value > 0.0f) {
    float4 base = sample_nv12_plain_values(p, x, y);
    float3 points[3];
    stochastic_samples(p, x, y, points);
    float4 a = sample_nv12_plain_values(p, points[0].x, points[0].y),
           b = sample_nv12_plain_values(p, points[1].x, points[1].y),
           c = sample_nv12_plain_values(p, points[2].x, points[2].y);
    sample = lerp(base, a * points[0].z + b * points[1].z + c * points[2].z,
                  stochastic_amount(p, distance_value));
  } else
    sample = sample_nv12_plain_values(p, x, y);
  return from_bt709_ycbcr(sample.x, sample.y, sample.z, sample.w);
}

uint blend_noise(uint seed, uint index) {
  uint value = seed ^ index * 0x9e3779b9u;
  value ^= value >> 16;
  value *= 0x7feb352du;
  value ^= value >> 15;
  value *= 0x846ca68bu;
  return value ^ (value >> 16);
}
uint blend_pixel(uint source_value, uint destination_value, LayerBlendMode mode,
                 float opacity, uint seed, uint index) {
  Color source = color_from_rgba(source_value),
        destination = color_from_rgba(destination_value);
  float alpha = source.a * clamp(opacity, 0.0f, 1.0f);
  if (mode == LayerBlendMode::Dissolve)
    alpha =
        float(blend_noise(seed, index)) <= alpha * 4294967295.0f ? 1.0f : 0.0f;
  return color_to_rgba(blend_over(source, destination, mode, alpha, false));
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void composite_layered_image_layer(
    uint3 id : SV_DispatchThreadID, uniform LayerCompositeParams params,
    uniform uint *output, uniform uint64_t output_count) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint column = i % params.width, row = i / params.width;
  uint source =
      params.source[uint64_t(row) * (params.source_pitch / 4u) + column];
  float clipping = 1.0f;
  if (params.clipping_base != nullptr)
    clipping = color_from_rgba(
                   params.clipping_base[uint64_t(row) *
                                            (params.clipping_base_pitch / 4u) +
                                        column])
                   .a *
               clamp(params.clipping_base_opacity, 0.0f, 1.0f);
  output[i] = blend_pixel(source, output[i], params.mode,
                          params.opacity * clipping, params.noise_seed, i);
} bool sample_layer(Nv12LayerParams p, Mat3 inverse, float canvas_x,
                    float canvas_y, out Color result) {
  result = color(0.0f, 0.0f, 0.0f, 0.0f);
  float2 source = transform_point2(inverse, float2(canvas_x, canvas_y));
  float left = float(p.source_width) * p.crop[3],
        right = float(p.source_width) * (1.0f - p.crop[1]),
        top = float(p.source_height) * p.crop[0],
        bottom = float(p.source_height) * (1.0f - p.crop[2]);
  if (source.x < left - p.padding[3] || source.y < top - p.padding[0] ||
      source.x >= right + p.padding[1] || source.y >= bottom + p.padding[2] ||
      (p.address_mode == TextureAddressMode::Transparent &&
       (source.x < left || source.y < top || source.x >= right ||
        source.y >= bottom)))
    return false;
  result = p.kind == LayerKind::Nv12 ? sample_nv12_layer(p, source.x, source.y)
                                     : sample_rgba_layer(p, source.x, source.y);
  return true;
}
uint composite_pixel(Nv12LayerParams *layers, uint64_t layer_count,
                     Mat3 *transforms, uint64_t transform_count, uint x, uint y,
                     uint background) {
  float canvas_x = float(x) + 0.5f, canvas_y = float(y) + 0.5f;
  uint pixel = background;
  for (uint64_t index = 0; index < layer_count; ++index) {
    Nv12LayerParams p = layers[index];
    if (p.motion_sample_count == 0u) {
      Color sample;
      if (sample_layer(p, p.inverse, canvas_x, canvas_y, sample))
        pixel =
            color_to_rgba(blend_over(sample, color_from_rgba(pixel),
                                     p.blend_mode, sample.a * p.opacity, true));
      continue;
    }
    float4 sum = float4(0.0f);
    uint start = p.motion_transform_offset,
         end = min(start + p.motion_transform_count, uint(transform_count));
    if (end - start != p.motion_transform_count)
      continue;
    for (uint t = start; t < end; ++t) {
      Color sample;
      if (sample_layer(p, transforms[t], canvas_x, canvas_y, sample))
        sum += float4(sample.r * sample.a, sample.g * sample.a,
                      sample.b * sample.a, sample.a);
    }
    if (sum.w <= 0.0f)
      continue;
    Color source = color(sum.x / sum.w, sum.y / sum.w, sum.z / sum.w, sum.w);
    pixel = color_to_rgba(
        blend_over(source, color_from_rgba(pixel), p.blend_mode,
                   sum.w / float(p.motion_sample_count) * p.opacity, true));
  }
  return pixel;
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
composite_nv12_layers(uint3 id : SV_DispatchThreadID,
                      uniform Nv12LayerParams *layers,
                      uniform uint64_t layer_count, uniform Mat3 *transforms,
                      uniform uint64_t transform_count, uniform uint *output,
                      uniform uint64_t output_count, uniform uint background) {
  uint i = id.x;
  if (i >= output_count)
    return;
  output[i] = background;
  if (layer_count == 0u)
    return;
  uint x = i % layers[0].canvas_width, y = i / layers[0].canvas_width;
  output[i] = composite_pixel(layers, layer_count, transforms, transform_count,
                              x, y, background);
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void tone_map_hdr(uint3 id : SV_DispatchThreadID,
                                uniform Color *input, uniform Color *background,
                                uniform uint *output,
                                uniform uint64_t output_count,
                                uniform float levels) {
  uint i = id.x;
  if (i >= output_count)
    return;
  Color source = input[i], bg = background[i], value = aces_tone_mapped(source);
  if (levels > 0.0f) {
    value.r = floor(value.r * levels) / levels;
    value.g = floor(value.g * levels) / levels;
    value.b = floor(value.b * levels) / levels;
  }
  value.r = clamp(value.r + bg.r, 0.0f, 1.0f);
  value.g = clamp(value.g + bg.g, 0.0f, 1.0f);
  value.b = clamp(value.b + bg.b, 0.0f, 1.0f);
  output[i] = color_to_rgba(linear_to_srgb(color_with_alpha(value, source.a)));
}
