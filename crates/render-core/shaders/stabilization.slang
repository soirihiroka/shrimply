import abi;
import color;
import common;

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void affine_stabilization(uniform AffineStabilizationParams params,
                                       uniform uint *out,
                                       uniform uint64_t out_len,
                                       uint3 id : SV_DispatchThreadID) {
  uint i = linear_index(id);
  if (i >= out_len)
    return;
  float2 source = transform_point2(
      params.source_transform,
      float2(float(i % params.width), float(i / params.width)));
  if (source.x < 0.0f || source.y < 0.0f ||
      source.x > float(params.width > 0u ? params.width - 1u : 0u) ||
      source.y > float(params.height > 0u ? params.height - 1u : 0u)) {
    out[i] = 0u;
    return;
  }
  uint *input = (uint *)params.input;
  out[i] = sample_bilinear_rgba(input, params.width, params.height, source.x,
                                source.y);
}
