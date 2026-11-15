import abi;
import color;

Color load_pixel(uint *rgba, uint width, uint x, uint y) {
  if (buffer_is_null(rgba))
    return color(0.0f, 0.0f, 0.0f, 1.0f);
  return color_from_rgba(rgba[y * width + x]);
}

Color average_block(uint *rgba, uint width, uint height, uint cx, uint cy) {
  Color result = color(0.0f, 0.0f, 0.0f, 0.0f);
  float count = 0.0f;
  for (uint y = cy * 2u; y < min(cy * 2u + 2u, height); ++y)
    for (uint x = cx * 2u; x < min(cx * 2u + 2u, width); ++x) {
      Color p = load_pixel(rgba, width, x, y);
      result.r += p.r;
      result.g += p.g;
      result.b += p.b;
      count += 1.0f;
    }
  return color(result.r / count, result.g / count, result.b / count, 1.0f);
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
rgba_to_nv12_luma(uint3 id : SV_DispatchThreadID, uniform uint *rgba,
                  uniform uint width, uniform uint height,
                  uniform ByteStorage *y_plane, uniform BufferIndex y_pitch) {
  uint i = id.x;
  if (i >= width * height)
    return;
  store_byte(y_plane, BufferIndex(i / width) * y_pitch + i % width,
      to_bt709_ycbcr(load_pixel(rgba, width, i % width, i / width)).x);
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void rgba_to_nv12_chroma(uint3 id : SV_DispatchThreadID,
                                          uniform uint *rgba,
                                          uniform uint width,
                                          uniform uint height,
                                          uniform ByteStorage *uv_plane,
                                          uniform BufferIndex uv_pitch) {
  uint cw = max((width + 1u) / 2u, 1u), ch = max((height + 1u) / 2u, 1u),
       i = id.x;
  if (i >= cw * ch)
    return;
  uint cx = i % cw, cy = i / cw;
  uint3 value = to_bt709_ycbcr(average_block(rgba, width, height, cx, cy));
  BufferIndex offset = BufferIndex(cy) * uv_pitch + BufferIndex(cx) * 2u;
  store_byte(uv_plane, offset, value.y);
  store_byte(uv_plane, offset + 1u, value.z);
}

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void
rgba_to_p010_luma(uint3 id : SV_DispatchThreadID, uniform uint *rgba,
                  uniform uint width, uniform uint height,
                  uniform HalfStorage *y_plane, uniform BufferIndex y_pitch) {
  uint i = id.x;
  if (i >= width * height)
    return;
  uint x = i % width, y = i / width;
  store_half(y_plane, BufferIndex(y) * (y_pitch / 2u) + x,
      to_bt709_ycbcr(load_pixel(rgba, width, x, y)).x << 8u);
}

    [shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)] void rgba_to_p010_chroma(uint3 id : SV_DispatchThreadID,
                                          uniform uint *rgba,
                                          uniform uint width,
                                          uniform uint height,
                                          uniform HalfStorage *uv_plane,
                                          uniform BufferIndex uv_pitch) {
  uint cw = max((width + 1u) / 2u, 1u), ch = max((height + 1u) / 2u, 1u),
       i = id.x;
  if (i >= cw * ch)
    return;
  uint cx = i % cw, cy = i / cw;
  uint3 value = to_bt709_ycbcr(average_block(rgba, width, height, cx, cy));
  BufferIndex offset = BufferIndex(cy) * (uv_pitch / 2u) + BufferIndex(cx) * 2u;
  store_half(uv_plane, offset, value.y << 8u);
  store_half(uv_plane, offset + 1u, value.z << 8u);
}
