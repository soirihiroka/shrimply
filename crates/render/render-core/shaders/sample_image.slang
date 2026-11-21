import abi;
import color;

static const uint IMAGE_GROUP_SIZE = 16u;
[[vk::binding(0, 0)]] Sampler2D<float4> sampled_image;

// Native image imports supply the sampler's format conversion. Compositing,
// filtering and effects then consume the same packed RGBA buffers as other inputs.
[shader("compute")][numthreads(IMAGE_GROUP_SIZE, IMAGE_GROUP_SIZE, 1)]
void sample_image_to_rgba(uint3 id : SV_DispatchThreadID,
                          uniform uint *output,
                          uniform uint width, uniform uint height,
                          uniform float4 crop, uniform float2 source_size) {
  if (id.x >= width || id.y >= height)
    return;
  float2 position = (float2(id.xy) + 0.5f) / float2(width, height);
  float2 coordinate = (crop.xy + position * crop.zw) / source_size;
  float4 value = sampled_image.SampleLevel(coordinate, 0.0f);
  output[id.y * width + id.x] = color_to_rgba(color(value.x, value.y, value.z, value.w));
}
