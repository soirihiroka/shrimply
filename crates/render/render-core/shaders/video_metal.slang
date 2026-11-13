import abi;
import color;

float2 video_texture_chroma(Texture2D<float2> plane, uint2 size, int2 position,
                            bool p010) {
  float2 value = plane.Load(int3(clamp(position, int2(0), int2(size) - 1), 0));
  return p010 ? floor(value * 65535.0f / 256.0f) / 255.0f : value;
}

// Read decoder-owned Core Video textures directly; no CPU pixels or staging planes.
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void video_texture_to_rgba(uint3 id : SV_DispatchThreadID,
                          uniform Texture2D<float> y_texture,
                          uniform Texture2D<float2> uv_texture,
                          uniform uint* output, uniform uint width,
                          uniform uint height, uniform int4 coefficients,
                          uniform bool full_range, uniform bool p010,
                          uniform float2 chroma_offset) {
  uint i = id.x;
  if (i >= width * height) return;
  uint2 pixel = uint2(i % width, i / width);
  uint2 chroma_size = uint2((width + 1u) / 2u, (height + 1u) / 2u);
  float2 position = (float2(pixel) - chroma_offset) * 0.5f;
  int2 low = int2(floor(position));
  float2 amount = frac(position);
  float2 chroma = lerp(
      lerp(video_texture_chroma(uv_texture, chroma_size, low, p010),
           video_texture_chroma(uv_texture, chroma_size, low + int2(1, 0), p010), amount.x),
      lerp(video_texture_chroma(uv_texture, chroma_size, low + int2(0, 1), p010),
           video_texture_chroma(uv_texture, chroma_size, low + 1, p010), amount.x), amount.y);
  float luma = y_texture.Load(int3(pixel, 0));
  if (p010) luma = floor(luma * 65535.0f / 256.0f) / 255.0f;
  output[i] = color_to_rgba(from_ycbcr(luma, chroma, coefficients, full_range));
}
