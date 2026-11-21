// Source-derived OpenCV 5.1.0 modules/video/src/lkpyramid.cpp:109-835.
// Copyright Intel Corporation 2000-2008, Willow Garage 2009,
// OpenCV Foundation 2013. See ../LICENSE for the complete BSD notice.
// One cooperative workgroup owns a feature through every pyramid level. Image pyramids
// and Scharr derivatives use the same integer filters as the CPU reference.
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

// OpenCV pyrDown: Gaussian [1,4,6,4,1], BORDER_REFLECT_101 and
// FixPtCast<int,uchar,8>. Store only the final rounded eight-bit value.
static const int VISION_GAUSSIAN[5] = { 1, 4, 6, 4, 1 };
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void vision_pyr_down(uint3 id : SV_DispatchThreadID, uniform uint *image,
                     uniform uint4 *levels, uniform uint level) {
  uint4 source = levels[level - 1];
  uint4 target = levels[level];
  if (id.x >= target.x * target.y) return;
  int2 position = int2(id.x % target.x, id.x / target.x) * 2;
  int sum = 0;
  for (int y = 0; y < 5; ++y)
    for (int x = 0; x < 5; ++x)
      sum += VISION_GAUSSIAN[x] * VISION_GAUSSIAN[y] *
             vision_pixel(image, source, position + int2(x - 2, y - 2));
  image[target.z + id.x] = uint((sum + 128) >> 8);
}

// ScharrDerivInvoker_SIMD, including reflected image borders; the sampler above
// separately supplies the original zero-padded derivative border to LK.
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void vision_scharr(uint3 id : SV_DispatchThreadID, uniform uint *image,
                    uniform int2 *derivatives, uniform uint4 *levels,
                    uniform uint level) {
  uint4 size = levels[level];
  if (id.x >= size.x * size.y) return;
  int2 p = int2(id.x % size.x, id.x / size.x);
  int dx = 3 * (vision_pixel(image, size, p + int2(1, -1)) +
                vision_pixel(image, size, p + int2(1, 1)) -
                vision_pixel(image, size, p + int2(-1, -1)) -
                vision_pixel(image, size, p + int2(-1, 1))) +
           10 * (vision_pixel(image, size, p + int2(1, 0)) -
                 vision_pixel(image, size, p + int2(-1, 0)));
  int dy = 3 * (vision_pixel(image, size, p + int2(-1, 1)) +
                vision_pixel(image, size, p + int2(1, 1)) -
                vision_pixel(image, size, p + int2(-1, -1)) -
                vision_pixel(image, size, p + int2(1, -1))) +
           10 * (vision_pixel(image, size, p + int2(0, 1)) -
                 vision_pixel(image, size, p + int2(0, -1)));
  derivatives[size.z + id.x] = int2(dx, dy);
}

// OpenCV contrib 5.1 cudaoptflow/src/cuda/pyrlk.cu:sparseKernel maps one
// workgroup to each feature and cooperatively loads its window. Preserve the
// scalar reference's row-major reductions while parallelizing memory access.
static const uint VISION_GROUP_SIZE = 64;
static const uint VISION_MAX_WINDOW = 20;
static const uint VISION_MAX_AREA = VISION_MAX_WINDOW * VISION_MAX_WINDOW;
groupshared int vision_patch[VISION_MAX_AREA];
groupshared int2 vision_gradients[VISION_MAX_AREA];
groupshared int vision_differences[VISION_MAX_AREA];
groupshared float2 vision_tracked;
groupshared float2 vision_next;
groupshared float2 vision_previous_delta;
groupshared float4 vision_covariance;
groupshared float vision_error;
groupshared uint vision_valid;
groupshared uint vision_usable;
groupshared uint vision_stop;

