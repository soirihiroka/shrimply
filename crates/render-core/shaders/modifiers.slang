import abi;
import color;
import common;
import math;

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void invert(uniform uint *input, uniform uint *out,
                         uniform uint64_t out_len, uniform float amount,
                         uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  Color c = color_from_rgba(input[i]);
  out[i] = color_to_rgba(color(c.r + (1.0f - 2.0f * c.r) * amount,
                               c.g + (1.0f - 2.0f * c.g) * amount,
                               c.b + (1.0f - 2.0f * c.b) * amount, c.a));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void posterize(uniform uint *input, uniform uint *out,
                                uniform uint64_t out_len, uniform float levels,
                                uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  Color c = color_from_rgba(input[i]);
  float n = max(rust_round(levels) - 1.0f, 1.0f);
  out[i] = color_to_rgba(color(rust_round(c.r * n) / n, rust_round(c.g * n) / n,
                               rust_round(c.b * n) / n, c.a));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void channel_mixer(uniform uint *input, uniform uint *out,
                                uniform uint64_t out_len,
                                uniform ChannelMixerParams params,
                                uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  Color c = color_from_rgba(input[i]);
  float3 mixed = mat3_mul(params.matrix, float3(c.r, c.g, c.b));
  out[i] = color_to_rgba(color(mixed.x, mixed.y, mixed.z, c.a));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void color_correction(uniform uint *input, uniform uint *out,
                                       uniform uint64_t out_len,
                                       uniform ColorCorrectionParams params,
                                       uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  out[i] = color_to_rgba(color_corrected(color_from_rgba(input[i]), params));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void colorize_duotone(uniform uint *input, uniform uint *out,
                                   uniform uint64_t out_len,
                                   uniform ColorizeDuotoneParams params,
                                   uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  Color c = color_from_rgba(input[i]);
  out[i] = color_to_rgba(color_with_alpha(
      color_lerp(params.shadow, params.highlight, rec709_luma(c)), c.a));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void chromatic_aberration(
        uniform uint *input, uniform uint width, uniform uint height,
        uniform uint *out, uniform uint64_t out_len,
        uniform ChromaticAberrationParams params,
        uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  uint x = i % width;
  uint y = i / width;
  uint r_pixel =
      input[clamp_index(int(y) + int(rust_round(params.offsets[1])), height) *
                width +
            clamp_index(int(x) + int(rust_round(params.offsets[0])), width)];
  uint b_pixel =
      input[clamp_index(int(y) + int(rust_round(params.offsets[3])), height) *
                width +
            clamp_index(int(x) + int(rust_round(params.offsets[2])), width)];
  Color r = color_from_rgba(r_pixel);
  Color center = color_from_rgba(input[i]);
  Color b = color_from_rgba(b_pixel);
  out[i] = color_to_rgba(color(r.r, center.g, b.b, center.a));
}

float input_luminance(uint *input, uint width, uint height, int x, int y) {
  return rec709_luma(color_from_rgba(
      input[clamp_index(y, height) * width + clamp_index(x, width)]));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void edge_detection(uniform uint *input, uniform uint width,
                                 uniform uint height, uniform uint *out,
                                 uniform uint64_t out_len,
                                 uniform EdgeDetectionParams params,
                                 uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  int x = int(i % width);
  int y = int(i / width);
  float gx = -input_luminance(input, width, height, x - 1, y - 1) -
             2.0f * input_luminance(input, width, height, x - 1, y) -
             input_luminance(input, width, height, x - 1, y + 1) +
             input_luminance(input, width, height, x + 1, y - 1) +
             2.0f * input_luminance(input, width, height, x + 1, y) +
             input_luminance(input, width, height, x + 1, y + 1);
  float gy = -input_luminance(input, width, height, x - 1, y - 1) -
             2.0f * input_luminance(input, width, height, x, y - 1) -
             input_luminance(input, width, height, x + 1, y - 1) +
             input_luminance(input, width, height, x - 1, y + 1) +
             2.0f * input_luminance(input, width, height, x, y + 1) +
             input_luminance(input, width, height, x + 1, y + 1);
  float edge = clamp(sqrt(gx * gx + gy * gy), 0.0f, 1.0f) * params.amount;
  out[i] = color_to_rgba(
      color_with_alpha(color_lerp(params.background, params.edge, edge),
                       color_from_rgba(input[i]).a));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void emboss(uniform uint *input, uniform uint width,
                             uniform uint height, uniform uint *out,
                             uniform uint64_t out_len, uniform float direction,
                             uniform float depth, uniform float amount,
                             uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  uint x = i % width;
  uint y = i / width;
  float radians = direction * (PI / 180.0f);
  int dx = int(rust_round(cos(radians)));
  int dy = int(rust_round(sin(radians)));
  Color previous =
      color_from_rgba(input[clamp_index(int(y) - dy, height) * width +
                            clamp_index(int(x) - dx, width)]);
  Color next = color_from_rgba(input[clamp_index(int(y) + dy, height) * width +
                                     clamp_index(int(x) + dx, width)]);
  Color c = color_from_rgba(input[i]);
  float value = 0.5f + ((previous.r + previous.g + previous.b) -
                        (next.r + next.g + next.b)) /
                           3.0f * depth;
  out[i] = color_to_rgba(color(c.r + (value - c.r) * amount,
                               c.g + (value - c.g) * amount,
                               c.b + (value - c.b) * amount, c.a));
}

float grain_hash(uint x) {
  x ^= x >> 16;
  x *= 0x7feb352du;
  x ^= x >> 15;
  x *= 0x846ca68bu;
  x ^= x >> 16;
  return float(x) / float(0xffffffffu);
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void film_grain(uniform uint *input, uniform uint width,
                             uniform uint *out, uniform uint64_t out_len,
                             uniform float amount, uniform float size,
                             uniform float colored, uniform float seed,
                             uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  Color c = color_from_rgba(input[i]);
  uint block = uint(max(rust_round(size), 1.0f));
  uint base = (i % width) / block + ((i / width) / block) * 65537u + uint(seed);
  float mono = (grain_hash(base) - 0.5f) * amount;
  float mix_amount = clamp(colored, 0.0f, 1.0f);
  float nr = (grain_hash(base + 17u) - 0.5f) * amount;
  float ng = (grain_hash(base + 31u) - 0.5f) * amount;
  float nb = (grain_hash(base + 47u) - 0.5f) * amount;
  out[i] = color_to_rgba(
      color(c.r + mono * (1.0f - mix_amount) + nr * mix_amount,
            c.g + mono * (1.0f - mix_amount) + ng * mix_amount,
            c.b + mono * (1.0f - mix_amount) + nb * mix_amount, c.a));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void scanlines_crt(uniform uint *input, uniform uint width,
                                    uniform uint height, uniform uint *out,
                                    uniform uint64_t out_len,
                                    uniform ScanlinesCrtParams params,
                                    uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  uint x = i % width;
  uint y = i / width;
  float nx = float(x) / float(max(width, 2u) - 1u) * 2.0f - 1.0f;
  float ny = float(y) / float(max(height, 2u) - 1u) * 2.0f - 1.0f;
  float bend = 1.0f + params.curvature * (nx * nx + ny * ny);
  int sx = int(rust_round((nx * bend + 1.0f) * 0.5f * float(width - 1u)));
  int sy = int(rust_round((ny * bend + 1.0f) * 0.5f * float(height - 1u)));
  if (sx < 0 || sy < 0 || sx >= int(width) || sy >= int(height)) {
    out[i] = 0;
    return;
  }
  Color c = color_from_rgba(input[uint(sy) * width + uint(sx)]);
  float line = frac(float(y) / max(params.spacing, 1.0f)) > 0.5f
                   ? 1.0f - params.intensity
                   : 1.0f;
  uint phase = x % 3u;
  float mr = phase == 0u ? 1.0f : 1.0f - params.mask;
  float mg = phase == 1u ? 1.0f : 1.0f - params.mask;
  float mb = phase == 2u ? 1.0f : 1.0f - params.mask;
  out[i] = color_to_rgba(
      color(c.r * line * mr, c.g * line * mg, c.b * line * mb, c.a));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
threshold(uniform uint *input, uniform uint *out, uniform uint64_t out_len,
          uniform ThresholdParams params, uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  Color c = color_from_rgba(input[i]);
  Color selected =
      rec709_luma(c) >= params.threshold ? params.high : params.low;
  out[i] = color_to_rgba(color_with_alpha(selected, c.a));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void vignette(uniform uint *input, uniform uint width,
                               uniform uint height, uniform uint *out,
                               uniform uint64_t out_len, uniform float amount,
                               uniform float midpoint, uniform float softness,
                               uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  Color c = color_from_rgba(input[i]);
  float x = float(i % width) / float(max(width, 1u)) * 2.0f - 1.0f;
  float y = float(i / width) / float(max(height, 1u)) * 2.0f - 1.0f;
  float distance = sqrt(x * x + y * y) / SQRT_2;
  float edge = smoothstep_exact(clamp(midpoint, 0.0f, 1.0f),
                                clamp(midpoint + softness, 0.00001f, 1.00001f),
                                distance);
  float shade = 1.0f - clamp(amount, 0.0f, 1.0f) * edge;
  out[i] = color_to_rgba(color(c.r * shade, c.g * shade, c.b * shade, c.a));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
alpha_outline_horizontal(uniform uint *input, uniform uint width,
                         uniform float *out, uniform uint64_t out_len,
                         uniform uint radius, uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  uint x = i % width;
  uint row = i - x;
  uint start = x > radius ? x - radius : 0u;
  uint end = min(x + radius, width - 1u);
  float alpha = 0.0f;
  for (uint sample_x = start; sample_x <= end; sample_x++)
    alpha = max(alpha, color_from_rgba(input[row + sample_x]).a);
  out[i] = alpha;
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void alpha_outline_vertical(
        uniform uint *input, uniform float *horizontal, uniform uint width,
        uniform uint height, uniform uint *out, uniform uint64_t out_len,
        uniform uint radius, uniform uint outline_value,
        uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  Color base = color_from_rgba(input[i]);
  uint x = i % width;
  uint y = i / width;
  uint start = y > radius ? y - radius : 0u;
  uint end = min(y + radius, height - 1u);
  float alpha = 0.0f;
  for (uint sample_y = start; sample_y <= end; sample_y++)
    alpha = max(alpha, horizontal[sample_y * width + x]);
  Color outline = color_from_rgba(outline_value);
  float outline_alpha = outline.a * alpha * (1.0f - base.a);
  float output_alpha = base.a + outline_alpha;
  float inverse_alpha = output_alpha > 0.0f ? 1.0f / output_alpha : 0.0f;
  out[i] = color_to_rgba(
      color((base.r * base.a + outline.r * outline_alpha) * inverse_alpha,
            (base.g * base.a + outline.g * outline_alpha) * inverse_alpha,
            (base.b * base.a + outline.b * outline_alpha) * inverse_alpha,
            output_alpha));
}

float bayer(DitheringPattern pattern, uint x, uint y) {
  static const uint bayer_2[4] = {0, 2, 3, 1};
  static const uint bayer_4[16] = {0, 8,  2, 10, 12, 4, 14, 6,
                                   3, 11, 1, 9,  15, 7, 13, 5};
  static const uint bayer_8[64] = {
      0,  32, 8,  40, 2,  34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26,
      12, 44, 4,  36, 14, 46, 6,  38, 60, 28, 52, 20, 62, 30, 54, 22,
      3,  35, 11, 43, 1,  33, 9,  41, 51, 19, 59, 27, 49, 17, 57, 25,
      15, 47, 7,  39, 13, 45, 5,  37, 63, 31, 55, 23, 61, 29, 53, 21};
  if (pattern == DitheringPattern::Bayer2x2)
    return (float(bayer_2[(y & 1u) * 2u + (x & 1u)]) + 0.5f) / 4.0f;
  if (pattern == DitheringPattern::Bayer4x4)
    return (float(bayer_4[(y & 3u) * 4u + (x & 3u)]) + 0.5f) / 16.0f;
  return (float(bayer_8[(y & 7u) * 8u + (x & 7u)]) + 0.5f) / 64.0f;
}

void nearest_palette_colors(float r, float g, float b, uint *palette,
                            uint palette_len, out uint best, out uint second,
                            out float mix_amount) {
  best = palette[0];
  second = best;
  float best_distance = 3.40282347e38f;
  float second_distance = 3.40282347e38f;
  for (uint index = 0; index < palette_len; index++) {
    uint candidate = palette[index];
    Color c = color_from_rgba(candidate);
    float dr = r - c.r;
    float dg = g - c.g;
    float db = b - c.b;
    float distance = dr * dr + dg * dg + db * db;
    if (distance < best_distance) {
      second = best;
      second_distance = best_distance;
      best = candidate;
      best_distance = distance;
    } else if (distance < second_distance) {
      second = candidate;
      second_distance = distance;
    }
  }
  Color first_color = color_from_rgba(best);
  Color second_color = color_from_rgba(second);
  float dr = second_color.r - first_color.r;
  float dg = second_color.g - first_color.g;
  float db = second_color.b - first_color.b;
  float denominator = dr * dr + dg * dg + db * db;
  mix_amount =
      denominator > 0.000001f
          ? clamp(((r - first_color.r) * dr + (g - first_color.g) * dg +
                   (b - first_color.b) * db) /
                      denominator,
                  0.0f, 1.0f)
          : 0.0f;
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void dithering(uniform uint *input, uniform uint width,
                            uniform uint *out, uniform uint64_t out_len,
                            uniform DitheringParams params,
                            uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  Color c = color_from_rgba(input[i]);
  uint x = i % width;
  uint y = i / width;
  float pattern_value = bayer(params.pattern, x, y);
  float threshold_value = pattern_value - 0.5f;
  float steps = max(rust_round(params.levels) - 1.0f, 1.0f);
  float amount = clamp(params.amount, 0.0f, 1.0f);
  float target_r =
      rust_round(clamp(c.r + threshold_value / steps, 0.0f, 1.0f) * steps) /
      steps;
  float target_g =
      rust_round(clamp(c.g + threshold_value / steps, 0.0f, 1.0f) * steps) /
      steps;
  float target_b =
      rust_round(clamp(c.b + threshold_value / steps, 0.0f, 1.0f) * steps) /
      steps;
  if (params.color_mode == DitheringColorMode::Color)
    out[i] = color_to_rgba(color(c.r + (target_r - c.r) * amount,
                                 c.g + (target_g - c.g) * amount,
                                 c.b + (target_b - c.b) * amount, c.a));
  else if (params.color_mode == DitheringColorMode::Grayscale) {
    float v =
        rust_round(clamp(rec709_luma(c) + threshold_value / steps, 0.0f, 1.0f) *
                   steps) /
        steps;
    out[i] =
        color_to_rgba(color(c.r + (v - c.r) * amount, c.g + (v - c.g) * amount,
                            c.b + (v - c.b) * amount, c.a));
  } else if (params.palette_len > 0u) {
    uint first, second;
    float palette_mix;
    nearest_palette_colors(c.r, c.g, c.b, params.palette, params.palette_len,
                           first, second, palette_mix);
    Color target =
        color_from_rgba(pattern_value < palette_mix ? second : first);
    out[i] = color_to_rgba(color(c.r + (target.r - c.r) * amount,
                                 c.g + (target.g - c.g) * amount,
                                 c.b + (target.b - c.b) * amount, c.a));
  } else
    out[i] = input[i];
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void drop_shadow_horizontal(
        uniform uint *input, uniform uint width, uniform uint height,
        uniform float *out, uniform uint64_t out_len,
        uniform DropShadowParams params, uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  int x = int(i % width) - int(rust_round(params.offset.x));
  int y = int(i / width) - int(rust_round(params.offset.y));
  int radius = int(params.radius);
  float alpha = 0.0f;
  for (int dx = -radius; dx <= radius; dx++) {
    int sample_x = x + dx;
    if (sample_x >= 0 && sample_x < int(width) && y >= 0 && y < int(height))
      alpha += color_from_rgba(input[uint(y) * width + uint(sample_x)]).a;
  }
  out[i] = alpha / float(2 * radius + 1);
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
drop_shadow_vertical(uniform uint *input, uniform float *horizontal,
                     uniform uint width, uniform uint height, uniform uint *out,
                     uniform uint64_t out_len, uniform DropShadowParams params,
                     uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  uint x = i % width;
  uint y = i / width;
  int radius = int(params.radius);
  float alpha = 0.0f;
  for (int dy = -radius; dy <= radius; dy++) {
    int sample_y = int(y) + dy;
    if (sample_y >= 0 && sample_y < int(height))
      alpha += horizontal[uint(sample_y) * width + x];
  }
  alpha /= float(2 * radius + 1);
  Color base = color_from_rgba(input[i]);
  Color shadow = color_from_rgba(params.color);
  float shadow_alpha = shadow.a * alpha;
  float shadow_weight = shadow_alpha * (1.0f - base.a);
  float output_alpha = base.a + shadow_weight;
  float inverse_alpha = output_alpha > 0.0f ? 1.0f / output_alpha : 0.0f;
  out[i] = color_to_rgba(
      color((base.r * base.a + shadow.r * shadow_weight) * inverse_alpha,
            (base.g * base.a + shadow.g * shadow_weight) * inverse_alpha,
            (base.b * base.a + shadow.b * shadow_weight) * inverse_alpha,
            output_alpha));
}

float halftone_dot(float x, float y, float cell, float angle, float offset_x,
                   float offset_y, float channel, float contrast) {
  float radians = angle * (PI / 180.0f);
  float cell_x = (x * cos(radians) - y * sin(radians) - offset_x) / cell;
  float cell_y = (x * sin(radians) + y * cos(radians) - offset_y) / cell;
  float px = cell_x - floor(cell_x) - 0.5f;
  float py = cell_y - floor(cell_y) - 0.5f;
  float distance = sqrt(px * px + py * py) * 2.0f;
  float value = clamp((channel - 0.5f) * contrast + 0.5f, 0.0f, 1.0f);
  return distance < sqrt(value) ? 1.0f : 0.0f;
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void halftone(uniform uint *input, uniform uint width,
                           uniform uint *out, uniform uint64_t out_len,
                           uniform HalftoneParams params,
                           uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  Color c = color_from_rgba(input[i]);
  float x = float(i % width);
  float y = float(i / width);
  float cell = max(params.size, 1.0f);
  float separation = max(params.channel_offset, 0.0f);
  if (params.mode == 1u) {
    float red = halftone_dot(
        x, y, cell, params.angle - params.channel_angle_offset,
        -separation * 0.5f, -separation * 0.28867513f, c.r, params.contrast);
    float green = halftone_dot(
        x, y, cell, params.angle + params.channel_angle_offset,
        separation * 0.5f, -separation * 0.28867513f, c.g, params.contrast);
    float blue = halftone_dot(x, y, cell, params.angle, 0.0f,
                              separation * 0.57735026f, c.b, params.contrast);
    out[i] = color_to_rgba(color(red, green, blue, c.a));
  } else if (params.mode == 2u) {
    float key = 1.0f - max(c.r, max(c.g, c.b));
    float color_amount = 1.0f - key;
    float cyan =
        color_amount == 0.0f ? 0.0f : (1.0f - c.r - key) / color_amount;
    float magenta =
        color_amount == 0.0f ? 0.0f : (1.0f - c.g - key) / color_amount;
    float yellow =
        color_amount == 0.0f ? 0.0f : (1.0f - c.b - key) / color_amount;
    cyan = halftone_dot(x, y, cell, params.angle - params.channel_angle_offset,
                        -separation * 0.5f, -separation * 0.5f, cyan,
                        params.contrast);
    magenta = halftone_dot(
        x, y, cell, params.angle + params.channel_angle_offset,
        separation * 0.5f, -separation * 0.5f, magenta, params.contrast);
    yellow = halftone_dot(
        x, y, cell, params.angle - params.channel_angle_offset * 1.5f,
        -separation * 0.5f, separation * 0.5f, yellow, params.contrast);
    key = halftone_dot(x, y, cell, params.angle, separation * 0.5f,
                       separation * 0.5f, key, params.contrast);
    out[i] = color_to_rgba(color((1.0f - cyan) * (1.0f - key),
                                 (1.0f - magenta) * (1.0f - key),
                                 (1.0f - yellow) * (1.0f - key), c.a));
  } else {
    float value = halftone_dot(x, y, cell, params.angle, 0.0f, 0.0f,
                               rec709_luma(c), params.contrast);
    out[i] = color_to_rgba(color(value, value, value, c.a));
  }
}
