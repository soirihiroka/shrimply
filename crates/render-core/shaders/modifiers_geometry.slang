import abi;
import color;
import common;
import math;

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
bulge_pinch(uniform uint *input, uniform uint w, uniform uint h,
            uniform uint *out, uniform uint64_t out_len,
            uniform BulgePinchParams params, uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  float x = float(i % w), y = float(i / w),
        px = params.center.x * float(w - 1u),
        py = params.center.y * float(h - 1u), dx = x - px, dy = y - py,
        d = sqrt(dx * dx + dy * dy);
  if (d >= params.radius || d == 0.0f || params.radius <= 0.0f) {
    out[i] = input[i];
    return;
  }
  float t = d / params.radius;
  float scale = 1.0f - params.strength * (1.0f - t) * (1.0f - t);
  uint sx = uint(clamp(rust_round(px + dx * scale), 0.0f, float(w - 1u))),
       sy = uint(clamp(rust_round(py + dy * scale), 0.0f, float(h - 1u)));
  out[i] = input[sy * w + sx];
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void displacement_map(uniform uint *input, uniform uint w,
                                       uniform uint h, uniform uint *out,
                                       uniform uint64_t out_len,
                                       uniform float amount,
                                       uniform float scale, uniform float phase,
                                       uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  float x = float(i % w), y = float(i / w), frequency = (2.0f * PI) / scale,
        dx = amount * sin_f32(y * frequency + phase),
        dy = amount * sin_f32(x * frequency + phase * 1.37f);
  uint sx = uint(clamp(rust_round(x + dx), 0.0f, float(w - 1u))),
       sy = uint(clamp(rust_round(y + dy), 0.0f, float(h - 1u)));
  out[i] = input[sy * w + sx];
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
fisheye(uniform uint *input, uniform uint width, uniform uint height,
        uniform uint *out, uniform uint64_t out_len, uniform float intensity,
        uniform Vec2 center_value, uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  float x = float(i % width), y = float(i / width);
  float2 size = max(float2(float(width), float(height)), float2(1.0f));
  float2 center = clamp(vec2_value(center_value), float2(0.0f), float2(1.0f)) *
                  (size - float2(1.0f));
  float2 offset = (float2(x, y) - center) / size;
  float scale = 1.0f - clamp(intensity, -1.0f, 1.0f) *
                           max(1.0f - dot(offset, offset) * 4.0f, 0.0f);
  float2 source = center + offset * scale * size;
  uint sx = uint(clamp(rust_round(source.x), 0.0f,
                       float(width > 0u ? width - 1u : 0u))),
       sy = uint(clamp(rust_round(source.y), 0.0f,
                       float(height > 0u ? height - 1u : 0u)));
  out[i] = input[sy * width + sx];
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void kaleidoscope(uniform uint *input, uniform uint w,
                                   uniform uint h, uniform uint *out,
                                   uniform uint64_t out_len,
                                   uniform KaleidoscopeParams params,
                                   uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  float x = float(i % w), y = float(i / w),
        px = params.center.x * float(w - 1u),
        py = params.center.y * float(h - 1u), dx = x - px, dy = y - py,
        d = sqrt(dx * dx + dy * dy),
        slice = 2.0f * PI / float(max(params.segments, 2u)),
        a = atan2_f32(dy, dx) - params.rotation;
  a = a - floor_f32(a / slice) * slice;
  if (a > slice * 0.5f)
    a = slice - a;
  a += params.rotation;
  uint sx = uint(clamp(rust_round(px + sin_f32(a + PI * 0.5f) * d), 0.0f,
                       float(w - 1u))),
       sy = uint(clamp(rust_round(py + sin_f32(a) * d), 0.0f, float(h - 1u)));
  out[i] = input[sy * w + sx];
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void lens_distortion(uniform uint *input, uniform uint w,
                                  uniform uint h, uniform uint *out,
                                  uniform uint64_t out_len,
                                  uniform float distortion, uniform Vec2 center,
                                  uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  float x = float(i % w), y = float(i / w), px = center.x * float(w - 1u),
        py = center.y * float(h - 1u), nx = (x - px) / float(w),
        ny = (y - py) / float(h),
        scale = 1.0f + distortion * (nx * nx + ny * ny) * 4.0f;
  int sx = int(rust_round(px + nx * scale * float(w))),
      sy = int(rust_round(py + ny * scale * float(h)));
  out[i] = sx >= 0 && sy >= 0 && sx < int(w) && sy < int(h)
               ? input[uint(sy) * w + uint(sx)]
               : 0u;
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void mirror(uniform uint *input, uniform uint w,
                             uniform uint h, uniform uint *out,
                             uniform uint64_t out_len, uniform uint horizontal,
                             uniform uint vertical,
                             uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  uint x = i % w, y = i / w, sx = horizontal != 0u ? w - 1u - x : x,
       sy = vertical != 0u ? h - 1u - y : y;
  out[i] = input[sy * w + sx];
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
pixelate_mosaic(uniform uint *input, uniform uint width, uniform uint height,
                uniform uint *out, uniform uint64_t out_len,
                uniform uint block_width, uniform uint block_height,
                uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  uint x = i % width, y = i / width, bw = max(block_width, 1u),
       bh = max(block_height, 1u), sx = min(x / bw * bw + bw / 2u, width - 1u),
       sy = min(y / bh * bh + bh / 2u, height - 1u);
  out[i] = input[sy * width + sx];
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void twirl(uniform uint *input, uniform uint w, uniform uint h,
                            uniform uint *out, uniform uint64_t out_len,
                            uniform TwirlParams params,
                            uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  float x = float(i % w), y = float(i / w),
        px = params.center.x * float(w - 1u),
        py = params.center.y * float(h - 1u), dx = x - px, dy = y - py,
        d = sqrt(dx * dx + dy * dy);
  if (d >= params.radius || params.radius <= 0.0f) {
    out[i] = input[i];
    return;
  }
  float a = atan2_f32(dy, dx) + params.angle * (1.0f - d / params.radius);
  uint sx = uint(clamp(rust_round(px + sin_f32(a + PI * 0.5f) * d), 0.0f,
                       float(w - 1u))),
       sy = uint(clamp(rust_round(py + sin_f32(a) * d), 0.0f, float(h - 1u)));
  out[i] = input[sy * w + sx];
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
wave_ripple(uniform uint *input, uniform uint w, uniform uint h,
            uniform uint *out, uniform uint64_t out_len,
            uniform WaveRippleParams params, uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  float x = float(i % w), y = float(i / w),
        c = sin_f32(params.angle + PI * 0.5f), s = sin_f32(params.angle),
        displacement =
            params.amplitude *
            sin_f32((x * c + y * s) / params.wavelength * (2.0f * PI) +
                    params.phase);
  uint sx = uint(clamp(rust_round(x - s * displacement), 0.0f, float(w - 1u))),
       sy = uint(clamp(rust_round(y + c * displacement), 0.0f, float(h - 1u)));
  out[i] = input[sy * w + sx];
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void corner_pin(uniform CornerPinParams params,
                                 uniform uint *out, uniform uint64_t out_len,
                                 uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  float x = float(i % params.width), y = float(i / params.width);
  float2 point =
      float2(x / float(max(params.width > 0u ? params.width - 1u : 0u, 1u)),
             y / float(max(params.height > 0u ? params.height - 1u : 0u, 1u)));
  float2 projective;
  if (!projective_point(params.inverse_homography, point, projective)) {
    out[i] = 0u;
    return;
  }
  if (projective.x < -0.00001f || projective.x > 1.00001f ||
      projective.y < -0.00001f || projective.y > 1.00001f) {
    out[i] = 0u;
    return;
  }
  projective = clamp(projective, float2(0.0f), float2(1.0f));
  float2 source = projective;
  if (params.perspective < 1.0f) {
    float2 bilinear;
    if (!inverse_bilinear_quad(params.corners, point, projective, bilinear)) {
      out[i] = 0u;
      return;
    }
    source =
        clamp(lerp(bilinear, projective, clamp(params.perspective, 0.0f, 1.0f)),
              float2(0.0f), float2(1.0f));
  }
  uint *input = (uint *)params.input;
  out[i] = sample_bilinear_rgba(
      input, params.width, params.height,
      source.x * float(params.width > 0u ? params.width - 1u : 0u),
      source.y * float(params.height > 0u ? params.height - 1u : 0u));
}