[shader("compute")][numthreads(VISION_GROUP_SIZE, 1, 1)]
void vision_sparse_lk(uint3 group : SV_GroupID, uint lane : SV_GroupIndex,
                      uniform uint *previous, uniform uint *current,
                      uniform int2 *derivatives, uniform uint4 *levels,
                      uniform float2 *points, uniform float4 *output,
                      uniform uint count, uniform uint level_count,
                      uniform uint window, uniform uint iterations,
                      uniform float epsilon_squared, uniform float minimum_eigenvalue) {
  uint index = group.x;
  if (index >= count || window > VISION_MAX_WINDOW) return;
  uint area = window * window;
  float half_window = float(window - 1) * 0.5f;
  float2 original = points[index];
  if (lane == 0) {
    vision_tracked = original;
    vision_valid = 1;
    vision_error = 0;
  }
  GroupMemoryBarrierWithGroupSync();
  for (int level_index = int(level_count) - 1; level_index >= 0; --level_index) {
    uint4 level = levels[level_index];
    float scale = 1.0f / float(1u << uint(level_index));
    VISION_PRECISE float2 old_point = original * scale;
    if (lane == 0)
      vision_tracked = uint(level_index) == level_count - 1 ? old_point : vision_tracked * 2.0f;
    old_point -= half_window;
    int2 old_position;
    int4 old_weights = vision_weights(old_point, old_position);
    GroupMemoryBarrierWithGroupSync();
    if (!vision_in_bounds(level, old_position, window)) {
      if (lane == 0 && level_index == 0) { vision_valid = 0; vision_error = 0; }
      continue;
    }
    for (uint pixel_index = lane; pixel_index < area; pixel_index += VISION_GROUP_SIZE) {
      int2 p = old_position + int2(pixel_index % window, pixel_index / window);
      vision_patch[pixel_index] = vision_interpolate(previous, level, p, old_weights);
      vision_gradients[pixel_index] =
          (vision_derivative(derivatives, level, p) * old_weights.x +
           vision_derivative(derivatives, level, p + int2(1, 0)) * old_weights.y +
           vision_derivative(derivatives, level, p + int2(0, 1)) * old_weights.z +
           vision_derivative(derivatives, level, p + int2(1, 1)) * old_weights.w +
           (1 << (VISION_WEIGHT_BITS - 1))) >> VISION_WEIGHT_BITS;
    }
    GroupMemoryBarrierWithGroupSync();
    if (lane == 0) {
      VISION_PRECISE float a11 = 0, a12 = 0, a22 = 0;
      for (uint pixel_index = 0; pixel_index < area; ++pixel_index) {
        int2 gradient = vision_gradients[pixel_index];
        a11 += float(gradient.x * gradient.x);
        a12 += float(gradient.x * gradient.y);
        a22 += float(gradient.y * gradient.y);
      }
      a11 *= VISION_COVARIANCE_SCALE;
      a12 *= VISION_COVARIANCE_SCALE;
      a22 *= VISION_COVARIANCE_SCALE;
      VISION_PRECISE float determinant = a11 * a22 - a12 * a12;
      VISION_PRECISE float minimum = (a22 + a11 - sqrt((a11 - a22) * (a11 - a22) + 4.0f * a12 * a12)) /
                                      float(2 * area);
      vision_usable = uint(minimum >= minimum_eigenvalue && determinant >= VISION_FLOAT_EPSILON);
      if (vision_usable == 0 && level_index == 0) vision_valid = 0;
      vision_covariance = float4(a11, a12, a22, 1.0f / determinant);
      vision_next = vision_tracked - half_window;
      vision_previous_delta = 0;
    }
    GroupMemoryBarrierWithGroupSync();
    if (vision_usable == 0) continue;
    for (uint iteration = 0; iteration < iterations; ++iteration) {
      int2 next_position;
      int4 next_weights = vision_weights(vision_next, next_position);
      if (!vision_in_bounds(level, next_position, window)) {
        if (lane == 0 && level_index == 0) vision_valid = 0;
        break;
      }
      for (uint pixel_index = lane; pixel_index < area; pixel_index += VISION_GROUP_SIZE) {
        int2 p = next_position + int2(pixel_index % window, pixel_index / window);
        vision_differences[pixel_index] = vision_interpolate(current, level, p, next_weights) - vision_patch[pixel_index];
      }
      GroupMemoryBarrierWithGroupSync();
      if (lane == 0) {
        VISION_PRECISE float b1 = 0, b2 = 0;
        for (uint pixel_index = 0; pixel_index < area; ++pixel_index) {
          int difference = vision_differences[pixel_index];
          int2 gradient = vision_gradients[pixel_index];
          b1 += float(difference * gradient.x);
          b2 += float(difference * gradient.y);
        }
        b1 *= VISION_COVARIANCE_SCALE;
        b2 *= VISION_COVARIANCE_SCALE;
        float4 a = vision_covariance;
        VISION_PRECISE float2 delta = float2((a.y * b2 - a.z * b1) * a.w,
                                           (a.y * b1 - a.x * b2) * a.w);
        vision_next += delta;
        vision_tracked = vision_next + half_window;
        vision_stop = uint(dot(delta, delta) <= epsilon_squared);
        if (vision_stop == 0 && iteration > 0 && all(abs(delta + vision_previous_delta) < 0.01f)) {
          vision_tracked -= delta * 0.5f;
          vision_stop = 1;
        }
        vision_previous_delta = delta;
      }
      GroupMemoryBarrierWithGroupSync();
      if (vision_stop != 0) break;
    }
    // All lanes observe the same point/status before deciding whether to compute error.
    GroupMemoryBarrierWithGroupSync();
    if (vision_valid != 0 && level_index == 0) {
      int2 next_position;
      int4 next_weights = vision_weights(vision_tracked - half_window, next_position);
      if (!vision_in_bounds(level, next_position, window)) {
        if (lane == 0) vision_valid = 0;
        continue;
      }
      for (uint pixel_index = lane; pixel_index < area; pixel_index += VISION_GROUP_SIZE) {
        int2 p = next_position + int2(pixel_index % window, pixel_index / window);
        vision_differences[pixel_index] = abs(vision_interpolate(current, level, p, next_weights) - vision_patch[pixel_index]);
      }
      GroupMemoryBarrierWithGroupSync();
      if (lane == 0) {
        float error = 0;
        for (uint pixel_index = 0; pixel_index < area; ++pixel_index)
          error += float(vision_differences[pixel_index]);
        vision_error = error / float((1 << VISION_INTENSITY_BITS) * area);
      }
    }
    GroupMemoryBarrierWithGroupSync();
  }
  if (lane == 0) output[index] = float4(vision_tracked, vision_error, float(vision_valid));
}

#undef VISION_PRECISE
