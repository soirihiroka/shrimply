// Source-derived OpenCV 4.13.0 modules/video/src/lkpyramid.cpp:189-760.
// Copyright Intel Corporation 2000-2008, Willow Garage 2009,
// OpenCV Foundation 2013. See ../LICENSE for the complete BSD notice.
// One invocation owns a feature through every pyramid level. Image pyramids
// and Scharr derivatives come from the same Rust preparation as the CPU path.
import abi;

// Vulkan preserves precise as NoContraction. Slang ignores it on CUDA/Metal;
// their tracking module is compiled with NVCC --fmad=false / Metal safe math.
#if defined(SHRIMPLY_VULKAN)
#define VISION_PRECISE precise
#else
#define VISION_PRECISE
#endif

static const int VISION_WEIGHT_BITS = 14;
static const int VISION_INTENSITY_BITS = 5;
static const float VISION_COVARIANCE_SCALE = 1.0f / float(1 << 20);
static const float VISION_FLOAT_EPSILON = 1.1920928955078125e-7f;

int vision_reflect(int p, uint length) {
  if (length == 1) return 0;
  while (p < 0 || p >= int(length))
    p = p < 0 ? -p : 2 * int(length) - p - 2;
  return p;
}

int vision_pixel(uint *image, uint4 level, int2 p) {
  return int(image[level.z + uint(vision_reflect(p.y, level.y)) * level.x +
                   uint(vision_reflect(p.x, level.x))]);
}

int2 vision_derivative(int2 *derivatives, uint4 level, int2 p) {
  if (any(p < 0) || p.x >= int(level.x) || p.y >= int(level.y)) return int2(0);
  return derivatives[level.z + uint(p.y) * level.x + uint(p.x)];
}

// Slang's CUDA target lowers round() to roundf (ties away from zero),
// whereas cvRound's nearest-even convention is required for interpolation.
int vision_round_even(float value) {
  int lower = int(floor(value));
  float fraction = value - float(lower);
  return lower + int(fraction > 0.5f || (fraction == 0.5f && (lower & 1) != 0));
}

int4 vision_weights(float2 point, out int2 position) {
  position = int2(floor(point));
  VISION_PRECISE float2 amount = point - float2(position);
  VISION_PRECISE float scale = float(1 << VISION_WEIGHT_BITS);
  int w0 = vision_round_even((1.0f - amount.x) * (1.0f - amount.y) * scale);
  int w1 = vision_round_even(amount.x * (1.0f - amount.y) * scale);
  int w2 = vision_round_even((1.0f - amount.x) * amount.y * scale);
  return int4(w0, w1, w2, (1 << VISION_WEIGHT_BITS) - w0 - w1 - w2);
}

int vision_interpolate(uint *image, uint4 level, int2 p, int4 weights) {
  int value = vision_pixel(image, level, p) * weights.x +
              vision_pixel(image, level, p + int2(1, 0)) * weights.y +
              vision_pixel(image, level, p + int2(0, 1)) * weights.z +
              vision_pixel(image, level, p + int2(1, 1)) * weights.w;
  return (value + (1 << (VISION_WEIGHT_BITS - VISION_INTENSITY_BITS - 1))) >>
         (VISION_WEIGHT_BITS - VISION_INTENSITY_BITS);
}

