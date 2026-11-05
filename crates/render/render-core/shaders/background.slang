import background_common;

void write_background_pixel(uint3 id, uint* output, BackgroundUniforms uniforms) {
    if (id.x >= uniforms.common.width || id.y >= uniforms.common.height) return;
    output[id.y * uniforms.common.width + id.x] = packRgba(backgroundColor(uniforms, id.xy));
}

[shader("compute")]
[numthreads(16, 16, 1)]
void render_background(uint3 id : SV_DispatchThreadID,
                       uniform uint* output,
                       uniform BackgroundUniforms uniforms) {
    write_background_pixel(id, output, uniforms);
}
