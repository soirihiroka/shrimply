import abi;
import color;

static const uint ACTIVATION_POSITIVE = 1u, ACTIVATION_NEGATIVE = 2u;
float4 image_load(ImageDescriptor image, int x, int y) {
  x = clamp(x, 0, int(image.width) - 1);
  y = clamp(y, 0, int(image.height) - 1);
  return ((float4 *)image.pixels)[uint(y) * image.width + uint(x)];
}
float4 image_sample(ImageDescriptor image, uint x, uint y, uint output_width,
                    uint output_height, float offset_x, float offset_y) {
  float sx = (float(x) + 0.5f) * float(image.width) / float(output_width) -
             0.5f + offset_x,
        sy = (float(y) + 0.5f) * float(image.height) / float(output_height) -
             0.5f + offset_y;
  int x0 = int(floor(sx)), y0 = int(floor(sy));
  float fx = sx - float(x0), fy = sy - float(y0);
  return lerp(
      lerp(image_load(image, x0, y0), image_load(image, x0 + 1, y0), fx),
      lerp(image_load(image, x0, y0 + 1), image_load(image, x0 + 1, y0 + 1),
           fx),
      fy);
}
float2 chroma_load(uint8_t *source, uint64_t pitch, uint width, uint height,
                   int x, int y) {
  x = clamp(x, 0, int(width) - 1);
  y = clamp(y, 0, int(height) - 1);
  uint64_t address = uint64_t(y) * pitch + uint64_t(x) * 2u;
  return float2(float(source[address]) / 255.0f - 0.5f,
                float(source[address + 1u]) / 255.0f - 0.5f);
}
float2 chroma_sample(uint8_t *source, uint64_t pitch, uint width, uint height,
                     float x, float y) {
  int x0 = int(floor(x)), y0 = int(floor(y));
  float fx = x - float(x0), fy = y - float(y0);
  return lerp(lerp(chroma_load(source, pitch, width, height, x0, y0),
                   chroma_load(source, pitch, width, height, x0 + 1, y0), fx),
              lerp(chroma_load(source, pitch, width, height, x0, y0 + 1),
                   chroma_load(source, pitch, width, height, x0 + 1, y0 + 1),
                   fx),
              fy);
}
float alpha_load(uint8_t *source, uint64_t pitch, uint width, uint height,
                 int x, int y) {
  x = clamp(x, 0, int(width) - 1);
  y = clamp(y, 0, int(height) - 1);
  return float(source[uint64_t(y) * pitch + uint64_t(x) * 4u + 3u]) / 255.0f;
}
float alpha_sample(uint8_t *source, uint64_t pitch, uint width, uint height,
                   float x, float y) {
  int x0 = int(floor(x)), y0 = int(floor(y));
  float fx = x - float(x0), fy = y - float(y0);
  return lerp(lerp(alpha_load(source, pitch, width, height, x0, y0),
                   alpha_load(source, pitch, width, height, x0 + 1, y0), fx),
              lerp(alpha_load(source, pitch, width, height, x0, y0 + 1),
                   alpha_load(source, pitch, width, height, x0 + 1, y0 + 1),
                   fx),
              fy);
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void rgba_to_float(uint3 id : SV_DispatchThreadID,
                                uniform uint8_t *source, uniform uint64_t pitch,
                                uniform uint width, uniform float4 *output,
                                uniform uint64_t output_count) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % width, y = i / width;
  uint64_t address = uint64_t(y) * pitch + uint64_t(x) * 4u;
  uint pixel = uint(source[address]) | (uint(source[address + 1u]) << 8) |
               (uint(source[address + 2u]) << 16) |
               (uint(source[address + 3u]) << 24);
  Color c = color_premultiply(color_from_rgba(pixel));
  output[i] = float4(c.r, c.g, c.b, c.a);
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void nv12_to_float(uint3 id : SV_DispatchThreadID,
                                 uniform uint8_t *y_plane,
                                 uniform uint8_t *uv_plane,
                                 uniform uint64_t y_pitch,
                                 uniform uint64_t uv_pitch, uniform uint width,
                                 uniform uint height, uniform float4 *output,
                                 uniform uint64_t output_count) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % width, y = i / width;
  float luma = float(y_plane[uint64_t(y) * y_pitch + x]) / 255.0f;
  float2 chroma =
      chroma_sample(uv_plane, uv_pitch, (width + 1u) / 2u, (height + 1u) / 2u,
                    (float(x) - 0.5f) * 0.5f, (float(y) - 0.5f) * 0.5f);
  float yy = max(luma - 0.0625f, 0.0f) * 1.1643835f;
  output[i] = float4(
      clamp(yy + 1.7927411f * chroma.y, 0.0f, 1.0f),
      clamp(yy - 0.21324861f * chroma.x - 0.53290933f * chroma.y, 0.0f, 1.0f),
      clamp(yy + 2.1124017f * chroma.x, 0.0f, 1.0f), 1.0f);
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
convolution(uint3 id : SV_DispatchThreadID, uniform ConvolutionParams params,
            uniform ConvolutionTerm *terms, uniform uint64_t term_count,
            uniform float4 *output, uniform uint64_t output_count) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % params.width, y = i / params.width;
  float4 result =
      float4(params.bias[0], params.bias[1], params.bias[2], params.bias[3]);
  for (uint64_t t = 0; t < term_count; ++t) {
    ConvolutionTerm term = terms[t];
    float4 value = image_sample(params.images[term.input], x, y, params.width,
                                params.height, term.offset_x, term.offset_y);
    if (term.activation == ACTIVATION_POSITIVE)
      value = max(value, float4(0.0f));
    else if (term.activation == ACTIVATION_NEGATIVE)
      value = max(-value, float4(0.0f));
    for (uint row = 0u; row < 4u; ++row)
      result[row] +=
          term.weights[row] * value.x + term.weights[4u + row] * value.y +
          term.weights[8u + row] * value.z + term.weights[12u + row] * value.w;
  }
  result *= params.result_scale;
  if (params.residual.pixels != nullptr)
    result += image_sample(params.residual, x, y, params.width, params.height,
                           0.0f, 0.0f) *
              params.residual_scale;
  output[i] = result;
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void depth_to_space_x2(uint3 id : SV_DispatchThreadID,
                                     uniform ImageDescriptor convolution,
                                     uniform ImageDescriptor residual,
                                     uniform uint width, uniform float4 *output,
                                     uniform uint64_t output_count) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint height = uint(output_count) / width, x = i % width, y = i / width,
       source_x = x / 2u, source_y = y / 2u, channel = (y % 2u) * 2u + x % 2u;
  float value = ((float4 *)convolution
                     .pixels)[source_y * convolution.width + source_x][channel];
  output[i] = value + image_sample(residual, x, y, width, height, 0.0f, 0.0f);
}
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void float_to_rgba_opaque(uint3 id : SV_DispatchThreadID,
                                       uniform float4 *source,
                                       uniform uint *output,
                                       uniform uint64_t output_count) {
  uint i = id.x;
  if (i < output_count)
    output[i] =
        color_to_rgba(color(source[i].x, source[i].y, source[i].z, 1.0f));
}[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void float_to_rgba_alpha(uint3 id : SV_DispatchThreadID,
                                       uniform AlphaParams params,
                                       uniform uint *output,
                                       uniform uint64_t output_count) {
  uint i = id.x;
  if (i >= output_count)
    return;
  uint x = i % params.width, y = i / params.width;
  float4 value = ((float4 *)params.source)[i];
  float alpha = alpha_sample(
      (uint8_t *)params.alpha_source, params.alpha_pitch, params.alpha_width,
      params.alpha_height,
      (float(x) + 0.5f) * float(params.alpha_width) / float(params.width) -
          0.5f,
      (float(y) + 0.5f) * float(params.alpha_height) / float(params.height) -
          0.5f);
  output[i] = color_to_rgba(color_unpremultiply(
      color(clamp(value.x, 0.0f, alpha), clamp(value.y, 0.0f, alpha),
            clamp(value.z, 0.0f, alpha), alpha)));
}
