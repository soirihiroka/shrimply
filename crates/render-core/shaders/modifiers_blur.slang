import abi;
import color;
import common;
import math;

uint gaussian_horizontal(uint *input, uint index, uint width, uint radius_u) {
  int x = int(index % width);
  uint y = index / width;
  int radius = int(min(radius_u, 100u));
  float4 sum = float4(0.0f);
  float total = 0.0f;
  for (int d = -radius; d <= radius; ++d) {
    uint sx = uint(clamp(x + d, 0, int(width) - 1));
    Color c = color_from_rgba(input[y * width + sx]);
    float w = gaussian_weight(d, uint(radius));
    sum += float4(c.r * c.a, c.g * c.a, c.b * c.a, c.a) * w;
    total += w;
  }
  return color_to_rgba(
      color(sum.x / total, sum.y / total, sum.z / total, sum.w / total));
}
uint gaussian_vertical(uint *input, uint index, uint width, uint height,
                       uint radius_u) {
  uint x = index % width;
  int y = int(index / width), radius = int(min(radius_u, 100u));
  float4 sum = float4(0.0f);
  float total = 0.0f;
  for (int d = -radius; d <= radius; ++d) {
    uint sy = uint(clamp(y + d, 0, int(height) - 1));
    Color c = color_from_rgba(input[sy * width + x]);
    float w = gaussian_weight(d, uint(radius));
    sum += float4(c.r, c.g, c.b, c.a) * w;
    total += w;
  }
  float alpha = sum.w / total, divisor = max(alpha, 0.00001f);
  return color_to_rgba(color(sum.x / total / divisor, sum.y / total / divisor,
                             sum.z / total / divisor, alpha));
}
Color sample_premultiplied(uint *input, uint width, uint height, float x,
                           float y) {
  x = clamp(x, 0.0f, float(width - 1u));
  y = clamp(y, 0.0f, float(height - 1u));
  uint x0 = uint(floor_f32(x)), y0 = uint(floor_f32(y)),
       x1 = min(x0 + 1u, width - 1u), y1 = min(y0 + 1u, height - 1u);
  float fx = x - float(x0), fy = y - float(y0);
  Color tl = color_premultiply(color_from_rgba(input[y0 * width + x0])),
        tr = color_premultiply(color_from_rgba(input[y0 * width + x1])),
        bl = color_premultiply(color_from_rgba(input[y1 * width + x0])),
        br = color_premultiply(color_from_rgba(input[y1 * width + x1]));
  return color_lerp(color_lerp(tl, tr, fx), color_lerp(bl, br, fx), fy);
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void directional_blur(uint3 id : SV_DispatchThreadID,
                                   uniform uint *input, uniform uint width,
                                   uniform uint height, uniform uint *output,
                                   uniform uint64_t output_count,
                                   uniform uint radius, uniform float angle) {
  uint i = id.x;
  if (i >= output_count)
    return;
  int x = int(i % width), y = int(i / width);
  float dx = sin_f32(angle + PI * 0.5f), dy = sin_f32(angle);
  float4 sum = float4(0.0f);
  for (int d = -int(radius); d <= int(radius); ++d) {
    uint sx = uint(clamp(rust_round(float(x) + dx * float(d)), 0.0f,
                         float(width - 1u))),
         sy = uint(clamp(rust_round(float(y) + dy * float(d)), 0.0f,
                         float(height - 1u)));
    Color p = color_from_rgba(input[sy * width + sx]);
    sum += float4(p.r * p.a, p.g * p.a, p.b * p.a, p.a);
  }
  float inverse = sum.w > 0.0f ? 1.0f / sum.w : 0.0f;
  output[i] =
      color_to_rgba(color(sum.x * inverse, sum.y * inverse, sum.z * inverse,
                          sum.w / float(radius * 2u + 1u)));
}

void morphology_horizontal(uint3 id, uint *input, uint width, float *output,
                           uint64_t output_count, uint radius_u, bool erode) {
  uint i = id.x;
  if (i >= output_count)
    return;
  int x = int(i % width);
  uint y = i / width;
  int radius = int(min(radius_u, 100u));
  float alpha = erode ? 1.0f : 0.0f;
  for (int d = -radius; d <= radius; ++d) {
    float sample = color_from_rgba(
                       input[y * width + uint(clamp(x + d, 0, int(width) - 1))])
                       .a;
    alpha = erode ? min(alpha, sample) : max(alpha, sample);
  }
  output[i] = alpha;
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void erode_horizontal(uint3 id : SV_DispatchThreadID,
                                   uniform uint *input, uniform uint width,
                                   uniform float *output,
                                   uniform uint64_t output_count,
                                   uniform uint radius) {
  morphology_horizontal(id, input, width, output, output_count, radius, true);
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void dilate_horizontal(uint3 id : SV_DispatchThreadID,
                                     uniform uint *input, uniform uint width,
                                     uniform float *output,
                                     uniform uint64_t output_count,
                                     uniform uint radius) {
  morphology_horizontal(id, input, width, output, output_count, radius, false);
}
void morphology_vertical(uint3 id, uint *source, float *horizontal, uint width,
                         uint height, uint *output, uint64_t output_count,
                         uint radius_u, bool erode) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % width;
  int y = int(i / width), radius = int(min(radius_u, 100u));
  float alpha = erode ? 1.0f : 0.0f;
  for (int d = -radius; d <= radius; ++d) {
    float sample =
        horizontal[uint(clamp(y + d, 0, int(height) - 1)) * width + x];
    alpha = erode ? min(alpha, sample) : max(alpha, sample);
  }
  Color value = color_from_rgba(source[i]);
  output[i] = color_to_rgba(color(value.r, value.g, value.b, alpha));
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void erode_vertical(uint3 id : SV_DispatchThreadID,
                                 uniform uint *source,
                                 uniform float *horizontal, uniform uint width,
                                 uniform uint height, uniform uint *output,
                                 uniform uint64_t output_count,
                                 uniform uint radius) {
  morphology_vertical(id, source, horizontal, width, height, output,
                      output_count, radius, true);
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void dilate_vertical(uint3 id : SV_DispatchThreadID,
                                   uniform uint *source,
                                   uniform float *horizontal,
                                   uniform uint width, uniform uint height,
                                   uniform uint *output,
                                   uniform uint64_t output_count,
                                   uniform uint radius) {
  morphology_vertical(id, source, horizontal, width, height, output,
                      output_count, radius, false);
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
gaussian_blur_horizontal(uint3 id : SV_DispatchThreadID, uniform uint *input,
                         uniform uint width, uniform uint *output,
                         uniform uint64_t output_count, uniform uint radius) {
  uint i = id.x;
  if (i < output_count)
    output[i] = gaussian_horizontal(input, i, width, radius);
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void gaussian_blur_vertical(uint3 id : SV_DispatchThreadID,
                                          uniform uint *input,
                                          uniform uint *original,
                                          uniform uint width,
                                          uniform uint height,
                                          uniform uint *output,
                                          uniform uint64_t output_count,
                                          uniform uint radius,
                                          uniform bool blur_rgb,
                                          uniform bool blur_alpha) {
  uint i = id.x;
  if (i >= output_count)
    return;
  Color blurred =
            color_from_rgba(gaussian_vertical(input, i, width, height, radius)),
        source = color_from_rgba(original[i]);
  output[i] = color_to_rgba(color(
      blur_rgb ? blurred.r : source.r, blur_rgb ? blurred.g : source.g,
      blur_rgb ? blurred.b : source.b, blur_alpha ? blurred.a : source.a));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void glow_bloom_horizontal(uint3 id : SV_DispatchThreadID,
                                        uniform uint *input, uniform uint width,
                                        uniform Color *output,
                                        uniform uint64_t output_count,
                                        uniform float threshold,
                                        uniform uint radius_u) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % width, row = i - x;
  int radius = int(radius_u);
  Color glow = color(0.0f, 0.0f, 0.0f, 0.0f);
  for (int dx = -radius; dx <= radius; ++dx) {
    uint sx = uint(clamp(int(x) + dx, 0, int(width) - 1));
    Color p = color_from_rgba(input[row + sx]);
    if ((p.r + p.g + p.b) / 3.0f * p.a >= threshold) {
      glow.r += p.r * p.a;
      glow.g += p.g * p.a;
      glow.b += p.b * p.a;
      glow.a += p.a;
    }
  }
  float scale = 1.0f / float(radius * 2 + 1);
  output[i] =
      color(glow.r * scale, glow.g * scale, glow.b * scale, glow.a * scale);
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void glow_bloom_vertical(uint3 id : SV_DispatchThreadID,
                                       uniform uint *input,
                                       uniform Color *horizontal,
                                       uniform uint width, uniform uint height,
                                       uniform uint *output,
                                       uniform uint64_t output_count,
                                       uniform uint radius_u,
                                       uniform float intensity) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % width, y = i / width;
  int radius = int(radius_u);
  Color glow = color(0.0f, 0.0f, 0.0f, 0.0f);
  for (int dy = -radius; dy <= radius; ++dy) {
    Color sample =
        horizontal[uint(clamp(int(y) + dy, 0, int(height) - 1)) * width + x];
    glow.r += sample.r;
    glow.g += sample.g;
    glow.b += sample.b;
    glow.a += sample.a;
  }
  float scale = intensity / float(radius * 2 + 1);
  Color base = color_from_rgba(input[i]);
  float ga = clamp(glow.a * scale, 0.0f, 1.0f),
        alpha = base.a + ga * (1.0f - base.a),
        inverse = alpha > 0.0f ? 1.0f / alpha : 0.0f;
  output[i] =
      color_to_rgba(color((base.r * base.a + glow.r * scale) * inverse,
                          (base.g * base.a + glow.g * scale) * inverse,
                          (base.b * base.a + glow.b * scale) * inverse, alpha));
}

void accumulate(inout float4 value, uint pixel) {
  Color c = color_from_rgba(pixel);
  value += float4(c.r, c.g, c.b, rec709_luma(c) * rec709_luma(c));
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void kuwahara_horizontal_statistics(
    uint3 id : SV_DispatchThreadID, uniform uint *input, uniform uint width,
    uniform float *output, uniform uint64_t output_count, uniform uint radius) {
  uint i = id.x;
  if (i >= output_count)
    return;
  int x = int(i % width);
  uint row = i / width;
  float4 left = float4(0.0f), right = float4(0.0f);
  for (int d = 0; d <= int(radius); ++d) {
    accumulate(left,
               input[row * width + uint(clamp(x - d, 0, int(width) - 1))]);
    accumulate(right,
               input[row * width + uint(clamp(x + d, 0, int(width) - 1))]);
  }
  uint offset = i * 8u;
  output[offset] = left.x;
  output[offset + 1u] = left.y;
  output[offset + 2u] = left.z;
  output[offset + 3u] = left.w;
  output[offset + 4u] = right.x;
  output[offset + 5u] = right.y;
  output[offset + 6u] = right.z;
  output[offset + 7u] = right.w;
} float region_variance(float4 region, float area) {
  float3 rgb = region.xyz / area;
  float mean = 0.2126f * rgb.x + 0.7152f * rgb.y + 0.0722f * rgb.z;
  return max(region.w / area - mean * mean, 0.0f);
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
kuwahara_vertical(uint3 id : SV_DispatchThreadID, uniform uint *input,
                  uniform float *statistics, uniform uint width,
                  uniform uint height, uniform uint *output,
                  uniform uint64_t output_count, uniform uint radius,
                  uniform bool generalized) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % width;
  int y = int(i / width);
  float4 regions[4] = {float4(0.0f), float4(0.0f), float4(0.0f), float4(0.0f)};
  for (int d = 0; d <= int(radius); ++d) {
    uint top = uint(clamp(y - d, 0, int(height) - 1)),
         bottom = uint(clamp(y + d, 0, int(height) - 1)),
         to = (top * width + x) * 8u, bo = (bottom * width + x) * 8u;
    regions[0] += float4(statistics[to], statistics[to + 1u],
                         statistics[to + 2u], statistics[to + 3u]);
    regions[1] += float4(statistics[to + 4u], statistics[to + 5u],
                         statistics[to + 6u], statistics[to + 7u]);
    regions[2] += float4(statistics[bo], statistics[bo + 1u],
                         statistics[bo + 2u], statistics[bo + 3u]);
    regions[3] += float4(statistics[bo + 4u], statistics[bo + 5u],
                         statistics[bo + 6u], statistics[bo + 7u]);
  }
  float area = float(radius + 1u) * float(radius + 1u),
        alpha = color_from_rgba(input[i]).a;
  float3 rgb = float3(0.0f);
  if (generalized) {
    float total = 0.0f;
    for (uint r = 0u; r < 4u; ++r) {
      float inverse = 1.0f / (region_variance(regions[r], area) + 0.0001f),
            weight = inverse * inverse;
      rgb += regions[r].xyz / area * weight;
      total += weight;
    }
    rgb /= total;
  } else {
    uint best = 0u;
    float variance = region_variance(regions[0], area);
    for (uint r = 1u; r < 4u; ++r) {
      float candidate = region_variance(regions[r], area);
      if (candidate < variance) {
        best = r;
        variance = candidate;
      }
    }
    rgb = regions[best].xyz / area;
  }
  output[i] = color_to_rgba(color(rgb.x, rgb.y, rgb.z, alpha));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void radial_blur(uint3 id : SV_DispatchThreadID,
                                  uniform uint *input, uniform uint width,
                                  uniform uint height, uniform uint *output,
                                  uniform uint64_t output_count,
                                  uniform RadialBlurParams params) {
  uint i = id.x;
  if (i >= output_count)
    return;
  float x = float(i % width), y = float(i / width),
        cx = params.center.x * float(width - 1u),
        cy = params.center.y * float(height - 1u), dx = x - cx, dy = y - cy;
  uint count = clamp(params.samples, 1u, 128u);
  float4 sum = float4(0.0f);
  for (uint sample = 0u; sample < count; ++sample) {
    float fraction = count == 1u ? 0.5f : float(sample) / float(count - 1u),
          angle = params.angle * (fraction - 0.5f), sine = sin_f32(angle),
          cosine = sin_f32(angle + PI * 0.5f);
    Color c =
        sample_premultiplied(input, width, height, cx + dx * cosine - dy * sine,
                             cy + dx * sine + dy * cosine);
    sum += float4(c.r, c.g, c.b, c.a);
  }
  float alpha = sum.w / float(count), divisor = max(sum.w, 0.00001f);
  output[i] = color_to_rgba(
      color(sum.x / divisor, sum.y / divisor, sum.z / divisor, alpha));
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void zoom_blur(uint3 id : SV_DispatchThreadID, uniform uint *input,
                            uniform uint width, uniform uint height,
                            uniform uint *output, uniform uint64_t output_count,
                            uniform ZoomBlurParams params) {
  uint i = id.x;
  if (i >= output_count)
    return;
  float x = float(i % width), y = float(i / width),
        cx = params.center.x * float(width - 1u),
        cy = params.center.y * float(height - 1u);
  uint count = clamp(params.samples, 1u, 128u);
  float4 sum = float4(0.0f);
  for (uint sample = 0u; sample < count; ++sample) {
    float fraction = count == 1u ? 0.0f : float(sample) / float(count - 1u),
          scale = params.strength * fraction;
    Color c = sample_premultiplied(input, width, height, x + (cx - x) * scale,
                                   y + (cy - y) * scale);
    sum += float4(c.r, c.g, c.b, c.a);
  }
  float alpha = sum.w / float(count), divisor = max(sum.w, 0.00001f);
  output[i] = color_to_rgba(
      color(sum.x / divisor, sum.y / divisor, sum.z / divisor, alpha));
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void sharpen_blur_horizontal(uint3 id : SV_DispatchThreadID,
                                           uniform uint *input,
                                           uniform uint width,
                                           uniform uint *output,
                                           uniform uint64_t output_count,
                                           uniform uint radius) {
  uint i = id.x;
  if (i < output_count)
    output[i] = gaussian_horizontal(input, i, width, radius);
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void sharpen_blur_vertical(uint3 id : SV_DispatchThreadID,
                                        uniform uint *input, uniform uint width,
                                        uniform uint height,
                                        uniform uint *output,
                                        uniform uint64_t output_count,
                                        uniform uint radius) {
  uint i = id.x;
  if (i < output_count)
    output[i] = gaussian_vertical(input, i, width, height, radius);
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void unsharp_mask(uint3 id : SV_DispatchThreadID,
                                uniform uint *original, uniform uint *blurred,
                                uniform uint *output,
                                uniform uint64_t output_count,
                                uniform float amount) {
  uint i = id.x;
  if (i >= output_count)
    return;
  Color a = color_from_rgba(original[i]), b = color_from_rgba(blurred[i]);
  output[i] = color_to_rgba(color(a.r + (a.r - b.r) * amount,
                                  a.g + (a.g - b.g) * amount,
                                  a.b + (a.b - b.b) * amount, a.a));
}
