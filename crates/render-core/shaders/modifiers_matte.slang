import abi;
import color;
import common;
import math;

float closest_segment_distance(float2 point, float2 start, float2 end) {
  float2 segment = end - start;
  float length_squared = dot(segment, segment);
  float2 nearest = length_squared <= F32_EPSILON
                       ? start
                       : start + segment * clamp(dot(point - start, segment) /
                                                     length_squared,
                                                 0.0f, 1.0f);
  return length(point - nearest);
}

float polygon_signed_distance(float2 point, Vec2 *vertices, uint count) {
  if (vertices == nullptr || count < 3u)
    return 3.402823466e38f;
  bool inside = false;
  float distance_value = 3.402823466e38f;
  float2 previous = vec2_value(vertices[count - 1u]);
  for (uint i = 0u; i < count; ++i) {
    float2 current = vec2_value(vertices[i]);
    distance_value =
        min(distance_value, closest_segment_distance(point, previous, current));
    if ((previous.y > point.y) != (current.y > point.y) &&
        point.x < (current.x - previous.x) * (point.y - previous.y) /
                          (current.y - previous.y) +
                      previous.x)
      inside = !inside;
    previous = current;
  }
  return inside ? -distance_value : distance_value;
}

float shape_alpha_mask_amount(float2 point, ShapeAlphaMaskParams params) {
  float2 center = vec2_value(params.center), size = vec2_value(params.size),
         half_size = max(size, float2(0.0f)) * 0.5f;
  float amount = 0.0f;
  if (min(half_size.x, half_size.y) > F32_EPSILON) {
    point = rotate_degrees(point - center, -params.rotation_degrees);
    float distance_value;
    if (params.shape == ShapeAlphaMaskKind::Rectangle) {
      float radius =
          min(half_size.x, half_size.y) * clamp(params.rounding, 0.0f, 1.0f);
      float2 edge = abs(point) - half_size + radius;
      distance_value = length(max(edge, float2(0.0f))) +
                       min(max(edge.x, edge.y), 0.0f) - radius;
    } else if (params.shape == ShapeAlphaMaskKind::Ellipse)
      distance_value =
          (length(point / half_size) - 1.0f) * min(half_size.x, half_size.y);
    else
      distance_value = polygon_signed_distance(point, (Vec2 *)params.vertices,
                                               params.vertex_count);
    float softness = min(max(size.x, 0.0f), max(size.y, 0.0f)) *
                     clamp(params.feather, 0.0f, 1.0f) * 0.5f;
    if (softness <= F32_EPSILON)
      amount = distance_value <= 0.0f ? 1.0f : 0.0f;
    else {
      float value = clamp(0.5f - distance_value / softness, 0.0f, 1.0f);
      amount = value * value * (3.0f - 2.0f * value);
    }
  }
  return params.invert != 0u ? 1.0f - amount : amount;
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
alpha_mask(uint3 id : SV_DispatchThreadID, uniform uint *input,
           uniform AlphaMaskParams params, uniform uint *output,
           uniform uint64_t output_count) {
  uint i = id.x;
  if (i >= output_count)
    return;
  Color source = color_from_rgba(input[i]);
  uint *mask_pixels = (uint *)params.mask;
  if (mask_pixels == nullptr) {
    output[i] = color_to_rgba(color(source.r, source.g, source.b, 0.0f));
    return;
  }
  uint x = i % params.input_width, y = i / params.input_width;
  uint mask_x = uint((float(x) + 0.5f) * float(params.mask_width) /
                     float(params.input_width));
  uint mask_y = uint((float(y) + 0.5f) * float(params.mask_height) /
                     float(params.input_height));
  Color mask = color_from_rgba(
      mask_pixels[min(mask_y, params.mask_height - 1u) * params.mask_width +
                  min(mask_x, params.mask_width - 1u)]);
  output[i] = color_to_rgba(color(source.r, source.g, source.b,
                                  source.a * rec709_luma(mask) * mask.a));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void shape_alpha_mask(uint3 id : SV_DispatchThreadID,
                                       uniform uint *input,
                                       uniform ShapeAlphaMaskParams params,
                                       uniform uint *output,
                                       uniform uint64_t output_count) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % params.input_width, y = i / params.input_width;
  float2 local = transform_point2(params.canvas_to_local,
                                  float2(float(x) + 0.5f, float(y) + 0.5f));
  float amount =
      shape_alpha_mask_amount(local / float2(max(params.local_width, 1.0f),
                                             max(params.local_height, 1.0f)),
                              params);
  Color source = color_from_rgba(input[i]);
  uint *base = (uint *)params.base;
  if (base == nullptr) {
    output[i] =
        color_to_rgba(color(source.r, source.g, source.b, source.a * amount));
    return;
  }
  Color background = color_from_rgba(base[i]);
  float alpha = background.a + (source.a - background.a) * amount;
  if (alpha <= F32_EPSILON) {
    output[i] = 0u;
    return;
  }
  float keep = 1.0f - amount;
  output[i] = color_to_rgba(color(
      (background.r * background.a * keep + source.r * source.a * amount) /
          alpha,
      (background.g * background.a * keep + source.g * source.a * amount) /
          alpha,
      (background.b * background.a * keep + source.b * source.a * amount) /
          alpha,
      alpha));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void chroma_key(uint3 id : SV_DispatchThreadID,
                             uniform uint *input, uniform uint *output,
                             uniform uint64_t output_count,
                             uniform ChromaKeyParams params) {
  uint i = id.x;
  if (i >= output_count)
    return;
  Color value = color_from_rgba(input[i]);
  float3 delta = float3(value.r - params.key.r, value.g - params.key.g,
                        value.b - params.key.b);
  float keep = smoothstep_exact(
      params.similarity, params.similarity + max(params.softness, 0.00001f),
      length(delta) / 1.7320508f);
  float key_max = max(params.key.r, max(params.key.g, params.key.b)),
        key_min = min(params.key.r, min(params.key.g, params.key.b)),
        chroma = max(key_max - key_min, 0.00001f);
  float3 key = float3((params.key.r - key_min) / chroma,
                      (params.key.g - key_min) / chroma,
                      (params.key.b - key_min) / chroma);
  float projection = dot(float3(value.r, value.g, value.b), key) /
                     max(dot(key, key), 0.00001f);
  float removal =
      max(projection, 0.0f) * clamp(params.spill, 0.0f, 1.0f) * (1.0f - keep);
  float3 rgb =
      max(float3(value.r, value.g, value.b) - key * removal, float3(0.0f));
  output[i] = color_to_rgba(color(rgb.x, rgb.y, rgb.z, value.a * keep));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void luma_key(uint3 id : SV_DispatchThreadID,
                               uniform uint *input, uniform uint *output,
                               uniform uint64_t output_count,
                               uniform float threshold, uniform float softness,
                               uniform bool invert) {
  uint i = id.x;
  if (i >= output_count)
    return;
  Color value = color_from_rgba(input[i]);
  float half_softness = max(softness, 0.00001f) * 0.5f;
  float keep = smoothstep_exact(threshold - half_softness,
                                threshold + half_softness, rec709_luma(value));
  if (invert)
    keep = 1.0f - keep;
  output[i] = color_to_rgba(color_with_alpha(value, value.a * keep));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void mask(uint3 id : SV_DispatchThreadID, uniform uint *input,
                       uniform MaskParams params, uniform uint *output,
                       uniform uint64_t output_count) {
  uint i = id.x;
  if (i >= output_count)
    return;
  Color source = color_from_rgba(input[i]);
  uint *pixels = (uint *)params.mask;
  float amount = 0.0f;
  if (pixels != nullptr) {
    uint x = i % params.input_width, y = i / params.input_width;
    float2 canvas = transform_point2(params.transform,
                                     float2(float(x) + 0.5f, float(y) + 0.5f));
    if (canvas.x < 0.0f || canvas.y < 0.0f ||
        canvas.x >= float(params.mask_width) ||
        canvas.y >= float(params.mask_height)) {
      output[i] = color_to_rgba(
          color(source.r, source.g, source.b, params.invert ? source.a : 0.0f));
      return;
    }
    Color mask_value = color_from_rgba(
        pixels[uint(canvas.y) * params.mask_width + uint(canvas.x)]);
    amount = params.luminance ? rec709_luma(mask_value) * mask_value.a
                              : mask_value.a;
  }
  if (params.invert)
    amount = 1.0f - amount;
  output[i] =
      color_to_rgba(color(source.r, source.g, source.b, source.a * amount));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void sam2_proxy(uint3 id : SV_DispatchThreadID,
                                 uniform uint *input, uniform uint *output,
                                 uniform Sam2ProxyParams params) {
  uint i = id.x, pixels = params.model_size * params.model_size;
  if (i >= pixels || params.input_width == 0u || params.input_height == 0u)
    return;
  uint x = i % params.model_size, y = i / params.model_size;
  float sx = clamp((float(x) + 0.5f) * float(params.input_width) /
                           float(params.model_size) -
                       0.5f,
                   0.0f, float(params.input_width - 1u));
  float sy = clamp((float(y) + 0.5f) * float(params.input_height) /
                           float(params.model_size) -
                       0.5f,
                   0.0f, float(params.input_height - 1u));
  Color value = color_from_rgba(sample_bilinear_rgba(
      input, params.input_width, params.input_height, sx, sy));
  output[i] = color_to_rgba(
      color(value.r * value.a, value.g * value.a, value.b * value.a, 1.0f));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
sam2_apply_mask(uint3 id : SV_DispatchThreadID, uniform uint *input,
                uniform int8_t *masks, uniform uint *output,
                uniform uint64_t output_count, uniform Sam2MaskParams params) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % params.output_width, y = i / params.output_width;
  float mx = clamp((float(x) + 0.5f) * float(params.mask_size) /
                           float(params.output_width) -
                       0.5f,
                   0.0f, float(params.mask_size - 1u)),
        my = clamp((float(y) + 0.5f) * float(params.mask_size) /
                           float(params.output_height) -
                       0.5f,
                   0.0f, float(params.mask_size - 1u));
  uint x0 = uint(floor_f32(mx)), y0 = uint(floor_f32(my)),
       x1 = min(x0 + 1u, params.mask_size - 1u),
       y1 = min(y0 + 1u, params.mask_size - 1u);
  float top =
      lerp(float(masks[y0 * params.mask_size + x0]) / params.quantization_scale,
           float(masks[y0 * params.mask_size + x1]) / params.quantization_scale,
           mx - float(x0));
  float bottom =
      lerp(float(masks[y1 * params.mask_size + x0]) / params.quantization_scale,
           float(masks[y1 * params.mask_size + x1]) / params.quantization_scale,
           mx - float(x0));
  float logit = lerp(top, bottom, my - float(y0)),
        half_softness = max(params.softness, 0.0f) * 0.5f;
  float selected =
      half_softness > 0.0f
          ? smoothstep_exact(params.threshold - half_softness,
                             params.threshold + half_softness, logit)
          : (logit > params.threshold ? 1.0f : 0.0f);
  float keep = params.invert ? 1.0f - selected : selected;
  Color value = color_from_rgba(input[i]);
  output[i] = color_to_rgba(color_with_alpha(value, value.a * keep));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void transparent_fill_apply_mask(
        uint3 id : SV_DispatchThreadID, uniform uint *input,
        uniform uint8_t *mask_bits, uniform uint *output,
        uniform uint64_t output_count,
        uniform TransparentFillMaskParams params) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % params.width, y = i / params.width;
  if (y >= params.height)
    return;
  uint8_t byte = mask_bits[y * params.stride + x / 8u];
  output[i] =
      (byte & (uint8_t(0x80u) >> uint8_t(x % 8u))) != 0u ? 0u : input[i];
}

float wipe_alpha(float x, float y, uint width, uint height, float visibility,
                 float angle_degrees, float softness) {
  float radians = angle_degrees * (PI / 180.0f),
        dx = sin_f32(radians + PI * 0.5f), dy = sin_f32(radians);
  float position = ((x / float(max(width, 1u))) - 0.5f) * dx +
                   ((y / float(max(height, 1u))) - 0.5f) * dy;
  position = position / max(abs(dx) + abs(dy), 0.00001f) + 0.5f;
  softness = clamp(softness, 0.0f, 0.5f);
  float edge = visibility * (1.0f + softness * 2.0f) - softness;
  return 1.0f - smoothstep_exact(edge - softness, edge + softness + 0.00001f,
                                 position);
}

float iris_alpha(float x, float y, uint width_u, uint height_u,
                 float visibility, float center_x, float center_y,
                 float softness, bool from_inside) {
  float width = float(max(width_u, 1u)), height = float(max(height_u, 1u));
  center_x = clamp(center_x, 0.0f, width);
  center_y = clamp(center_y, 0.0f, height);
  float maximum = max(length(float2(max(center_x, width - center_x),
                                    max(center_y, height - center_y))),
                      0.00001f);
  float distance_value = length(float2(x - center_x, y - center_y)) / maximum;
  if (!from_inside)
    distance_value = 1.0f - distance_value;
  softness = clamp(softness, 0.0f, 0.5f);
  float radius = visibility * (1.0f + softness * 2.0f) - softness;
  return 1.0f - smoothstep_exact(radius - softness,
                                 radius + softness + 0.00001f, distance_value);
}

float dissolve_alpha(uint x, uint y, float visibility, uint grain_size) {
  uint grain = max(grain_size, 1u),
       hash = (x / grain) * 0x8da6b343u ^ (y / grain) * 0xd8163841u ^ 19u;
  hash ^= hash >> 13;
  hash *= 0x85ebca6bu;
  hash ^= hash >> 16;
  return float(hash & 0x00ffffffu) / float(0x00ffffffu) <= visibility ? 1.0f
                                                                      : 0.0f;
}

float clock_wipe_alpha(float x, float y, float visibility, float start_degrees,
                       float softness, float center_x, float center_y,
                       bool clockwise) {
  float tau = PI * 2.0f, start = start_degrees * (PI / 180.0f),
        angle = fmod(atan2_f32(y - center_y, x - center_x) - start, tau);
  if (angle < 0.0f)
    angle += tau;
  if (!clockwise && angle > 0.0f)
    angle = tau - angle;
  float position = angle / tau;
  softness = clamp(softness, 0.0f, 0.25f);
  float edge = visibility * (1.0f + softness * 2.0f) - softness;
  return 1.0f - smoothstep_exact(edge - softness, edge + softness + 0.00001f,
                                 position);
}

uint triangular_fold_pixel(uint *input, float x, float y, uint width,
                           uint height, float visibility, float angle_degrees,
                           float depth, uint fold_size) {
  float width_f = float(max(width, 1u)), height_f = float(max(height, 1u));
  if (visibility >= 0.9999f)
    return input[uint(y) * width + uint(x)];
  float cell_size = float(clamp(fold_size, 32u, 512u));
  int cell_x = int(floor_f32(x / cell_size)),
      cell_y = int(floor_f32(y / cell_size));
  float origin_x = float(cell_x) * cell_size,
        origin_y = float(cell_y) * cell_size;
  float u = clamp((x - origin_x) / cell_size, 0.0f, 1.0f),
        v = clamp((y - origin_y) / cell_size, 0.0f, 1.0f),
        radians = angle_degrees * (PI / 180.0f),
        dx = sin_f32(radians + PI * 0.5f), dy = sin_f32(radians);
  float sweep = (((origin_x + cell_size * 0.5f) / width_f - 0.5f) * dx +
                 ((origin_y + cell_size * 0.5f) / height_f - 0.5f) * dy) /
                    max(abs(dx) + abs(dy), 0.00001f) +
                0.5f;
  bool backslash = ((cell_x + cell_y) & 1) == 0;
  float diagonal = backslash ? u - v : u + v - 1.0f,
        side_delay = diagonal >= 0.0f ? 0.07f : 0.0f;
  float delay = min(clamp(sweep, 0.0f, 1.0f) * 0.55f +
                        hash_unit(cell_x, cell_y, 29u) * 0.14f + side_delay,
                    0.82f),
        progress = clamp((visibility - delay) / max(1.0f - delay, 0.00001f),
                         0.0f, 1.0f);
  if (progress <= 0.0f)
    return 0u;
  float fold = smoothstep_exact(0.0f, 1.0f, progress),
        distance_value = abs(diagonal);
  if (distance_value > fold)
    return 0u;
  float adjustment = (diagonal / max(fold, 0.0001f) - diagonal) * 0.5f;
  float source_u = u + adjustment,
        source_v = backslash ? v - adjustment : v + adjustment;
  uint source_x = uint(clamp(origin_x + clamp(source_u, 0.0f, 1.0f) * cell_size,
                             0.0f, width_f - 1.0f)),
       source_y = uint(clamp(origin_y + clamp(source_v, 0.0f, 1.0f) * cell_size,
                             0.0f, height_f - 1.0f));
  Color source = color_from_rgba(input[source_y * width + source_x]);
  float fold_height = max(sin_f32(fold * PI), 0.0f),
        side_light = diagonal >= 0.0f ? 1.0f : -0.65f,
        crease = 1.0f - smoothstep_exact(0.0f, 0.045f, distance_value),
        moving =
            smoothstep_exact(max(fold - 0.08f, 0.0f), fold, distance_value);
  depth = clamp(depth, 0.0f, 1.0f);
  float shade = clamp(1.0f + side_light * fold_height * depth * 0.34f +
                          crease * fold_height * depth * 0.22f -
                          moving * fold_height * depth * 0.42f,
                      0.28f, 1.55f),
        coverage = 1.0f - smoothstep_exact(max(fold - 0.025f, 0.0f),
                                           fold + 0.00001f, distance_value);
  return color_to_rgba(color(source.r * shade, source.g * shade,
                             source.b * shade, source.a * coverage));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
visual_transition_mask(uint3 id : SV_DispatchThreadID, uniform uint *input,
                       uniform uint width, uniform uint height,
                       uniform uint *output, uniform uint64_t output_count,
                       uniform VisualTransitionMaskParams params) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint pixel = input[i];
  float x = float(i % width), y = float(i / width),
        visibility = clamp(params.visibility, 0.0f, 1.0f), alpha;
  if (params.kind == VisualTransitionMaskKind::Wipe)
    alpha = wipe_alpha(x, y, width, height, visibility, params.angle_degrees,
                       params.softness);
  else if (params.kind == VisualTransitionMaskKind::Iris)
    alpha =
        iris_alpha(x, y, width, height, visibility, params.center.x,
                   params.center.y, params.softness, params.grain_size != 0u);
  else if (params.kind == VisualTransitionMaskKind::Dissolve)
    alpha = dissolve_alpha(uint(x), uint(y), visibility, params.grain_size);
  else if (params.kind == VisualTransitionMaskKind::ClockWipe)
    alpha = clock_wipe_alpha(x, y, visibility, params.angle_degrees,
                             params.softness, params.center.x, params.center.y,
                             params.grain_size != 0u);
  else if (params.kind == VisualTransitionMaskKind::TriangularFold) {
    output[i] = triangular_fold_pixel(input, x, y, width, height, visibility,
                                      params.angle_degrees, params.softness,
                                      params.grain_size);
    return;
  } else
    alpha = streak_wipe_alpha(
        float2(x + 0.5f, y + 0.5f),
        float2(float(max(width, 1u)), float(max(height, 1u))), visibility,
        params.angle_degrees, float(params.grain_size), params.line_variation,
        params.softness);
  Color value = color_from_rgba(pixel);
  output[i] = color_to_rgba(color_with_alpha(value, value.a * alpha));
}

float4 vertex(float *vertices, uint index) {
  uint offset = index * 3u;
  return float4(vertices[offset], vertices[offset + 1u], vertices[offset + 2u],
                0.0f);
}
float4 triangle_hit(float2 point, float4 a, float4 b, float4 c, float2 uv_a,
                    float2 uv_b, float2 uv_c, float visibility) {
  float denominator = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
  if (abs(denominator) < 0.00001f)
    return float4(-3.402823466e38f, 0.0f, 0.0f, 1.0f);
  float wa = ((b.y - c.y) * (point.x - c.x) + (c.x - b.x) * (point.y - c.y)) /
             denominator,
        wb = ((c.y - a.y) * (point.x - c.x) + (a.x - c.x) * (point.y - c.y)) /
             denominator,
        wc = 1.0f - wa - wb;
  if (wa < -0.0005f || wb < -0.0005f || wc < -0.0005f)
    return float4(-3.402823466e38f, 0.0f, 0.0f, 1.0f);
  float3 ab = b.xyz - a.xyz, ac = c.xyz - a.xyz, normal = cross(ab, ac);
  float normal_length = max(length(normal), 0.00001f),
        light = (normal.x * 0.32f - normal.y * 0.38f + normal.z * 0.87f) /
                normal_length,
        crease = 1.0f - smoothstep_exact(0.0f, 0.035f, min(wa, min(wb, wc))),
        folded = 0.84f + abs(light) * 0.16f;
  folded += (1.0f - folded) * crease * 0.7f;
  if (normal.z < 0.0f)
    folded *= 0.92f;
  float shade = 1.0f - (1.0f - folded) * (1.0f - visibility);
  return float4(
      wa * a.z + wb * b.z + wc * c.z, wa * uv_a.x + wb * uv_b.x + wc * uv_c.x,
      wa * uv_a.y + wb * uv_b.y + wc * uv_c.y, clamp(shade, 0.78f, 1.0f));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void origami_transition(uint3 id : SV_DispatchThreadID,
                                     uniform uint *input, uniform uint width,
                                     uniform uint height, uniform uint *output,
                                     uniform uint64_t output_count,
                                     uniform float *vertices, uniform uint grid,
                                     uniform float visibility) {
  uint i = id.x;
  if (i >= output_count)
    return;
  if (visibility >= 0.9999f) {
    output[i] = input[i];
    return;
  }
  if (visibility <= 0.0f) {
    output[i] = 0u;
    return;
  }
  float2 point = float2(float(i % width) + 0.5f, float(i / width) + 0.5f);
  grid = clamp(grid, 2u, 6u);
  uint stride = grid + 1u;
  float4 best = float4(-3.402823466e38f, 0.0f, 0.0f, 1.0f);
  for (uint row = 0u; row < grid; ++row)
    for (uint column = 0u; column < grid; ++column) {
      uint i00 = row * stride + column, i10 = i00 + 1u, i01 = i00 + stride,
           i11 = i01 + 1u;
      float u0 = float(column) / float(grid),
            u1 = float(column + 1u) / float(grid),
            v0 = float(row) / float(grid), v1 = float(row + 1u) / float(grid);
      float4 hit =
          triangle_hit(point, vertex(vertices, i00), vertex(vertices, i10),
                       vertex(vertices, i11), float2(u0, v0), float2(u1, v0),
                       float2(u1, v1), visibility);
      if (hit.x > best.x)
        best = hit;
      hit = triangle_hit(point, vertex(vertices, i00), vertex(vertices, i11),
                         vertex(vertices, i01), float2(u0, v0), float2(u1, v1),
                         float2(u0, v1), visibility);
      if (hit.x > best.x)
        best = hit;
    }
  if (best.x == -3.402823466e38f) {
    output[i] = 0u;
    return;
  }
  float sx = clamp(best.y, 0.0f, 1.0f) * float(width - 1u),
        sy = clamp(best.z, 0.0f, 1.0f) * float(height - 1u);
  Color source =
      color_from_rgba(sample_bilinear_rgba(input, width, height, sx, sy));
  output[i] = color_to_rgba(
      color(source.r * best.w, source.g * best.w, source.b * best.w, source.a));
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void transition_gaussian_blur_horizontal(
        uint3 id : SV_DispatchThreadID, uniform uint *input, uniform uint width,
        uniform uint *output, uniform uint64_t output_count,
        uniform uint radius_u) {
  uint i = id.x;
  if (i >= output_count)
    return;
  int x = int(i % width);
  uint y = i / width;
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
  output[i] = color_to_rgba(
      color(sum.x / total, sum.y / total, sum.z / total, sum.w / total));
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void transition_gaussian_blur_vertical(
    uint3 id : SV_DispatchThreadID, uniform uint *input, uniform uint width,
    uniform uint height, uniform uint *output, uniform uint64_t output_count,
    uniform uint radius_u) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % width;
  int y = int(i / width), radius = int(min(radius_u, 100u));
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
  output[i] =
      color_to_rgba(color(sum.x / total / divisor, sum.y / total / divisor,
                          sum.z / total / divisor, alpha));
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void transition_pixelate(uint3 id : SV_DispatchThreadID,
                                       uniform uint *input, uniform uint width,
                                       uniform uint height,
                                       uniform uint *output,
                                       uniform uint64_t output_count,
                                       uniform uint block_width,
                                       uniform uint block_height) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % width, y = i / width;
  block_width = max(block_width, 1u);
  block_height = max(block_height, 1u);
  uint sx = min(x / block_width * block_width + block_width / 2u, width - 1u),
       sy = min(y / block_height * block_height + block_height / 2u,
                height - 1u);
  output[i] = input[sy * width + sx];
}
