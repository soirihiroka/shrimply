import abi;
import background;
import background_common;

struct BackgroundArguments {
  uint* output;
  BackgroundUniforms uniforms;
};
[[vk::binding(0, 0)]] ConstantBuffer<BackgroundArguments> arguments;

[shader("compute")][numthreads(16, 16, 1)]
void render_background_vulkan(uint3 id : SV_DispatchThreadID) {
  write_background_pixel(id, arguments.output, arguments.uniforms);
}