bool vision_in_bounds(uint4 level, int2 point, uint window) {
  return all(point >= -int(window)) && point.x < int(level.x) && point.y < int(level.y);
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void vision_sparse_lk(uint3 id : SV_DispatchThreadID,
                      uniform uint *previous, uniform uint *current,
                      uniform int2 *derivatives, uniform uint4 *levels,
                      uniform float2 *points, uniform float4 *output,
                      uniform int *patch, uniform int2 *gradients,
                      uniform uint count, uniform uint level_count,
                      uniform uint window, uniform uint iterations,
                      uniform float epsilon_squared, uniform float minimum_eigenvalue) {
  uint index = id.x;
  if (index >= count) return;
  uint area = window * window;
  uint scratch = index * area;
  float half_window = float(window - 1) * 0.5f;
  float2 original = points[index];
  VISION_PRECISE float2 tracked = original;
  bool valid = true;
  float error = 0;
  for (int level_index = int(level_count) - 1; level_index >= 0; --level_index) {
    uint4 level = levels[level_index];
    float scale = 1.0f / float(1u << uint(level_index));
    VISION_PRECISE float2 old_point = original * scale;
    tracked = uint(level_index) == level_count - 1 ? old_point : tracked * 2.0f;
    old_point -= half_window;
    int2 old_position;
    int4 old_weights = vision_weights(old_point, old_position);
    if (!vision_in_bounds(level, old_position, window)) {
      if (level_index == 0) { valid = false; error = 0; }
      continue;
    }
    VISION_PRECISE float a11 = 0, a12 = 0, a22 = 0;
    for (uint y = 0; y < window; ++y) {
      for (uint x = 0; x < window; ++x) {
        uint pixel_index = scratch + y * window + x;
        int2 p = old_position + int2(x, y);
        patch[pixel_index] = vision_interpolate(previous, level, p, old_weights);
        int2 gradient = (vision_derivative(derivatives, level, p) * old_weights.x +
                         vision_derivative(derivatives, level, p + int2(1, 0)) * old_weights.y +
                         vision_derivative(derivatives, level, p + int2(0, 1)) * old_weights.z +
                         vision_derivative(derivatives, level, p + int2(1, 1)) * old_weights.w +
                         (1 << (VISION_WEIGHT_BITS - 1))) >> VISION_WEIGHT_BITS;
        gradients[pixel_index] = gradient;
        a11 += float(gradient.x * gradient.x);
        a12 += float(gradient.x * gradient.y);
        a22 += float(gradient.y * gradient.y);
      }
    }
    a11 *= VISION_COVARIANCE_SCALE;
    a12 *= VISION_COVARIANCE_SCALE;
    a22 *= VISION_COVARIANCE_SCALE;
    VISION_PRECISE float determinant = a11 * a22 - a12 * a12;
    VISION_PRECISE float minimum = (a22 + a11 - sqrt((a11 - a22) * (a11 - a22) + 4.0f * a12 * a12)) /
                            float(2 * area);
    if (minimum < minimum_eigenvalue || determinant < VISION_FLOAT_EPSILON) {
      if (level_index == 0) valid = false;
      continue;
    }
    float inverse = 1.0f / determinant;
    VISION_PRECISE float2 next_point = tracked - half_window;
    float2 previous_delta = 0;
    for (uint iteration = 0; iteration < iterations; ++iteration) {
      int2 next_position;
      int4 next_weights = vision_weights(next_point, next_position);
      if (!vision_in_bounds(level, next_position, window)) {
        if (level_index == 0) valid = false;
        break;
      }
      VISION_PRECISE float b1 = 0, b2 = 0;
      for (uint y = 0; y < window; ++y) {
        for (uint x = 0; x < window; ++x) {
          uint pixel_index = scratch + y * window + x;
          int difference = vision_interpolate(current, level, next_position + int2(x, y), next_weights) - patch[pixel_index];
          int2 gradient = gradients[pixel_index];
          b1 += float(difference * gradient.x);
          b2 += float(difference * gradient.y);
        }
      }
      b1 *= VISION_COVARIANCE_SCALE;
      b2 *= VISION_COVARIANCE_SCALE;
      VISION_PRECISE float2 delta = float2((a12 * b2 - a22 * b1) * inverse,
                                    (a12 * b1 - a11 * b2) * inverse);
      next_point += delta;
      tracked = next_point + half_window;
      if (dot(delta, delta) <= epsilon_squared) break;
      if (iteration > 0 && all(abs(delta + previous_delta) < 0.01f)) {
        tracked -= delta * 0.5f;
        break;
      }
      previous_delta = delta;
    }
    if (valid && level_index == 0) {
      int2 next_position;
      int4 next_weights = vision_weights(tracked - half_window, next_position);
      if (!vision_in_bounds(level, next_position, window)) { valid = false; continue; }
      error = 0;
      for (uint y = 0; y < window; ++y) {
        for (uint x = 0; x < window; ++x) {
          uint pixel_index = scratch + y * window + x;
          error += float(abs(vision_interpolate(current, level, next_position + int2(x, y), next_weights) - patch[pixel_index]));
        }
      }
      error /= float((1 << VISION_INTENSITY_BITS) * area);
    }
  }
  output[index] = float4(tracked, error, valid ? 1.0f : 0.0f);
}

#undef VISION_PRECISE
