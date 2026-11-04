import abi;
import color;

float2 video_chroma(uint8_t *plane, uint64_t pitch, uint width, uint height,
                    int x, int y) {
  uint64_t offset = uint64_t(clamp(y, 0, int((height + 1u) / 2u) - 1)) * pitch +
                    uint64_t(clamp(x, 0, int((width + 1u) / 2u) - 1)) * 2u;
  return float2(plane[offset], plane[offset + 1u]) / 255.0f;
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void nv12_to_rgba(uint3 id : SV_DispatchThreadID,
                  uniform uint8_t *y_plane, uniform uint8_t *uv_plane,
                  uniform uint64_t y_pitch, uniform uint64_t uv_pitch,
                  uniform uint width, uniform uint height, uniform uint *output,
                  uniform int4 coefficients, uniform bool full_range,
                  uniform float2 chroma_offset) {
  uint i = id.x;
  if (i >= width * height)
    return;
  uint x = i % width, y = i / width;
  float2 position = (float2(x, y) - chroma_offset) * 0.5f;
  int2 low = int2(floor(position));
  float2 amount = position - float2(low);
  float2 chroma = lerp(
      lerp(video_chroma(uv_plane, uv_pitch, width, height, low.x, low.y),
           video_chroma(uv_plane, uv_pitch, width, height, low.x + 1, low.y), amount.x),
      lerp(video_chroma(uv_plane, uv_pitch, width, height, low.x, low.y + 1),
           video_chroma(uv_plane, uv_pitch, width, height, low.x + 1, low.y + 1), amount.x),
      amount.y);
  output[i] = color_to_rgba(from_ycbcr(
      float(y_plane[uint64_t(y) * y_pitch + x]) / 255.0f,
      chroma, coefficients, full_range));
}

// The current compositor produces eight-bit RGBA. Reduce P010 only on the GPU,
// retaining its original limited/full range coding for the shared YUV sampler.
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void p010_to_nv12(uint3 id : SV_DispatchThreadID,
                  uniform uint16_t *input, uniform uint8_t *output,
                  uniform uint64_t count, uniform uint64_t y_count,
                  uniform uint64_t uv_offset) {
  if (id.x < count)
    output[id.x] = uint8_t(input[id.x < y_count ? id.x : uv_offset + id.x - y_count] >> 8u);
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void unpremultiply_rgba(uint3 id : SV_DispatchThreadID,
                        uniform uint *input, uniform uint *output,
                        uniform uint64_t count) {
  if (id.x < count)
    output[id.x] = color_to_rgba(color_unpremultiply(color_from_rgba(input[id.x])));
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void mesh_flow_warp(uint3 id : SV_DispatchThreadID,
                    uniform uint *input, uniform float2 *offsets,
                    uniform uint *output, uniform uint width,
                    uniform uint height, uniform uint grid_width,
                    uniform uint grid_height) {
  uint index = id.x;
  if (index >= width * height)
    return;
  float2 position = float2(index % width, index / width);
  float2 grid = position * float2(grid_width - 1, grid_height - 1) /
                float2(max(width - 1, 1), max(height - 1, 1));
  uint2 low = uint2(floor(grid));
  uint2 high = min(low + 1, uint2(grid_width - 1, grid_height - 1));
  float2 amount = grid - float2(low);
  float2 top = lerp(offsets[low.y * grid_width + low.x],
                    offsets[low.y * grid_width + high.x], amount.x);
  float2 bottom = lerp(offsets[high.y * grid_width + low.x],
                       offsets[high.y * grid_width + high.x], amount.x);
  float2 source = position + lerp(top, bottom, amount.y);
  output[index] = source.x < 0 || source.y < 0 ||
                  source.x > width - 1 || source.y > height - 1
                      ? 0 : sample_bilinear_rgba(input, width, height, source.x, source.y);
}
