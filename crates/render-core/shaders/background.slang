import background_common;

[shader("compute")]
[numthreads(16, 16, 1)]
void render_background(uint3 id : SV_DispatchThreadID,
                       uniform uint* output,
                       uniform BackgroundUniforms uniforms) {
    if (id.x >= uniforms.common.width || id.y >= uniforms.common.height) return;
    output[id.y * uniforms.common.width + id.x] = packRgba(backgroundColor(uniforms, id.xy));
}
