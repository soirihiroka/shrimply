import abi;
import color;

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void nv12_to_rgba(uint3 id : SV_DispatchThreadID,
                  uniform uint8_t *y_plane, uniform uint8_t *uv_plane,
                  uniform uint64_t y_pitch, uniform uint64_t uv_pitch,
                  uniform uint width, uniform uint height, uniform uint *output) {
  uint i = id.x;
  if (i >= width * height)
    return;
  uint x = i % width, y = i / width;
  uint64_t chroma = uint64_t(y / 2u) * uv_pitch + uint64_t(x / 2u) * 2u;
  output[i] = color_to_rgba(from_bt709_ycbcr(
      float(y_plane[uint64_t(y) * y_pitch + x]) / 255.0f,
      float(uv_plane[chroma]) / 255.0f - 0.5f,
      float(uv_plane[chroma + 1u]) / 255.0f - 0.5f, 1.0f));
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
