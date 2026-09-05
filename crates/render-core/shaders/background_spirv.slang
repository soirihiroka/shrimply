import background_common;

[[vk::binding(0, 0)]] RWStructuredBuffer<uint> outputPixels;
[[vk::binding(1, 0)]] ConstantBuffer<BackgroundUniforms> uniforms;

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x >= uniforms.common.width || id.y >= uniforms.common.height) return;
    outputPixels[id.y * uniforms.common.width + id.x] = packRgba(backgroundColor(uniforms, id.xy));
}
