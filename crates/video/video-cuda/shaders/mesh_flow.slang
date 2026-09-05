struct MeshFlowUniforms
{
    uint width;
    uint height;
    uint grid_width;
    uint grid_height;
};

[[vk::binding(0, 0)]] StructuredBuffer<uint> input_pixels;
[[vk::binding(1, 0)]] StructuredBuffer<float2> source_offsets;
[[vk::binding(2, 0)]] RWStructuredBuffer<uint> output_pixels;
[[vk::binding(3, 0)]] ConstantBuffer<MeshFlowUniforms> uniforms;

float4 unpack_rgba(uint pixel)
{
    return float4(
        pixel & 0xff,
        (pixel >> 8) & 0xff,
        (pixel >> 16) & 0xff,
        (pixel >> 24) & 0xff) / 255.0;
}

uint pack_rgba(float4 color)
{
    uint4 bytes = uint4(round(saturate(color) * 255.0));
    return bytes.x | (bytes.y << 8) | (bytes.z << 16) | (bytes.w << 24);
}

float2 mesh_offset(float2 position)
{
    float2 image_extent = float2(max(uniforms.width - 1, 1), max(uniforms.height - 1, 1));
    float2 grid_extent = float2(uniforms.grid_width - 1, uniforms.grid_height - 1);
    float2 grid_position = position * grid_extent / image_extent;
    uint2 low = uint2(floor(grid_position));
    uint2 high = min(low + 1, uint2(uniforms.grid_width - 1, uniforms.grid_height - 1));
    float2 amount = grid_position - float2(low);
    float2 top_left = source_offsets[low.y * uniforms.grid_width + low.x];
    float2 top_right = source_offsets[low.y * uniforms.grid_width + high.x];
    float2 bottom_left = source_offsets[high.y * uniforms.grid_width + low.x];
    float2 bottom_right = source_offsets[high.y * uniforms.grid_width + high.x];
    return lerp(lerp(top_left, top_right, amount.x), lerp(bottom_left, bottom_right, amount.x), amount.y);
}

float4 sample_bilinear(float2 position)
{
    uint2 low = uint2(floor(position));
    uint2 high = min(low + 1, uint2(uniforms.width - 1, uniforms.height - 1));
    float2 amount = position - float2(low);
    float4 top_left = unpack_rgba(input_pixels[low.y * uniforms.width + low.x]);
    float4 top_right = unpack_rgba(input_pixels[low.y * uniforms.width + high.x]);
    float4 bottom_left = unpack_rgba(input_pixels[high.y * uniforms.width + low.x]);
    float4 bottom_right = unpack_rgba(input_pixels[high.y * uniforms.width + high.x]);
    top_left.rgb *= top_left.a;
    top_right.rgb *= top_right.a;
    bottom_left.rgb *= bottom_left.a;
    bottom_right.rgb *= bottom_right.a;
    float4 color = lerp(
        lerp(top_left, top_right, amount.x),
        lerp(bottom_left, bottom_right, amount.x),
        amount.y);
    if (color.a <= 0.0)
        return float4(0.0);
    color.rgb /= color.a;
    return color;
}

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 dispatch_thread_id : SV_DispatchThreadID)
{
    uint2 pixel = dispatch_thread_id.xy;
    if (pixel.x >= uniforms.width || pixel.y >= uniforms.height)
        return;
    uint index = pixel.y * uniforms.width + pixel.x;
    float2 source = float2(pixel) + mesh_offset(float2(pixel));
    if (source.x < 0.0 || source.y < 0.0
        || source.x > uniforms.width - 1 || source.y > uniforms.height - 1)
    {
        output_pixels[index] = 0;
        return;
    }
    output_pixels[index] = pack_rgba(sample_bilinear(source));
}
